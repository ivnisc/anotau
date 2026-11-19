mod commands;
mod deletion;
mod export;
mod files;
mod password;
mod receive;
mod spaces;
mod suggestion;
mod transfer;

pub(crate) use commands::{Command, OPEN_HINT, opens_palette};
use deletion::Deletion;
use export::Export;
use password::PasswordChange;
pub(crate) use receive::Destination;
pub(crate) use spaces::{SpaceAction, SpaceRequest};
use suggestion::Suggestion;
use transfer::Transfer;
pub(crate) use transfer::TransferRequest;

use rat_text::{
    HasScreenCursor,
    event::TextOutcome,
    text_input::{TextInput, TextInputState, handle_events},
};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{ListItem, ListState, Paragraph, StatefulWidget, Wrap},
};

use anotau_protocol::EntryKind;

use crate::{
    popup,
    workspace::{EntryId, FileAction, FolderId, NoteId, WorkspaceId},
};

pub(crate) enum Action {
    KeepOpen,
    Close,
    Run(Command),
    Setting(Setting),
    Theme(crate::theme::Theme),
    File(FileRequest),
    Space(SpaceRequest),
    Transfer(TransferRequest),
    Create {
        parent: Option<FolderId>,
        title: String,
        kind: EntryKind,
    },
    Suggest {
        text: String,
        anonymous: bool,
    },
    ChangePassword {
        current: String,
        new: String,
    },
    Export {
        source: EntryId,
        directory: String,
    },
    /// Confirmado con la contraseña: borrar la cuenta y todos sus datos.
    DeleteAccount {
        password: String,
    },
}

pub(crate) enum FileRequest {
    Move {
        source: EntryId,
        destination: Option<FolderId>,
    },
    Copy {
        source: EntryId,
        destination: Option<FolderId>,
    },
    Delete(EntryId),
    SaveAndClose(NoteId),
    Rename {
        source: EntryId,
        title: String,
    },
}

/// Ajustes de F3: preferencias de la cuenta y de la sesión que no son comandos habituales.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Setting {
    Autosave,
    Mouse,
    ResizeTree,
    ResizeDocument,
    Theme,
    Password,
    Suggestions,
    DeleteAccount,
}

impl Setting {
    const ALL: [Self; 8] = [
        Self::Autosave,
        Self::Mouse,
        Self::ResizeTree,
        Self::ResizeDocument,
        Self::Theme,
        Self::Password,
        Self::Suggestions,
        Self::DeleteAccount,
    ];

    fn position(self) -> Option<usize> {
        Self::ALL.iter().position(|&setting| setting == self)
    }
}

/// Valores vigentes que muestra la configuración.
pub(crate) struct Settings {
    pub(crate) autosave: bool,
    pub(crate) mouse: bool,
    pub(crate) theme: crate::theme::Theme,
    /// Remitente que firma una sugerencia no anónima.
    pub(crate) username: String,
}

impl Settings {
    fn label(&self, setting: Setting) -> String {
        let state = |enabled: bool, on: &str, off: &str| if enabled { on } else { off }.to_owned();
        match setting {
            Setting::Autosave => format!(
                "Autoguardado · {}",
                state(self.autosave, "activado", "desactivado")
            ),
            Setting::Mouse => format!(
                "Captura de ratón · {}",
                state(self.mouse, "activada", "desactivada")
            ),
            Setting::ResizeTree => "Ajustar ancho del árbol".into(),
            Setting::ResizeDocument => "Ajustar ancho de editor y vista previa".into(),
            Setting::Theme => format!("Tema · {}", crate::theme::name(self.theme)),
            Setting::Password => "Cambiar contraseña".into(),
            Setting::Suggestions => "Sugerencias".into(),
            Setting::DeleteAccount => "Borrar datos y cuenta permanentemente".into(),
        }
    }
}

/// Teclas por contexto; F1 y F3 cubren lo demás.
const HELP: &[(&str, &str)] = &[
    ("General", ""),
    ("F1 / Ctrl+P", "comandos"),
    ("F2", "mostrar, enfocar u ocultar el árbol"),
    ("F3", "configuración"),
    ("Ctrl+S · Ctrl+Q", "guardar · salir"),
    ("Editor", ""),
    ("Enter · Esc", "escribir · volver a acciones"),
    ("← →", "cambiar de panel"),
    ("↑ ↓ · 1–9", "cambiar de nota abierta"),
    ("Q", "cerrar la nota; sin nota, cerrar sesión o salir"),
    (
        "C M D R T E",
        "copiar, mover, eliminar, renombrar, transferir, exportar",
    ),
    ("Ctrl+Z · Ctrl+Mayús+Z", "deshacer · rehacer"),
    ("/table{c}{f} Enter", "insertar una tabla"),
    ("Árbol", ""),
    ("↑ ↓ · Inicio · Fin", "recorrer"),
    ("Enter", "ver nota o plegar carpeta"),
    ("Mayús+← →", "plegar o desplegar"),
    ("Tab · W", "cambiar de espacio · gestionar espacios"),
    ("N", "crear nota o carpeta"),
    ("Usuarios", ""),
    ("T", "transferir una nota o pedir contacto"),
    ("Enter", "revisar archivos o solicitudes recibidas"),
    ("Vista previa", ""),
    ("Tab · Enter", "elegir y abrir enlaces"),
    ("Mayús+← →", "desplazar tablas y código"),
];

/// Opciones de Q fuera de una nota: volver a la bienvenida o terminar el programa.
const LEAVE: [(Command, &str); 2] = [
    (Command::SignOut, "Cerrar sesión"),
    (Command::Exit, "Salir de Anotau"),
];

enum Mode {
    Search,
    Help,
    Leave,
    Settings(Settings),
    Themes(Settings),
    Create {
        parent: Option<FolderId>,
        destination: String,
        kind: EntryKind,
    },
    File {
        action: FileAction,
        source: EntryId,
        path: String,
        destinations: Vec<(Option<FolderId>, String)>,
        has_unsaved_changes: bool,
        /// Entradas directas de una carpeta; el servidor solo elimina carpetas vacías.
        contents: usize,
    },
    Spaces {
        entries: Vec<(WorkspaceId, String)>,
        action: SpaceAction,
        columns: usize,
    },
    Transfer(Transfer),
    Suggestion(Box<Suggestion>),
    DeleteAccount(Box<Deletion>),
    Password(Box<PasswordChange>),
    Export(Box<Export>),
}

pub(crate) struct Palette {
    input: TextInputState,
    mode: Mode,
    list: ListState,
    error: Option<String>,
    opening_key: Option<char>,
}

impl Palette {
    pub(crate) fn new() -> Self {
        let input = TextInputState::new();
        input.focus.set(true);
        Self {
            input,
            mode: Mode::Search,
            list: ListState::default().with_selected(Some(0)),
            error: None,
            opening_key: None,
        }
    }

    pub(crate) fn help() -> Self {
        Self {
            mode: Mode::Help,
            ..Self::new()
        }
    }

    pub(crate) fn leave() -> Self {
        Self {
            mode: Mode::Leave,
            opening_key: Some('q'),
            ..Self::new()
        }
    }

    pub(crate) fn settings(settings: Settings) -> Self {
        Self {
            mode: Mode::Settings(settings),
            ..Self::new()
        }
    }

    pub(crate) fn create_entry(parent: Option<FolderId>, destination: String) -> Self {
        Self {
            mode: Mode::Create {
                parent,
                destination,
                kind: EntryKind::Note,
            },
            opening_key: Some('n'),
            ..Self::new()
        }
    }

    pub(crate) fn ignore_opening_repeat(&mut self, key: KeyEvent) -> bool {
        if let KeyCode::Char(ch) = key.code {
            if self.opening_key == Some(ch.to_ascii_lowercase()) {
                if key.kind == KeyEventKind::Press {
                    self.opening_key = None;
                }
                return key.kind == KeyEventKind::Repeat && self.opening_key.is_some();
            }
        }
        false
    }

    pub(crate) fn show_error(&mut self, message: &str) {
        self.error = Some(message.into());
    }

    fn results(&self) -> Vec<Command> {
        let query = self.input.text().to_lowercase();
        let mut matches = Command::ALL
            .into_iter()
            .filter_map(|command| {
                let label = command.label().to_lowercase();
                let keywords = command.keywords();
                query
                    .split_whitespace()
                    .all(|term| label.contains(term) || keywords.contains(term))
                    .then_some((!label.starts_with(query.trim()), command))
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|&(later, _)| later);
        matches.into_iter().map(|(_, command)| command).collect()
    }

    pub(crate) fn handle(&mut self, event: &Event) -> Action {
        if let Event::Key(key) = event {
            match (key.code, key.modifiers) {
                (KeyCode::Esc, KeyModifiers::NONE) => {
                    // tras cambiarla, la sesión ya no existe: Esc también lleva a iniciarla
                    if matches!(&self.mode, Mode::Password(change) if change.done()) {
                        return Action::Run(Command::SignOut);
                    }
                    return if self.spaces_back() || self.receive_back() || self.settings_back() {
                        Action::KeepOpen
                    } else {
                        Action::Close
                    };
                }
                (KeyCode::Left | KeyCode::Right, KeyModifiers::NONE) => {
                    if let Mode::Create { kind, .. } = &mut self.mode {
                        *kind = if key.code == KeyCode::Left {
                            EntryKind::Note
                        } else {
                            EntryKind::Folder
                        };
                        self.error = None;
                        return Action::KeepOpen;
                    }
                }
                (KeyCode::Enter, KeyModifiers::NONE) if key.kind == KeyEventKind::Press => {
                    return self.confirm();
                }
                (KeyCode::Up | KeyCode::Down, KeyModifiers::NONE)
                    if matches!(
                        self.mode,
                        Mode::Search
                            | Mode::Settings(_)
                            | Mode::Themes(_)
                            | Mode::Help
                            | Mode::Leave
                    ) =>
                {
                    let count = match self.mode {
                        Mode::Help => HELP.len(),
                        Mode::Leave => LEAVE.len(),
                        Mode::Settings(_) => Setting::ALL.len(),
                        Mode::Themes(_) => crate::theme::ALL.len(),
                        _ => self.results().len(),
                    };
                    crate::selection::navigate(&mut self.list, key.code, count);
                    return Action::KeepOpen;
                }
                _ => {}
            }
        }
        if let Some(action) = self.handle_file(event) {
            return action;
        }
        if let Some(action) = self.handle_spaces(event) {
            return action;
        }
        if let Some(action) = self.handle_transfer(event) {
            return action;
        }
        if let Some(action) = self.handle_suggestion(event) {
            return action;
        }
        if let Some(action) = self.handle_deletion(event) {
            return action;
        }
        if let Some(action) = self.handle_password(event) {
            return action;
        }
        if let Some(action) = self.handle_export() {
            return action;
        }
        if matches!(
            self.mode,
            Mode::Settings(_) | Mode::Themes(_) | Mode::Help | Mode::Leave
        ) {
            return Action::KeepOpen;
        }
        if matches!(
            handle_events(&mut self.input, true, event),
            TextOutcome::TextChanged
        ) {
            if matches!(self.mode, Mode::Search) {
                self.list.select(Some(0));
                *self.list.offset_mut() = 0;
            }
            self.error = None;
        }
        Action::KeepOpen
    }

    fn settings_back(&mut self) -> bool {
        let mode = std::mem::replace(&mut self.mode, Mode::Search);
        let (mode, back) = match mode {
            Mode::Themes(settings) => {
                self.list.select(Setting::Theme.position());
                (Mode::Settings(settings), true)
            }
            Mode::Password(change) => {
                self.list.select(Setting::Password.position());
                self.error = None;
                (Mode::Settings((*change).into_settings()), true)
            }
            Mode::DeleteAccount(deletion) => {
                self.list.select(Setting::DeleteAccount.position());
                self.error = None;
                (Mode::Settings((*deletion).into_settings()), true)
            }
            Mode::Suggestion(suggestion) => {
                self.list.select(Setting::Suggestions.position());
                self.error = None;
                (Mode::Settings((*suggestion).into_settings()), true)
            }
            other => (other, false),
        };
        self.mode = mode;
        back
    }

    fn confirm(&mut self) -> Action {
        match self.mode {
            Mode::Help => Action::Close,
            Mode::Leave => Action::Run(LEAVE[self.list.selected().unwrap_or_default()].0),
            Mode::Search => self
                .results()
                .get(self.list.selected().unwrap_or_default())
                .copied()
                .map_or(Action::KeepOpen, Action::Run),
            Mode::Settings(_) => {
                let setting = Setting::ALL[self.list.selected().unwrap_or_default()];
                if !matches!(
                    setting,
                    Setting::Theme
                        | Setting::Password
                        | Setting::Suggestions
                        | Setting::DeleteAccount
                ) {
                    return Action::Setting(setting);
                }
                let Mode::Settings(settings) = std::mem::replace(&mut self.mode, Mode::Search)
                else {
                    unreachable!("modo comprobado arriba");
                };
                if setting == Setting::Suggestions {
                    self.mode = Mode::Suggestion(Box::new(Suggestion::new(settings)));
                    return Action::KeepOpen;
                }
                if setting == Setting::Password {
                    self.mode = Mode::Password(Box::new(PasswordChange::new(settings)));
                    return Action::KeepOpen;
                }
                if setting == Setting::DeleteAccount {
                    self.mode = Mode::DeleteAccount(Box::new(Deletion::new(settings)));
                    return Action::KeepOpen;
                }
                let current = crate::theme::ALL
                    .iter()
                    .position(|&theme| theme == settings.theme);
                self.list.select(current);
                self.mode = Mode::Themes(settings);
                Action::KeepOpen
            }
            Mode::Themes(_) => crate::theme::ALL
                .get(self.list.selected().unwrap_or_default())
                .copied()
                .map_or(Action::KeepOpen, Action::Theme),
            Mode::Create { parent, kind, .. } => Action::Create {
                parent,
                kind,
                title: self.input.text().into(),
            },
            Mode::File { .. } => self.confirm_file(),
            Mode::Spaces { .. } => self.confirm_space(),
            Mode::Transfer(_) => self.confirm_transfer(),
            Mode::Suggestion(_) => self.confirm_suggestion(),
            Mode::DeleteAccount(_) => self.confirm_deletion(),
            Mode::Password(_) => self.confirm_password(),
            Mode::Export(_) => self.confirm_export(),
        }
    }

    fn presentation(&self) -> (&'static str, u16, &'static str, &'static str) {
        match self.mode {
            Mode::Search => (
                "Comandos",
                17,
                "Buscar un comando…",
                "↑↓ elegir  ·  Enter ejecutar  ·  Esc cerrar",
            ),
            Mode::Help => (
                "Ayuda",
                29,
                "Atajos de teclado",
                "↑↓ desplazar  ·  Esc cerrar",
            ),
            Mode::Leave => (
                "Salir",
                6,
                "Cerrar sesión o salir de Anotau",
                "↑↓ elegir  ·  Enter confirmar  ·  Esc cancelar",
            ),
            Mode::Settings(_) => (
                "Configuración",
                // una fila por opción, más etiqueta, separaciones y teclas
                u16::try_from(Setting::ALL.len()).unwrap_or(u16::MAX) + 5,
                "Preferencias de la cuenta y de esta sesión",
                "↑↓ elegir  ·  Enter cambiar  ·  Esc cerrar",
            ),
            Mode::Themes(_) => (
                "Tema",
                12,
                "Se guarda en la cuenta",
                "↑↓ elegir  ·  Enter aplicar  ·  Esc volver",
            ),
            Mode::Create { kind, .. } => (
                match kind {
                    EntryKind::Note => "Crear nota",
                    EntryKind::Folder => "Crear carpeta",
                },
                7,
                match kind {
                    EntryKind::Note => "Título de la nota",
                    EntryKind::Folder => "Nombre de la carpeta",
                },
                "Enter crear · Esc cancelar · ← nota · → carpeta",
            ),
            Mode::File { .. } => self.file_presentation(),
            Mode::Spaces { action, .. } => action.presentation(),
            Mode::Transfer(_) => self.transfer_presentation(),
            Mode::Export(ref export) => ("Exportar", 7, "Carpeta de destino", export.hint()),
            Mode::Password(ref change) => (
                "Cambiar contraseña",
                change.height(),
                "Se cerrarán todas tus sesiones",
                change.hint(),
            ),
            Mode::DeleteAccount(ref deletion) => (
                "Borrar cuenta",
                deletion.height(),
                "¿Estás seguro de abandonar Anotau?",
                deletion.hint(),
            ),
            Mode::Suggestion(ref suggestion) => (
                "Sugerencias",
                14,
                "Comparte tus sugerencias para mejorar!",
                suggestion.hint(),
            ),
        }
    }

    /// `status` reemplaza las teclas del pie mientras la acción confirmada está en curso.
    pub(crate) fn draw(&mut self, frame: &mut Frame<'_>, status: Option<Line<'static>>) {
        let (title, height, placeholder, instructions) = self.presentation();
        let width = if matches!(
            self.mode,
            Mode::Help
                | Mode::Spaces {
                    action: SpaceAction::Browse,
                    ..
                }
        ) {
            72
        } else {
            58
        };
        // dos líneas en blanco: bajo la etiqueta y sobre las teclas
        let inner = popup::draw(frame, title, width, height + 2);
        if inner.is_empty() {
            return;
        }

        let [prompt, _, content, _, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        self.draw_prompt(frame, prompt, placeholder);
        let [content, error_area] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(if self.error.is_some() { 2 } else { 0 }),
        ])
        .areas(content);

        match &self.mode {
            Mode::Search => self.draw_results(frame, content, &self.results()),
            Mode::Settings(_) | Mode::Themes(_) | Mode::Leave => {
                self.draw_choices(frame, content);
            }
            Mode::Help => self.draw_help(frame, content),
            Mode::Create { destination, .. } => {
                frame.render_widget(
                    Paragraph::new(format!("Destino · {destination}"))
                        .style(Style::new().dim())
                        .wrap(Wrap { trim: true }),
                    content,
                );
            }
            Mode::File { .. } => self.draw_file(frame, content),
            Mode::Spaces { .. } => self.draw_spaces(frame, content),
            Mode::Transfer(_) => self.draw_transfer(frame, content),
            Mode::Suggestion(_) => self.draw_suggestion(frame, content),
            Mode::DeleteAccount(_) => self.draw_deletion(frame, content),
            Mode::Password(_) => self.draw_password(frame, content),
            Mode::Export(_) => self.draw_export(frame, content),
        }
        if let Some(error) = &self.error {
            frame.render_widget(
                Paragraph::new(error.as_str())
                    .style(Style::new().fg(crate::theme::warning()))
                    .wrap(Wrap { trim: true }),
                error_area,
            );
        }
        frame.render_widget(
            status.unwrap_or_else(|| Line::styled(instructions, Style::new().dim())),
            hint,
        );
    }

    /// Primera fila: campo de texto, o una etiqueta en los popups sin entrada.
    fn draw_prompt(&mut self, frame: &mut Frame<'_>, prompt: Rect, placeholder: &str) {
        if matches!(
            self.mode,
            Mode::Transfer(_)
                | Mode::Settings(_)
                | Mode::Themes(_)
                | Mode::Help
                | Mode::Leave
                | Mode::Suggestion(_)
                | Mode::DeleteAccount(_)
                | Mode::Password(_)
        ) || matches!(&self.mode, Mode::Export(export) if export.done())
        {
            frame.render_widget(Span::styled(placeholder, Style::new().dim()), prompt);
        } else if matches!(
            self.mode,
            Mode::Spaces {
                action: SpaceAction::Browse | SpaceAction::Delete,
                ..
            }
        ) {
            frame.render_widget(
                Span::styled("Espacios de esta cuenta", Style::new().dim()),
                prompt,
            );
        } else if let Mode::File {
            action: FileAction::Move | FileAction::Copy | FileAction::Delete | FileAction::Close,
            path,
            ..
        } = &self.mode
        {
            frame.render_widget(
                Paragraph::new(path.as_str()).style(Style::new().dim()),
                prompt,
            );
        } else {
            TextInput::new().render(prompt, frame.buffer_mut(), &mut self.input);
            if self.input.is_empty() {
                frame.render_widget(Span::styled(placeholder, Style::new().dim()), prompt);
            }
            if let Some(cursor) = self.input.screen_cursor() {
                frame.set_cursor_position(cursor);
            }
        }
    }

    fn draw_help(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let width = HELP
            .iter()
            .map(|(keys, _)| Span::raw(*keys).width())
            .max()
            .unwrap_or_default();
        // ↑↓ desplazan el texto: la selección de la lista es la primera fila visible
        let visible = usize::from(area.height.max(1));
        let top = self
            .list
            .selected()
            .unwrap_or_default()
            .min(HELP.len().saturating_sub(visible));
        self.list.select(Some(top));
        let lines = HELP
            .iter()
            .skip(top)
            .take(visible)
            .map(|&(keys, action)| {
                if action.is_empty() {
                    Line::styled(keys, Style::new().fg(crate::theme::accent()).bold())
                } else {
                    Line::from(vec![
                        Span::raw(format!("  {keys:<width$}  ")),
                        Span::styled(action, Style::new().dim()),
                    ])
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn draw_choices(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let items: Vec<ListItem<'_>> = match &self.mode {
            Mode::Leave => LEAVE
                .iter()
                .map(|&(_, label)| ListItem::new(label))
                .collect(),
            Mode::Settings(settings) => Setting::ALL
                .iter()
                .map(|&setting| ListItem::new(settings.label(setting)))
                .collect(),
            Mode::Themes(settings) => crate::theme::ALL
                .iter()
                .map(|&theme| {
                    let mark = if theme == settings.theme {
                        " · actual"
                    } else {
                        ""
                    };
                    ListItem::new(format!("{}{mark}", crate::theme::name(theme)))
                })
                .collect(),
            _ => return,
        };
        frame.render_stateful_widget(popup::list(items), area, &mut self.list);
    }

    fn draw_results(&mut self, frame: &mut Frame<'_>, area: Rect, results: &[Command]) {
        if results.is_empty() {
            frame.render_widget(Span::styled("Sin resultados", Style::new().dim()), area);
            return;
        }
        let items = results
            .iter()
            .map(|command| ListItem::new(command.label()))
            .collect::<Vec<_>>();
        frame.render_stateful_widget(popup::list(items), area, &mut self.list);
        // solo el comando elegido muestra su tecla, alineada a la derecha de su fila
        let Some(selected) = self.list.selected() else {
            return;
        };
        let Some(hint) = results.get(selected).and_then(|command| command.hint()) else {
            return;
        };
        let row = selected.saturating_sub(self.list.offset());
        let label = 2 + Span::raw(results[selected].label()).width();
        let width = Span::raw(hint).width();
        if let Ok(row) = u16::try_from(row)
            && row < area.height
            && usize::from(area.width) >= label + width + 2
        {
            frame.render_widget(
                Line::styled(hint, Style::new().dim()).right_aligned(),
                Rect::new(area.x, area.y + row, area.width, 1),
            );
        }
    }
}
