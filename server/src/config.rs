use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, ensure};

pub(crate) struct TlsFiles {
    pub(crate) certificate: PathBuf,
    pub(crate) key: PathBuf,
}

pub(crate) struct Config {
    pub(crate) listen: SocketAddr,
    pub(crate) database: PathBuf,
    pub(crate) tls: Option<TlsFiles>,
    /// Nombre visible de la instancia en `/v1/health`.
    pub(crate) name: String,
    /// Mantención: no abre `SQLite` y rechaza todo salvo `/v1/health`.
    pub(crate) maintenance: bool,
    /// Directorio privado de respaldos diarios y cuántos conservar.
    pub(crate) backup: Option<(PathBuf, usize)>,
}

impl Config {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let address = match env::var("ANOTAU_LISTEN") {
            Ok(value) => value,
            Err(env::VarError::NotPresent) => "127.0.0.1:8443".into(),
            Err(error) => return Err(error).context("ANOTAU_LISTEN no contiene texto válido"),
        };
        let tls = match (
            env::var_os("ANOTAU_TLS_CERT"),
            env::var_os("ANOTAU_TLS_KEY"),
        ) {
            (None, None) => None,
            (Some(certificate), Some(key)) => Some(TlsFiles {
                certificate: certificate.into(),
                key: key.into(),
            }),
            _ => anyhow::bail!("Configura juntos ANOTAU_TLS_CERT y ANOTAU_TLS_KEY"),
        };
        let config = Self {
            listen: address
                .parse()
                .context("ANOTAU_LISTEN debe ser una IP y puerto")?,
            database: env::var_os("ANOTAU_DATABASE")
                .context("Falta ANOTAU_DATABASE")?
                .into(),
            tls,
            name: match env::var("ANOTAU_SERVER_NAME") {
                Ok(value) => value.trim().to_owned(),
                Err(env::VarError::NotPresent) => "anotau".into(),
                Err(error) => {
                    return Err(error).context("ANOTAU_SERVER_NAME no contiene texto válido");
                }
            },
            maintenance: match env::var("ANOTAU_MAINTENANCE").as_deref() {
                Err(env::VarError::NotPresent) | Ok("" | "0" | "false") => false,
                Ok("1" | "true") => true,
                _ => anyhow::bail!("ANOTAU_MAINTENANCE debe ser 1, true, 0 o false"),
            },
            backup: match env::var_os("ANOTAU_BACKUP_DIR") {
                None => None,
                Some(directory) => Some((
                    directory.into(),
                    match env::var("ANOTAU_BACKUP_KEEP") {
                        Err(env::VarError::NotPresent) => 7,
                        Ok(value) => value
                            .parse()
                            .ok()
                            .filter(|keep| (1..=365).contains(keep))
                            .context("ANOTAU_BACKUP_KEEP debe ser un número de 1 a 365")?,
                        Err(error) => {
                            return Err(error)
                                .context("ANOTAU_BACKUP_KEEP no contiene texto válido");
                        }
                    },
                )),
            },
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            !self.name.is_empty()
                && self.name.len() <= 64
                && !self.name.chars().any(char::is_control),
            "ANOTAU_SERVER_NAME debe tener de 1 a 64 bytes, sin caracteres de control"
        );
        ensure!(
            self.database.is_absolute() && self.database.file_name().is_some(),
            "ANOTAU_DATABASE debe ser una ruta absoluta de archivo"
        );
        ensure!(
            self.listen.ip().is_loopback() || self.tls.is_some(),
            "La escucha fuera de loopback requiere TLS"
        );
        if let Some((directory, _)) = &self.backup {
            ensure!(
                directory.is_absolute(),
                "ANOTAU_BACKUP_DIR debe ser una ruta absoluta"
            );
        }
        if let Some(tls) = &self.tls {
            ensure!(
                tls.certificate.is_absolute() && tls.key.is_absolute(),
                "Las rutas de certificado y clave deben ser absolutas"
            );
            ensure!(
                tls.certificate != tls.key,
                "El certificado y su clave deben estar en archivos separados"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_addresses_require_tls_and_paths_do_not_depend_on_the_working_directory() {
        let mut config = Config {
            listen: "127.0.0.1:8443".parse().unwrap(),
            database: env::temp_dir().join("anotau.sqlite3"),
            tls: None,
            name: "anotau".into(),
            maintenance: false,
            backup: None,
        };
        assert!(config.validate().is_ok());
        for name in ["", "vm\u{1b}[2J", &"a".repeat(65)] {
            config.name = name.into();
            assert!(config.validate().is_err(), "{name}");
        }
        config.name = "anotau-vm".into();
        for address in ["0.0.0.0:8443", "[::]:8443", "10.33.196.179:8443"] {
            config.listen = address.parse().unwrap();
            assert!(config.validate().is_err());
        }
        config.tls = Some(TlsFiles {
            certificate: env::temp_dir().join("server.pem"),
            key: env::temp_dir().join("server.key"),
        });
        assert!(config.validate().is_ok());
        config.database = "relative.sqlite3".into();
        assert!(config.validate().is_err());
    }
}
