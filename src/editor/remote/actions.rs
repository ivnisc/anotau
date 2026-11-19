use super::Editor;
use crate::{
    Update,
    palette::{FileRequest, SpaceAction, SpaceRequest, TransferRequest},
    workspace::FolderId,
};
use anotau_protocol::{self as api, Command, EntryKind};

impl Editor {
    pub(in crate::editor) fn apply_create(
        &mut self,
        parent: Option<FolderId>,
        title: String,
        kind: EntryKind,
    ) -> Update {
        let current = self.workspace().tree.id();
        if !self.popup_current(current)
            || self
                .popup_context
                .as_ref()
                .is_none_or(|context| context.workspace != current)
        {
            self.notify("El árbol cambió. Abre otra vez el formulario.".into());
            return Update::Draw;
        }
        self.queue(
            Command::CreateEntry {
                workspace: self.workspace().tree.id().0,
                parent: parent.map(FolderId::server),
                title,
                kind,
            },
            None,
            None,
        )
    }

    pub(in crate::editor) fn apply_file(&mut self, request: FileRequest) -> Update {
        if let FileRequest::SaveAndClose(id) = request {
            if self.save_pending(id)
                || self
                    .workspace()
                    .notes
                    .iter()
                    .any(|note| note.id == id && note.modified)
            {
                return self.save_active(true);
            }
            self.workspaces[self.workspace].close_note(id);
            self.palette = None;
            return Update::Draw;
        }
        if !self.popup_current(self.workspace().tree.id()) {
            self.notify("El árbol cambió. Abre otra vez el formulario.".into());
            return Update::Draw;
        }
        let source = match &request {
            FileRequest::Rename { source, .. }
            | FileRequest::Move { source, .. }
            | FileRequest::Copy { source, .. }
            | FileRequest::Delete(source) => *source,
            FileRequest::SaveAndClose(_) => return Update::Idle,
        };
        let Some(revision) = self.workspace().tree.entry_revision(source) else {
            self.notify("La entrada ya no existe. Abre otra vez el formulario.".into());
            return Update::Draw;
        };
        let command = match request {
            FileRequest::Rename { title, .. } => Command::RenameEntry {
                id: source.server(),
                revision,
                title,
            },
            FileRequest::Move { destination, .. } => Command::MoveEntry {
                id: source.server(),
                revision,
                parent: destination.map(FolderId::server),
            },
            FileRequest::Copy { destination, .. } => Command::CopyEntry {
                id: source.server(),
                revision,
                parent: destination.map(FolderId::server),
                drafts: super::drafts(self.workspace(), Some(source)),
            },
            FileRequest::Delete(_) => Command::DeleteEntry {
                id: source.server(),
                revision,
            },
            FileRequest::SaveAndClose(_) => return Update::Idle,
        };
        self.queue(command, None, None)
    }

    pub(in crate::editor) fn apply_space(&mut self, request: &SpaceRequest) -> Update {
        let space = request
            .id
            .and_then(|id| self.workspaces.iter().find(|space| space.tree.id() == id));
        let command = match request.action {
            SpaceAction::Browse => {
                if let Some(index) = self
                    .workspaces
                    .iter()
                    .position(|space| Some(space.tree.id()) == request.id)
                {
                    self.workspace = index;
                    self.palette = None;
                    self.focus_tree();
                }
                return Update::Draw;
            }
            SpaceAction::Create => Command::CreateWorkspace {
                title: request.title.clone(),
            },
            action => {
                let Some(space) = space else {
                    return Update::Idle;
                };
                let id = space.tree.id().0;
                let revision = space.tree.revision;
                if !self.popup_current(space.tree.id()) {
                    self.notify("El espacio cambió. Abre otra vez el formulario.".into());
                    return Update::Draw;
                }
                match action {
                    SpaceAction::Rename => Command::RenameWorkspace {
                        id,
                        revision,
                        title: request.title.clone(),
                    },
                    SpaceAction::Copy => Command::CopyWorkspace {
                        id,
                        revision,
                        title: request.title.clone(),
                        drafts: super::drafts(space, None),
                    },
                    SpaceAction::Delete => {
                        if space.notes.iter().any(|note| note.modified) {
                            self.notify("Guarda los cambios antes de eliminar el espacio.".into());
                            return Update::Draw;
                        }
                        Command::DeleteWorkspace { id, revision }
                    }
                    _ => return Update::Idle,
                }
            }
        };
        self.queue(command, None, None)
    }

    pub(in crate::editor) fn apply_transfer(&mut self, request: TransferRequest) -> Update {
        let command = match request {
            TransferRequest::Contact(id) => Command::Contact { recipient: id.0 },
            TransferRequest::ResolveContact { id, resolution } => {
                Command::ResolveContact { id, resolution }
            }
            TransferRequest::Copy { note, recipient } => {
                if self
                    .workspaces
                    .iter()
                    .flat_map(|space| &space.notes)
                    .any(|buffer| buffer.id == note && buffer.modified)
                {
                    self.notify("Guarda la nota antes de ofrecer su revisión confirmada.".into());
                    return Update::Draw;
                }
                let Some(space) = self
                    .workspaces
                    .iter()
                    .find(|space| space.tree.has_note(note))
                else {
                    return Update::Idle;
                };
                Command::Offer {
                    id: note.server(),
                    revision: space.tree.note_version(note).0,
                    recipient: recipient.0,
                }
            }
            TransferRequest::Accept {
                request,
                workspace,
                parent,
            } => Command::ResolveTransfer {
                id: request.0,
                destination: Some(api::Destination {
                    workspace: workspace.0,
                    parent: parent.map(FolderId::server),
                }),
            },
            TransferRequest::Reject(id) => Command::ResolveTransfer {
                id: id.0,
                destination: None,
            },
        };
        self.queue(command, None, None)
    }
}

#[cfg(test)]
mod tests {
    use crate::editor::{Editor, fixtures::demo};
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers},
    };

    fn press(editor: &mut Editor, code: KeyCode) {
        editor.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    #[test]
    fn workspace_popup_reuses_forms_and_copy_keeps_independent_ids_and_drafts() {
        let (mut server, mut editor) = demo();
        let original = editor.note().unwrap().id;
        press(&mut editor, KeyCode::Enter);
        editor.handle(Event::Paste("Borrador ".into()));
        let draft = editor.note().unwrap().source.text();
        press(&mut editor, KeyCode::Esc);
        press(&mut editor, KeyCode::Char('w'));
        press(&mut editor, KeyCode::Char('r'));
        editor.handle(Event::Paste("Privado".into()));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert_eq!(editor.workspace().tree.name, "Privado");
        assert_eq!(editor.note().unwrap().id, original);

        // un espacio con borradores no se elimina
        press(&mut editor, KeyCode::Char('w'));
        press(&mut editor, KeyCode::Char('d'));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert!(editor.palette.is_some());
        assert_eq!(editor.workspaces.len(), 2);

        press(&mut editor, KeyCode::Esc);
        press(&mut editor, KeyCode::Char('c'));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert_eq!(editor.workspace().tree.name, "Privado (2)");
        assert!(editor.workspace().notes.is_empty());
        let copy = editor.workspace().tree.id();
        let copied = server
            .state()
            .entries
            .into_iter()
            .find(|entry| entry.workspace == copy.0 && entry.title == "Bienvenida")
            .unwrap();
        assert_ne!(copied.id, original.server());
        assert_eq!(server.markdown(copied.id), draft);
        assert!(!editor.workspace().tree.has_note(original));

        press(&mut editor, KeyCode::Char('w'));
        press(&mut editor, KeyCode::Char('d'));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert_eq!(editor.workspaces.len(), 2);
        press(&mut editor, KeyCode::Char('w'));
        press(&mut editor, KeyCode::Char('n'));
        editor.handle(Event::Paste("Nuevo".into()));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert!(editor.workspace().tree.id() > copy);
        assert_eq!(editor.workspace().tree.name, "Nuevo");
        let created = editor.workspace().tree.id();
        press(&mut editor, KeyCode::Char('w'));
        press(&mut editor, KeyCode::Char('r'));
        editor.handle(Event::Paste("Renombrado".into()));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert_eq!(editor.workspace().tree.id(), created);
        assert_eq!(editor.workspace().tree.name, "Renombrado");
        assert_eq!(editor.workspaces[0].tree.name, "Privado");
        press(&mut editor, KeyCode::Char('w'));
        press(&mut editor, KeyCode::Char('c'));
        editor.handle(Event::Paste("Copia vacía".into()));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert!(editor.workspace().tree.notes().is_empty());
        assert_eq!(editor.workspace().tree.name, "Copia vacía");
        let mut terminal = Terminal::new(TestBackend::new(120, 25)).unwrap();
        terminal.draw(|frame| editor.draw(frame)).unwrap();
    }
}
