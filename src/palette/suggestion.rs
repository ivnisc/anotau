//! Sugerencias desde Configuración: un texto libre firmado con la cuenta o anónimo.

use rat_text::{
    HasScreenCursor,
    event::TextOutcome,
    text_area::{TextArea, TextAreaState, TextWrap, handle_events},
};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, StatefulWidget},
};

use super::{Action, Mode, Palette, Settings};

pub(super) struct Suggestion {
    /// Configuración a la que vuelve Esc.
    settings: Settings,
    anonymous: bool,
    /// ← → eligen el remitente cuando el foco está en «De:»; si no, mueven el cursor del texto.
    author_focused: bool,
    text: TextAreaState,
}

impl Suggestion {
    pub(super) fn new(settings: Settings) -> Self {
        let mut text = TextAreaState::new();
        text.set_text_wrap(TextWrap::Word(4));
        text.focus.set(true);
        Self {
            settings,
            anonymous: false,
            author_focused: false,
            text,
        }
    }

    pub(super) fn into_settings(self) -> Settings {
        self.settings
    }

    pub(super) fn hint(&self) -> &'static str {
        if self.author_focused {
            "← → remitente  ·  Tab texto  ·  Enter enviar  ·  Esc volver"
        } else {
            "Tab remitente  ·  Enter enviar  ·  Esc volver"
        }
    }
}

impl Palette {
    pub(super) fn handle_suggestion(&mut self, event: &Event) -> Option<Action> {
        let Mode::Suggestion(suggestion) = &mut self.mode else {
            return None;
        };
        if let Event::Key(key) = event
            && key.modifiers == KeyModifiers::NONE
        {
            match key.code {
                KeyCode::Tab => {
                    suggestion.author_focused = !suggestion.author_focused;
                    suggestion.text.focus.set(!suggestion.author_focused);
                    return Some(Action::KeepOpen);
                }
                KeyCode::Left | KeyCode::Right if suggestion.author_focused => {
                    suggestion.anonymous = key.code == KeyCode::Right;
                    return Some(Action::KeepOpen);
                }
                _ if suggestion.author_focused => return Some(Action::KeepOpen),
                _ => {}
            }
        }
        if !suggestion.author_focused
            && matches!(
                handle_events(&mut suggestion.text, true, event),
                TextOutcome::TextChanged
            )
        {
            self.error = None;
        }
        Some(Action::KeepOpen)
    }

    pub(super) fn confirm_suggestion(&mut self) -> Action {
        let Mode::Suggestion(suggestion) = &self.mode else {
            return Action::KeepOpen;
        };
        let text = suggestion.text.text();
        let text = text.trim();
        if text.is_empty() {
            self.error = Some("Escribe tu sugerencia antes de enviarla.".into());
            return Action::KeepOpen;
        }
        if text.len() > anotau_protocol::MAX_SUGGESTION_BYTES {
            self.error = Some("La sugerencia supera 4 KiB; acórtala un poco.".into());
            return Action::KeepOpen;
        }
        Action::Suggest {
            text: text.to_owned(),
            anonymous: suggestion.anonymous,
        }
    }

    pub(super) fn draw_suggestion(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Mode::Suggestion(suggestion) = &mut self.mode else {
            return;
        };
        let [author, _, text] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(area);
        let label = if suggestion.author_focused {
            crate::theme::selected()
        } else {
            Style::new().dim()
        };
        let mut line = Line::from(Span::styled("De: ", label));
        let options = [suggestion.settings.username.as_str(), "Anónimo"];
        let selected = usize::from(suggestion.anonymous);
        line.extend(crate::popup::choice(&options, selected, Style::new()).spans);
        frame.render_widget(line, author);

        let border = if suggestion.author_focused {
            Style::new().dim()
        } else {
            Style::new().fg(crate::theme::accent())
        };
        let block = Block::bordered().border_style(border);
        let inner = block.inner(text);
        TextArea::new()
            .block(block)
            .render(text, frame.buffer_mut(), &mut suggestion.text);
        if suggestion.text.is_empty() {
            frame.render_widget(
                Span::styled("Escribe aquí tu sugerencia…", Style::new().dim()),
                inner,
            );
        }
        if !suggestion.author_focused
            && let Some(cursor) = suggestion.text.screen_cursor()
        {
            frame.set_cursor_position(cursor);
        }
    }
}
