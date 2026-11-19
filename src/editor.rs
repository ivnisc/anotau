mod buffer;
mod files;
#[cfg(test)]
pub(crate) mod fixtures;
mod markdown_input;
mod preview;
mod remote;
mod split;
mod transfers;
mod view;
mod workspace;

use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind},
    layout::Rect,
};

use crate::{
    Update,
    network::Job,
    palette::{Action, Command, Palette, Setting, opens_palette},
    workspace::{FileAction, Navigation, NoteId},
};

use buffer::NoteBuffer;
use split::{Split, Splits};
use workspace::{Selection, Workspace};

const TAB_SHORTCUTS: &str = "123456789";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Source,
    Writing,
    Preview,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sidebar {
    Hidden,
    Visible,
    Focused,
    Users,
}

impl Sidebar {
    fn focused(self) -> bool {
        matches!(self, Self::Focused | Self::Users)
    }
}

pub(crate) struct Editor {
    workspaces: Vec<Workspace>,
    workspace: usize,
    remote: remote::Remote,
    pane: Pane,
    preview_area: Rect,
    sidebar: Sidebar,
    room_for_sidebar: bool,
    palette: Option<Palette>,
    popup_context: Option<remote::PopupContext>,
    popup_sequence: u64,
    /// Escritura confirmada desde el popup abierto; lo anima y lo cierra al confirmarse.
    progress: Option<crate::progress::Progress>,
    splits: Splits,
    mouse_capture: bool,
    command_notice: Option<&'static str>,
    people: crate::people::People,
    incoming: Vec<crate::inbox::Incoming>,
    pub(crate) away: bool,
    /// Nombre de la cuenta; firma las sugerencias que no son anónimas.
    pub(crate) username: String,
    /// Última carpeta de exportación de la sesión; empieza en Descargas.
    export_directory: Option<String>,
}

impl Editor {
    fn with_workspaces(workspaces: Vec<Workspace>, remote: remote::Remote) -> Self {
        Self {
            workspaces,
            workspace: 0,
            remote,
            pane: Pane::Source,
            preview_area: Rect::ZERO,
            sidebar: Sidebar::Visible,
            room_for_sidebar: false,
            palette: None,
            popup_context: None,
            popup_sequence: 0,
            progress: None,
            splits: Splits::new(),
            mouse_capture: false,
            command_notice: None,
            people: crate::people::People::empty(),
            incoming: Vec::new(),
            away: false,
            username: String::new(),
            export_directory: None,
        }
    }

    fn workspace(&self) -> &Workspace {
        &self.workspaces[self.workspace]
    }

    fn note(&self) -> Option<&NoteBuffer> {
        self.workspace().note()
    }

    fn note_mut(&mut self) -> Option<&mut NoteBuffer> {
        self.workspaces[self.workspace].note_mut()
    }

    pub(crate) fn handle(&mut self, event: Event) -> Update {
        // un aviso de comando dura hasta la siguiente entrada, no hasta el siguiente dibujo
        let input = match &event {
            Event::Key(key) => key.kind != KeyEventKind::Release,
            Event::Paste(_) => true,
            Event::Mouse(mouse) => matches!(mouse.kind, MouseEventKind::Down(_)),
            _ => false,
        };
        if input {
            self.command_notice = None;
        }
        let opening = self.palette.is_none();
        let update = self.handle_event(event);
        if self.palette.is_none() {
            self.popup_context = None;
            self.progress = None;
        } else if opening {
            self.popup_sequence = self.popup_sequence.saturating_add(1);
            self.popup_context = Some(remote::PopupContext::capture(self));
        }
        update
    }

    fn handle_event(&mut self, mut event: Event) -> Update {
        if matches!(event, Event::Resize(..) | Event::FocusLost) {
            self.splits.stop_drag();
            return Update::Draw;
        }
        if self.splits.keyboard_target().is_some() {
            return match event {
                Event::Key(key) if self.splits.keyboard(key) => Update::Draw,
                _ => Update::Idle,
            };
        }
        if let Event::Paste(text) = &mut event {
            *text = buffer::normalize_line_breaks(std::mem::take(text));
        }
        if let Event::Paste(text) = &mut event
            && text.chars().any(anotau_protocol::is_c1)
        {
            // rat-text protege C0 al mostrarlo, pero puede dejar pasar C1.
            *text = text
                .chars()
                .map(|ch| {
                    if anotau_protocol::is_c1(ch) {
                        '�'
                    } else {
                        ch
                    }
                })
                .collect();
        }
        if let Event::Key(key) = &event {
            if key.kind == KeyEventKind::Release {
                return Update::Idle;
            }
            if opens_palette(*key) {
                if key.kind == KeyEventKind::Repeat {
                    return Update::Idle;
                }
                self.splits.stop_drag();
                self.palette = self.palette.is_none().then(Palette::new);
                return Update::Draw;
            }
        }

        if self.progress.is_some() && self.palette.is_some() {
            // antirrebote: la acción ya va en camino; solo Esc cierra el popup
            if matches!(&event, Event::Key(key) if key.code == KeyCode::Esc) {
                self.palette = None;
                return Update::Draw;
            }
            return Update::Idle;
        }
        if let Some(palette) = &mut self.palette {
            if let Event::Key(key) = &mut event {
                if palette.ignore_opening_repeat(*key) {
                    return Update::Idle;
                }
                if key.code != KeyCode::Enter {
                    key.kind = KeyEventKind::Press;
                }
            }
            let action = palette.handle(&event);
            return self.apply_palette(action);
        }

        if self.mouse_capture
            && let Event::Mouse(mouse) = event
        {
            if let Some(draw) = self.splits.mouse(mouse) {
                return if draw { Update::Draw } else { Update::Idle };
            }
            if let Some(update) = self.preview_mouse(mouse) {
                return update;
            }
        }
        if let Event::Key(key) = event {
            if let Some(update) = self.shortcut(key) {
                return update;
            }
            if self.pane != Pane::Writing {
                return self.navigate(key);
            }
            if key.code == KeyCode::Enter && key.kind == KeyEventKind::Repeat {
                return Update::Idle;
            }
            if key.code == KeyCode::Esc && key.modifiers.is_empty() {
                if key.kind == KeyEventKind::Press {
                    self.focus(Pane::Source);
                    return Update::Draw;
                }
                return Update::Idle;
            }
        }
        if self.pane == Pane::Writing
            && let Some(note) = self.note_mut()
        {
            note.handle_input(event)
        } else {
            Update::Idle
        }
    }

    pub(crate) fn mouse_capture_changed(&mut self, enabled: Option<bool>) {
        self.splits.stop_drag();
        self.command_notice = Some(match enabled {
            Some(enabled) => {
                self.mouse_capture = enabled;
                if enabled {
                    "Captura de ratón activada"
                } else {
                    "Captura de ratón desactivada"
                }
            }
            None => "No se pudo cambiar la captura de ratón",
        });
    }

    pub(crate) fn suspend(&mut self) {
        self.pause_remote();
        self.palette = None;
        self.popup_context = None;
        self.splits.stop_drag();
        self.focus(Pane::Source);
        self.away = false;
    }

    pub(crate) fn link_opened(&mut self, result: Result<(), &'static str>) {
        self.command_notice = Some(
            result
                .err()
                .unwrap_or("Apertura solicitada a la aplicación predeterminada."),
        );
    }

    fn apply_palette(&mut self, action: Action) -> Update {
        let idle = !self.remote.has_pending();
        let update = self.apply_palette_action(action);
        if idle && self.palette.is_some() && !matches!(update, Update::Send(_)) {
            self.progress = self.remote.pending_command().map(|command| {
                crate::progress::Progress::start(command, std::time::Instant::now())
            });
        }
        update
    }

    fn apply_palette_action(&mut self, action: Action) -> Update {
        // las escrituras mantienen el popup hasta su confirmación
        if !matches!(
            action,
            Action::KeepOpen
                | Action::Create { .. }
                | Action::File(_)
                | Action::Space(_)
                | Action::Transfer(_)
                | Action::Theme(_)
                | Action::Suggest { .. }
                | Action::DeleteAccount { .. }
                | Action::ChangePassword { .. }
                | Action::Export { .. }
                | Action::Setting(Setting::Autosave)
        ) {
            self.palette = None;
        }
        match action {
            Action::Run(command) => self.run_command(command),
            Action::Setting(setting) => self.apply_setting(setting),
            Action::Theme(theme) => {
                let mut settings = self.remote.preferences.settings.clone();
                settings.theme = theme;
                self.save_settings(settings)
            }
            Action::File(request) => self.apply_file(request),
            Action::Space(request) => self.apply_space(&request),
            Action::Transfer(request) => self.apply_transfer(request),
            Action::Create {
                parent,
                title,
                kind,
            } => self.apply_create(parent, title, kind),
            Action::DeleteAccount { password } => {
                self.send("Eliminando cuenta…", Job::DeleteAccount { password })
            }
            Action::Export { source, directory } => self.apply_export(source, directory),
            Action::ChangePassword { current, new } => self.send(
                "Cambiando contraseña…",
                Job::ChangePassword { current, new },
            ),
            Action::Suggest { text, anonymous } => self.queue(
                anotau_protocol::Command::Suggest { text, anonymous },
                None,
                None,
            ),
            Action::Close | Action::KeepOpen => Update::Draw,
        }
    }

    /// Envía una petición fuera de la cola de escrituras y anima el popup hasta su respuesta.
    pub(super) fn send(&mut self, label: &'static str, job: Job) -> Update {
        self.progress = Some(crate::progress::Progress::account(
            label,
            std::time::Instant::now(),
        ));
        Update::Send(job)
    }

    fn shortcut(&mut self, key: KeyEvent) -> Option<Update> {
        // en navegación del documento, las letras de archivo actúan sobre la nota abierta
        let document = !self.sidebar.focused() && self.pane != Pane::Writing;
        // sin nota abierta, el editor no tiene objetivo: no se actúa sobre una entrada oculta
        let target = self.sidebar == Sidebar::Focused || (document && self.note().is_some());
        if key.code == KeyCode::Char('t')
            && key.modifiers.is_empty()
            && (self.sidebar == Sidebar::Users || target)
        {
            if key.kind == KeyEventKind::Press {
                self.target_active_note();
                self.start_transfer();
            }
            return Some(Update::Draw);
        }
        if key.code == KeyCode::Char('e') && key.modifiers.is_empty() && target {
            if key.kind == KeyEventKind::Press {
                self.target_active_note();
                return Some(self.start_export());
            }
            return Some(Update::Draw);
        }
        if key.code == KeyCode::Char('w')
            && key.modifiers.is_empty()
            && (self.pane != Pane::Writing || self.sidebar == Sidebar::Focused)
        {
            if key.kind == KeyEventKind::Press {
                self.palette = Some(Palette::spaces(
                    self.workspaces
                        .iter()
                        .map(|workspace| (workspace.tree.id(), workspace.tree.name.clone()))
                        .collect(),
                    self.workspace,
                ));
            }
            return Some(Update::Draw);
        }
        if target
            && key.modifiers.is_empty()
            && let KeyCode::Char(ch) = key.code
            && let Some(action) = FileAction::from_key(ch)
        {
            return Some(if key.kind == KeyEventKind::Press {
                self.target_active_note();
                self.start_file(action)
            } else {
                Update::Idle
            });
        }
        let command = Command::from_shortcut(key).filter(|command| match command {
            Command::NewEntry => self.sidebar == Sidebar::Focused || document,
            Command::Undo | Command::Redo => !self.sidebar.focused(),
            _ => true,
        });
        if let Some(command) = command {
            return Some(if key.kind == KeyEventKind::Press {
                self.run_command(command)
            } else {
                Update::Idle
            });
        }
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
            && key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL
        {
            if key.kind != KeyEventKind::Press {
                return Some(Update::Idle);
            }
            let backward =
                key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
            return Some(self.cycle_note(backward));
        }
        None
    }

    fn run_command(&mut self, command: Command) -> Update {
        if matches!(command, Command::Undo | Command::Redo)
            && self.note().is_some_and(NoteBuffer::loading)
        {
            return Update::Draw;
        }
        if matches!(self.workspace().active, Some(Selection::Temporary(_)))
            && matches!(command, Command::Undo | Command::Redo)
        {
            self.command_notice = Some("Pulsa Enter para editar esta nota");
            self.focus(Pane::Source);
            return Update::Draw;
        }
        match command {
            Command::Exit => Update::Exit,
            Command::SignOut => Update::SignOut,
            Command::Help => {
                self.palette = Some(Palette::help());
                Update::Draw
            }
            Command::NewEntry => {
                // una entrada nueva se ubica según el árbol, que se muestra si estaba oculto
                if self.sidebar == Sidebar::Hidden {
                    self.sidebar = Sidebar::Visible;
                }
                let (parent, destination) = self.workspace().tree.destination();
                self.palette = Some(Palette::create_entry(parent, destination));
                Update::Draw
            }
            Command::File(action) => {
                self.target_active_note();
                self.start_file(action)
            }
            Command::Transfer => {
                self.target_active_note();
                self.start_transfer();
                Update::Draw
            }
            Command::Export => {
                self.target_active_note();
                self.start_export()
            }
            Command::Save => self.save_active(false),
            Command::ToggleTree => {
                if self.sidebar.focused() {
                    self.sidebar = Sidebar::Hidden;
                    self.sync_source_focus();
                } else {
                    self.focus_tree();
                }
                Update::Draw
            }
            Command::Settings => {
                self.palette = Some(Palette::settings(crate::palette::Settings {
                    autosave: self.remote.preferences.settings.autosave,
                    mouse: self.mouse_capture,
                    theme: self.remote.preferences.settings.theme,
                    username: self.username.clone(),
                }));
                Update::Draw
            }
            Command::Undo | Command::Redo => {
                let Some(note) = self.note_mut() else {
                    return Update::Idle;
                };
                let changed = if matches!(command, Command::Undo) {
                    note.source.undo()
                } else {
                    note.source.redo()
                };
                if changed {
                    note.source.scroll_cursor_to_visible();
                    note.text_changed();
                }
                Update::Draw
            }
        }
    }

    fn apply_setting(&mut self, setting: Setting) -> Update {
        match setting {
            Setting::Autosave => {
                let mut settings = self.remote.preferences.settings.clone();
                settings.autosave = !settings.autosave;
                self.save_settings(settings)
            }
            Setting::Mouse => Update::ToggleMouseCapture,
            Setting::ResizeTree | Setting::ResizeDocument => {
                let split = if setting == Setting::ResizeTree {
                    Split::Tree
                } else {
                    Split::Document
                };
                if !self.splits.start_keyboard(split) {
                    self.command_notice = Some(match split {
                        Split::Tree => "Muestra el árbol en una terminal más ancha para ajustarlo",
                        Split::Document => "Amplía la terminal para ajustar ambos paneles",
                    });
                }
                Update::Draw
            }
            // el tema y las sugerencias abren su propio paso dentro de la paleta
            Setting::Theme | Setting::Password | Setting::Suggestions | Setting::DeleteAccount => {
                Update::Draw
            }
        }
    }

    /// Fuera del árbol, las acciones de archivo de la paleta actúan sobre la nota del editor.
    fn target_active_note(&mut self) {
        if self.sidebar.focused() {
            return;
        }
        if let Some(id) = self.note().map(|note| note.id) {
            self.workspaces[self.workspace]
                .tree
                .reveal_server(id.server());
        }
    }

    fn start_transfer(&mut self) {
        if self.sidebar == Sidebar::Users {
            if let Some(person) = self.people.selected() {
                self.palette = Some(Palette::transfer_person(
                    person.clone(),
                    self.workspace().tree.notes(),
                ));
            }
        } else if let Some(crate::workspace::EntryId::Note(note)) = self.workspace().tree.selected()
        {
            self.palette = Some(Palette::transfer_note(
                note,
                self.workspace().tree.title(note).into(),
                self.people.entries.clone(),
            ));
        } else {
            self.command_notice = Some("Selecciona una nota para transferir.");
        }
    }

    fn focus(&mut self, pane: Pane) {
        self.pane = pane;
        if self.sidebar.focused() {
            self.sidebar = Sidebar::Visible;
        }
        self.sync_source_focus();
    }

    fn focus_tree(&mut self) {
        if self.pane == Pane::Writing {
            self.pane = Pane::Source;
        }
        self.sidebar = Sidebar::Focused;
        self.sync_source_focus();
    }

    fn sync_source_focus(&self) {
        if let Some(note) = self.note() {
            note.source.focus.set(self.pane == Pane::Writing);
        }
    }

    fn navigate(&mut self, key: KeyEvent) -> Update {
        if let Some(update) = self.preview_key(key) {
            return update;
        }
        if self.sidebar == Sidebar::Focused && key.kind == KeyEventKind::Press {
            let backward = match (key.code, key.modifiers) {
                (KeyCode::Tab, KeyModifiers::NONE) => Some(false),
                (KeyCode::Tab, KeyModifiers::SHIFT)
                | (KeyCode::BackTab, KeyModifiers::NONE | KeyModifiers::SHIFT) => Some(true),
                _ => None,
            };
            if let Some(backward) = backward {
                let count = self.workspaces.len();
                let step = if backward { count - 1 } else { 1 };
                self.workspace = (self.workspace + step) % count;
                self.pane = Pane::Source;
                self.sync_source_focus();
                return Update::Draw;
            }
        }
        if key.modifiers.is_empty() && key.kind == KeyEventKind::Press {
            match key.code {
                KeyCode::Left | KeyCode::Right => return self.move_between_panes(key.code),
                // Q cierra la nota abierta; sin nota, o desde Notas y Usuarios, ofrece salir
                KeyCode::Char('q') => {
                    if !self.sidebar.focused() && self.note().is_some() {
                        return self.close_active_note();
                    }
                    self.palette = Some(Palette::leave());
                    return Update::Draw;
                }
                KeyCode::Enter
                    if self.pane == Pane::Source
                        && !self.sidebar.focused()
                        && self.note().is_some() =>
                {
                    if self.note().is_some_and(NoteBuffer::loading) {
                        return Update::Draw;
                    }
                    self.workspaces[self.workspace].keep_note();
                    self.focus(Pane::Writing);
                    return Update::Draw;
                }
                KeyCode::Char(slot) if !self.sidebar.focused() => {
                    if let Some(index) = TAB_SHORTCUTS
                        .find(slot)
                        .filter(|&index| index < self.workspace().notes.len())
                    {
                        self.activate(index);
                        return Update::Draw;
                    }
                    return Update::Idle;
                }
                _ => {}
            }
        }

        let modifiers = if matches!(key.code, KeyCode::Left | KeyCode::Right) {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        if key.modifiers != modifiers
            || (key.code == KeyCode::Enter && key.kind != KeyEventKind::Press)
        {
            return Update::Idle;
        }
        if self.sidebar == Sidebar::Users {
            if key.code == KeyCode::Enter {
                return self.review_incoming();
            }
            if key.code == KeyCode::Esc {
                self.focus(Pane::Source);
                return Update::Draw;
            }
            if self.people.navigate(key.code) {
                Update::Draw
            } else {
                Update::Idle
            }
        } else if self.sidebar == Sidebar::Focused {
            self.navigate_tree(key.code)
        } else if self.pane == Pane::Preview {
            let area = self.preview_area;
            self.note_mut()
                .map_or(Update::Idle, |note| note.scroll_preview(key.code, area))
        } else if self.pane == Pane::Source && matches!(key.code, KeyCode::Up | KeyCode::Down) {
            self.cycle_note(key.code == KeyCode::Up)
        } else {
            Update::Idle
        }
    }

    fn move_between_panes(&mut self, key: KeyCode) -> Update {
        match (key, self.sidebar, self.pane) {
            (KeyCode::Left, Sidebar::Focused, _) => self.sidebar = Sidebar::Users,
            (KeyCode::Right, Sidebar::Users, _)
            | (KeyCode::Left, Sidebar::Visible, Pane::Source) => self.focus_tree(),
            (KeyCode::Right, Sidebar::Focused, _)
            | (KeyCode::Left, Sidebar::Hidden | Sidebar::Visible, Pane::Preview) => {
                self.focus(Pane::Source);
            }
            (KeyCode::Right, _, Pane::Source) => self.focus(Pane::Preview),
            _ => return Update::Idle,
        }
        Update::Draw
    }

    fn navigate_tree(&mut self, key: KeyCode) -> Update {
        if key == KeyCode::Esc {
            self.focus(self.pane);
            return Update::Draw;
        }
        match self.workspaces[self.workspace].tree.navigate(key) {
            Navigation::Changed => Update::Draw,
            Navigation::Open(id) => {
                self.preview_note(id);
                Update::Draw
            }
            Navigation::Idle => Update::Idle,
        }
    }

    fn open_note(&mut self, id: NoteId) {
        self.preview_note(id);
        self.workspaces[self.workspace].keep_note();
    }

    fn preview_note(&mut self, id: NoteId) {
        self.workspaces[self.workspace].preview_note(id);
        let (revision, at) = self.workspace().tree.note_version(id);
        if let Some(note) = self.note_mut()
            && note.saved_at.is_none()
        {
            note.revision = revision;
            note.saved_at = Some(at);
            note.load = buffer::LoadState::Pending;
        }
        self.focus(Pane::Source);
    }

    fn cycle_note(&mut self, backward: bool) -> Update {
        let workspace = self.workspace();
        let count = workspace.notes.len();
        if count == 0 {
            return Update::Idle;
        }
        let next = match (workspace.active_tab(), backward) {
            (Some(index), true) => (index + count - 1) % count,
            (Some(index), false) => (index + 1) % count,
            (None, true) => count - 1,
            (None, false) => 0,
        };
        self.activate(next);
        Update::Draw
    }

    fn activate(&mut self, index: usize) {
        self.workspaces[self.workspace].activate(index);
        self.focus(Pane::Source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixtures::{WELCOME as WELCOME_MARKDOWN, demo};
    use ratatui::{Terminal, backend::TestBackend};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    #[test]
    fn users_panel_scrolls_and_transfer_uses_both_entry_points_without_sending() {
        let (_server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        for code in [KeyCode::F(2), KeyCode::Left, KeyCode::End] {
            editor.handle(key(code, KeyModifiers::NONE));
        }
        assert!(editor.sidebar == Sidebar::Users);
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Usuarios"));
        assert!(screen.contains("Zoe"));
        // el contacto aceptado encabeza la lista; después, el resto
        editor.handle(key(KeyCode::Home, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Char('t'), KeyModifiers::NONE));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Destinatario · Carla"));
        editor.handle(key(KeyCode::Esc, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Down, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Char('t'), KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Solicitar contacto"));
        assert!(screen.contains("Usuario · Ana"));
        // la etiqueta, el texto y las teclas del popup van separados por una línea en blanco
        let cells: Vec<char> = screen.chars().collect();
        let lines: Vec<String> = cells.chunks(120).map(|row| row.iter().collect()).collect();
        let row = |text: &str| lines.iter().position(|line| line.contains(text)).unwrap();
        let (label, hint) = (row("Usuario · Ana"), row("Enter solicitar contacto"));
        let column = lines[label].chars().position(|c| c == 'U').unwrap();
        for blank in [label + 1, hint - 1] {
            let cell: String = lines[blank].chars().skip(column).take(20).collect();
            assert_eq!(cell.trim(), "", "fila {blank}");
        }
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(editor.palette.is_some());
        editor.handle(key(KeyCode::Esc, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Up, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Right, KeyModifiers::NONE));
        assert!(editor.sidebar == Sidebar::Focused);
        editor.handle(key(KeyCode::Char('t'), KeyModifiers::NONE));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Nota · Bienvenida"));
        assert!(screen.contains("Destinatario · Carla"));
        editor.handle(key(KeyCode::Esc, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Left, KeyModifiers::NONE));
        for (width, height) in [(80, 24), (22, 8), (1, 1)] {
            terminal.backend_mut().resize(width, height);
            terminal.resize(Rect::new(0, 0, width, height)).unwrap();
            terminal.draw(|frame| editor.draw(frame)).unwrap();
            assert!(editor.sidebar == Sidebar::Users);
        }
    }

    #[test]
    fn navigation_and_writing_use_distinct_keys() {
        let (_server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        editor.handle(key(KeyCode::Left, KeyModifiers::NONE));
        assert!(editor.sidebar == Sidebar::Focused);
        editor.handle(key(KeyCode::Right, KeyModifiers::NONE));
        assert!(editor.sidebar == Sidebar::Visible && editor.pane == Pane::Source);
        editor.handle(key(KeyCode::Right, KeyModifiers::NONE));
        assert!(editor.pane == Pane::Preview);
        editor.handle(key(KeyCode::Left, KeyModifiers::NONE));
        editor.handle(Event::Paste("ignorado".into()));
        assert_eq!(editor.note().unwrap().source.text(), WELCOME_MARKDOWN);

        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Char('2'), KeyModifiers::NONE));
        editor.handle(key(KeyCode::Left, KeyModifiers::NONE));
        assert!(editor.pane == Pane::Writing);
        assert!(editor.note().unwrap().source.text().starts_with('2'));
        editor.handle(key(KeyCode::Esc, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Char('2'), KeyModifiers::NONE));
        assert!(editor.pane == Pane::Source);
        assert_eq!(editor.workspace().notes.len(), 1);
        assert_eq!(
            editor.note().unwrap().source.text(),
            format!("2{WELCOME_MARKDOWN}")
        );

        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains(" Acciones"));
        assert!(screen.contains("Cambios sin guardar"));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        assert!(fixtures::screen(&terminal).contains("Escritura · Esc acciones"));
    }

    #[test]
    fn save_shortcuts_save_to_the_server_and_preserve_writing_mode() {
        let (mut server, mut editor) = demo();
        let id = editor.note().unwrap().id;
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Char('s'), KeyModifiers::NONE));

        for (code, modifiers) in [
            (KeyCode::Char('s'), KeyModifiers::CONTROL),
            (KeyCode::Enter, KeyModifiers::SHIFT),
        ] {
            assert!(matches!(editor.handle(key(code, modifiers)), Update::Draw));
            assert!(editor.save_pending(id));
            for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
                let event = Event::Key(KeyEvent::new_with_kind(code, modifiers, kind));
                assert!(matches!(editor.handle(event), Update::Idle));
            }
        }
        server.sync(&mut editor);

        assert!(editor.pane == Pane::Writing);
        assert!(!editor.note().unwrap().modified);
        assert!(!editor.has_changes());
        assert_eq!(server.markdown(id.server()), format!("s{WELCOME_MARKDOWN}"));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(editor.note().unwrap().source.text().starts_with("s\n"));
    }

    #[test]
    fn slash_table_expands_at_end_of_line_and_undoes_as_one_action() {
        let (_server, mut editor) = demo();
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        editor.note_mut().unwrap().source.set_text("");
        editor.handle(Event::Paste("Texto\n/table{3}{2}\nMás texto".into()));
        editor.note_mut().unwrap().source.set_cursor((12, 1), false);
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));

        let markdown = editor.note_mut().unwrap().source.text();
        assert!(
            markdown.starts_with("Texto\n\n| Columna 1 | Columna 2 | Columna 3 |"),
            "{markdown}"
        );
        assert!(markdown.ends_with("|   |   |   |\n|   |   |   |\n\nMás texto"));
        assert_eq!(
            editor.note_mut().unwrap().source.selected_text(),
            "Columna 1"
        );

        editor.handle(key(KeyCode::Char('z'), KeyModifiers::SUPER));
        assert_eq!(
            editor.note_mut().unwrap().source.text(),
            "Texto\n/table{3}{2}\nMás texto"
        );
        editor.handle(key(
            KeyCode::Char('Z'),
            KeyModifiers::SUPER | KeyModifiers::SHIFT,
        ));
        assert_eq!(editor.note_mut().unwrap().source.text(), markdown);
    }

    #[test]
    fn command_palette_is_the_same_in_every_pane_and_shows_only_the_chosen_hint() {
        let (_server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let labels = [
            "Crear nota o carpeta",
            "Copiar nota o carpeta",
            "Mover nota o carpeta",
            "Eliminar nota o carpeta",
            "Renombrar nota o carpeta",
            "Transferir nota",
            "Guardar nota",
            "Deshacer",
            "Rehacer",
            "Mostrar u ocultar árbol",
        ];
        for opening in [
            vec![],
            vec![KeyCode::F(2)],
            vec![KeyCode::F(2), KeyCode::Left],
        ] {
            for code in opening {
                editor.handle(key(code, KeyModifiers::NONE));
            }
            editor.handle(key(KeyCode::F(1), KeyModifiers::NONE));
            terminal.draw(|frame| editor.draw(frame)).unwrap();
            let screen = fixtures::screen(&terminal);
            for label in labels {
                assert!(screen.contains(label), "{label}");
            }
            // solo el primero, elegido, muestra su tecla
            assert!(screen.contains("› Crear nota o carpeta"));
            // la nota de fondo puede mencionar atajos; se mira solo la fila de cada comando
            let cells: Vec<char> = screen.chars().collect();
            for row in cells.chunks(100).map(String::from_iter) {
                if row.contains("Guardar nota") || row.contains("Deshacer") {
                    assert!(!row.contains("Ctrl+S") && !row.contains("Ctrl+Z"), "{row}");
                }
            }
            assert!(!screen.contains("Insertar tabla") && !screen.contains("Ajustar ancho"));
            editor.handle(key(KeyCode::Esc, KeyModifiers::NONE));
            editor.handle(key(KeyCode::Esc, KeyModifiers::NONE));
        }

        editor.handle(key(KeyCode::F(1), KeyModifiers::NONE));
        editor.handle(Event::Paste("guardar".into()));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("› Guardar nota") && screen.contains("Ctrl+S"));
        assert!(!screen.contains("Deshacer"));
        assert_eq!(editor.note_mut().unwrap().source.text(), WELCOME_MARKDOWN);
    }

    #[test]
    fn settings_persist_autosave_and_theme_and_paint_every_cell_with_the_theme() {
        use crate::theme::Theme;
        use ratatui::style::Color;
        let (mut server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(100, 26)).unwrap();
        editor.handle(key(KeyCode::F(3), KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Autoguardado · activado"));
        assert!(screen.contains("Tema · Terminal"));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        server.sync(&mut editor);
        assert!(!server.state().preferences.settings.autosave);
        assert!(!editor.remote.preferences.settings.autosave);
        assert!(editor.palette.is_none());

        editor.handle(key(KeyCode::F(3), KeyModifiers::NONE));
        for _ in 0..4 {
            editor.handle(key(KeyCode::Down, KeyModifiers::NONE));
        }
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        assert!(fixtures::screen(&terminal).contains("Terminal · actual"));
        // Esc vuelve a la configuración y Enter reabre la lista de temas
        editor.handle(key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(editor.palette.is_some());
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Down, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        server.sync(&mut editor);
        let settings = server.state().preferences.settings;
        assert_eq!(settings.theme, Theme::Nord);
        assert!(
            !settings.autosave,
            "cambiar el tema conserva el autoguardado"
        );
        let background = Color::Rgb(0x2E, 0x34, 0x40);
        // también con un popup abierto, ninguna celda conserva el fondo de la terminal
        editor.handle(key(KeyCode::F(1), KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        assert!(buffer.content.iter().all(|cell| cell.bg == background));
        assert!(
            buffer
                .content
                .iter()
                .any(|cell| cell.fg == Color::Rgb(0x88, 0xC0, 0xD0))
        );
        crate::theme::set(Theme::Terminal);
    }

    #[test]
    fn switching_workspaces_restores_each_tree_and_open_buffer() {
        let (mut server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle(Event::Paste("Personal ".into()));
        editor
            .note_mut()
            .unwrap()
            .source
            .set_selection((0, 0), (3, 0));
        let selection = editor.note().unwrap().source.selection();

        editor.handle(key(KeyCode::F(2), KeyModifiers::NONE));
        editor.handle(key(KeyCode::Down, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Left, KeyModifiers::SHIFT));
        editor.handle(key(KeyCode::Tab, KeyModifiers::NONE));
        assert!(editor.sidebar == Sidebar::Focused);
        assert_eq!(editor.workspace().tree.name, "Matemática");
        assert!(editor.note().is_none());
        assert!(editor.tabs(100).to_string().is_empty());
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        // solo el espacio actual, con su posición en el pie del panel
        assert!(screen.contains("›Matemática"));
        assert!(!screen.contains("Personal"));
        assert!(screen.contains(" 2 de 2 "));
        assert!(screen.contains("Vectores"));
        assert!(!screen.contains("Bienvenida"));

        editor.handle(key(KeyCode::Char('n'), KeyModifiers::NONE));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        server.sync(&mut editor);
        assert!(editor.note().is_none());
        assert!(
            editor.palette.is_some(),
            "el servidor rechaza un nombre vacío"
        );
        editor.handle(Event::Paste("  Geometría  ".into()));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        server.sync(&mut editor);
        assert!(editor.pane == Pane::Source);
        assert_eq!(editor.workspace().tree.destination().1, "Matemática");
        assert!(editor.tabs(100).to_string().contains("[1 Geometría]"));

        editor.handle(key(KeyCode::F(2), KeyModifiers::NONE));
        editor.handle(key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(
            editor.workspace().tree.destination().1,
            "Personal / Universidad"
        );
        assert_eq!(editor.note().unwrap().source.selection(), selection);
        assert!(
            editor
                .note()
                .unwrap()
                .source
                .text()
                .starts_with("Personal ")
        );
        assert!(editor.tabs(100).to_string().contains("[1 *Bienvenida]"));
        editor.handle(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            editor.workspace().tree.destination().1,
            "Personal / Lecturas"
        );

        editor.handle(key(KeyCode::Tab, KeyModifiers::NONE));
        assert!(editor.tabs(100).to_string().contains("[1 Geometría]"));
        assert_eq!(editor.workspace().tree.destination().1, "Matemática");
    }

    #[test]
    fn a_confirmed_popup_animates_ignores_repeated_input_and_closes_on_success() {
        let (mut server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        editor.handle(key(KeyCode::F(2), KeyModifiers::NONE));
        editor.handle(key(KeyCode::Char('n'), KeyModifiers::NONE));
        editor.handle(Event::Paste("Ramo".into()));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("⣾ Creando…"), "{screen}");
        assert!(!screen.contains("Enter crear"));
        // antirrebote: otra confirmación o texto no se acumulan ni producen avisos
        for event in [
            key(KeyCode::Enter, KeyModifiers::NONE),
            Event::Paste("x".into()),
        ] {
            assert!(matches!(editor.handle(event), Update::Idle));
        }
        assert!(editor.animating());
        server.sync(&mut editor);
        assert!(editor.palette.is_none() && !editor.animating());
        assert_eq!(
            editor.workspace().tree.title(editor.note().unwrap().id),
            "Ramo"
        );
        assert!(editor.remote_error().is_none());

        // un fallo devuelve el control al popup con el motivo
        editor.handle(key(KeyCode::Char('n'), KeyModifiers::NONE));
        editor.handle(Event::Paste("Otra".into()));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        server.sync(&mut editor);
        editor.handle(key(KeyCode::Char('r'), KeyModifiers::NONE));
        editor.handle(Event::Paste("Ramo".into()));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(editor.animating());
        server.sync(&mut editor);
        assert!(editor.palette.is_some() && !editor.animating());
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Ya hay una nota o carpeta"), "{screen}");
        assert!(screen.contains("Enter renombrar"));
    }

    #[test]
    fn suggestions_open_from_settings_and_can_be_sent_signed_or_anonymous() {
        let (mut server, mut editor) = demo();
        editor.username = "yo".into();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        for anonymous in [false, true] {
            editor.handle(key(KeyCode::F(3), KeyModifiers::NONE));
            for _ in 0..6 {
                editor.handle(key(KeyCode::Down, KeyModifiers::NONE));
            }
            editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
            terminal.draw(|frame| editor.draw(frame)).unwrap();
            let screen = fixtures::screen(&terminal);
            assert!(
                screen.contains("Comparte tus sugerencias para mejorar!"),
                "{screen}"
            );
            assert!(screen.contains("De: [yo]   Anónimo"), "{screen}");
            assert!(screen.contains("Escribe aquí tu sugerencia…"));

            editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
            terminal.draw(|frame| editor.draw(frame)).unwrap();
            let screen = fixtures::screen(&terminal);
            assert!(screen.contains("Escribe tu sugerencia antes de enviarla."));

            // ← → mueven el cursor del texto hasta que Tab enfoca el remitente
            for ch in "Mas temas".chars() {
                editor.handle(key(KeyCode::Char(ch), KeyModifiers::NONE));
            }
            editor.handle(key(KeyCode::Right, KeyModifiers::NONE));
            if anonymous {
                editor.handle(key(KeyCode::Tab, KeyModifiers::NONE));
                editor.handle(key(KeyCode::Right, KeyModifiers::NONE));
                terminal.draw(|frame| editor.draw(frame)).unwrap();
                let screen = fixtures::screen(&terminal);
                assert!(screen.contains("De:  yo   [Anónimo]"), "{screen}");
                editor.handle(key(KeyCode::Char('x'), KeyModifiers::NONE));
            }
            editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
            server.sync(&mut editor);
            assert!(editor.palette.is_none());
            assert_eq!(editor.command_notice, Some("Sugerencia enviada. ¡Gracias!"));
        }
        let suggestions = server.suggestions();
        assert_eq!(suggestions.len(), 2);
        assert!(suggestions[0].0.is_some());
        assert_eq!(suggestions[0].1, "Mas temas");
        assert_eq!(suggestions[1], (None, "Mas temas".to_owned()));
    }

    #[test]
    fn q_closes_the_note_first_and_then_offers_to_sign_out_or_exit_from_any_panel() {
        let (_server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        editor.handle(key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(editor.note().is_none() && editor.palette.is_none());
        editor.handle(key(KeyCode::Char('q'), KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(
            screen.contains("Cerrar sesión") && screen.contains("Salir de Anotau"),
            "{screen}"
        );
        assert!(matches!(
            editor.handle(key(KeyCode::Enter, KeyModifiers::NONE)),
            Update::SignOut
        ));

        // desde Notas o Usuarios, Q ofrece salir aunque haya una nota abierta
        let (_server, mut editor) = demo();
        let open = editor.note().unwrap().id;
        for panel in [KeyCode::F(2), KeyCode::Left] {
            editor.handle(key(panel, KeyModifiers::NONE));
            editor.handle(key(KeyCode::Char('q'), KeyModifiers::NONE));
            assert!(editor.palette.is_some());
            assert_eq!(editor.note().unwrap().id, open);
            editor.handle(key(KeyCode::Esc, KeyModifiers::NONE));
        }
        assert!(editor.sidebar == Sidebar::Users);
        editor.handle(key(KeyCode::Char('q'), KeyModifiers::NONE));
        editor.handle(key(KeyCode::Down, KeyModifiers::NONE));
        assert!(matches!(
            editor.handle(key(KeyCode::Enter, KeyModifiers::NONE)),
            Update::Exit
        ));

        // escribiendo, Q es texto
        let (_server, mut editor) = demo();
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle(key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(editor.palette.is_none());
        assert!(editor.note().unwrap().source.text().starts_with('q'));
    }

    #[test]
    fn deleting_the_account_asks_first_and_defaults_to_going_back() {
        let (_server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let open_option = |editor: &mut Editor| {
            editor.handle(key(KeyCode::F(3), KeyModifiers::NONE));
            for _ in 0..7 {
                editor.handle(key(KeyCode::Down, KeyModifiers::NONE));
            }
            editor.handle(key(KeyCode::Enter, KeyModifiers::NONE))
        };
        editor.handle(key(KeyCode::F(3), KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Borrar datos y cuenta permanentemente"));
        editor.handle(key(KeyCode::Esc, KeyModifiers::NONE));

        open_option(&mut editor);
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(
            screen.contains("¿Estás seguro de abandonar Anotau?"),
            "{screen}"
        );
        assert!(screen.contains("Se eliminará permanentemente todo registro tuyo."));
        assert!(
            screen.contains(" Sí, seguro   [No, volver atrás]"),
            "{screen}"
        );
        // Enter sobre «No» vuelve a la configuración sin borrar nada
        assert!(matches!(
            editor.handle(key(KeyCode::Enter, KeyModifiers::NONE)),
            Update::Draw
        ));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        assert!(fixtures::screen(&terminal).contains("Preferencias de la cuenta"));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));

        editor.handle(key(KeyCode::Left, KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        assert!(fixtures::screen(&terminal).contains("[Sí, seguro]   No, volver atrás"));
        // «Sí, seguro» pide la contraseña; sin ella no se envía nada
        assert!(matches!(
            editor.handle(key(KeyCode::Enter, KeyModifiers::NONE)),
            Update::Draw
        ));
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Contraseña de tu cuenta"), "{screen}");
        assert!(screen.contains("Ingresa tu contraseña para confirmar."));
        for ch in "clave".chars() {
            editor.handle(key(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(
            !screen.contains("clave") && screen.contains("*****"),
            "{screen}"
        );
        let update = editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(update, Update::Send(Job::DeleteAccount { password }) if password == "clave")
        );
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("⣾ Eliminando cuenta…"));
        assert!(!screen.contains("*****"), "el campo se vacía al enviar");
        // antirrebote: otra confirmación no repite la petición
        assert!(matches!(
            editor.handle(key(KeyCode::Enter, KeyModifiers::NONE)),
            Update::Idle
        ));
        // si el servidor falla, la cuenta sigue y el popup explica el motivo
        editor.failed(
            crate::network::Origin::DeleteAccount,
            crate::network::Failure::new(crate::network::Reason::Uncertain, "Sin conexión."),
        );
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Sin conexión.") && !screen.contains("Eliminando cuenta"));
    }

    #[test]
    fn changing_the_password_checks_each_field_and_then_asks_to_sign_in_again() {
        let (_server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let typed = |editor: &mut Editor, text: &str| {
            for ch in text.chars() {
                editor.handle(key(KeyCode::Char(ch), KeyModifiers::NONE));
            }
        };
        let enter = |editor: &mut Editor| editor.handle(key(KeyCode::Enter, KeyModifiers::NONE));
        let screen = |editor: &mut Editor, terminal: &mut Terminal<TestBackend>| {
            terminal.draw(|frame| editor.draw(frame)).unwrap();
            fixtures::screen(terminal)
        };
        editor.handle(key(KeyCode::F(3), KeyModifiers::NONE));
        for _ in 0..5 {
            editor.handle(key(KeyCode::Down, KeyModifiers::NONE));
        }
        assert!(screen(&mut editor, &mut terminal).contains("› Cambiar contraseña"));
        enter(&mut editor);
        let form = screen(&mut editor, &mut terminal);
        assert!(form.contains("Contraseña actual") && form.contains("Repite la contraseña nueva"));

        enter(&mut editor);
        assert!(screen(&mut editor, &mut terminal).contains("Completa este campo"));
        typed(&mut editor, "vieja-clave");
        enter(&mut editor);
        typed(&mut editor, "corta");
        enter(&mut editor);
        typed(&mut editor, "corta");
        enter(&mut editor);
        let form = screen(&mut editor, &mut terminal);
        assert!(form.contains("al menos 8 caracteres"), "{form}");
        assert!(!form.contains("vieja-clave") && !form.contains("corta"));

        // vuelve al campo nueva; Tab llega a la repetición
        typed(&mut editor, "-y-larga");
        editor.handle(key(KeyCode::Tab, KeyModifiers::NONE));
        typed(&mut editor, "-distinta");
        enter(&mut editor);
        assert!(screen(&mut editor, &mut terminal).contains("no coinciden"));

        // se corrige la repetición desde cero
        for _ in 0.."corta-distinta".len() {
            editor.handle(key(KeyCode::Backspace, KeyModifiers::NONE));
        }
        typed(&mut editor, "corta-y-larga");
        let update = enter(&mut editor);
        assert!(matches!(
            update,
            Update::Send(Job::ChangePassword { current, new })
                if current == "vieja-clave" && new == "corta-y-larga"
        ));
        assert!(screen(&mut editor, &mut terminal).contains("⣾ Cambiando contraseña…"));
        editor.password_changed();
        let done = screen(&mut editor, &mut terminal);
        assert!(
            done.contains("Listo, contraseña cambiada. Debes volver a iniciar"),
            "{done}"
        );
        assert!(matches!(enter(&mut editor), Update::SignOut));
    }
}
