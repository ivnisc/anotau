//! Borrado de la cuenta desde Configuración: confirmación Sí/No y luego la contraseña.

use rat_text::text_input::{TextInputState, handle_events};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Line,
};

use super::{Action, Mode, Palette, Settings};

pub(super) struct Deletion {
    /// Configuración a la que vuelve Esc o «No, volver atrás».
    settings: Settings,
    /// «Sí, seguro» elegido; empieza en «No, volver atrás».
    confirm: bool,
    /// Tras «Sí, seguro», la contraseña que confirma al titular.
    password: Option<TextInputState>,
}

impl Deletion {
    pub(super) fn new(settings: Settings) -> Self {
        Self {
            settings,
            confirm: false,
            password: None,
        }
    }

    pub(super) fn into_settings(self) -> Settings {
        self.settings
    }

    pub(super) fn height(&self) -> u16 {
        if self.password.is_some() { 12 } else { 9 }
    }

    pub(super) fn hint(&self) -> &'static str {
        if self.password.is_some() {
            "Enter eliminar cuenta  ·  Esc volver"
        } else {
            "← → elegir  ·  Enter confirmar  ·  Esc volver"
        }
    }
}

impl Palette {
    pub(super) fn handle_deletion(&mut self, event: &Event) -> Option<Action> {
        let Mode::DeleteAccount(deletion) = &mut self.mode else {
            return None;
        };
        match &mut deletion.password {
            None => {
                if let Event::Key(key) = event
                    && key.modifiers == KeyModifiers::NONE
                    && matches!(key.code, KeyCode::Left | KeyCode::Right)
                {
                    deletion.confirm = key.code == KeyCode::Left;
                }
            }
            Some(password) => {
                if !matches!(event, Event::Paste(text) if text.chars().any(char::is_control)) {
                    handle_events(password, true, event);
                    self.error = None;
                }
            }
        }
        Some(Action::KeepOpen)
    }

    pub(super) fn confirm_deletion(&mut self) -> Action {
        let Mode::DeleteAccount(deletion) = &mut self.mode else {
            return Action::KeepOpen;
        };
        match &mut deletion.password {
            None if deletion.confirm => {
                let password = crate::popup::secret_input();
                password.focus.set(true);
                deletion.password = Some(password);
                Action::KeepOpen
            }
            None => {
                self.settings_back();
                Action::KeepOpen
            }
            Some(password) if password.is_empty() => {
                self.error = Some("Ingresa tu contraseña para confirmar.".into());
                Action::KeepOpen
            }
            Some(password) => {
                let typed = password.text().to_owned();
                password.set_text("");
                Action::DeleteAccount { password: typed }
            }
        }
    }

    pub(super) fn draw_deletion(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Mode::DeleteAccount(deletion) = &mut self.mode else {
            return;
        };
        let [warning, _, buttons, _, field] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .areas(area);
        let alert = Style::new().fg(crate::theme::warning());
        frame.render_widget(
            Line::styled("Se eliminará permanentemente todo registro tuyo.", alert),
            warning,
        );
        // «Sí» se resalta como aviso; «No», que es la opción inicial, como cualquier elección
        let (selected, highlight) = if deletion.confirm {
            (0, alert.bold())
        } else {
            (1, crate::theme::selected())
        };
        frame.render_widget(
            crate::popup::choice(&["Sí, seguro", "No, volver atrás"], selected, highlight),
            buttons,
        );
        if let Some(password) = &mut deletion.password {
            let label = "Contraseña de tu cuenta";
            crate::popup::field(frame, field, label, password, true, true);
        }
    }
}
