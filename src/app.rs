use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use ratatui::{Frame, crossterm::event::Event};

use crate::{
    Update,
    activity::Activity,
    editor::Editor,
    network::{Job, Network, Origin, Reason, Reply, Response},
    terminal,
    welcome::Welcome,
};

enum Screen {
    Welcome(Box<Welcome>),
    Editor(Box<Editor>),
}

pub(crate) struct App {
    screen: Screen,
    /// Servidor configurado, que la bienvenida muestra mientras no conoce su nombre.
    address: String,
    owner: Option<i64>,
    suspended: HashMap<i64, Box<Editor>>,
    activity: Activity,
    epoch: u64,
    pending: Option<Job>,
    /// Petición del usuario que espera a que `pending` quede libre, como borrar la cuenta.
    deferred: Option<Job>,
    next_sync: Instant,
    quitting: bool,
    pub(crate) draw: bool,
    pub(crate) finished: bool,
}

impl App {
    pub(crate) fn new(address: String) -> anyhow::Result<Self> {
        Ok(Self {
            screen: Screen::Welcome(Box::new(Welcome::new(&address)?)),
            address,
            owner: None,
            suspended: HashMap::new(),
            activity: Activity::new(Instant::now()),
            epoch: 0,
            pending: None,
            deferred: None,
            next_sync: Instant::now(),
            quitting: false,
            draw: true,
            finished: false,
        })
    }

    fn editor(&mut self) -> Option<&mut Editor> {
        match &mut self.screen {
            Screen::Editor(editor) => Some(editor),
            Screen::Welcome(_) => self
                .owner
                .and_then(|id| self.suspended.get_mut(&id))
                .map(Box::as_mut),
        }
    }

    fn next_epoch(&mut self) -> anyhow::Result<()> {
        self.epoch = self
            .epoch
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Se agotó la generación de sesión"))?;
        Ok(())
    }

    fn sign_out(&mut self, session: &mut terminal::Session) -> anyhow::Result<()> {
        // el tema pertenece a la cuenta: la siguiente persona ve el de la terminal
        crate::theme::set(crate::theme::Theme::Terminal);
        let welcome = Welcome::new(&self.address)?;
        let old = std::mem::replace(&mut self.screen, Screen::Welcome(Box::new(welcome)));
        if let Some(id) = self.owner.take() {
            if let Screen::Editor(mut editor) = old {
                editor.suspend();
                self.suspended.insert(id, editor);
            } else if let Some(editor) = self.suspended.get_mut(&id) {
                editor.suspend();
            }
        }
        session.set_mouse_capture(false)?;
        self.next_epoch()?;
        self.deferred = None;
        self.pending = Some(Job::Logout);
        self.draw = true;
        Ok(())
    }

    /// La cuenta ya no existe: se descarta su editor, sin conservar borradores, y se vuelve a
    /// la bienvenida.
    fn account_deleted(&mut self, session: &mut terminal::Session) -> anyhow::Result<()> {
        crate::theme::set(crate::theme::Theme::Terminal);
        if let Some(id) = self.owner.take() {
            self.suspended.remove(&id);
        }
        let mut welcome = Welcome::new(&self.address)?;
        welcome.notify("Tu cuenta y todos tus datos se eliminaron.".into());
        self.screen = Screen::Welcome(Box::new(welcome));
        session.set_mouse_capture(false)?;
        // respuestas tardías de la cuenta borrada ya no aplican
        self.next_epoch()?;
        self.pending = None;
        self.deferred = None;
        self.draw = true;
        Ok(())
    }

    fn authenticated(&mut self, auth: anotau_protocol::Session) {
        let id = auth.account_id;
        let editor = if let Some(mut previous) = self.suspended.remove(&id) {
            if let Err(error) = previous.refresh_remote(auth.state) {
                previous.notify(error.into());
            }
            previous
        } else {
            match Editor::from_remote(auth.state) {
                Ok(editor) => Box::new(editor),
                Err(error) => {
                    if let Screen::Welcome(welcome) = &mut self.screen {
                        welcome.failed(error.into());
                    }
                    return;
                }
            }
        };
        let mut editor = editor;
        editor.username.clone_from(&auth.username);
        self.suspended.insert(id, editor);
        self.owner = Some(id);
        self.activity = Activity::new(Instant::now());
        self.next_sync = Instant::now();
        if let Screen::Welcome(welcome) = &mut self.screen {
            welcome.authenticated(auth.username);
        }
    }

    pub(crate) fn response(
        &mut self,
        response: Response,
        session: &mut terminal::Session,
    ) -> anyhow::Result<()> {
        if response.epoch != self.epoch {
            return Ok(());
        }
        self.draw = true;
        match response.result {
            Ok(Reply::RecoveryRequested) => {
                if let Screen::Welcome(welcome) = &mut self.screen {
                    welcome.recovery_sent();
                }
            }
            Ok(Reply::Health(health)) => {
                if let Screen::Welcome(welcome) = &mut self.screen {
                    welcome.server_answered(Some(health));
                }
            }
            Ok(Reply::Authenticated(auth)) => self.authenticated(auth),
            Ok(Reply::AccountDeleted) => self.account_deleted(session)?,
            Ok(Reply::Exported { path, notes }) => {
                if let Some(editor) = self.editor() {
                    editor.exported(&path, notes);
                }
            }
            Ok(Reply::PasswordChanged) => {
                if let Some(editor) = self.editor() {
                    editor.password_changed();
                }
            }
            Ok(Reply::LoggedOut) => {
                if self.quitting {
                    self.finished = true;
                }
            }
            Ok(reply) => {
                if let Some(editor) = self.editor() {
                    match reply {
                        Reply::Changed(change) => editor.changed(change),
                        Reply::Note(note) => editor.received_note(note),
                        Reply::State(state) => {
                            if let Err(error) = editor.refresh_remote(state) {
                                editor.notify(error.into());
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(error) => {
                if matches!(response.origin, Origin::Logout) && self.quitting {
                    self.finished = true;
                } else if matches!(response.origin, Origin::Health) {
                    if let Screen::Welcome(welcome) = &mut self.screen {
                        welcome.server_answered(None);
                    }
                } else if matches!(response.origin, Origin::Authenticate | Origin::Recover) {
                    if let Screen::Welcome(welcome) = &mut self.screen {
                        welcome.failed(error.message);
                    }
                } else if error.reason == Reason::Unauthorized && self.owner.is_some() {
                    if let Some(editor) = self.editor() {
                        editor.failed(response.origin, error);
                    }
                    self.sign_out(session)?;
                } else if let Some(editor) = self.editor() {
                    editor.failed(response.origin, error);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn tick(
        &mut self,
        network: &Network,
        session: &mut terminal::Session,
    ) -> anyhow::Result<()> {
        let now = Instant::now();
        if !self.quitting {
            if self.owner.is_some() && self.activity.expired(now) {
                self.sign_out(session)?;
            }
            if self.pending.is_none() {
                self.pending = self.deferred.take();
            }
            if let Screen::Editor(editor) = &mut self.screen {
                self.draw |= editor.tick(now);
                let away = self.activity.away(now);
                self.draw |= editor.away != away;
                editor.away = away;
                if self.pending.is_none() {
                    self.pending = editor.next_job();
                    self.draw |= self.pending.is_some();
                }
            }
            if let Screen::Welcome(welcome) = &mut self.screen
                && self.pending.is_none()
                && welcome.take_server_check()
            {
                self.pending = Some(Job::Health);
            }
            if self.pending.is_none() && self.owner.is_some() && now >= self.next_sync {
                self.pending = Some(Job::Sync {
                    last_activity: self.activity.last_activity(),
                });
                self.next_sync = now + Duration::from_secs(30);
            }
        }
        if let Some(job) = self.pending.take() {
            if let Err(job) = network.submit(self.epoch, job) {
                self.pending = Some(job);
            }
        }
        Ok(())
    }

    pub(crate) fn render(&mut self, frame: &mut Frame<'_>) {
        match &mut self.screen {
            Screen::Welcome(welcome) => welcome.draw(frame),
            Screen::Editor(editor) => editor.draw(frame),
        }
        self.draw = false;
    }

    pub(crate) fn wait(&self) -> Duration {
        if let Screen::Editor(editor) = &self.screen
            && editor.animating()
        {
            return crate::progress::FRAME;
        }
        if self.owner.is_some() {
            self.activity
                .wait(Instant::now())
                .min(Duration::from_millis(100))
        } else {
            Duration::from_millis(100)
        }
    }

    pub(crate) fn input(
        &mut self,
        event: Event,
        session: &mut terminal::Session,
    ) -> anyhow::Result<()> {
        if self.quitting {
            return Ok(());
        }
        let now = Instant::now();
        if self.owner.is_some() && self.activity.expired(now) {
            return self.sign_out(session);
        }
        let was_away = self.activity.away(now);
        self.activity.observe(&event, now);
        if was_away && !self.activity.away(now) {
            self.next_sync = now;
        }
        let update = match &mut self.screen {
            Screen::Welcome(welcome) => welcome.handle(&event),
            Screen::Editor(editor) => editor.handle(event),
        };
        match update {
            Update::Draw => self.draw = true,
            Update::Idle => {}
            Update::Exit => {
                let changed = matches!(
                    &self.screen,
                    Screen::Editor(editor) if editor.has_changes()
                ) || self.suspended.values().any(|editor| editor.has_changes());
                if changed {
                    let message = "Hay cambios u operaciones sin confirmar. Inicia sesión y \
                     guárdalos antes de salir.";
                    if let Some(editor) = self.editor() {
                        editor.notify(message.into());
                    }
                    if let Screen::Welcome(welcome) = &mut self.screen {
                        welcome.failed(message.into());
                    }
                    self.draw = true;
                } else {
                    self.next_epoch()?;
                    self.pending = Some(Job::Logout);
                    self.quitting = true;
                }
            }
            Update::Authenticate {
                credentials,
                account,
            } => {
                self.next_epoch()?;
                self.pending = Some(Job::Authenticate {
                    credentials,
                    account,
                });
                self.draw = true;
            }
            Update::SignOut => self.sign_out(session)?,
            Update::Send(job) => {
                self.deferred = Some(job);
                self.draw = true;
            }
            Update::StartEditor => {
                if let Some(mut editor) = self.owner.and_then(|id| self.suspended.remove(&id)) {
                    editor.mouse_capture_changed(session.set_mouse_capture(true).ok());
                    self.screen = Screen::Editor(editor);
                    self.draw = true;
                }
            }
            Update::ToggleMouseCapture => {
                if let Screen::Editor(editor) = &mut self.screen {
                    editor.mouse_capture_changed(session.toggle_mouse_capture().ok());
                }
                self.draw = true;
            }
            Update::OpenLink(destination) => {
                if let Some(editor) = self.editor() {
                    editor.link_opened(crate::links::open(&destination));
                }
                self.draw = true;
            }
        }
        Ok(())
    }
}
