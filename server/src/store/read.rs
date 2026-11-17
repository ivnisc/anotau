use anotau_protocol::{
    Contact, Cursor, Directory, Entry, EntryKind, Id, Incoming, Note, Person, Preferences,
    Presence, Snapshot, Workspace,
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::api::{Failure, Result};

const PAGE_SIZE: u16 = 128;

const NOTE: &str = "
    SELECT e.id, e.markdown, e.revision, e.updated_at
    FROM entries e JOIN workspaces w ON w.id = e.workspace_id
    WHERE e.id = ?1 AND w.account_id = ?2 AND e.kind = 'note'";
// presencia: 2 activa (< 5 min), 1 ausente (< 15 min con la sesión aún viva), 0 desconectada.
const PEOPLE: &str = "
    SELECT a.id, a.username,
        coalesce((
            SELECT max(CASE WHEN last_activity_at > unixepoch() - 300 THEN 2 ELSE 1 END)
            FROM sessions
            WHERE account_id = a.id AND revoked_at IS NULL
              AND last_activity_at > unixepoch() - 900 AND last_seen_at > unixepoch() - 90
        ), 0),
        c.id, c.state, c.recipient_id
    FROM accounts a
    LEFT JOIN contacts c
        ON min(c.sender_id, c.recipient_id) = min(a.id, ?1)
       AND max(c.sender_id, c.recipient_id) = max(a.id, ?1)
    WHERE a.id != ?1 AND a.id > ?2
    ORDER BY a.id LIMIT ?3";
const INCOMING: &str = "
    SELECT id, sender_id, title FROM transfers
    WHERE recipient_id = ?1 AND state = 'pending' AND id > ?2
    ORDER BY id LIMIT ?3";
const WORKSPACES: &str = "
    SELECT id, title, revision FROM workspaces
    WHERE account_id = ?1 AND id > ?2
    ORDER BY id LIMIT ?3";
const ENTRIES: &str = "
    SELECT e.id, e.workspace_id, e.parent_id, e.title, e.kind = 'folder', e.revision, e.updated_at
    FROM entries e JOIN workspaces w ON w.id = e.workspace_id
    WHERE w.account_id = ?1 AND e.id > ?2
    ORDER BY e.id LIMIT ?3";

fn page<T>(mut rows: Vec<T>, cursor: &mut Id, id: impl Fn(&T) -> Id) -> (Vec<T>, bool) {
    let more = rows.len() > usize::from(PAGE_SIZE);
    if more {
        rows.pop();
    }
    if let Some(last) = rows.last() {
        *cursor = id(last);
    }
    (rows, more)
}

pub fn note(connection: &Connection, owner: Id, id: Id) -> Result<Note> {
    let note = connection
        .query_row(NOTE, params![id, owner], |row| {
            Ok(Note {
                id: row.get(0)?,
                markdown: row.get(1)?,
                revision: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })
        .optional()?
        .ok_or_else(Failure::missing)?;
    super::markdown(&note.markdown)?;
    Ok(note)
}

fn directory(connection: &Connection, owner: Id, cursor: &mut Cursor) -> Result<(Directory, bool)> {
    let people = connection
        .prepare(PEOPLE)?
        .query_map(params![owner, cursor.person, PAGE_SIZE + 1], |row| {
            let state: Option<String> = row.get(4)?;
            let contact = match state.as_deref() {
                Some("accepted") => Contact::Accepted,
                Some("pending") if row.get::<_, Id>(5)? == owner => Contact::Incoming(row.get(3)?),
                Some("pending") => Contact::Pending,
                Some("rejected") => Contact::Rejected,
                _ => Contact::None,
            };
            Ok(Person {
                id: row.get(0)?,
                name: row.get(1)?,
                presence: match row.get::<_, u8>(2)? {
                    2 => Presence::Active,
                    1 => Presence::Away,
                    _ => Presence::Offline,
                },
                contact,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (people, more_people) = page(people, &mut cursor.person, |person| person.id);
    let incoming = connection
        .prepare(INCOMING)?
        .query_map(params![owner, cursor.offer, PAGE_SIZE + 1], |row| {
            Ok(Incoming {
                id: row.get(0)?,
                sender: row.get(1)?,
                title: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (incoming, more_offers) = page(incoming, &mut cursor.offer, |offer| offer.id);
    Ok((Directory { people, incoming }, more_people || more_offers))
}

pub fn snapshot(connection: &mut Connection, owner: Id) -> Result<Snapshot> {
    snapshot_at(connection, owner, Cursor::default())
}

pub fn snapshot_at(connection: &mut Connection, owner: Id, mut cursor: Cursor) -> Result<Snapshot> {
    if [cursor.entry, cursor.workspace, cursor.person, cursor.offer]
        .iter()
        .any(|id| *id < 0)
    {
        return Err(Failure::invalid("Cursor inválido."));
    }
    let transaction = connection.transaction()?;
    let version: i64 = transaction.query_row(
        "SELECT state_revision FROM accounts WHERE id = ?1",
        [owner],
        |row| row.get(0),
    )?;
    if cursor.version.is_some_and(|previous| previous != version) {
        return Err(Failure::conflict(
            "El árbol cambió durante la consulta. Reintenta la lectura.",
        ));
    }
    cursor.version = Some(version);
    let workspaces = transaction
        .prepare(WORKSPACES)?
        .query_map(params![owner, cursor.workspace, PAGE_SIZE + 1], |row| {
            Ok(Workspace {
                id: row.get(0)?,
                title: row.get(1)?,
                revision: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (workspaces, more_spaces) = page(workspaces, &mut cursor.workspace, |space| space.id);
    let entries = transaction
        .prepare(ENTRIES)?
        .query_map(params![owner, cursor.entry, PAGE_SIZE + 1], |row| {
            Ok(Entry {
                id: row.get(0)?,
                workspace: row.get(1)?,
                parent: row.get(2)?,
                title: row.get(3)?,
                kind: if row.get(4)? {
                    EntryKind::Folder
                } else {
                    EntryKind::Note
                },
                revision: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (entries, more_entries) = page(entries, &mut cursor.entry, |entry| entry.id);
    let (revision, settings): (i64, String) = transaction.query_row(
        "SELECT revision, settings FROM account_preferences WHERE account_id = ?1",
        [owner],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let preferences = Preferences {
        revision,
        settings: serde_json::from_str(&settings)?,
    };
    let (directory, more_directory) = directory(&transaction, owner, &mut cursor)?;
    transaction.commit()?;
    let next = (more_spaces || more_entries || more_directory).then_some(cursor);
    Ok(Snapshot {
        workspaces,
        entries,
        directory,
        preferences,
        next,
    })
}
