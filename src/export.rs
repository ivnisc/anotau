//! Exportación de notas como archivos Markdown en el equipo del cliente.
//!
//! Nunca se sobrescribe: si el nombre existe se usa el primer sufijo libre, `Nota (2).md`, como
//! al crear notas en Anotau. Una carpeta se exporta como un directorio con su misma estructura.

use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
};

/// Nota a exportar: carpetas dentro de la carpeta exportada, título y contenido.
pub(crate) struct File {
    pub(crate) folders: Vec<String>,
    pub(crate) title: String,
    pub(crate) markdown: String,
}

fn home() -> Option<PathBuf> {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// Carpeta de descargas del sistema; si no se encuentra, el directorio personal.
pub(crate) fn default_directory() -> PathBuf {
    let Some(home) = home() else {
        return PathBuf::from(".");
    };
    // en Linux la carpeta puede estar traducida, como ~/Descargas; la declara xdg-user-dirs
    if cfg!(target_os = "linux")
        && let Some(directory) = env::var_os("XDG_DOWNLOAD_DIR")
            .map(PathBuf::from)
            .or_else(|| xdg_download_dir(&home))
        && directory.is_dir()
    {
        return directory;
    }
    ["Downloads", "Descargas"]
        .into_iter()
        .map(|name| home.join(name))
        .find(|directory| directory.is_dir())
        .unwrap_or(home)
}

fn xdg_download_dir(home: &Path) -> Option<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from);
    let text = fs::read_to_string(config.join("user-dirs.dirs")).ok()?;
    let value = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("XDG_DOWNLOAD_DIR="))?
        .trim_matches('"');
    Some(match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(value),
    })
}

/// Ruta escrita por el usuario; `~` es su directorio personal.
pub(crate) fn expand(input: &str) -> PathBuf {
    let input = input.trim();
    match (input.strip_prefix('~'), home()) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            home.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(input),
    }
}

/// Nombre válido en Windows, macOS y Linux a partir de un título de Anotau.
fn file_name(title: &str) -> String {
    let name: String = title
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '-'
            } else {
                ch
            }
        })
        .take(120)
        .collect();
    // Windows no admite espacios ni puntos finales, ni algunos nombres de dispositivo
    let name = name.trim().trim_end_matches('.').trim_end();
    if name.is_empty() || name.chars().all(|ch| ch == '.') {
        return "nota".into();
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(|ch: char| ch.is_ascii_digit()));
    if reserved {
        format!("_{name}")
    } else {
        name.to_owned()
    }
}

/// Primer nombre libre en `directory`: `nombre`, `nombre (2)`, …, con `extension` al final.
fn available(directory: &Path, name: &str, extension: &str) -> io::Result<PathBuf> {
    (1_u32..=10_000)
        .map(|index| {
            let suffix = if index == 1 {
                String::new()
            } else {
                format!(" ({index})")
            };
            directory.join(format!("{name}{suffix}{extension}"))
        })
        .find(|path| fs::symlink_metadata(path).is_err())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("Ya hay demasiados archivos llamados «{name}» en el destino"),
            )
        })
}

/// Escribe las notas en `directory`. Con `folder`, dentro de un directorio nuevo con ese
/// nombre; devuelve la ruta de ese directorio o del único archivo escrito.
pub(crate) fn write(directory: &Path, folder: Option<&str>, files: &[File]) -> io::Result<PathBuf> {
    if !directory.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("La carpeta de destino no existe: {}", directory.display()),
        ));
    }
    let root = match folder {
        Some(title) => {
            let root = available(directory, &file_name(title), "")?;
            fs::create_dir(&root)?;
            root
        }
        None => directory.to_path_buf(),
    };
    let mut written = root.clone();
    for file in files {
        let mut parent = root.clone();
        for folder in &file.folders {
            parent.push(file_name(folder));
        }
        fs::create_dir_all(&parent)?;
        let path = available(&parent, &file_name(&file.title), ".md")?;
        // create_new: si otro proceso creó el archivo entre medio, falla en vez de pisarlo
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?
            .write_all(file.markdown.as_bytes())?;
        if folder.is_none() {
            written = path;
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(folders: &[&str], title: &str, markdown: &str) -> File {
        File {
            folders: folders.iter().map(|&folder| folder.into()).collect(),
            title: title.into(),
            markdown: markdown.into(),
        }
    }

    #[test]
    fn titles_become_portable_file_names() {
        assert_eq!(
            file_name("Cálculo: límites / derivadas?"),
            "Cálculo- límites - derivadas-"
        );
        assert_eq!(file_name("  Resumen. "), "Resumen");
        assert_eq!(file_name("con"), "_con");
        assert_eq!(file_name("COM1.notas"), "_COM1.notas");
        assert_eq!(file_name("comida"), "comida");
        assert_eq!(file_name("..."), "nota");
        assert_eq!(file_name("a\u{1b}[2J"), "a-[2J");
    }

    #[test]
    fn exports_never_overwrite_and_folders_keep_their_structure() {
        let directory = tempfile::tempdir().unwrap();
        let first = write(directory.path(), None, &[note(&[], "Apuntes", "# Uno")]).unwrap();
        let second = write(directory.path(), None, &[note(&[], "Apuntes", "# Dos")]).unwrap();
        assert_eq!(first, directory.path().join("Apuntes.md"));
        assert_eq!(second, directory.path().join("Apuntes (2).md"));
        assert_eq!(fs::read_to_string(&first).unwrap(), "# Uno");

        let folder = write(
            directory.path(),
            Some("Ramos"),
            &[
                note(&[], "Programa", "p"),
                note(&["Cálculo"], "Límites", "l"),
            ],
        )
        .unwrap();
        assert_eq!(folder, directory.path().join("Ramos"));
        assert_eq!(
            fs::read_to_string(folder.join("Cálculo").join("Límites.md")).unwrap(),
            "l"
        );
        let again = write(directory.path(), Some("Ramos"), &[]).unwrap();
        assert_eq!(again, directory.path().join("Ramos (2)"));

        let missing = directory.path().join("no-existe");
        assert!(write(&missing, None, &[note(&[], "x", "")]).is_err());
    }
}
