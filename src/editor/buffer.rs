use std::{collections::HashMap, sync::Arc};

use rat_text::{
    event::TextOutcome,
    text_area::{TextAreaState, TextWrap, handle_events},
};
use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::Rect,
    text::Text,
};

use super::preview::Horizontal;
use crate::{
    Update, markdown,
    table::{self, DIMENSIONS_HELP, SlashCommand},
    workspace::NoteId,
};

const NOTE_TOO_LARGE: &str = "El pegado superaría el límite de 128 KiB por nota.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LoadState {
    Ready,
    Pending,
    InFlight,
    Failed,
}

/// Por qué el autoguardado no envía este buffer, aunque tenga cambios.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hold {
    /// Otra revisión ganó; solo un guardado manual vuelve a escribir.
    Conflict,
    /// El servidor rechazó el contenido; la siguiente edición vuelve a intentarlo.
    Rejected,
}

pub(super) struct NoteBuffer {
    pub(super) id: NoteId,
    pub(super) revision: i64,
    pub(super) saved_at: Option<i64>,
    pub(super) load: LoadState,
    pub(super) pending_generation: Option<u64>,
    pub(super) hold: Option<Hold>,
    pub(super) generation: u64,
    pub(super) last_edit: std::time::Instant,
    pub(super) dirty_since: Option<std::time::Instant>,
    pub(super) saving: bool,
    initial_markdown: Arc<str>,
    pub(super) source: TextAreaState,
    pub(super) preview: Text<'static>,
    pub(super) preview_info: Vec<markdown::LineInfo>,
    pub(super) preview_anchors: HashMap<String, usize>,
    preview_columns: u16,
    raw_preview: markdown::Preview,
    pub(super) selected_link: Option<(usize, usize)>,
    pub(super) horizontal: HashMap<usize, Horizontal>,
    pub(super) active_block: Option<usize>,
    pub(super) preview_scroll: usize,
    pub(super) modified: bool,
    pub(super) notice: Option<&'static str>,
    pub(super) tables: Vec<markdown::Table>,
    pub(super) items: Vec<markdown::ListItem>,
}

/// Saltos de línea como `\n`: rat-text 3.1 entra en pánico al dibujar con ajuste de línea un
/// texto con `\r` (CRLF de Windows o un CR suelto), que llega al pegar o desde otra fuente.
pub(super) fn normalize_line_breaks(text: String) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text
    }
}

impl NoteBuffer {
    /// Buffer vacío; su contenido llega con `loaded` desde el servidor.
    pub(super) fn new(id: NoteId) -> Self {
        let mut source = TextAreaState::new();
        source.set_text_wrap(TextWrap::Word(8));
        let mut note = Self {
            id,
            revision: 1,
            saved_at: None,
            load: LoadState::Ready,
            pending_generation: None,
            hold: None,
            generation: 0,
            last_edit: std::time::Instant::now(),
            dirty_since: None,
            saving: false,
            initial_markdown: Arc::from(""),
            source,
            preview: Text::default(),
            preview_info: Vec::new(),
            preview_anchors: HashMap::new(),
            preview_columns: 0,
            raw_preview: markdown::Preview::default(),
            selected_link: None,
            horizontal: HashMap::new(),
            active_block: None,
            preview_scroll: 0,
            modified: false,
            notice: None,
            tables: Vec::new(),
            items: Vec::new(),
        };
        note.refresh_preview();
        note
    }
    /// → al final de un texto sin salto final lleva el cursor a la fila virtual que rat-text 3.1
    /// añade tras él, y Tab allí entra en pánico (`insert_tab`: `ColumnIndexOutOfBounds`). Antes de cada
    /// evento, el cursor y el ancla de la selección vuelven al final real del texto.
    fn clamp_cursor_to_text(&mut self) {
        let Ok(end) = self.source.try_byte_pos(self.source.len_bytes()) else {
            return;
        };
        let (anchor, cursor) = (self.source.anchor(), self.source.cursor());
        if anchor > end || cursor > end {
            self.source.set_selection(anchor.min(end), cursor.min(end));
        }
    }

    pub(super) fn loading(&self) -> bool {
        self.load != LoadState::Ready
    }

    pub(super) fn handle_input(&mut self, mut event: Event) -> Update {
        if self.loading() {
            return Update::Idle;
        }
        self.clamp_cursor_to_text();
        // un pegado masivo se rechaza antes de analizarlo; el servidor aplica el mismo límite
        if let Event::Paste(text) = &event
            && self.source.len_bytes().saturating_add(text.len()) > anotau_protocol::MAX_NOTE_BYTES
        {
            self.notice = Some(NOTE_TOO_LARGE);
            return Update::Draw;
        }
        if let Event::Key(key) = &mut event {
            // ningún teclado escribe controles como carácter; rat-text no los admite en el texto
            if matches!(key.code, KeyCode::Char(ch) if ch.is_control()) {
                return Update::Idle;
            }
            if key.kind == KeyEventKind::Press
                && key.code == KeyCode::Enter
                && key.modifiers.is_empty()
                && (self.expand_table_command() || self.continue_list())
            {
                return Update::Draw;
            }
            key.kind = KeyEventKind::Press;
            if matches!(
                (key.code, key.modifiers),
                (
                    KeyCode::Tab | KeyCode::BackTab,
                    KeyModifiers::NONE | KeyModifiers::SHIFT
                )
            ) {
                let backward =
                    key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
                if self.table_cell(backward) || self.indent_list(backward) {
                    return Update::Draw;
                }
            }
            if key.code == KeyCode::End
                && key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL
            {
                // rat-text añade una fila virtual si falta el salto final.
                if let Ok(end) = self.source.try_byte_pos(self.source.len_bytes()) {
                    self.source
                        .set_cursor(end, key.modifiers.contains(KeyModifiers::SHIFT));
                    self.source.scroll_cursor_to_visible();
                }
                return Update::Draw;
            }
        }
        match handle_events(&mut self.source, true, &event) {
            TextOutcome::TextChanged => {
                self.text_changed();
                Update::Draw
            }
            TextOutcome::Changed => Update::Draw,
            TextOutcome::Continue | TextOutcome::Unchanged => Update::Idle,
        }
    }

    fn expand_table_command(&mut self) -> bool {
        let cursor = self.source.cursor();
        if self.source.has_selection()
            || self.source.try_line_width(cursor.y).ok() != Some(cursor.x)
        {
            return false;
        }
        let start = (0, cursor.y);
        let Ok(line) = self.source.try_line_at(cursor.y) else {
            return false;
        };
        let command = table::parse_slash(&line);
        if matches!(command, SlashCommand::Other) {
            return false;
        }
        if self.source.try_byte_at(start).ok().is_some_and(|range| {
            [markdown::CODE_HIGHLIGHT, markdown::BLOCK_CODE_HIGHLIGHT]
                .iter()
                .any(|&style| self.source.styles_at_match(range.start, style).is_some())
        }) {
            return false;
        }
        let SlashCommand::Table(size) = command else {
            self.notice = Some(DIMENSIONS_HELP);
            return true;
        };
        self.source.set_selection(start, cursor);
        self.insert_table(&size);
        true
    }

    pub(super) fn insert_table(&mut self, size: &table::Size) {
        let Ok(selection) = self.source.try_bytes_at_range(self.source.selection()) else {
            return;
        };
        let text = self.source.text();
        let before = &text[..selection.start];
        let after = &text[selection.end..];
        let prefix = block_separator(before.chars().rev());
        let suffix = block_separator(after.chars());
        let table = size.generate();
        let insertion = format!("{prefix}{}{suffix}", table.markdown);

        self.source.begin_undo_seq();
        self.source.insert_str(insertion);
        self.source.end_undo_seq();
        let offset = selection.start + prefix.len();
        if let (Ok(start), Ok(end)) = (
            self.source.try_byte_pos(offset + table.first_heading.start),
            self.source.try_byte_pos(offset + table.first_heading.end),
        ) {
            self.source.set_selection(start, end);
            self.source.scroll_cursor_to_visible();
        }
        self.text_changed();
    }

    pub(super) fn text_changed(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.last_edit = std::time::Instant::now();
        self.dirty_since.get_or_insert(self.last_edit);
        self.notice = None;
        if self.hold == Some(Hold::Rejected) {
            self.hold = None;
        }
        self.horizontal.clear();
        self.active_block = None;
        self.refresh_preview();
    }

    pub(super) fn saved(&mut self, markdown: Arc<str>, revision: i64, at: i64) {
        self.initial_markdown = markdown;
        self.revision = revision;
        self.saved_at = Some(at);
        self.hold = None;
        self.modified = self.source.text() != self.initial_markdown.as_ref();
        self.saving = false;
        self.pending_generation = None;
        if !self.modified {
            self.dirty_since = None;
        }
    }

    /// Una escritura de la generación actual quedó en vuelo.
    pub(super) fn begin_save(&mut self, generation: u64) {
        self.saving = true;
        self.pending_generation = Some(generation);
    }

    /// La escritura no llegó a enviarse; el borrador sigue como estaba.
    pub(super) fn save_abandoned(&mut self) {
        self.saving = false;
        self.pending_generation = None;
    }

    /// El servidor no aplicó la escritura: se conserva el texto y se retiene el autoguardado.
    pub(super) fn save_refused(&mut self, hold: Hold) {
        self.save_abandoned();
        self.load = LoadState::Ready;
        self.modified = true;
        self.hold = Some(hold);
    }

    /// Otra revisión llegó mientras había cambios locales; ninguno se descarta.
    pub(super) fn conflict(&mut self) {
        self.load = LoadState::Ready;
        self.hold = Some(Hold::Conflict);
    }

    pub(super) fn loaded(&mut self, note: anotau_protocol::Note) {
        let cursor = self.source.cursor();
        // una nota escrita fuera de este cliente puede traer CRLF; se guarda normalizada
        let markdown = normalize_line_breaks(note.markdown);
        self.source.set_text(&markdown);
        self.source.set_cursor(cursor, false);
        self.load = LoadState::Ready;
        self.saved(markdown.into(), note.revision, note.updated_at);
        self.refresh_preview();
    }

    pub(super) fn status(&self) -> String {
        if self.load == LoadState::Failed {
            return " Error al cargar ".into();
        }
        if self.loading() {
            return " Cargando… ".into();
        }
        if self.saving {
            return " Guardando… ".into();
        }
        match self.hold {
            Some(Hold::Conflict) => return " Conflicto · cambios conservados ".into(),
            Some(Hold::Rejected) => return " Guardado rechazado · cambios conservados ".into(),
            None => {}
        }
        if self.modified {
            return " Cambios sin guardar ".into();
        }
        self.saved_at
            .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
            .map_or_else(
                || " Sin guardar ".into(),
                |at| {
                    let minutes = (chrono::Utc::now() - at).num_minutes().max(0);
                    if minutes < 1 {
                        " Guardado hace un momento ".into()
                    } else if minutes == 1 {
                        " Guardado hace 1 minuto ".into()
                    } else if minutes < 30 {
                        format!(" Guardado hace {minutes} minutos ")
                    } else {
                        format!(
                            " Guardado el {} ",
                            at.with_timezone(&chrono::Local).format("%d-%m-%Y %H:%M")
                        )
                    }
                },
            )
    }

    /// Vuelve a analizar y componer la vista previa, también tras cambiar el tema.
    pub(super) fn refresh_preview(&mut self) {
        let text = self.source.text();
        self.modified = text != self.initial_markdown.as_ref()
            || self
                .pending_generation
                .is_some_and(|generation| generation != self.generation);
        if !self.modified {
            self.dirty_since = None;
        }
        let document = markdown::render(text);
        self.source.set_styles(document.highlights);
        self.raw_preview = document.view;
        self.tables = document.tables;
        self.items = document.items;
        self.layout_preview();
    }

    fn layout_preview(&mut self) {
        let view = self.raw_preview.wrap(self.preview_columns);
        self.preview = view.text;
        self.preview_info = view.info;
        self.preview_anchors = view.anchors;
        self.selected_link = None;
        self.refresh_horizontal();
    }

    pub(super) fn preview_offset(&self, area: Rect) -> usize {
        self.preview_scroll.min(
            self.preview
                .height()
                .saturating_sub(usize::from(area.height)),
        )
    }

    pub(super) fn preview_width_changed(&mut self, width: u16) {
        if self.preview_columns != width {
            self.preview_columns = width;
            self.layout_preview();
        }
    }

    pub(super) fn sync_preview(&mut self, area: Rect) {
        let source = &self.source;
        if source.inner.is_empty() {
            return;
        }
        let Ok(top) = source.try_byte_at((source.sub_row_offset(), source.vertical_offset()))
        else {
            return;
        };
        let len = source.len_bytes();
        let end_visible = source
            .try_byte_pos(len)
            .ok()
            .is_some_and(|end| source.pos_to_relative_screen(end).is_some());
        let remaining = if end_visible {
            0
        } else {
            let bottom = (
                i16::try_from(source.inner.width - 1).unwrap_or(i16::MAX),
                i16::try_from(source.inner.height - 1).unwrap_or(i16::MAX),
            );
            let Some(bottom) = source.relative_screen_to_pos(bottom) else {
                return;
            };
            let Ok(bottom) = source.try_byte_at(bottom) else {
                return;
            };
            len.saturating_sub(bottom.end)
        };
        let maximum = self
            .preview
            .height()
            .saturating_sub(usize::from(area.height));
        // usa el tramo visible, incluido el ajuste de líneas, sin recorrer la nota.
        let row = maximum as u128 * top.start as u128
            / top.start.saturating_add(remaining).max(1) as u128;
        self.preview_scroll = usize::try_from(row).unwrap_or(maximum);
    }

    pub(super) fn scroll_preview(&mut self, key: KeyCode, area: Rect) -> Update {
        if matches!(key, KeyCode::Left | KeyCode::Right) {
            return self.scroll_horizontal(key, area, None);
        }
        let mut row = self.preview_offset(area);
        let page = usize::from(area.height.max(1));
        match key {
            KeyCode::Up => row = row.saturating_sub(1),
            KeyCode::Down => row = row.saturating_add(1),
            KeyCode::PageUp => row = row.saturating_sub(page),
            KeyCode::PageDown => row = row.saturating_add(page),
            KeyCode::Home => row = 0,
            KeyCode::End => row = self.preview.height(),
            _ => return Update::Idle,
        }
        self.preview_scroll = row;
        self.selected_link = None;
        Update::Draw
    }
}

fn block_separator(chars: impl Iterator<Item = char>) -> &'static str {
    let mut chars = chars.filter(|&ch| ch != '\r');
    match chars.next() {
        None => "",
        Some('\n') if chars.next() == Some('\n') => "",
        Some('\n') => "\n",
        Some(_) => "\n\n",
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::editor::Editor;
    use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyEvent};

    #[test]
    fn pastes_beyond_the_note_limit_are_refused_without_touching_the_text() {
        let (_server, mut editor) = crate::editor::fixtures::demo();
        key(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
        let before = editor.note().unwrap().source.text();
        let room = anotau_protocol::MAX_NOTE_BYTES - before.len();
        editor.handle(Event::Paste("a".repeat(room + 1)));
        let note = editor.note().unwrap();
        assert_eq!(note.source.text(), before);
        assert_eq!(note.notice, Some(NOTE_TOO_LARGE));
        assert!(!note.modified);
        editor.handle(Event::Paste("a".repeat(room)));
        let note = editor.note().unwrap();
        assert_eq!(note.source.len_bytes(), anotau_protocol::MAX_NOTE_BYTES);
        assert_eq!(note.notice, None);
    }

    fn key(editor: &mut Editor, code: KeyCode, modifiers: KeyModifiers) {
        editor.handle(Event::Key(KeyEvent::new(code, modifiers)));
    }

    #[test]
    fn preview_follows_typing_and_vertical_navigation_but_keeps_manual_scrolling() {
        let (_server, mut editor) = crate::editor::fixtures::demo();
        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        key(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
        let mut text = String::from("```\n");
        for n in 0..80 {
            writeln!(text, "fila {n:03}").unwrap();
        }
        text.push_str("```");
        let note = editor.note_mut().unwrap();
        note.source.set_text(text);
        note.text_changed();
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        assert_eq!(editor.note().unwrap().preview_scroll, 0);

        key(&mut editor, KeyCode::End, KeyModifiers::CONTROL);
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let bottom =
            editor.note().unwrap().preview.height() - usize::from(editor.preview_area.height);
        assert_eq!(editor.note().unwrap().preview_scroll, bottom);
        key(&mut editor, KeyCode::PageUp, KeyModifiers::NONE);
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let page_up = editor.note().unwrap().preview_scroll;
        assert!(page_up > 0 && page_up < bottom);
        key(&mut editor, KeyCode::PageDown, KeyModifiers::NONE);
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        assert_eq!(editor.note().unwrap().preview_scroll, bottom);
        key(&mut editor, KeyCode::Home, KeyModifiers::CONTROL);
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        assert_eq!(editor.note().unwrap().preview_scroll, 0);

        key(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut editor, KeyCode::Right, KeyModifiers::NONE);
        key(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        assert_eq!(editor.note().unwrap().preview_scroll, 1);
        key(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut editor, KeyCode::Left, KeyModifiers::NONE);
        key(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
        key(&mut editor, KeyCode::End, KeyModifiers::CONTROL);
        for n in 0..30 {
            editor.handle(Event::Paste(format!("\nnueva {n:03}")));
            terminal.draw(|frame| editor.draw(frame)).unwrap();
            let note = editor.note().unwrap();
            assert_eq!(
                note.preview_scroll,
                note.preview.height() - usize::from(editor.preview_area.height)
            );
        }
    }
}
