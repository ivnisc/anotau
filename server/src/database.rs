use std::{fs::OpenOptions, io, path::Path, time::Duration};

use anyhow::{Context, ensure};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

pub const SCHEMA_VERSION: u32 = 6;
const APPLICATION_ID: i32 = 0x414e_4f54;
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_bootstrap.sql"),
    include_str!("../migrations/0002_domain.sql"),
    include_str!("../migrations/0003_tree_revisions.sql"),
    include_str!("../migrations/0004_suggestions.sql"),
    include_str!("../migrations/0005_recovery.sql"),
    include_str!("../migrations/0006_temporary_passwords.sql"),
];

pub fn open(path: &Path) -> anyhow::Result<Connection> {
    let parent = path
        .parent()
        .context("SQLite necesita un directorio padre")?;
    crate::files::require_private_directory(parent)?;
    let path = parent
        .canonicalize()
        .context("No se pudo resolver el directorio de SQLite")?
        .join(
            path.file_name()
                .context("SQLite necesita un nombre de archivo")?,
        );
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error)
                .context("No se pudo crear SQLite; comprueba el directorio de ANOTAU_DATABASE");
        }
    }
    crate::files::require_private_file(&path)?;
    let mut connection = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .context("No se pudo abrir SQLite")?;
    connection.busy_timeout(Duration::from_secs(3))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    migrate(&mut connection).context("No se pudo migrar SQLite")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "cache_size", -2048)?;
    Ok(connection)
}

fn migrate(connection: &mut Connection) -> anyhow::Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let application: i32 =
        transaction.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let tables: u32 = transaction.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        application == APPLICATION_ID || (application == 0 && version == 0 && tables == 0),
        "La base no pertenece a Anotau"
    );
    ensure!(
        version <= SCHEMA_VERSION,
        "La base requiere una versión más reciente del servidor"
    );
    if version > 0 {
        let (count, latest): (u32, Option<u32>) = transaction.query_row(
            "SELECT count(*), max(version) FROM schema_migrations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        ensure!(
            count == version && latest == Some(version),
            "El registro de migraciones no coincide con el esquema"
        );
    }
    ensure!(
        usize::try_from(SCHEMA_VERSION)? == MIGRATIONS.len(),
        "La versión del servidor no coincide con sus migraciones"
    );
    for (index, sql) in MIGRATIONS
        .iter()
        .enumerate()
        .skip(usize::try_from(version)?)
    {
        let next = u32::try_from(index)? + 1;
        transaction
            .execute_batch(sql)
            .with_context(|| format!("Falló la migración {next:04}"))?;
        transaction.execute("INSERT INTO schema_migrations(version) VALUES (?1)", [next])?;
        transaction.pragma_update(None, "user_version", next)?;
    }
    transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
    transaction
        .commit()
        .context("No se pudo confirmar la inicialización de SQLite")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        directory
    }

    #[test]
    fn concurrent_initializers_apply_each_migration_once() {
        let directory = private_directory();
        let path = directory.path().join("concurrent.sqlite3");
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let start = || {
                barrier.wait();
                open(&path).map(drop)
            };
            let one = scope.spawn(start);
            let two = scope.spawn(start);
            one.join().unwrap().unwrap();
            two.join().unwrap().unwrap();
        });
        let connection = open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM schema_migrations", [], |row| row
                    .get::<_, u32>(0))
                .unwrap(),
            SCHEMA_VERSION
        );
    }

    #[test]
    fn migrations_are_persistent_and_reopening_does_not_duplicate_them() {
        let directory = private_directory();
        let path = directory.path().join("notes.sqlite3");
        for _ in 0..2 {
            let connection = open(&path).unwrap();
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM schema_migrations", [], |row| row
                        .get::<_, u32>(0))
                    .unwrap(),
                SCHEMA_VERSION
            );
            assert_eq!(
                connection
                    .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                    .unwrap(),
                SCHEMA_VERSION
            );
            assert_eq!(
                connection
                    .pragma_query_value(None, "foreign_keys", |row| row.get::<_, u32>(0))
                    .unwrap(),
                1
            );
        }
    }

    #[test]
    fn unrelated_or_newer_databases_are_rejected_without_migration() {
        let directory = private_directory();
        let path = directory.path().join("foreign.sqlite3");
        drop(open(&path).unwrap());
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(connection);
        assert!(open(&path).is_err());
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            SCHEMA_VERSION + 1
        );
        connection.pragma_update(None, "user_version", 0).unwrap();
        connection.pragma_update(None, "application_id", 0).unwrap();
        let journal: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        drop(connection);
        assert!(open(&path).is_err());
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
                .unwrap(),
            journal
        );
    }
}
