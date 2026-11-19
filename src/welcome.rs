mod account;
mod art;

use std::io;

use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::Update;
use account::{Choice, Form, Outcome};

enum AccountState {
    /// Bienvenida sin sesión, con la acción elegida.
    SignedOut(Choice),
    Form(Box<Form>),
    Authenticated {
        user: String,
        action: WelcomeAction,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WelcomeAction {
    Start,
    SignOut,
}

/// Servidor según su última respuesta a `/v1/health`.
enum Server {
    /// Aún no se abrió el popup de cuenta.
    /// La bienvenida espera que la aplicación envíe la consulta.
    Requested,
    Checking,
    Answered(anotau_protocol::Health),
    Unreachable,
}

pub(crate) struct Welcome {
    logo: Paragraph<'static>,
    address: String,
    server: Server,
    logo_width: u16,
    logo_height: u16,
    account: AccountState,
    pending: bool,
    notice: Option<String>,
}

impl Welcome {
    pub(crate) fn signed_in(&self) -> bool {
        matches!(self.account, AccountState::Authenticated { .. })
    }

    pub(crate) fn authenticated(&mut self, user: String) {
        self.pending = false;
        self.account = AccountState::Authenticated {
            user,
            action: WelcomeAction::Start,
        };
    }

    /// El servidor recibió la solicitud de recuperación: se vuelve a la bienvenida con el aviso.
    pub(crate) fn recovery_sent(&mut self) {
        self.pending = false;
        self.account = AccountState::SignedOut(Choice::Login);
        self.notice = Some(
            "Solicitud enviada. El administrador te contactará para verificar tu identidad.".into(),
        );
    }

    /// Aviso en la línea de teclas, como tras borrar la cuenta.
    pub(crate) fn notify(&mut self, message: String) {
        self.notice = Some(message);
    }

    pub(crate) fn failed(&mut self, message: String) {
        self.pending = false;
        self.notice = Some(message.clone());
        if let AccountState::Form(form) = &mut self.account {
            form.feedback(message);
        }
    }

    /// Consulta pendiente del estado del servidor, que se pide al mostrar la bienvenida.
    pub(crate) fn take_server_check(&mut self) -> bool {
        if matches!(self.server, Server::Requested) {
            self.server = Server::Checking;
            return true;
        }
        false
    }

    pub(crate) fn server_answered(&mut self, health: Option<anotau_protocol::Health>) {
        self.server = health.map_or(Server::Unreachable, Server::Answered);
    }

    /// `nombre | luna estado`, con las lunas y colores de presencia: ● funcionando,
    /// ◐ con problemas o versión incompatible, ○ en mantención.
    fn server_line(&self) -> Line<'static> {
        use crate::people::{Presence, indicator};
        let (name, presence, status) = match &self.server {
            Server::Requested | Server::Checking => {
                return Line::from(vec![
                    Span::raw(self.address.clone()),
                    Span::styled(" | comprobando…", Style::new().dim()),
                ]);
            }
            Server::Unreachable => (self.address.clone(), Presence::Away, "con problemas"),
            Server::Answered(health) => {
                // el nombre llega por red: sin controles que la terminal pueda interpretar
                let name: String = health
                    .name
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .take(64)
                    .collect();
                let name = if name.trim().is_empty() {
                    self.address.clone()
                } else {
                    name
                };
                let (presence, status) =
                    if health.status == anotau_protocol::ServerStatus::Maintenance {
                        (Presence::Offline, "en mantención")
                    } else if health.api_version == anotau_protocol::API_VERSION {
                        (Presence::Active, "funcionando")
                    } else {
                        (Presence::Away, "versión incompatible")
                    };
                (name, presence, status)
            }
        };
        let moon = indicator(presence);
        let style = moon.style;
        Line::from(vec![
            Span::raw(name),
            Span::styled(" | ", Style::new().dim()),
            moon,
            Span::styled(format!(" {status}"), style),
        ])
    }

    pub(crate) fn new(address: &str) -> io::Result<Self> {
        let art = art::logo()?;
        Ok(Self {
            address: address.to_owned(),
            server: Server::Requested,
            logo_width: u16::try_from(art.width()).unwrap_or(u16::MAX),
            logo_height: u16::try_from(art.height()).unwrap_or(u16::MAX),
            logo: Paragraph::new(art).centered(),
            account: AccountState::SignedOut(Choice::Login),
            pending: false,
            notice: None,
        })
    }

    pub(crate) fn handle(&mut self, event: &Event) -> Update {
        if matches!(event, Event::Key(key) if key.kind == KeyEventKind::Release) {
            return Update::Idle;
        }
        if self.pending {
            return match event {
                Event::Resize(..) => Update::Draw,
                Event::Key(key) if key.code == KeyCode::Esc && key.kind == KeyEventKind::Press => {
                    Update::SignOut
                }
                _ => Update::Idle,
            };
        }
        if let AccountState::Form(form) = &mut self.account {
            match form.handle(event) {
                Outcome::Open => {}
                Outcome::Close => {
                    // al volver, la bienvenida comprueba de nuevo el servidor
                    self.account = AccountState::SignedOut(form.choice);
                    self.server = Server::Requested;
                }
                Outcome::Recover(request) => {
                    self.pending = true;
                    form.feedback("Enviando la solicitud…".into());
                    return Update::Send(crate::network::Job::Recover(request));
                }
                Outcome::Submit {
                    credentials,
                    account,
                } => {
                    self.pending = true;
                    form.feedback("Conectando con el servidor…".into());
                    return Update::Authenticate {
                        credentials,
                        account,
                    };
                }
            }
            return Update::Draw;
        }
        match event {
            Event::Resize(..) => Update::Draw,
            Event::Key(key) if key.kind == KeyEventKind::Press => match (key.code, key.modifiers) {
                (KeyCode::Enter, KeyModifiers::NONE) => {
                    self.account = match self.account {
                        AccountState::Authenticated {
                            action: WelcomeAction::Start,
                            ..
                        } => return Update::StartEditor,
                        AccountState::Authenticated {
                            action: WelcomeAction::SignOut,
                            ..
                        } => {
                            self.account = AccountState::SignedOut(Choice::Login);
                            self.server = Server::Requested;
                            return Update::SignOut;
                        }
                        AccountState::SignedOut(choice) => {
                            self.notice = None;
                            AccountState::Form(Box::new(Form::new(choice)))
                        }
                        AccountState::Form(_) => return Update::Idle,
                    };
                    Update::Draw
                }
                (KeyCode::Up | KeyCode::Down, KeyModifiers::NONE) => {
                    let up = key.code == KeyCode::Up;
                    match &mut self.account {
                        AccountState::Authenticated { action, .. } => {
                            *action = if up {
                                WelcomeAction::Start
                            } else {
                                WelcomeAction::SignOut
                            };
                        }
                        AccountState::SignedOut(choice) => {
                            let index = Choice::ALL
                                .iter()
                                .position(|candidate| candidate == choice)
                                .unwrap_or_default();
                            let index = if up {
                                index.saturating_sub(1)
                            } else {
                                (index + 1).min(Choice::ALL.len() - 1)
                            };
                            *choice = Choice::ALL[index];
                        }
                        AccountState::Form(_) => return Update::Idle,
                    }
                    Update::Draw
                }
                (KeyCode::Esc, KeyModifiers::NONE)
                | (KeyCode::Char('q'), KeyModifiers::CONTROL) => Update::Exit,
                _ => Update::Idle,
            },
            _ => Update::Idle,
        }
    }

    pub(crate) fn draw(&mut self, frame: &mut Frame<'_>) {
        frame.render_widget(
            ratatui::widgets::Block::new().style(crate::theme::base()),
            frame.area(),
        );
        let area = frame.area();
        let signed_in = self.signed_in();
        let action_rows = if signed_in { 2 } else { 3 };
        let greeting_rows = u16::from(signed_in && area.height >= 3);
        let hint_rows = u16::from(area.height > action_rows + greeting_rows);
        // una línea en blanco separa las acciones de sus teclas cuando cabe
        let hint_gap = u16::from(area.height > action_rows + greeting_rows + 2);
        // sin sesión, el estado del servidor va bajo las acciones
        let server_rows = u16::from(!signed_in && area.height > action_rows + hint_gap + 4);
        let menu_height = action_rows + greeting_rows + 2 * server_rows + hint_gap + hint_rows;
        let color_logo = area.width >= self.logo_width
            && area.height >= self.logo_height.saturating_add(menu_height + 1);
        let logo_height = if color_logo {
            self.logo_height
        } else {
            u16::from(area.height > menu_height)
        };
        let gap = u16::from(logo_height > 0 && area.height > logo_height + menu_height);
        let height = logo_height + gap + menu_height;
        let [logo, _, greeting, actions, _, server, _, hint] = Layout::vertical([
            Constraint::Length(logo_height),
            Constraint::Length(gap),
            Constraint::Length(greeting_rows),
            Constraint::Length(action_rows),
            Constraint::Length(server_rows),
            Constraint::Length(server_rows),
            Constraint::Length(hint_gap),
            Constraint::Length(hint_rows),
        ])
        .areas(area.centered_vertically(Constraint::Max(height)));
        if color_logo {
            frame.render_widget(&self.logo, logo);
        } else {
            frame.render_widget(
                Paragraph::new("Anotau")
                    .centered()
                    .style(Style::new().bold()),
                logo,
            );
        }
        let hint_text = if let AccountState::Authenticated { user, action } = &self.account {
            frame.render_widget(
                Paragraph::new(format!("Hola, {user}"))
                    .centered()
                    .wrap(Wrap { trim: true }),
                greeting,
            );
            draw_actions(
                frame,
                actions,
                &[
                    ("[ Comenzar ]".into(), *action == WelcomeAction::Start),
                    (
                        "[ Cerrar sesión ]".into(),
                        *action == WelcomeAction::SignOut,
                    ),
                ],
                false,
            );
            "↑↓ elegir · Enter confirmar"
        } else {
            let (selected, form_open) = match &self.account {
                AccountState::SignedOut(choice) => (*choice, false),
                AccountState::Form(form) => (form.choice, true),
                AccountState::Authenticated { .. } => unreachable!("rama con sesión"),
            };
            let choices =
                Choice::ALL.map(|choice| (format!("[ {} ]", choice.label()), choice == selected));
            draw_actions(frame, actions, &choices, form_open);
            frame.render_widget(self.server_line().centered(), server);
            "↑↓ elegir  ·  Enter abrir  ·  Esc salir"
        };
        frame.render_widget(
            Paragraph::new(self.notice.as_deref().unwrap_or(hint_text))
                .centered()
                .style(Style::new().dim()),
            hint,
        );

        if let AccountState::Form(form) = &mut self.account {
            form.draw(frame);
        }
    }
}

/// Acciones centradas; con un formulario encima se atenúan.
fn draw_actions(frame: &mut Frame<'_>, area: Rect, actions: &[(String, bool)], dimmed: bool) {
    let lines = actions
        .iter()
        .filter_map(|&(ref label, active)| {
            if area.height < 2 && !active {
                return None;
            }
            let (marker, style) = match (active, dimmed) {
                (true, false) => ("› ", crate::theme::selected()),
                (true, true) => ("› ", Style::new().dim()),
                (false, _) => ("  ", Style::new()),
            };
            Some(Line::styled(format!("{marker}{label}"), style))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).centered(), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, buffer::Cell, crossterm::event::KeyEvent};

    fn press(welcome: &mut Welcome, key: KeyCode) -> Update {
        welcome.handle(&Event::Key(KeyEvent::new(key, KeyModifiers::NONE)))
    }

    fn screen(welcome: &mut Welcome, terminal: &mut Terminal<TestBackend>) -> String {
        terminal.draw(|frame| welcome.draw(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(Cell::symbol)
            .collect()
    }

    #[test]
    fn login_and_registration_return_to_welcome_without_displaying_the_password() {
        let mut welcome = Welcome::new("localhost:8443").unwrap();
        let mut terminal = Terminal::new(TestBackend::new(90, 28)).unwrap();
        for mode in [KeyCode::Enter, KeyCode::Down] {
            let home = screen(&mut welcome, &mut terminal);
            assert!(home.contains("› [ Iniciar sesión ]") && home.contains("[ Registrarse ]"));
            if mode == KeyCode::Down {
                press(&mut welcome, mode);
                assert!(screen(&mut welcome, &mut terminal).contains("› [ Registrarse ]"));
            }
            press(&mut welcome, KeyCode::Enter);
            // el acceso conserva nombres anteriores a la regla; el registro usa la regla
            let user = if mode == KeyCode::Down {
                "d.angelo"
            } else {
                "D'Angelo"
            };
            welcome.handle(&Event::Paste(user.into()));
            press(&mut welcome, KeyCode::Tab);
            // sin errores, bajo los campos no hay ningún aviso
            let form = screen(&mut welcome, &mut terminal);
            assert!(
                !form.contains("Usuario:") && !form.contains("distingue"),
                "{form}"
            );
            press(&mut welcome, KeyCode::Enter);
            assert!(screen(&mut welcome, &mut terminal).contains("Ingresa una contraseña"));
            welcome.handle(&Event::Paste("secreto de prueba".into()));
            let form = screen(&mut welcome, &mut terminal);
            assert!(form.contains("Contraseña"));
            assert!(!form.contains("secreto de prueba"));
            let submit = press(&mut welcome, KeyCode::Enter);
            assert!(matches!(submit, Update::Authenticate { .. }));
            assert!(!welcome.signed_in());
            welcome.authenticated(user.into());
            assert!(matches!(
                &welcome.account,
                AccountState::Authenticated { user: name, action: WelcomeAction::Start }
                    if name == user
            ));
            let home = screen(&mut welcome, &mut terminal);
            assert!(home.contains(&format!("Hola, {user}")));
            assert!(home.contains("› [ Comenzar ]"));
            assert!(home.contains("[ Cerrar sesión ]"));
            assert!(!home.contains("Contraseña"));
            assert!(matches!(
                press(&mut welcome, KeyCode::Enter),
                Update::StartEditor
            ));
            press(&mut welcome, KeyCode::Down);
            assert!(screen(&mut welcome, &mut terminal).contains("› [ Cerrar sesión ]"));
            press(&mut welcome, KeyCode::Enter);
        }
        assert!(matches!(
            welcome.account,
            AccountState::SignedOut(Choice::Login)
        ));
    }

    #[test]
    fn registration_names_the_failing_username_check_before_contacting_the_server() {
        let mut welcome = Welcome::new("localhost:8443").unwrap();
        let mut terminal = Terminal::new(TestBackend::new(90, 28)).unwrap();
        press(&mut welcome, KeyCode::Down);
        press(&mut welcome, KeyCode::Enter);
        for (name, problem) in [
            ("Carla", "no admite mayúsculas"),
            ("cаrla", "letras de otros alfabetos"),
            ("ab", "de 3 a 32 caracteres"),
            (".carla", "empezar por una letra"),
            ("carla rojas", "no admite espacios"),
        ] {
            welcome.handle(&Event::Paste(name.into()));
            press(&mut welcome, KeyCode::Tab);
            welcome.handle(&Event::Paste("secreto".into()));
            assert!(
                matches!(press(&mut welcome, KeyCode::Enter), Update::Draw),
                "{name}"
            );
            let form = screen(&mut welcome, &mut terminal);
            assert!(form.contains(problem), "{name}: {form}");
            // vuelve al usuario para corregirlo
            for _ in 0..name.chars().count() {
                press(&mut welcome, KeyCode::Backspace);
            }
        }
    }

    #[test]
    fn forgotten_password_sends_a_request_with_a_contact_and_returns_to_the_welcome() {
        let mut welcome = Welcome::new("localhost:8443").unwrap();
        let mut terminal = Terminal::new(TestBackend::new(90, 28)).unwrap();
        press(&mut welcome, KeyCode::Down);
        press(&mut welcome, KeyCode::Down);
        press(&mut welcome, KeyCode::Down);
        let home = screen(&mut welcome, &mut terminal);
        assert!(home.contains("› [ Olvidé mi contraseña ]"), "{home}");
        press(&mut welcome, KeyCode::Enter);
        welcome.handle(&Event::Paste("carla.rojas".into()));
        press(&mut welcome, KeyCode::Enter);
        let form = screen(&mut welcome, &mut terminal);
        assert!(
            form.contains("Contacto para verificar tu identidad"),
            "{form}"
        );
        assert!(matches!(press(&mut welcome, KeyCode::Enter), Update::Draw));
        assert!(screen(&mut welcome, &mut terminal).contains("Indica un contacto"));
        // el contacto se ve al escribirlo: no es una contraseña
        welcome.handle(&Event::Paste("carla@usm.cl".into()));
        assert!(screen(&mut welcome, &mut terminal).contains("carla@usm.cl"));
        let Update::Send(crate::network::Job::Recover(request)) =
            press(&mut welcome, KeyCode::Enter)
        else {
            panic!("se esperaba la solicitud")
        };
        assert_eq!(
            (request.username.as_str(), request.contact.as_str()),
            ("carla.rojas", "carla@usm.cl")
        );
        welcome.recovery_sent();
        let home = screen(&mut welcome, &mut terminal);
        assert!(home.contains("El administrador te contactará"), "{home}");
        assert!(home.contains("› [ Iniciar sesión ]"));
    }
}
