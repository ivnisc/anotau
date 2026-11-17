//! Tipos compartidos por el cliente y la API de Anotau.
use serde::{Deserialize, Serialize};

pub type Id = i64;

/// Versión del formato de la API; cambia con cada cambio incompatible del protocolo.
pub const API_VERSION: u32 = 2;

/// Tamaño máximo del Markdown de una nota, en bytes UTF-8.
///
/// La TUI reanaliza la nota completa en cada pulsación; con este tamaño cuesta unos 10 ms.
pub const MAX_NOTE_BYTES: usize = 128 * 1024;

/// Notas por cuenta, sumando todos sus espacios; las carpetas no cuentan.
pub const MAX_NOTES: usize = 50;

/// Tamaño máximo de una sugerencia, en bytes UTF-8.
pub const MAX_SUGGESTION_BYTES: usize = 4 * 1024;

/// Respuesta de `/v1/health`, que el cliente consulta antes de autenticar.
#[derive(Clone, Serialize, Deserialize)]
pub struct Health {
    pub service: String,
    pub api_version: u32,
    pub schema_version: u32,
    /// Nombre de la instancia que se muestra al iniciar sesión, como el de la VM.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: ServerStatus,
}

/// Disponibilidad declarada por el servidor; en mantención solo responde `/v1/health`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatus {
    #[default]
    Operational,
    Maintenance,
}

#[derive(Serialize, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// Cuerpo de `DELETE /v1/accounts`: la contraseña confirma que quien borra es el titular.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountDeletion {
    pub password: String,
}

/// Cuerpo de `POST /v1/recovery`: pide al administrador restablecer la contraseña. `contact` es
/// el medio por el que verificará la identidad antes de actuar, como el correo institucional.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRequest {
    pub username: String,
    pub contact: String,
}

/// Motivo por el que un contacto de recuperación no sirve, o `None` si es válido.
#[must_use]
pub fn contact_problem(contact: &str) -> Option<&'static str> {
    let contact = contact.trim();
    if contact.is_empty() {
        Some("Indica un contacto, como tu correo institucional.")
    } else if contact.len() > 254 {
        Some("El contacto admite hasta 254 bytes.")
    } else if contact.chars().any(char::is_control) {
        Some("El contacto no admite caracteres de control.")
    } else {
        None
    }
}

/// Cuerpo de `POST /v1/password`; al aplicarse, el servidor cierra todas las sesiones.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordChange {
    pub current: String,
    pub new: String,
}

/// Motivo por el que una contraseña nueva no se acepta, o `None` si sirve. Se aplica al
/// registrar y al cambiarla; las contraseñas anteriores siguen sirviendo para entrar.
#[must_use]
pub fn password_problem(password: &str) -> Option<&'static str> {
    if password.chars().count() < 8 {
        Some("La contraseña debe tener al menos 8 caracteres.")
    } else if password.len() > 1024 {
        Some("La contraseña admite hasta 1024 bytes.")
    } else if password.chars().any(char::is_control) {
        Some("La contraseña no admite caracteres de control.")
    } else {
        None
    }
}

#[derive(Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub account_id: Id,
    pub username: String,
    pub state: Snapshot,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub workspaces: Vec<Workspace>,
    pub entries: Vec<Entry>,
    pub directory: Directory,
    pub preferences: Preferences,
    pub next: Option<Cursor>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cursor {
    pub workspace: Id,
    pub entry: Id,
    pub person: Id,
    pub offer: Id,
    pub version: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Id,
    pub title: String,
    pub revision: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: Id,
    pub workspace: Id,
    pub parent: Option<Id>,
    pub title: String,
    pub kind: EntryKind,
    pub revision: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Note,
    Folder,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: Id,
    pub markdown: String,
    pub revision: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Preferences {
    pub revision: i64,
    pub settings: Settings,
}

/// Preferencias compartidas por cuenta.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "enabled")]
    pub autosave: bool,
    #[serde(default)]
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            autosave: true,
            theme: Theme::Terminal,
        }
    }
}

fn enabled() -> bool {
    true
}

/// Esquema de colores de la interfaz; `Terminal` hereda los colores del emulador.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Terminal,
    Nord,
    TokyoNight,
    Dracula,
    GruvboxDark,
    SolarizedDark,
    SolarizedLight,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Directory {
    pub people: Vec<Person>,
    pub incoming: Vec<Incoming>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Person {
    pub id: Id,
    pub name: String,
    pub presence: Presence,
    pub contact: Contact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Active,
    Away,
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Contact {
    None,
    Pending,
    Accepted,
    Incoming(Id),
    Rejected,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Incoming {
    pub id: Id,
    pub sender: Id,
    pub title: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Draft {
    pub id: Id,
    pub revision: i64,
    pub markdown: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mutation {
    pub operation_id: String,
    pub command: Command,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    CreateWorkspace {
        title: String,
    },
    RenameWorkspace {
        id: Id,
        revision: i64,
        title: String,
    },
    CopyWorkspace {
        id: Id,
        revision: i64,
        title: String,
        drafts: Vec<Draft>,
    },
    DeleteWorkspace {
        id: Id,
        revision: i64,
    },
    CreateEntry {
        workspace: Id,
        parent: Option<Id>,
        title: String,
        kind: EntryKind,
    },
    RenameEntry {
        id: Id,
        revision: i64,
        title: String,
    },
    MoveEntry {
        id: Id,
        revision: i64,
        parent: Option<Id>,
    },
    CopyEntry {
        id: Id,
        revision: i64,
        parent: Option<Id>,
        drafts: Vec<Draft>,
    },
    DeleteEntry {
        id: Id,
        revision: i64,
    },
    Save {
        id: Id,
        revision: i64,
        markdown: String,
    },
    Contact {
        recipient: Id,
    },
    ResolveContact {
        id: Id,
        resolution: Resolution,
    },
    Offer {
        id: Id,
        revision: i64,
        recipient: Id,
    },
    ResolveTransfer {
        id: Id,
        destination: Option<Destination>,
    },
    Preferences {
        revision: i64,
        settings: Settings,
    },
    /// Sugerencia para quien administra Anotau; anónima no guarda la cuenta ni un recibo.
    Suggest {
        text: String,
        anonymous: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Accept,
    Reject,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Destination {
    pub workspace: Id,
    pub parent: Option<Id>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Outcome {
    Changed {
        workspace: Option<Id>,
        entry: Option<Id>,
    },
    Saved {
        id: Id,
        revision: i64,
        at: i64,
        workspace_revision: i64,
    },
    Delivered {
        transfer: Id,
        note: Id,
    },
}

#[derive(Serialize, Deserialize)]
pub struct Change {
    pub outcome: Outcome,
    pub state: Option<Snapshot>,
}

#[derive(Serialize, Deserialize)]
pub struct Activity {
    pub idle_seconds: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Error {
    pub message: String,
}

/// Motivo por el que un nombre no sirve para registrar una cuenta, o `None` si es válido.
///
/// La regla es `[a-z0-9][a-z0-9._-]{2,31}`, solo ASCII, para que dos cuentas no puedan verse
/// iguales con letras de otro alfabeto (`carla` frente a `cаrla` con «а» cirílica). Cada
/// comprobación tiene su mensaje, para que el formulario diga qué corregir. Las cuentas
/// anteriores conservan su nombre.
#[must_use]
pub fn username_problem(name: &str) -> Option<&'static str> {
    if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Some("El usuario no admite mayúsculas.");
    }
    if !name.is_ascii() {
        return Some("El usuario no admite tildes, ñ ni letras de otros alfabetos.");
    }
    if name.contains(' ') {
        return Some("El usuario no admite espacios.");
    }
    if name
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    {
        return Some("El usuario solo admite punto, guion y guion bajo como símbolos.");
    }
    if name
        .bytes()
        .next()
        .is_some_and(|first| !first.is_ascii_alphanumeric())
    {
        return Some("El usuario debe empezar por una letra o un dígito.");
    }
    if !(3..=32).contains(&name.len()) {
        return Some("El usuario debe tener de 3 a 32 caracteres.");
    }
    None
}

/// Identifica controles C1 que algunos emuladores interpretan como secuencias de terminal.
#[must_use]
pub fn is_c1(ch: char) -> bool {
    matches!(ch, '\u{0080}'..='\u{009f}')
}

#[cfg(test)]
mod tests {
    use super::username_problem;

    fn valid_username(name: &str) -> bool {
        username_problem(name).is_none()
    }

    #[test]
    fn usernames_are_lowercase_ascii_with_simple_separators() {
        for name in [
            "carla.rojas",
            "test1",
            "prueba-crud",
            "a_b",
            "123",
            &"a".repeat(32),
        ] {
            assert!(valid_username(name), "{name}");
        }
        for name in [
            "ab",
            &"a".repeat(33),
            "Carla",
            "cаrla",
            "josé",
            ".carla",
            "-carla",
            "carla rojas",
            "carla@usm",
            "x');--",
        ] {
            assert!(!valid_username(name), "{name}");
        }
        for (name, problem) in [
            ("Carla", "mayúsculas"),
            ("josé", "tildes"),
            ("cаrla", "otros alfabetos"),
            ("carla rojas", "espacios"),
            ("carla@usm", "punto, guion y guion bajo"),
            (".carla", "empezar"),
            ("ab", "de 3 a 32"),
        ] {
            assert!(username_problem(name).unwrap().contains(problem), "{name}");
        }
    }
}
