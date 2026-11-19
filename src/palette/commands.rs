use crate::workspace::FileAction;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) const OPEN_HINT: &str = if cfg!(target_os = "macos") {
    "F1"
} else {
    "Ctrl+P / F1"
};

pub(crate) fn opens_palette(key: KeyEvent) -> bool {
    (key.code == KeyCode::F(1) && key.modifiers.is_empty())
        || (matches!(key.code, KeyCode::Char('p' | 'P')) && action_key(key.modifiers))
}

fn action_key(modifiers: KeyModifiers) -> bool {
    matches!(
        modifiers,
        KeyModifiers::CONTROL | KeyModifiers::SUPER | KeyModifiers::META
    )
}

/// Acciones de la paleta; la lista es la misma desde cualquier panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    NewEntry,
    File(FileAction),
    Transfer,
    Export,
    Save,
    Undo,
    Redo,
    ToggleTree,
    Settings,
    SignOut,
    Exit,
    /// Solo atajo (F4); no se lista en la paleta.
    Help,
}

impl Command {
    pub(super) const ALL: [Self; 14] = [
        Self::NewEntry,
        Self::File(FileAction::Copy),
        Self::File(FileAction::Move),
        Self::File(FileAction::Delete),
        Self::File(FileAction::Rename),
        Self::Transfer,
        Self::Export,
        Self::Save,
        Self::Undo,
        Self::Redo,
        Self::ToggleTree,
        Self::Settings,
        Self::SignOut,
        Self::Exit,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::NewEntry => "Crear nota o carpeta",
            Self::File(FileAction::Copy) => "Copiar nota o carpeta",
            Self::File(FileAction::Move) => "Mover nota o carpeta",
            Self::File(FileAction::Delete) => "Eliminar nota o carpeta",
            Self::File(FileAction::Rename) => "Renombrar nota o carpeta",
            Self::File(FileAction::Close) => "Cerrar nota",
            Self::Transfer => "Transferir nota",
            Self::Export => "Exportar nota o carpeta",
            Self::Save => "Guardar nota",
            Self::Undo => "Deshacer",
            Self::Redo => "Rehacer",
            Self::ToggleTree => "Mostrar u ocultar árbol",
            Self::Settings => "Configuración",
            Self::Help => "Ayuda",
            Self::SignOut => "Cerrar sesión",
            Self::Exit => "Salir de Anotau",
        }
    }

    pub(super) fn keywords(self) -> &'static str {
        match self {
            Self::NewEntry => "nueva new archivo carpeta folder",
            Self::File(_) => "archivo carpeta nota arbol",
            Self::Transfer => "enviar compartir copia usuario contacto",
            Self::Export => "descargar guardar archivo md disco local",
            Self::Save => "save manual escritura",
            Self::Undo => "undo revertir",
            Self::Redo => "redo repetir",
            Self::ToggleTree => "arbol carpetas notas archivos explorador",
            Self::Settings => "opciones ajustes raton mouse ancho tema color autoguardado",
            Self::Help => "ayuda atajos teclas",
            Self::SignOut => "logout cuenta usuario",
            Self::Exit => "quit cerrar",
        }
    }

    /// Atajo global; las letras del árbol se resuelven en el editor según el foco.
    fn shortcut(self) -> Option<(KeyCode, KeyModifiers)> {
        use KeyModifiers as Mod;

        Some(match self {
            Self::NewEntry => (KeyCode::Char('n'), Mod::NONE),
            Self::Save => (KeyCode::Char('s'), Mod::CONTROL),
            Self::ToggleTree => (KeyCode::F(2), Mod::NONE),
            Self::Settings => (KeyCode::F(3), Mod::NONE),
            Self::Help => (KeyCode::F(4), Mod::NONE),
            Self::Undo => (KeyCode::Char('z'), Mod::CONTROL),
            Self::Redo => (KeyCode::Char('z'), Mod::CONTROL | Mod::SHIFT),
            Self::Exit => (KeyCode::Char('q'), Mod::CONTROL),
            Self::File(_) | Self::Transfer | Self::Export | Self::SignOut => return None,
        })
    }

    pub(crate) fn from_shortcut(key: KeyEvent) -> Option<Self> {
        if (key.code, key.modifiers) == (KeyCode::Enter, KeyModifiers::SHIFT) {
            return Some(Self::Save);
        }
        let (code, modifiers) = if matches!(key.code, KeyCode::Char('z' | 'Z'))
            && action_key(key.modifiers - KeyModifiers::SHIFT)
        {
            (
                KeyCode::Char('z'),
                KeyModifiers::CONTROL | (key.modifiers & KeyModifiers::SHIFT),
            )
        } else {
            (key.code, key.modifiers)
        };
        Self::ALL
            .into_iter()
            .chain([Self::Help])
            .find(|command| command.shortcut() == Some((code, modifiers)))
    }

    /// Tecla que se muestra junto al comando elegido.
    pub(super) fn hint(self) -> Option<&'static str> {
        Some(match self {
            Self::NewEntry => "N",
            Self::File(FileAction::Copy) => "C",
            Self::File(FileAction::Move) => "M",
            Self::File(FileAction::Delete) => "D",
            Self::File(FileAction::Rename) => "R",
            Self::File(FileAction::Close) => "Q",
            Self::Transfer => "T",
            Self::Export => "E",
            Self::Save => "Ctrl+S",
            Self::Undo => "Ctrl+Z",
            Self::Redo => "Ctrl+Mayús+Z",
            Self::ToggleTree => "F2",
            Self::Settings => "F3",
            Self::Help => "F4",
            Self::Exit => "Ctrl+Q",
            Self::SignOut => return None,
        })
    }
}
