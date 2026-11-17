use std::collections::{HashMap, HashSet};

use anotau_protocol::{Command, Destination, Draft, EntryKind, Id, Outcome, Resolution};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use crate::api::{Failure, Result};

mod read;
#[cfg(test)]
mod tests;
pub use read::{note, snapshot, snapshot_at};

const WELCOME: &str = include_str!("../../assets/templates/bienvenida.md");
const MAX_COPY_ENTRIES: usize = 2048;
const MAX_COPY_BYTES: usize = 16 * 1024 * 1024;

const ENTRY: &str = "
    SELECT e.id, e.workspace_id, e.parent_id, e.title, e.markdown, e.revision
    FROM entries e JOIN workspaces w ON w.id = e.workspace_id
    WHERE e.id = ?1 AND w.account_id = ?2";
const ENTRY_IN_WORKSPACE: &str = "
    SELECT id, workspace_id, parent_id, title, markdown, revision
    FROM entries WHERE id = ?1 AND workspace_id = ?2";
const INSERT_ENTRY: &str = "
    INSERT INTO entries(workspace_id, parent_id, kind, title, markdown)
    VALUES (?1, ?2, ?3, ?4, ?5)";
const FOLDER_EXISTS: &str = "
    SELECT EXISTS(SELECT 1 FROM entries WHERE id = ?1 AND workspace_id = ?2 AND kind = 'folder')";
// recorre en anchura y lee solo tamaños, sin materializar el Markdown de origen.
const COPY_TREE: &str = "
    WITH RECURSIVE tree(id, depth) AS (
        SELECT id, 0 FROM entries
        WHERE workspace_id = ?1 AND ((?2 IS NULL AND parent_id IS NULL) OR id = ?2)
        UNION ALL
        SELECT e.id, t.depth + 1 FROM entries e JOIN tree t ON e.parent_id = t.id
        WHERE e.workspace_id = ?1
        LIMIT 2049
    )
    SELECT e.id, coalesce(length(CAST(e.markdown AS BLOB)), 0)
    FROM tree JOIN entries e ON e.id = tree.id
    ORDER BY tree.depth, e.id LIMIT 2049";
const DESTINATION_INSIDE_SOURCE: &str = "
    WITH RECURSIVE ancestors(id) AS (
        SELECT ?1
        UNION ALL
        SELECT e.parent_id FROM entries e JOIN ancestors a ON e.id = a.id
        WHERE e.parent_id IS NOT NULL
    )
    SELECT EXISTS(SELECT 1 FROM ancestors WHERE id = ?2)";
const HAS_CHILDREN: &str = "SELECT EXISTS(SELECT 1 FROM entries WHERE parent_id = ?1)";
const ENTRY_NAME_TAKEN: &str = "
    SELECT EXISTS(SELECT 1 FROM entries
    WHERE workspace_id = ?1 AND parent_id IS ?2 AND title = ?3 AND id != ?4)";
const WORKSPACE_NAME_TAKEN: &str = "
    SELECT EXISTS(SELECT 1 FROM workspaces WHERE account_id = ?1 AND title = ?2 AND id != ?3)";
const CONTACT_STATE: &str = "
    SELECT state FROM contacts
    WHERE min(sender_id, recipient_id) = min(?1, ?2) AND max(sender_id, recipient_id) = max(?1, ?2)";
const RECEIPT: &str = "
    SELECT request_hash, result FROM operation_receipts
    WHERE account_id = ?1 AND operation_id = ?2";
const INSERT_RECEIPT: &str = "
    INSERT INTO operation_receipts(account_id, operation_id, request_hash, result)
    VALUES (?1, ?2, ?3, ?4)";
const RENAME_WORKSPACE: &str = "
    UPDATE workspaces SET title = ?1, revision = revision + 1, updated_at = unixepoch()
    WHERE id = ?2";
const RENAME_ENTRY: &str = "
    UPDATE entries SET title = ?1, revision = revision + 1, updated_at = unixepoch()
    WHERE id = ?2";
const MOVE_ENTRY: &str = "
    UPDATE entries SET parent_id = ?1, revision = revision + 1, updated_at = unixepoch()
    WHERE id = ?2";
const SAVE_NOTE: &str = "
    UPDATE entries SET markdown = ?1, revision = revision + 1, updated_at = unixepoch()
    WHERE id = ?2 RETURNING revision, updated_at";
const RESOLVE_CONTACT: &str = "
    UPDATE contacts SET state = ?1, resolved_at = unixepoch()
    WHERE id = ?2 AND recipient_id = ?3";
const INSERT_OFFER: &str = "
    INSERT INTO transfers(sender_id, recipient_id, source_note_id, source_revision, title, markdown)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
const TRANSFER: &str = "
    SELECT state, title, markdown, delivered_note_id FROM transfers
    WHERE id = ?1 AND recipient_id = ?2";
const DELIVER_TRANSFER: &str = "
    UPDATE transfers SET state = 'accepted', delivered_note_id = ?1, resolved_at = unixepoch()
    WHERE id = ?2";
const UPDATE_PREFERENCES: &str = "
    UPDATE account_preferences SET settings = ?1, revision = revision + 1, updated_at = unixepoch()
    WHERE account_id = ?2";

pub fn register(connection: &mut Connection, username: &str, hash: &str) -> Result<Id> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        "INSERT INTO accounts(username, password_hash) VALUES (?1, ?2)",
        params![username, hash],
    )?;
    let account = transaction.last_insert_rowid();
    transaction.execute(
        "INSERT INTO account_preferences(account_id) VALUES (?1)",
        [account],
    )?;
    transaction.execute(
        "INSERT INTO workspaces(account_id, title) VALUES (?1, 'Personal')",
        [account],
    )?;
    let workspace = transaction.last_insert_rowid();
    transaction.execute(
        INSERT_ENTRY,
        params![workspace, None::<Id>, "note", "Bienvenida", WELCOME],
    )?;
    transaction.commit()?;
    Ok(account)
}

/// Elimina la cuenta y todo lo que la nombra: sesiones, preferencias, espacios con su árbol,
/// contactos, solicitudes de copia enviadas y recibidas, comprobantes y sugerencias firmadas.
///
/// Las copias que otros aceptaron ya pertenecen a sus espacios y se conservan; las sugerencias
/// anónimas no guardan la cuenta y tampoco se tocan. El nombre queda libre para un registro nuevo.
pub fn delete_account(connection: &mut Connection, owner: Id) -> Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // el árbol se borra en una sola sentencia: SQLite comprueba sus claves al terminarla
    for statement in [
        "DELETE FROM transfers WHERE sender_id = ?1 OR recipient_id = ?1",
        "DELETE FROM contacts WHERE sender_id = ?1 OR recipient_id = ?1",
        "DELETE FROM operation_receipts WHERE account_id = ?1",
        "DELETE FROM suggestions WHERE account_id = ?1",
        "DELETE FROM recovery_tickets WHERE account_id = ?1",
        "DELETE FROM sessions WHERE account_id = ?1",
        "DELETE FROM entries WHERE workspace_id IN (SELECT id FROM workspaces WHERE account_id = ?1)",
        "DELETE FROM workspaces WHERE account_id = ?1",
        "DELETE FROM account_preferences WHERE account_id = ?1",
        "DELETE FROM accounts WHERE id = ?1",
    ] {
        transaction.execute(statement, [owner])?;
    }
    transaction.commit()?;
    Ok(())
}

fn title(value: &str) -> Result<&str> {
    let value = value.trim();
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(Failure::invalid(
            "Usa un nombre no vacío, sin controles y de hasta 256 bytes.",
        ));
    }
    Ok(value)
}

fn markdown(value: &str) -> Result<()> {
    if value.chars().any(anotau_protocol::is_c1) {
        return Err(Failure::invalid(
            "El Markdown contiene controles C1 no admitidos.",
        ));
    }
    if value.len() > anotau_protocol::MAX_NOTE_BYTES {
        return Err(Failure::invalid("La nota supera 128 KiB."));
    }
    Ok(())
}

/// Tras aplicar un comando que puede añadir notas, la cuenta no debe superar
/// [`anotau_protocol::MAX_NOTES`]; si lo hace, el error revierte la transacción completa.
fn within_note_quota(connection: &Connection, owner: Id, command: &Command) -> Result<()> {
    let adds_notes = match command {
        Command::CreateEntry { kind, .. } => *kind == EntryKind::Note,
        Command::CopyEntry { .. } | Command::CopyWorkspace { .. } => true,
        Command::ResolveTransfer { destination, .. } => destination.is_some(),
        _ => false,
    };
    if !adds_notes {
        return Ok(());
    }
    let notes: i64 = connection.query_row(
        "SELECT count(*) FROM entries e JOIN workspaces w ON w.id = e.workspace_id
         WHERE w.account_id = ?1 AND e.kind = 'note'",
        [owner],
        |row| row.get(0),
    )?;
    if usize::try_from(notes).unwrap_or(usize::MAX) > anotau_protocol::MAX_NOTES {
        return Err(Failure::invalid(
            "Llegaste al límite de 50 notas. Elimina alguna antes de crear, copiar o recibir otra.",
        ));
    }
    Ok(())
}

fn suggestion(value: &str) -> Result<&str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Failure::invalid("Escribe tu sugerencia antes de enviarla."));
    }
    if value.len() > anotau_protocol::MAX_SUGGESTION_BYTES {
        return Err(Failure::invalid("La sugerencia supera 4 KiB."));
    }
    if value.chars().any(|ch| ch.is_control() && ch != '\n') {
        return Err(Failure::invalid(
            "La sugerencia contiene caracteres de control no admitidos.",
        ));
    }
    Ok(value)
}

fn kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Note => "note",
        EntryKind::Folder => "folder",
    }
}

fn changed(workspace: Option<Id>, entry: Option<Id>) -> Outcome {
    Outcome::Changed { workspace, entry }
}

fn workspace(connection: &Connection, owner: Id, id: Id) -> Result<i64> {
    connection
        .query_row(
            "SELECT revision FROM workspaces WHERE id = ?1 AND account_id = ?2",
            params![id, owner],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(Failure::missing)
}

struct Record {
    id: Id,
    workspace: Id,
    parent: Option<Id>,
    title: String,
    markdown: Option<String>,
    revision: i64,
}

fn record(row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        id: row.get(0)?,
        workspace: row.get(1)?,
        parent: row.get(2)?,
        title: row.get(3)?,
        markdown: row.get(4)?,
        revision: row.get(5)?,
    })
}

fn entry(connection: &Connection, owner: Id, id: Id) -> Result<Record> {
    connection
        .query_row(ENTRY, params![id, owner], record)
        .optional()?
        .ok_or_else(Failure::missing)
}

/// Resultado de una consulta `SELECT EXISTS(…)`.
fn exists(connection: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<bool> {
    Ok(connection.query_row(sql, params, |row| row.get(0))?)
}

fn revision(actual: i64, expected: i64) -> Result<()> {
    if actual != expected {
        return Err(Failure::conflict(
            "La revisión cambió en otro dispositivo. Tus cambios se conservan.",
        ));
    }
    Ok(())
}

fn parent(connection: &Connection, workspace: Id, id: Option<Id>) -> Result<()> {
    if let Some(id) = id {
        let valid: bool =
            connection.query_row(FOLDER_EXISTS, params![id, workspace], |row| row.get(0))?;
        if !valid {
            return Err(Failure::missing());
        }
    }
    Ok(())
}

/// Primer nombre libre del destino: `título`, `título (2)`, `título (3)`…
fn available(
    connection: &Connection,
    owner: Id,
    destination: Option<Destination>,
    requested: &str,
) -> Result<String> {
    let requested = title(requested)?;
    let names = if let Some(destination) = destination {
        connection
            .prepare("SELECT title FROM entries WHERE workspace_id = ?1 AND parent_id IS ?2")?
            .query_map(params![destination.workspace, destination.parent], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<HashSet<_>>>()?
    } else {
        connection
            .prepare("SELECT title FROM workspaces WHERE account_id = ?1")?
            .query_map([owner], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<HashSet<_>>>()?
    };
    if !names.contains(requested) {
        return Ok(requested.into());
    }
    for number in 2_u64.. {
        let suffix = format!(" ({number})");
        let mut boundary = requested.len().min(256 - suffix.len());
        while !requested.is_char_boundary(boundary) {
            boundary -= 1;
        }
        let candidate = format!("{}{suffix}", &requested[..boundary]);
        if !names.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(Failure::conflict("No queda un nombre disponible."))
}

struct CopyTree<'a> {
    workspace: Id,
    root: Option<Id>,
    destination: Destination,
    title: Option<&'a str>,
    drafts: &'a [Draft],
}

fn copy_tree(connection: &Connection, copy: &CopyTree<'_>) -> Result<Option<Id>> {
    let rows = connection
        .prepare(COPY_TREE)?
        .query_map(params![copy.workspace, copy.root], |row| {
            Ok((row.get::<_, Id>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let stored: i64 = rows.iter().map(|(_, size)| size).sum();
    if rows.len() > MAX_COPY_ENTRIES
        || usize::try_from(stored).unwrap_or(usize::MAX) > MAX_COPY_BYTES
    {
        return Err(Failure::invalid(
            "La copia supera el límite de 2048 entradas o 16 MiB por operación.",
        ));
    }
    let mut drafts = HashMap::with_capacity(copy.drafts.len());
    for draft in copy.drafts {
        if drafts.insert(draft.id, draft).is_some() {
            return Err(Failure::invalid("Hay borradores duplicados."));
        }
    }
    let mut assigned = HashMap::with_capacity(rows.len());
    let mut first = None;
    let mut copied_bytes = 0_usize;
    for (id, _) in rows {
        let source =
            connection.query_row(ENTRY_IN_WORKSPACE, params![id, copy.workspace], record)?;
        let parent = if copy.root == Some(source.id) || source.parent.is_none() {
            copy.destination.parent
        } else {
            let original = source.parent.ok_or_else(Failure::missing)?;
            Some(*assigned.get(&original).ok_or_else(Failure::missing)?)
        };
        let mut content = source.markdown.as_deref();
        if let Some(draft) = drafts.remove(&source.id) {
            revision(source.revision, draft.revision)?;
            if content.is_none() {
                return Err(Failure::invalid("Una carpeta no contiene Markdown."));
            }
            content = Some(&draft.markdown);
        }
        if let Some(content) = content {
            markdown(content)?;
            copied_bytes = copied_bytes.saturating_add(content.len());
        }
        // los borradores pueden superar el tamaño almacenado
        if copied_bytes > MAX_COPY_BYTES {
            return Err(Failure::invalid(
                "La copia supera 16 MiB de contenido efectivo.",
            ));
        }
        let name = match copy.title {
            Some(title) if copy.root == Some(source.id) => title,
            _ => &source.title,
        };
        let kind = if content.is_some() {
            EntryKind::Note
        } else {
            EntryKind::Folder
        };
        connection.execute(
            INSERT_ENTRY,
            params![
                copy.destination.workspace,
                parent,
                kind_name(kind),
                name,
                content
            ],
        )?;
        let id = connection.last_insert_rowid();
        first = first.or(Some(id));
        assigned.insert(source.id, id);
    }
    if !drafts.is_empty() {
        return Err(Failure::invalid("El borrador no pertenece al origen."));
    }
    Ok(first)
}

pub fn mutate(
    connection: &mut Connection,
    owner: Id,
    request: &anotau_protocol::Mutation,
) -> Result<anotau_protocol::Change> {
    if request.operation_id.len() != 64
        || !request
            .operation_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Failure::invalid("Identidad de operación inválida."));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // un recibo guarda cuenta, hora y huella del texto: una sugerencia anónima no lo deja, a costa
    // de que un reintento tras una respuesta perdida pueda duplicarla
    if let Command::Suggest {
        anonymous: true, ..
    } = &request.command
    {
        let outcome = apply(&transaction, owner, &request.command)?;
        transaction.commit()?;
        let state = Some(snapshot(connection, owner)?);
        return Ok(anotau_protocol::Change { outcome, state });
    }
    let hash = Sha256::digest(serde_json::to_vec(&request.command)?);
    let receipt: Option<(Vec<u8>, String)> = transaction
        .query_row(RECEIPT, params![owner, request.operation_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?;
    let outcome = if let Some((previous, result)) = receipt {
        if previous.as_slice() != hash.as_slice() {
            return Err(Failure::conflict(
                "La identidad de operación ya tiene otros argumentos.",
            ));
        }
        serde_json::from_str(&result)?
    } else {
        let outcome = apply(&transaction, owner, &request.command)?;
        within_note_quota(&transaction, owner, &request.command)?;
        transaction.execute(
            INSERT_RECEIPT,
            params![
                owner,
                request.operation_id,
                hash.as_slice(),
                serde_json::to_string(&outcome)?
            ],
        )?;
        outcome
    };
    transaction.commit()?;
    let state = if matches!(outcome, Outcome::Saved { .. }) {
        None
    } else {
        Some(snapshot(connection, owner)?)
    };
    Ok(anotau_protocol::Change { outcome, state })
}

#[expect(
    clippy::too_many_lines,
    reason = "un brazo por comando; cada uno valida y escribe en la misma transacción"
)]
fn apply(connection: &Connection, owner: Id, command: &Command) -> Result<Outcome> {
    Ok(match command {
        Command::CreateWorkspace { title } => {
            let title = available(connection, owner, None, title)?;
            connection.execute(
                "INSERT INTO workspaces(account_id, title) VALUES (?1, ?2)",
                params![owner, title],
            )?;
            changed(Some(connection.last_insert_rowid()), None)
        }
        Command::RenameWorkspace {
            id,
            revision: expected,
            title,
        } => {
            revision(workspace(connection, owner, *id)?, *expected)?;
            let title = self::title(title)?;
            if exists(connection, WORKSPACE_NAME_TAKEN, params![owner, title, id])? {
                return Err(Failure::conflict("Ya tienes otro espacio con ese nombre."));
            }
            connection.execute(RENAME_WORKSPACE, params![title, id])?;
            changed(None, None)
        }
        Command::CopyWorkspace {
            id,
            revision: expected,
            title,
            drafts,
        } => {
            revision(workspace(connection, owner, *id)?, *expected)?;
            let title = available(connection, owner, None, title)?;
            connection.execute(
                "INSERT INTO workspaces(account_id, title) VALUES (?1, ?2)",
                params![owner, title],
            )?;
            let target = connection.last_insert_rowid();
            let copy = CopyTree {
                workspace: *id,
                root: None,
                destination: Destination {
                    workspace: target,
                    parent: None,
                },
                title: None,
                drafts,
            };
            copy_tree(connection, &copy)?;
            changed(Some(target), None)
        }
        Command::DeleteWorkspace {
            id,
            revision: expected,
        } => {
            revision(workspace(connection, owner, *id)?, *expected)?;
            let count: i64 = connection.query_row(
                "SELECT count(*) FROM workspaces WHERE account_id = ?1",
                [owner],
                |row| row.get(0),
            )?;
            if count <= 1 {
                return Err(Failure::conflict("Conserva al menos un espacio."));
            }
            connection.execute("DELETE FROM entries WHERE workspace_id = ?1", [id])?;
            connection.execute("DELETE FROM workspaces WHERE id = ?1", [id])?;
            changed(None, None)
        }
        Command::CreateEntry {
            workspace: space,
            parent: destination,
            title,
            kind,
        } => {
            workspace(connection, owner, *space)?;
            parent(connection, *space, *destination)?;
            let target = Destination {
                workspace: *space,
                parent: *destination,
            };
            let title = available(connection, owner, Some(target), title)?;
            let content = (*kind == EntryKind::Note).then_some("");
            connection.execute(
                INSERT_ENTRY,
                params![space, destination, kind_name(*kind), title, content],
            )?;
            changed(Some(*space), Some(connection.last_insert_rowid()))
        }
        Command::RenameEntry {
            id,
            revision: expected,
            title,
        } => {
            let source = entry(connection, owner, *id)?;
            revision(source.revision, *expected)?;
            let title = self::title(title)?;
            let siblings = params![source.workspace, source.parent, title, id];
            if exists(connection, ENTRY_NAME_TAKEN, siblings)? {
                return Err(Failure::conflict(
                    "Ya hay una nota o carpeta con ese nombre en esta ubicación.",
                ));
            }
            connection.execute(RENAME_ENTRY, params![title, id])?;
            changed(None, None)
        }
        Command::MoveEntry {
            id,
            revision: expected,
            parent: destination,
        } => {
            let source = entry(connection, owner, *id)?;
            revision(source.revision, *expected)?;
            parent(connection, source.workspace, *destination)?;
            if exists(
                connection,
                DESTINATION_INSIDE_SOURCE,
                params![destination, id],
            )? {
                return Err(Failure::invalid(
                    "Una carpeta no puede moverse dentro de sí misma ni de sus subcarpetas.",
                ));
            }
            let siblings = params![source.workspace, destination, source.title, id];
            if exists(connection, ENTRY_NAME_TAKEN, siblings)? {
                return Err(Failure::conflict(
                    "El destino ya tiene una nota o carpeta con ese nombre. Renómbrala antes \
                     de moverla.",
                ));
            }
            connection.execute(MOVE_ENTRY, params![destination, id])?;
            changed(None, None)
        }
        Command::CopyEntry {
            id,
            revision: expected,
            parent: destination,
            drafts,
        } => {
            let source = entry(connection, owner, *id)?;
            revision(source.revision, *expected)?;
            parent(connection, source.workspace, *destination)?;
            if exists(
                connection,
                DESTINATION_INSIDE_SOURCE,
                params![destination, id],
            )? {
                return Err(Failure::invalid(
                    "Una carpeta no puede copiarse dentro de sí misma ni de sus subcarpetas.",
                ));
            }
            let target = Destination {
                workspace: source.workspace,
                parent: *destination,
            };
            let title = available(connection, owner, Some(target), &source.title)?;
            let copy = CopyTree {
                workspace: source.workspace,
                root: Some(*id),
                destination: target,
                title: Some(&title),
                drafts,
            };
            changed(Some(source.workspace), copy_tree(connection, &copy)?)
        }
        Command::DeleteEntry {
            id,
            revision: expected,
        } => {
            revision(entry(connection, owner, *id)?.revision, *expected)?;
            if exists(connection, HAS_CHILDREN, [id])? {
                return Err(Failure::conflict(
                    "La carpeta no está vacía. Mueve o elimina su contenido antes de \
                     eliminarla.",
                ));
            }
            connection.execute("DELETE FROM entries WHERE id = ?1", [id])?;
            changed(None, None)
        }
        Command::Save {
            id,
            revision,
            markdown,
        } => save_note(connection, owner, *id, *revision, markdown)?,
        Command::Contact { recipient } => {
            if *recipient == owner {
                return Err(Failure::invalid("No puedes pedirte contacto a ti mismo."));
            }
            let state: Option<String> = connection
                .query_row(CONTACT_STATE, params![owner, recipient], |row| row.get(0))
                .optional()?;
            if let Some(state) = state {
                return Err(Failure::conflict(match state.as_str() {
                    "accepted" => "Este usuario ya es tu contacto.",
                    "rejected" => "La solicitud de contacto con este usuario fue rechazada.",
                    _ => "Ya hay una solicitud de contacto pendiente con este usuario.",
                }));
            }
            connection.execute(
                "INSERT INTO contacts(sender_id, recipient_id) VALUES (?1, ?2)",
                params![owner, recipient],
            )?;
            changed(None, None)
        }
        Command::ResolveContact { id, resolution } => {
            let (state, recipient): (String, Id) = connection
                .query_row(
                    "SELECT state, recipient_id FROM contacts WHERE id = ?1 AND recipient_id = ?2",
                    params![id, owner],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .ok_or_else(Failure::missing)?;
            let target = match resolution {
                Resolution::Accept => "accepted",
                Resolution::Reject => "rejected",
            };
            if state == "pending" {
                connection.execute(RESOLVE_CONTACT, params![target, id, recipient])?;
            } else if state != target {
                return Err(Failure::conflict(
                    "La solicitud ya se resolvió en otro sentido.",
                ));
            }
            changed(None, None)
        }
        Command::Offer {
            id,
            revision: expected,
            recipient,
        } => {
            let source = entry(connection, owner, *id)?;
            revision(source.revision, *expected)?;
            let content = source
                .markdown
                .ok_or_else(|| Failure::invalid("Solo se pueden transferir notas, no carpetas."))?;
            let state: Option<String> = connection
                .query_row(CONTACT_STATE, params![owner, recipient], |row| row.get(0))
                .optional()?;
            if state.as_deref() != Some("accepted") {
                return Err(Failure::conflict(
                    "El destinatario debe aceptar tu solicitud de contacto antes de recibir notas.",
                ));
            }
            connection.execute(
                INSERT_OFFER,
                params![owner, recipient, id, expected, source.title, content],
            )?;
            changed(None, None)
        }
        Command::ResolveTransfer { id, destination } => {
            resolve_transfer(connection, owner, *id, destination.as_ref())?
        }
        Command::Preferences { revision, settings } => {
            preferences(connection, owner, *revision, settings)?;
            changed(None, None)
        }
        Command::Suggest { text, anonymous } => {
            connection.execute(
                "INSERT INTO suggestions(account_id, body) VALUES (?1, ?2)",
                params![(!anonymous).then_some(owner), suggestion(text)?],
            )?;
            changed(None, None)
        }
    })
}

fn resolve_transfer(
    connection: &Connection,
    owner: Id,
    id: Id,
    destination: Option<&Destination>,
) -> Result<Outcome> {
    let (state, title, content, delivered): (String, String, String, Option<Id>) = connection
        .query_row(TRANSFER, params![id, owner], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .optional()?
        .ok_or_else(Failure::missing)?;
    let target = if destination.is_some() {
        "accepted"
    } else {
        "rejected"
    };
    if state != "pending" {
        if state != target {
            return Err(Failure::conflict(
                "La solicitud ya se resolvió en otro sentido.",
            ));
        }
        // repetir una aceptación devuelve la entrega registrada, aunque se haya eliminado
        return Ok(match delivered {
            Some(note) => Outcome::Delivered { transfer: id, note },
            None => changed(None, None),
        });
    }
    let Some(destination) = destination else {
        connection.execute(
            "UPDATE transfers SET state = 'rejected', resolved_at = unixepoch() WHERE id = ?1",
            [id],
        )?;
        return Ok(changed(None, None));
    };
    workspace(connection, owner, destination.workspace)?;
    parent(connection, destination.workspace, destination.parent)?;
    let title = available(connection, owner, Some(*destination), &title)?;
    connection.execute(
        INSERT_ENTRY,
        params![
            destination.workspace,
            destination.parent,
            "note",
            title,
            content
        ],
    )?;
    let note = connection.last_insert_rowid();
    connection.execute(DELIVER_TRANSFER, params![note, id])?;
    Ok(Outcome::Delivered { transfer: id, note })
}

fn save_note(
    connection: &Connection,
    owner: Id,
    id: Id,
    expected: i64,
    content: &str,
) -> Result<Outcome> {
    let source = entry(connection, owner, id)?;
    revision(source.revision, expected)?;
    markdown(content)?;
    if source.markdown.is_none() {
        return Err(Failure::invalid("Selecciona una nota."));
    }
    let (revision, at) = connection.query_row(SAVE_NOTE, params![content, id], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    let workspace_revision = workspace(connection, owner, source.workspace)?;
    Ok(Outcome::Saved {
        id,
        revision,
        at,
        workspace_revision,
    })
}

fn preferences(
    connection: &Connection,
    owner: Id,
    expected: i64,
    settings: &anotau_protocol::Settings,
) -> Result<()> {
    let actual = connection.query_row(
        "SELECT revision FROM account_preferences WHERE account_id = ?1",
        [owner],
        |row| row.get(0),
    )?;
    revision(actual, expected)?;
    connection.execute(
        UPDATE_PREFERENCES,
        params![serde_json::to_string(settings)?, owner],
    )?;
    Ok(())
}
