mod actions;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anotau_protocol::{self as api, Command, EntryKind, Outcome};

use super::buffer::{Hold, LoadState};
use super::{Editor, NoteBuffer, Selection, Workspace};
use crate::{
    Update,
    network::{self, Job, Reason},
    workspace::{EntryId, NoteId, Tree, group_entries},
};

pub(super) struct Remote {
    pending: Option<Pending>,
    manual: Option<SaveIntent>,
    pub(super) preferences: api::Preferences,
    error: Option<String>,
    load_retry: Instant,
}

impl Remote {
    pub(super) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn pending_command(&self) -> Option<&Command> {
        self.pending
            .as_ref()
            .map(|pending| &pending.request.command)
    }
}

struct Pending {
    request: Arc<api::Mutation>,
    sent: bool,
    retry: Instant,
    attempts: u32,
    close: Option<NoteId>,
    generation: Option<u64>,
    popup: Option<u64>,
}

pub(super) struct PopupContext {
    token: u64,
    workspace: crate::workspace::WorkspaceId,
    revisions: std::collections::HashMap<crate::workspace::WorkspaceId, i64>,
}

impl PopupContext {
    pub(super) fn capture(editor: &Editor) -> Self {
        Self {
            token: editor.popup_sequence,
            workspace: editor.workspace().tree.id(),
            revisions: editor
                .workspaces
                .iter()
                .map(|space| (space.tree.id(), space.tree.revision))
                .collect(),
        }
    }
}

struct SaveIntent {
    id: NoteId,
    close: bool,
    revision: i64,
    markdown: String,
    generation: u64,
    popup: Option<u64>,
}
impl SaveIntent {
    fn capture(note: &NoteBuffer, close: bool) -> Self {
        Self {
            id: note.id,
            close,
            revision: note.revision,
            markdown: note.source.text(),
            generation: note.generation,
            popup: None,
        }
    }
}

impl Editor {
    pub(crate) fn from_remote(state: api::Snapshot) -> Result<Self, &'static str> {
        let workspaces = trees(state.workspaces, state.entries)?
            .into_iter()
            .map(Workspace::new)
            .collect();
        let remote = Remote {
            pending: None,
            manual: None,
            preferences: state.preferences,
            error: None,
            load_retry: Instant::now(),
        };
        let mut editor = Self::with_workspaces(workspaces, remote);
        editor.apply_preferences();
        editor.replace_directory(state.directory);
        if let Some(id) = editor.workspace().tree.first_note() {
            editor.open_note(id);
        }
        Ok(editor)
    }

    pub(crate) fn replace_directory(&mut self, directory: api::Directory) {
        self.people.replace(
            directory
                .people
                .into_iter()
                .map(|person| crate::people::Person {
                    id: crate::people::UserId(person.id),
                    name: person.name,
                    presence: person.presence,
                    contact: person.contact,
                })
                .collect(),
        );
        self.incoming = directory
            .incoming
            .into_iter()
            .map(|offer| crate::inbox::Incoming {
                id: crate::inbox::TransferId(offer.id),
                sender: crate::people::UserId(offer.sender),
                title: offer.title,
            })
            .collect();
    }

    pub(crate) fn refresh_remote(&mut self, state: api::Snapshot) -> Result<(), &'static str> {
        let selected = self.workspace().tree.id();
        let fresh = trees(state.workspaces, state.entries)?;
        // una eliminación remota no descarta borradores no confirmados.
        for previous in &self.workspaces {
            for note in previous.buffers() {
                if note.modified && !fresh.iter().any(|tree| tree.has_note(note.id)) {
                    return Err(
                        "Una nota con borrador fue eliminada remotamente. Sus cambios \
                         permanecen en memoria.",
                    );
                }
            }
        }
        let mut previous = std::mem::take(&mut self.workspaces);
        for mut tree in fresh {
            if let Some(index) = previous
                .iter()
                .position(|space| space.tree.id() == tree.id())
            {
                let mut space = previous.remove(index);
                tree.preserve_navigation(&space.tree);
                let removed: Vec<_> = space
                    .notes
                    .iter()
                    .filter(|note| !tree.has_note(note.id))
                    .map(|note| note.id)
                    .collect();
                for id in removed {
                    space.close_note(id);
                }
                if let Some(Selection::Temporary(note)) = &space.active
                    && !tree.has_note(note.id)
                {
                    space.close_note(note.id);
                }
                for note in space.buffers_mut() {
                    let (revision, _) = tree.note_version(note.id);
                    if revision != note.revision {
                        if note.modified {
                            note.conflict();
                        } else {
                            note.load = LoadState::Pending;
                        }
                    }
                }
                space.tree = tree;
                self.workspaces.push(space);
            } else {
                self.workspaces.push(Workspace::new(tree));
            }
        }
        self.workspace = self
            .workspaces
            .iter()
            .position(|space| space.tree.id() == selected)
            .unwrap_or_default();
        self.remote.preferences = state.preferences;
        self.apply_preferences();
        self.replace_directory(state.directory);
        Ok(())
    }

    /// Aplica el tema de la cuenta; las vistas previas guardan colores ya resueltos.
    pub(crate) fn apply_preferences(&mut self) {
        if crate::theme::set(self.remote.preferences.settings.theme) {
            for workspace in &mut self.workspaces {
                for note in workspace.buffers_mut() {
                    note.refresh_preview();
                }
            }
        }
    }

    /// Guarda las preferencias de la cuenta; el acento se aplica al confirmarse.
    pub(super) fn save_settings(&mut self, settings: api::Settings) -> Update {
        let revision = self.remote.preferences.revision;
        self.queue(Command::Preferences { revision, settings }, None, None)
    }

    pub(super) fn remote_error(&self) -> Option<&str> {
        self.remote.error.as_deref()
    }

    /// Cierra el popup que siguió la escritura, tras la duración mínima de su indicador.
    fn settle_popup(&mut self) {
        let now = Instant::now();
        match &mut self.progress {
            Some(progress) => {
                progress.succeed(now, crate::progress::MINIMUM);
                self.tick(now);
            }
            None => self.palette = None,
        }
    }

    /// Avanza el indicador del popup; devuelve si hay que redibujar.
    pub(crate) fn tick(&mut self, now: Instant) -> bool {
        let Some(progress) = &self.progress else {
            return false;
        };
        if progress.due(now) {
            self.palette = None;
            self.popup_context = None;
            self.progress = None;
        }
        true
    }

    pub(crate) fn animating(&self) -> bool {
        self.progress.is_some()
    }

    pub(crate) fn notify(&mut self, message: String) {
        if let Some(palette) = &mut self.palette {
            palette.show_error(&message);
        }
        self.remote.error = Some(message);
    }

    pub(super) fn queue(
        &mut self,
        command: Command,
        close: Option<NoteId>,
        generation: Option<u64>,
    ) -> Update {
        if self.remote.pending.is_some() {
            self.notify("Espera la confirmación de la operación pendiente.".into());
            return Update::Draw;
        }
        match network::operation(command) {
            Ok(request) => {
                self.remote.error = None;
                self.remote.pending = Some(Pending {
                    request,
                    sent: false,
                    retry: Instant::now(),
                    attempts: 0,
                    close,
                    generation,
                    popup: self.popup_context.as_ref().map(|context| context.token),
                });
            }
            Err(error) => self.notify(error.to_string()),
        }
        Update::Draw
    }

    fn buffer_mut(&mut self, id: api::Id) -> Option<&mut NoteBuffer> {
        for workspace in &mut self.workspaces {
            if let Some(index) = workspace
                .notes
                .iter()
                .position(|note| note.id.server() == id)
            {
                return workspace.notes.get_mut(index);
            }
            if let Some(Selection::Temporary(note)) = &mut workspace.active
                && note.id.server() == id
            {
                return Some(note);
            }
        }
        None
    }

    fn popup_current(&self, id: crate::workspace::WorkspaceId) -> bool {
        self.popup_context.as_ref().is_some_and(|context| {
            context.revisions.get(&id).is_some_and(|revision| {
                self.workspaces
                    .iter()
                    .any(|space| space.tree.id() == id && space.tree.revision == *revision)
            })
        })
    }

    pub(crate) fn has_changes(&self) -> bool {
        self.workspaces
            .iter()
            .flat_map(Workspace::buffers)
            .any(|note| note.modified || note.pending_generation.is_some())
            || self.remote.pending.is_some()
            || self.remote.manual.is_some()
    }

    pub(super) fn save_pending(&self, id: NoteId) -> bool {
        self.remote
            .manual
            .as_ref()
            .is_some_and(|intent| intent.id == id)
            || self.remote.pending.as_ref().is_some_and(|pending| {
                matches!(
                    &pending.request.command,
                    Command::Save { id: source, .. } if *source == id.server()
                )
            })
    }

    pub(super) fn save_active(&mut self, close: bool) -> Update {
        let Some(note) = self.note() else {
            return Update::Idle;
        };
        if note.loading() {
            self.notify("Espera que termine la carga de la nota.".into());
            return Update::Draw;
        }
        if self
            .remote
            .manual
            .as_ref()
            .is_some_and(|intent| intent.id != note.id)
        {
            self.notify(
                "Ya hay otro guardado manual en espera. Reintenta tras su confirmación.".into(),
            );
            return Update::Draw;
        }
        let mut intent = SaveIntent::capture(note, close);
        intent.popup = self.popup_context.as_ref().map(|context| context.token);
        let remote = &mut self.remote;
        intent.close |= remote
            .manual
            .as_ref()
            .is_some_and(|previous| previous.close);
        remote.manual = Some(intent);
        if let Some(pending) = &mut remote.pending {
            pending.retry = Instant::now();
        }
        Update::Draw
    }

    pub(crate) fn next_job(&mut self) -> Option<Job> {
        let now = Instant::now();
        let remote = &mut self.remote;
        if let Some(pending) = &mut remote.pending
            && !pending.sent
            && now >= pending.retry
        {
            pending.sent = true;
            return Some(Job::Mutate(Arc::clone(&pending.request)));
        }
        if remote.pending.is_none() {
            let manual = remote.manual.take();
            let intent = manual.or_else(|| {
                if !remote.preferences.settings.autosave {
                    return None;
                }
                self.workspaces
                    .iter()
                    .flat_map(|space| &space.notes)
                    .find(|note| {
                        note.modified
                            && !note.loading()
                            && note.hold.is_none()
                            && (note.last_edit.elapsed() >= Duration::from_millis(750)
                                || note
                                    .dirty_since
                                    .is_some_and(|at| at.elapsed() >= Duration::from_secs(5)))
                    })
                    .map(|note| SaveIntent::capture(note, false))
            });
            if let Some(intent) = intent {
                if let Some(note) = self.buffer_mut(intent.id.server()) {
                    note.begin_save(intent.generation);
                    let command = Command::Save {
                        id: intent.id.server(),
                        revision: intent.revision,
                        markdown: intent.markdown,
                    };
                    self.queue(
                        command,
                        intent.close.then_some(intent.id),
                        Some(intent.generation),
                    );
                    if let Some(pending) = self.remote.pending.as_mut() {
                        pending.popup = intent.popup;
                        pending.sent = true;
                        return Some(Job::Mutate(Arc::clone(&pending.request)));
                    }
                    if let Some(note) = self.buffer_mut(intent.id.server()) {
                        note.save_abandoned();
                    }
                }
            }
        }
        if now >= self.remote.load_retry
            && let Some(note) = self.note_mut()
            && note.load == LoadState::Pending
        {
            note.load = LoadState::InFlight;
            return Some(Job::Note(note.id.server()));
        }
        None
    }

    pub(crate) fn received_note(&mut self, note: api::Note) {
        if note.markdown.chars().any(api::is_c1) {
            self.failed(
                network::Origin::Note(note.id),
                network::Failure::new(
                    network::Reason::Rejected,
                    "La nota contiene controles que no se pueden mostrar de forma segura.",
                ),
            );
            return;
        }
        let (id, revision, at) = (note.id, note.revision, note.updated_at);
        let known = self.workspaces.iter().find_map(|space| {
            space
                .tree
                .note_id(id)
                .map(|note| space.tree.note_version(note).0)
        });
        if known.is_some_and(|known| known > revision) {
            if let Some(buffer) = self.buffer_mut(id) {
                buffer.load = LoadState::Pending;
            }
            return;
        }
        let mut loaded = false;
        if let Some(buffer) = self.buffer_mut(id)
            && buffer.loading()
        {
            if buffer.modified {
                buffer.conflict();
            } else {
                buffer.loaded(note);
                loaded = true;
            }
        }
        if loaded {
            for space in &mut self.workspaces {
                if let Some(note) = space.tree.note_id(id) {
                    if space.tree.note_version(note).0 != revision {
                        if let Some(context) = &mut self.popup_context {
                            context.revisions.remove(&space.tree.id());
                        }
                    }
                    space.tree.confirm_note(note, revision, at);
                }
            }
        }
        self.sync_source_focus();
    }

    fn confirm_save(
        &mut self,
        pending: &Pending,
        id: api::Id,
        revision: i64,
        at: i64,
        workspace_revision: i64,
    ) {
        let Command::Save {
            markdown,
            revision: base,
            ..
        } = &pending.request.command
        else {
            return;
        };
        let follow = self.follow_popup(pending);
        for space in &self.workspaces {
            if space.tree.note_id(id).is_some() {
                if let Some(context) = &mut self.popup_context {
                    context.revisions.remove(&space.tree.id());
                }
            }
        }
        let newer = self.workspaces.iter().any(|space| {
            space
                .tree
                .note_id(id)
                .is_some_and(|note| space.tree.note_version(note).0 > revision)
        });
        let mut close = false;
        if let Some(note) = self.buffer_mut(id) {
            note.saved(markdown.as_str().into(), revision, at);
            if pending.generation != Some(note.generation) || newer {
                note.modified = true;
                note.dirty_since.get_or_insert(Instant::now());
            }
            note.hold = newer.then_some(Hold::Conflict);
            close = pending.close == Some(note.id) && !note.modified && follow;
        }
        if let Some(intent) = self.remote.manual.as_mut()
            && intent.id.server() == id
            && intent.revision == *base
        {
            intent.revision = revision;
        }
        for space in &mut self.workspaces {
            if let Some(note) = space.tree.note_id(id) {
                if space.tree.note_version(note).0 <= revision {
                    space.tree.confirm_note(note, revision, at);
                }
                if space.tree.revision.checked_add(1) == Some(workspace_revision) {
                    space.tree.revision = workspace_revision;
                }
                if close {
                    space.close_note(note);
                }
            }
        }
        if close {
            self.settle_popup();
        }
    }

    fn follow_popup(&self, pending: &Pending) -> bool {
        self.palette.is_some()
            && pending.popup.is_some()
            && pending.popup == self.popup_context.as_ref().map(|context| context.token)
    }

    pub(crate) fn changed(&mut self, change: api::Change) {
        let Some(pending) = self.remote.pending.take() else {
            return;
        };
        let follow = self.follow_popup(&pending);
        match change.outcome {
            Outcome::Saved {
                id,
                revision,
                at,
                workspace_revision,
            } => self.confirm_save(&pending, id, revision, at, workspace_revision),
            outcome => {
                if let Command::DeleteEntry { id, .. } = &pending.request.command {
                    for space in &mut self.workspaces {
                        if let Some(note) = space.tree.note_id(*id) {
                            space.close_note(note);
                        }
                    }
                }
                if let Command::RenameEntry { id, revision, .. }
                | Command::MoveEntry { id, revision, .. } = &pending.request.command
                    && let Some(note) = self.buffer_mut(*id)
                    && note.revision == *revision
                {
                    note.revision = note.revision.saturating_add(1);
                }
                if let Some(state) = change.state
                    && let Err(error) = self.refresh_remote(state)
                {
                    self.notify(error.into());
                    return;
                }
                if let Outcome::Changed { workspace, entry } = outcome
                    && follow
                {
                    if let Some(index) = workspace.and_then(|id| {
                        self.workspaces
                            .iter()
                            .position(|space| space.tree.id().0 == id)
                    }) {
                        self.workspace = index;
                    }
                    if let Some(id) = entry {
                        self.workspaces[self.workspace].tree.reveal_server(id);
                        if matches!(
                            pending.request.command,
                            Command::CreateEntry {
                                kind: EntryKind::Note,
                                ..
                            }
                        ) && let Some(note) = self.workspace().tree.note_id(id)
                        {
                            self.open_note(note);
                        }
                    }
                }
                if matches!(pending.request.command, Command::Suggest { .. }) {
                    self.command_notice = Some("Sugerencia enviada. ¡Gracias!");
                }
                if follow {
                    self.settle_popup();
                }
            }
        }
        self.remote.error = None;
    }

    /// El servidor cambió la contraseña y cerró las sesiones: el popup lo dice.
    pub(crate) fn password_changed(&mut self) {
        self.progress = None;
        if let Some(palette) = &mut self.palette {
            palette.password_changed();
        }
    }

    pub(crate) fn failed(&mut self, origin: network::Origin, error: network::Failure) {
        match origin {
            network::Origin::DeleteAccount
            | network::Origin::ChangePassword
            | network::Origin::Export => {
                // la cuenta sigue intacta: el popup vuelve a aceptar teclas y explica el motivo
                self.progress = None;
                if let Some(palette) = &mut self.palette {
                    palette.show_error(&error.message);
                    return;
                }
            }
            network::Origin::Mutate => {
                // el popup vuelve a aceptar teclas y muestra el motivo
                self.progress = None;
                if let Some(mut pending) = self.remote.pending.take() {
                    // sin respuesta o sin sesión, la operación sigue viva con su identidad
                    let keep = matches!(error.reason, Reason::Uncertain | Reason::Unauthorized);
                    if let Command::Save { id, .. } = &pending.request.command
                        && let Some(note) = self.buffer_mut(*id)
                    {
                        if keep {
                            note.saving = false;
                        } else {
                            note.save_refused(if error.reason == Reason::Conflict {
                                Hold::Conflict
                            } else {
                                Hold::Rejected
                            });
                        }
                    }
                    if error.reason == Reason::Unauthorized {
                        pending.sent = false;
                        pending.retry = Instant::now();
                        self.remote.pending = Some(pending);
                    } else if error.reason == Reason::Uncertain {
                        pending.sent = false;
                        pending.attempts = pending.attempts.saturating_add(1);
                        let mut jitter = [0_u8; 1];
                        if getrandom::fill(&mut jitter).is_err() {
                            jitter[0] = 0;
                        }
                        pending.retry = Instant::now()
                            + Duration::from_millis(
                                (1000_u64 << pending.attempts.min(5)) + 2 * u64::from(jitter[0]),
                            );
                        self.remote.pending = Some(pending);
                    }
                }
            }
            network::Origin::Note(id) => {
                if let Some(note) = self.buffer_mut(id) {
                    note.load = match error.reason {
                        Reason::Uncertain | Reason::Unauthorized => LoadState::Pending,
                        Reason::Conflict | Reason::Rejected => LoadState::Failed,
                    };
                }
                self.remote.load_retry = Instant::now() + Duration::from_secs(3);
            }
            _ => {}
        }
        self.notify(error.message);
    }

    pub(super) fn pause_remote(&mut self) {
        if let Some(pending) = &mut self.remote.pending {
            pending.sent = false;
            pending.retry = Instant::now();
        }
        for workspace in &mut self.workspaces {
            for note in workspace.buffers_mut() {
                if note.load == LoadState::InFlight {
                    note.load = LoadState::Pending;
                }
            }
        }
    }
}

/// Árboles de los espacios del estado; toda entrada debe pertenecer a un espacio recibido.
fn trees(
    workspaces: Vec<api::Workspace>,
    entries: Vec<api::Entry>,
) -> Result<Vec<Tree>, &'static str> {
    let mut grouped = group_entries(entries);
    let trees = workspaces
        .into_iter()
        .map(|space| {
            let entries = grouped.remove(&space.id).unwrap_or_default();
            Tree::from_remote(space, entries)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if trees.is_empty() || !grouped.is_empty() {
        return Err("El servidor devolvió espacios inválidos.");
    }
    Ok(trees)
}

/// Borradores abiertos del espacio, o solo los de `source` y sus descendientes.
fn drafts(space: &Workspace, source: Option<EntryId>) -> Vec<api::Draft> {
    space
        .notes
        .iter()
        .filter(|note| {
            note.modified && source.is_none_or(|source| space.tree.contains_note(source, note.id))
        })
        .map(|note| api::Draft {
            id: note.id.server(),
            revision: note.revision,
            markdown: note.source.text(),
        })
        .collect()
}

#[cfg(test)]
mod tests;
