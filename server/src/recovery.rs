//! Recuperación de contraseña atendida por el administrador.
//!
//! No hay recuperación automática: Anotau no tiene un canal propio para comprobar identidad. La
//! persona deja una solicitud con un contacto; el administrador la verifica por ese medio y
//! entrega una contraseña temporal generada con `anotau-server reset-password <usuario>`.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::api::{Failure, Result};

/// Solicitud pendiente, con su fecha en UTC como `AAAA-MM-DD HH:MM:SS`.
pub struct Ticket {
    pub id: i64,
    pub username: String,
    pub contact: String,
    pub created_at: String,
}

/// Registra la solicitud si la cuenta existe y no tiene otra pendiente.
///
/// Responde igual exista o no la cuenta, para no revelar qué nombres están registrados.
pub fn request(connection: &mut Connection, username: &str, contact: &str) -> Result<()> {
    let account: Option<i64> = connection
        .query_row(
            "SELECT id FROM accounts WHERE username = ?1",
            [username],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(account) = account {
        connection.execute(
            "INSERT INTO recovery_tickets(account_id, contact) VALUES (?1, ?2)
             ON CONFLICT DO NOTHING",
            params![account, contact.trim()],
        )?;
    }
    Ok(())
}

/// Solicitudes sin resolver, de la más antigua a la más reciente.
pub fn pending(connection: &Connection) -> Result<Vec<Ticket>> {
    let mut statement = connection.prepare(
        "SELECT t.id, a.username, t.contact, datetime(t.created_at, 'unixepoch')
         FROM recovery_tickets t JOIN accounts a ON a.id = t.account_id
         WHERE t.resolved_at IS NULL ORDER BY t.id",
    )?;
    let tickets = statement
        .query_map([], |row| {
            Ok(Ticket {
                id: row.get(0)?,
                username: row.get(1)?,
                contact: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tickets)
}

/// Horas que sirve una contraseña temporal si no se cambia en F3.
pub const TEMPORARY_HOURS: i64 = 72;

/// Reemplaza la contraseña por la temporal `hash`, que vence en [`TEMPORARY_HOURS`], cierra
/// todas las sesiones de la cuenta y resuelve sus solicitudes pendientes, en una transacción.
/// Devuelve el vencimiento en UTC como `AAAA-MM-DD HH:MM:SS`.
pub fn reset(connection: &mut Connection, username: &str, hash: &str) -> Result<String> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let account: i64 = transaction
        .query_row(
            "SELECT id FROM accounts WHERE username = ?1",
            [username],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(Failure::missing)?;
    let expires: String = transaction.query_row(
        "UPDATE accounts SET password_hash = ?1, password_expires_at = unixepoch() + ?2 * 3600
         WHERE id = ?3 RETURNING datetime(password_expires_at, 'unixepoch')",
        params![hash, TEMPORARY_HOURS, account],
        |row| row.get(0),
    )?;
    transaction.execute(
        "UPDATE sessions SET revoked_at = unixepoch() WHERE account_id = ?1 AND revoked_at IS NULL",
        [account],
    )?;
    transaction.execute(
        "UPDATE recovery_tickets SET resolved_at = unixepoch()
         WHERE account_id = ?1 AND resolved_at IS NULL",
        [account],
    )?;
    transaction.commit()?;
    Ok(expires)
}

/// Cuenta cuyo verificador es `hash`, si su contraseña no es una temporal vencida. Una
/// temporal vencida se rechaza igual que una contraseña incorrecta.
pub fn signed_in_account(connection: &Connection, username: &str, hash: &str) -> Result<i64> {
    connection
        .query_row(
            "SELECT id FROM accounts WHERE username = ?1 AND password_hash = ?2
             AND (password_expires_at IS NULL OR password_expires_at > unixepoch())",
            params![username, hash],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(Failure::unauthorized)
}

/// Contraseña temporal de 12 caracteres sin letras ni dígitos que se confundan al dictarla.
pub fn temporary_password() -> Result<String> {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut bytes = [0_u8; 12];
    getrandom::fill(&mut bytes).map_err(|error| {
        Failure::internal(anyhow::anyhow!("No se pudo obtener aleatoriedad: {error}"))
    })?;
    // 256 no es múltiplo de 31: se descarta el sesgo leve rechazando los bytes altos
    let mut password = String::with_capacity(14);
    for (index, &byte) in bytes.iter().enumerate() {
        let mut value = byte;
        while usize::from(value) >= 256 / ALPHABET.len() * ALPHABET.len() {
            let mut retry = [0_u8; 1];
            getrandom::fill(&mut retry).map_err(|error| {
                Failure::internal(anyhow::anyhow!("No se pudo obtener aleatoriedad: {error}"))
            })?;
            value = retry[0];
        }
        if index > 0 && index % 4 == 0 {
            password.push('-');
        }
        password.push(char::from(ALPHABET[usize::from(value) % ALPHABET.len()]));
    }
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> (tempfile::TempDir, Connection) {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let connection = crate::database::open(&directory.path().join("test.sqlite3")).unwrap();
        (directory, connection)
    }

    #[test]
    fn requests_keep_one_pending_ticket_and_a_reset_resolves_it_and_closes_sessions() {
        let (_directory, mut connection) = database();
        let account = crate::store::register(&mut connection, "carla", "viejo").unwrap();
        connection
            .execute(
                "INSERT INTO sessions(token_hash, account_id) VALUES (zeroblob(32), ?1)",
                [account],
            )
            .unwrap();
        request(&mut connection, "carla", " carla@usm.cl ").unwrap();
        request(&mut connection, "carla", "otro@usm.cl").unwrap();
        request(&mut connection, "nadie", "x@usm.cl").unwrap();
        let tickets = pending(&connection).unwrap();
        assert_eq!(
            tickets.len(),
            1,
            "una pendiente por cuenta; nada para nombres ajenos"
        );
        assert_eq!(
            (tickets[0].username.as_str(), tickets[0].contact.as_str()),
            ("carla", "carla@usm.cl")
        );

        reset(&mut connection, "carla", "nuevo").unwrap();
        assert!(pending(&connection).unwrap().is_empty());
        let (hash, open): (String, i64) = connection
            .query_row(
                "SELECT password_hash, (SELECT count(*) FROM sessions WHERE revoked_at IS NULL)
                 FROM accounts WHERE id = ?1",
                [account],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((hash.as_str(), open), ("nuevo", 0));
        assert_eq!(
            reset(&mut connection, "nadie", "x").unwrap_err().status(),
            404
        );

        // tras resolverla, la persona puede pedir otra
        request(&mut connection, "carla", "carla@usm.cl").unwrap();
        assert_eq!(pending(&connection).unwrap().len(), 1);
    }

    #[test]
    fn a_temporary_password_signs_in_for_72_hours_and_then_is_refused_like_a_wrong_one() {
        let (_directory, mut connection) = database();
        crate::store::register(&mut connection, "carla", "definitiva").unwrap();
        assert!(signed_in_account(&connection, "carla", "definitiva").is_ok());

        let deadline = reset(&mut connection, "carla", "temporal").unwrap();
        let hours: i64 = connection
            .query_row(
                "SELECT (password_expires_at - unixepoch()) / 3600 FROM accounts",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            (TEMPORARY_HOURS - 1..=TEMPORARY_HOURS).contains(&hours),
            "{hours}"
        );
        assert_eq!(deadline.len(), "2026-11-19 01:00:00".len());
        assert!(signed_in_account(&connection, "carla", "temporal").is_ok());
        assert_eq!(
            signed_in_account(&connection, "carla", "definitiva")
                .unwrap_err()
                .status(),
            401
        );

        connection
            .execute(
                "UPDATE accounts SET password_expires_at = unixepoch() - 1",
                [],
            )
            .unwrap();
        let expired = signed_in_account(&connection, "carla", "temporal").unwrap_err();
        assert_eq!(expired.status(), 401);
        assert_eq!(
            expired.message(),
            crate::api::Failure::unauthorized().message(),
            "mismo mensaje que una contraseña incorrecta"
        );
    }
}
