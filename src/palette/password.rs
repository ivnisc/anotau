//! Cambio de contraseña desde Configuración: actual, nueva y repetición; luego el aviso de
//! volver a iniciar sesión en el mismo popup.

use rat_text::text_input::{TextInputState, handle_events};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{Paragraph, Wrap},
};

use super::{Action, Command, Mode, Palette, Settings};

const LABELS: [&str; 3] = [
    "Contraseña actual",
    "Contraseña nueva",
    "Repite la contraseña nueva",
];

pub(super) struct PasswordChange {
    /// Configuración a la que vuelve Esc antes de cambiarla.
    settings: Settings,
    fields: [TextInputState; 3],
    focused: usize,
    /// El servidor aceptó el cambio y cerró las sesiones de la cuenta.
    done: bool,
}

impl PasswordChange {
    pub(super) fn new(settings: Settings) -> Self {
        let fields = [(); 3].map(|()| crate::popup::secret_input());
        fields[0].focus.set(true);
        Self {
            settings,
            fields,
            focused: 0,
            done: false,
        }
    }

    pub(super) fn into_settings(self) -> Settings {
        self.settings
    }

    pub(super) fn done(&self) -> bool {
        self.done
    }

    pub(super) fn height(&self) -> u16 {
        if self.done { 7 } else { 14 }
    }

    pub(super) fn hint(&self) -> &'static str {
        if self.done {
            "Enter iniciar sesión"
        } else {
            "Tab cambiar campo  ·  Enter continuar  ·  Esc volver"
        }
    }

    fn focus(&mut self, index: usize) {
        self.focused = index;
        for (position, field) in self.fields.iter().enumerate() {
            field.focus.set(position == index);
        }
    }

    fn clear(&mut self) {
        for field in &mut self.fields {
            field.set_text("");
        }
        self.focus(0);
    }
}

impl Palette {
    /// Respuesta del servidor: el mismo popup pasa a pedir un nuevo inicio de sesión.
    pub(crate) fn password_changed(&mut self) {
        if let Mode::Password(change) = &mut self.mode {
            change.clear();
            change.done = true;
            self.error = None;
        }
    }

    pub(super) fn handle_password(&mut self, event: &Event) -> Option<Action> {
        let Mode::Password(change) = &mut self.mode else {
            return None;
        };
        if change.done {
            return Some(Action::KeepOpen);
        }
        match event {
            Event::Key(key)
                if matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
                    && (key.modifiers - KeyModifiers::SHIFT).is_empty() =>
            {
                let backward = key.code == KeyCode::BackTab || key.modifiers == KeyModifiers::SHIFT;
                let next = if backward {
                    change.focused + LABELS.len() - 1
                } else {
                    change.focused + 1
                };
                change.focus(next % LABELS.len());
            }
            Event::Paste(text) if text.chars().any(char::is_control) => {}
            _ => {
                handle_events(&mut change.fields[change.focused], true, event);
                self.error = None;
            }
        }
        Some(Action::KeepOpen)
    }

    pub(super) fn confirm_password(&mut self) -> Action {
        let Mode::Password(change) = &mut self.mode else {
            return Action::KeepOpen;
        };
        if change.done {
            return Action::Run(Command::SignOut);
        }
        // Enter avanza hasta el último campo, como en el formulario de acceso
        if change.focused + 1 < LABELS.len() && change.fields[change.focused].is_empty() {
            self.error = Some("Completa este campo para continuar.".into());
            return Action::KeepOpen;
        }
        if change.focused + 1 < LABELS.len() {
            change.focus(change.focused + 1);
            return Action::KeepOpen;
        }
        let [current, new, repeat] = &change.fields;
        let problem = if current.is_empty() {
            Some((0, "Ingresa tu contraseña actual."))
        } else if let Some(problem) = anotau_protocol::password_problem(new.text()) {
            Some((1, problem))
        } else if new.text() != repeat.text() {
            Some((2, "La contraseña nueva y su repetición no coinciden."))
        } else if new.text() == current.text() {
            Some((1, "La contraseña nueva debe ser distinta de la actual."))
        } else {
            None
        };
        if let Some((field, problem)) = problem {
            change.focus(field);
            self.error = Some(problem.into());
            return Action::KeepOpen;
        }
        let [current, new, _] = &change.fields;
        let action = Action::ChangePassword {
            current: current.text().to_owned(),
            new: new.text().to_owned(),
        };
        change.clear();
        action
    }

    pub(super) fn draw_password(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Mode::Password(change) = &mut self.mode else {
            return;
        };
        if change.done {
            frame.render_widget(
                Paragraph::new("Listo, contraseña cambiada. Debes volver a iniciar sesión.")
                    .style(Style::new().fg(crate::theme::active()))
                    .wrap(Wrap { trim: true }),
                area,
            );
            return;
        }
        let rows: [Rect; 3] = Layout::vertical([Constraint::Length(2); 3])
            .spacing(1)
            .areas(area);
        for (index, label) in LABELS.into_iter().enumerate() {
            let focused = index == change.focused;
            crate::popup::field(
                frame,
                rows[index],
                label,
                &mut change.fields[index],
                focused,
                true,
            );
        }
    }
}
