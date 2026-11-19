use rat_text::{
    event::TextOutcome,
    text_input::{TextInputState, handle_events},
};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Span,
    widgets::{Paragraph, Wrap},
};

use crate::popup;

use crate::network::Account;

/// Acción de la bienvenida sin sesión; cada una abre su formulario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    Login,
    Register,
    /// Pedir al administrador que restablezca la contraseña.
    Recover,
}

impl Choice {
    pub(super) const ALL: [Self; 3] = [Self::Login, Self::Register, Self::Recover];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Login => "Iniciar sesión",
            Self::Register => "Registrarse",
            Self::Recover => "Olvidé mi contraseña",
        }
    }

    fn account(self) -> Option<Account> {
        match self {
            Self::Login => Some(Account::Login),
            Self::Register => Some(Account::Register),
            Self::Recover => None,
        }
    }
}

pub(super) enum Outcome {
    Open,
    Close,
    Submit {
        credentials: anotau_protocol::Credentials,
        account: Account,
    },
    Recover(anotau_protocol::RecoveryRequest),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    User,
    Password,
}

impl Field {
    fn label(self) -> &'static str {
        match self {
            Self::User => "Usuario",
            Self::Password => "Contraseña",
        }
    }

    fn other(self) -> Self {
        match self {
            Self::User => Self::Password,
            Self::Password => Self::User,
        }
    }
}

pub(super) struct Form {
    pub(super) choice: Choice,
    user: TextInputState,
    password: TextInputState,
    field: Field,
    feedback: Option<String>,
}

impl Form {
    pub(super) fn new(choice: Choice) -> Self {
        let user = TextInputState::new();
        user.focus.set(true);
        // en la recuperación el segundo campo es el contacto, que sí puede pegarse
        let password = if choice == Choice::Recover {
            TextInputState::new()
        } else {
            crate::popup::secret_input()
        };
        Self {
            choice,
            user,
            password,
            field: Field::User,
            feedback: None,
        }
    }

    fn focus(&mut self, field: Field) {
        self.field = field;
        self.user.focus.set(field == Field::User);
        self.password.focus.set(field == Field::Password);
    }

    pub(super) fn feedback(&mut self, message: String) {
        self.feedback = Some(message);
    }

    pub(super) fn handle(&mut self, event: &Event) -> Outcome {
        if matches!(
            event,
            Event::Key(KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                ..
            })
        ) {
            return Outcome::Close;
        }
        let Some(credentials) = self.edit(event) else {
            return Outcome::Open;
        };
        match self.choice.account() {
            Some(account) => Outcome::Submit {
                credentials,
                account,
            },
            None => Outcome::Recover(anotau_protocol::RecoveryRequest {
                username: credentials.username,
                contact: credentials.password,
            }),
        }
    }

    fn second_label(&self) -> &'static str {
        if self.choice == Choice::Recover {
            "Contacto para verificar tu identidad (correo institucional)"
        } else {
            Field::Password.label()
        }
    }

    fn edit(&mut self, event: &Event) -> Option<anotau_protocol::Credentials> {
        match event {
            Event::Key(key) if key.code == KeyCode::Enter && key.modifiers.is_empty() => {
                if key.kind == KeyEventKind::Press {
                    match self.field {
                        Field::User => self.focus(Field::Password),
                        Field::Password => return self.submit(),
                    }
                }
            }
            Event::Key(key)
                if matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
                    && (key.modifiers - KeyModifiers::SHIFT).is_empty() =>
            {
                if key.kind == KeyEventKind::Press {
                    self.focus(self.field.other());
                }
            }
            Event::Paste(text) if text.chars().any(char::is_control) => {
                self.feedback = Some("Usa una sola línea, sin caracteres de control.".into());
            }
            Event::Key(key) if matches!(key.code, KeyCode::Char(ch) if ch.is_control()) => {}
            _ => {
                let field = match self.field {
                    Field::User => &mut self.user,
                    Field::Password => &mut self.password,
                };
                let outcome = if let Event::Key(key) = event
                    && key.kind == KeyEventKind::Repeat
                {
                    let key = KeyEvent {
                        kind: KeyEventKind::Press,
                        ..*key
                    };
                    handle_events(field, true, &Event::Key(key))
                } else {
                    handle_events(field, true, event)
                };
                if matches!(outcome, TextOutcome::TextChanged) {
                    self.feedback = None;
                }
            }
        }
        None
    }

    fn submit(&mut self) -> Option<anotau_protocol::Credentials> {
        let feedback = if self.user.text().trim().is_empty() {
            self.focus(Field::User);
            "Ingresa un usuario."
        } else if let Some(problem) = (self.choice == Choice::Register)
            .then(|| anotau_protocol::username_problem(self.user.text()))
            .flatten()
        {
            self.focus(Field::User);
            problem
        } else if let Some(problem) = (self.choice == Choice::Recover)
            .then(|| anotau_protocol::contact_problem(self.password.text()))
            .flatten()
        {
            problem
        } else if self.password.is_empty() {
            "Ingresa una contraseña."
        } else if let Some(problem) = (self.choice == Choice::Register)
            .then(|| anotau_protocol::password_problem(self.password.text()))
            .flatten()
        {
            problem
        } else {
            let credentials = anotau_protocol::Credentials {
                username: self.user.text().to_owned(),
                password: self.password.text().to_owned(),
            };
            self.password.set_text("");
            return Some(credentials);
        };
        self.feedback = Some(feedback.into());
        None
    }

    pub(super) fn draw(&mut self, frame: &mut Frame<'_>) {
        let inner = popup::draw(frame, self.choice.label(), 72, 16);
        if inner.height >= 8 {
            let [user, _, password, _, feedback, hint] = Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Fill(1),
                Constraint::Length(1),
            ])
            .areas(inner);
            self.draw_field(frame, user, Field::User);
            self.draw_field(frame, password, Field::Password);
            self.draw_feedback(frame, feedback);
            frame.render_widget(
                Span::styled(
                    "Tab cambiar campo  ·  Enter continuar  ·  Esc volver",
                    Style::new().dim(),
                ),
                hint,
            );
        } else {
            // con poca altura solo se ve el campo enfocado
            let [field, feedback] =
                Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(inner);
            self.draw_field(frame, field, self.field);
            self.draw_feedback(frame, feedback);
        }
    }

    fn draw_field(&mut self, frame: &mut Frame<'_>, area: Rect, field: Field) {
        let focused = self.field == field;
        match field {
            Field::User => popup::field(
                frame,
                area,
                Field::User.label(),
                &mut self.user,
                focused,
                false,
            ),
            Field::Password => {
                let label = self.second_label();
                let secret = self.choice != Choice::Recover;
                popup::field(frame, area, label, &mut self.password, focused, secret);
            }
        }
    }

    /// Solo el motivo del validador que detuvo el envío, o el avance de la conexión.
    fn draw_feedback(&self, frame: &mut Frame<'_>, area: Rect) {
        if let Some(message) = &self.feedback {
            frame.render_widget(
                Paragraph::new(message.as_str())
                    .style(Style::new().fg(crate::theme::warning()))
                    .wrap(Wrap { trim: true }),
                area,
            );
        }
    }
}
