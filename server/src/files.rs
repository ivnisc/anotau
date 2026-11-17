use std::{fs, path::Path};

use anyhow::{Context, ensure};

fn private_metadata(path: &Path) -> anyhow::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("No se pudo inspeccionar {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let shared_permissions = metadata.permissions().mode() & 0o077;
        ensure!(
            shared_permissions == 0,
            "Solo el usuario del servicio debe tener permisos sobre: {}",
            path.display()
        );
    }
    Ok(metadata)
}

pub fn require_private_file(path: &Path) -> anyhow::Result<()> {
    ensure!(
        private_metadata(path)?.is_file(),
        "Se requiere un archivo regular, sin enlaces simbólicos: {}",
        path.display()
    );
    Ok(())
}

pub fn require_private_directory(path: &Path) -> anyhow::Result<()> {
    ensure!(
        private_metadata(path)?.is_dir(),
        "Se requiere un directorio privado, sin enlaces simbólicos: {}",
        path.display()
    );
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn shared_permissions_and_symbolic_links_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(require_private_directory(directory.path()).is_ok());
        let key = directory.path().join("server.key");
        fs::write(&key, "test fixture").unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(require_private_file(&key).is_ok());
        let link = directory.path().join("link");
        symlink(&key, &link).unwrap();
        assert!(require_private_file(&link).is_err());
        fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(require_private_file(&key).is_err());
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(require_private_directory(directory.path()).is_err());
    }
}
