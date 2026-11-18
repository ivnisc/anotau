//! Respaldos de `SQLite` con `VACUUM INTO`.
//!
//! Cada respaldo es una base completa y compacta, coherente aunque el servidor siga escribiendo:
//! se lee desde una conexión aparte, de solo lectura, que ve una instantánea de WAL. Se escribe
//! primero con sufijo `.partial` y se renombra al terminar, para que un corte no deje un archivo
//! que parezca válido. Solo se conservan los `keep` más recientes.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, ensure};
use rusqlite::{Connection, OpenFlags};

const PREFIX: &str = "anotau-";
const SUFFIX: &str = ".sqlite3";

/// Copia la base `source` en `directory` y elimina los respaldos que excedan `keep`.
pub fn run(source: &Path, directory: &Path, keep: usize) -> anyhow::Result<PathBuf> {
    ensure!(keep > 0, "Se debe conservar al menos un respaldo");
    crate::files::require_private_directory(directory)?;
    // la conexión no sigue enlaces, tampoco en los directorios del destino
    let directory = &directory
        .canonicalize()
        .context("No se pudo resolver el directorio de respaldos")?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("El reloj del sistema es anterior a 1970")?;
    // segundos y nanosegundos de ancho fijo: el orden alfabético es el cronológico
    let name = format!(
        "{PREFIX}{:010}-{:09}{SUFFIX}",
        stamp.as_secs(),
        stamp.subsec_nanos()
    );
    let target = directory.join(&name);
    let partial = directory.join(format!("{name}.partial"));
    // como al abrir la base: se resuelven los directorios y se rechaza que el archivo sea un enlace
    let source = source
        .canonicalize()
        .context("No se pudo resolver la ruta de SQLite")?;
    let connection = Connection::open_with_flags(
        &source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .context("No se pudo abrir SQLite para respaldar")?;
    connection.busy_timeout(Duration::from_secs(10))?;
    let destination = partial
        .to_str()
        .context("La ruta de respaldos debe ser UTF-8")?;
    connection
        .execute("VACUUM INTO ?1", [destination])
        .context("Falló VACUUM INTO")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&partial, &target).context("No se pudo completar el respaldo")?;
    prune(directory, keep)?;
    Ok(target)
}

/// Respaldos completos del directorio, del más antiguo al más reciente.
pub fn list(directory: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut backups = fs::read_dir(directory)
        .with_context(|| format!("No se pudo leer {}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(PREFIX) && name.ends_with(SUFFIX))
        })
        .collect::<Vec<_>>();
    backups.sort();
    Ok(backups)
}

fn prune(directory: &Path, keep: usize) -> anyhow::Result<()> {
    let backups = list(directory)?;
    for old in &backups[..backups.len().saturating_sub(keep)] {
        fs::remove_file(old).with_context(|| format!("No se pudo borrar {}", old.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn private_directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }

    #[test]
    fn backups_are_complete_private_databases_and_only_the_newest_are_kept() {
        let data = private_directory();
        let source = data.path().join("anotau.sqlite3");
        let mut connection = crate::database::open(&source).unwrap();
        crate::store::register(&mut connection, "carla", "hash").unwrap();
        let backups = private_directory();
        for _ in 0..3 {
            run(&source, backups.path(), 2).unwrap();
        }
        let kept = list(backups.path()).unwrap();
        assert_eq!(kept.len(), 2);
        assert!(fs::read_dir(backups.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .path()
                .to_string_lossy()
                .ends_with(".partial")
        }));
        let newest = kept.last().unwrap();
        assert_eq!(
            fs::metadata(newest).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // el respaldo se abre como una base de Anotau con sus datos
        let restored = crate::database::open(newest).unwrap();
        let name: String = restored
            .query_row("SELECT username FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "carla");

        fs::set_permissions(backups.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            run(&source, backups.path(), 2).is_err(),
            "directorio compartido"
        );
    }
}
