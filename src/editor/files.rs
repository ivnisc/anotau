use super::{Editor, Selection};
use crate::{
    Update,
    network::{ExportContent, ExportRequest, Job},
    palette::{FileRequest, Palette},
    workspace::{EntryId, FileAction, NoteId},
};

impl Editor {
    pub(super) fn start_file(&mut self, action: FileAction) -> Update {
        let workspace = self.workspace();
        let Some(source) = workspace.tree.selected() else {
            return Update::Idle;
        };
        let has_unsaved_changes = workspace
            .notes
            .iter()
            .any(|note| note.modified && workspace.tree.contains_note(source, note.id));
        let popup = Palette::file(action, source, &workspace.tree, has_unsaved_changes);
        if matches!(action, FileAction::Delete)
            && let EntryId::Note(id) = source
        {
            self.preview_note(id);
        }
        self.palette = Some(popup);
        Update::Draw
    }

    /// Abre el popup de exportación para la entrada resaltada del árbol.
    pub(super) fn start_export(&mut self) -> Update {
        let workspace = self.workspace();
        let Some(source) = workspace.tree.selected() else {
            return Update::Idle;
        };
        let notes = workspace.tree.export_notes(source).len();
        let title = workspace.tree.entry_title(source);
        let label = match source {
            EntryId::Note(_) => format!("Nota · {title}"),
            EntryId::Folder(_) if notes == 0 => {
                self.command_notice = Some("La carpeta no tiene notas para exportar.");
                return Update::Draw;
            }
            EntryId::Folder(_) if notes == 1 => format!("Carpeta · {title} · 1 nota"),
            EntryId::Folder(_) => format!("Carpeta · {title} · {notes} notas"),
        };
        let directory = self.export_directory.clone().unwrap_or_else(|| {
            crate::export::default_directory()
                .to_string_lossy()
                .into_owned()
        });
        self.palette = Some(Palette::export(source, label, &directory));
        Update::Draw
    }

    /// Las notas abiertas se exportan como se ven, con sus cambios sin guardar; las demás, en
    /// su versión guardada.
    pub(super) fn apply_export(&mut self, source: EntryId, directory: String) -> Update {
        let tree = &self.workspace().tree;
        let folder =
            matches!(source, EntryId::Folder(_)).then(|| tree.entry_title(source).to_owned());
        let notes = tree
            .export_notes(source)
            .into_iter()
            .map(|(folders, title, id)| {
                let content = match self.loaded_text(id) {
                    Some(text) => ExportContent::Text(text),
                    None => ExportContent::Remote(id.server()),
                };
                (folders, title, content)
            })
            .collect();
        let request = ExportRequest {
            directory: crate::export::expand(&directory),
            folder,
            notes,
        };
        self.export_directory = Some(directory);
        self.send("Exportando…", Job::Export(request))
    }

    fn loaded_text(&self, id: NoteId) -> Option<String> {
        self.workspaces
            .iter()
            .flat_map(|workspace| {
                workspace.notes.iter().chain(match &workspace.active {
                    Some(Selection::Temporary(note)) => Some(&**note),
                    _ => None,
                })
            })
            .find(|note| note.id == id && !note.loading())
            .map(|note| note.source.text())
    }

    /// Respuesta de la exportación: el popup dice dónde quedó.
    pub(crate) fn exported(&mut self, path: &std::path::Path, notes: usize) {
        self.progress = None;
        if let Some(palette) = &mut self.palette {
            palette.exported(path, notes);
        }
    }

    pub(super) fn close_active_note(&mut self) -> Update {
        let Some(note) = self.note() else {
            return Update::Idle;
        };
        let id = note.id;
        let has_unsaved_changes = note.modified || self.save_pending(id);
        if has_unsaved_changes {
            self.palette = Some(Palette::file(
                FileAction::Close,
                EntryId::Note(id),
                &self.workspace().tree,
                has_unsaved_changes,
            ));
            Update::Draw
        } else {
            self.apply_file(FileRequest::SaveAndClose(id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::fixtures::{self, WELCOME, demo};
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers},
    };

    fn press(editor: &mut Editor, code: KeyCode) {
        editor.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    #[test]
    fn sidebar_actions_preserve_drafts_and_close_only_the_deleted_buffer() {
        let (mut server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        press(&mut editor, KeyCode::Enter);
        editor.handle(Event::Paste("Editado\n".into()));
        for ch in "mcdr".chars() {
            press(&mut editor, KeyCode::Char(ch));
            assert!(editor.palette.is_none());
        }
        let original = editor.note().unwrap().id;
        let text = editor.note().unwrap().source.text();
        press(&mut editor, KeyCode::F(2));
        press(&mut editor, KeyCode::Char('r'));
        editor.handle(Event::Paste("Inicio".into()));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert_eq!(editor.workspace().tree.title(original), "Inicio");
        assert_eq!(editor.note().unwrap().source.text(), text);

        press(&mut editor, KeyCode::Char('m'));
        press(&mut editor, KeyCode::Down);
        press(&mut editor, KeyCode::Down);
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert_eq!(
            editor.workspace().tree.note_folder(original),
            "Personal / Universidad"
        );
        assert_eq!(editor.note().unwrap().id, original);
        press(&mut editor, KeyCode::Char('c'));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        let Some(EntryId::Note(copy)) = editor.workspace().tree.selected() else {
            panic!("nota copiada")
        };
        assert_ne!(copy, original);
        assert_eq!(editor.workspace().tree.title(copy), "Inicio (2)");
        // la copia lleva el borrador abierto; el original en la base sigue intacto
        assert_eq!(server.markdown(copy.server()), text);
        assert_eq!(server.markdown(original.server()), WELCOME);
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert_eq!(editor.note().unwrap().source.text(), text);
        press(&mut editor, KeyCode::Enter);
        editor.handle(Event::Paste("Solo copia\n".into()));
        assert_eq!(editor.workspace().notes.len(), 2);

        press(&mut editor, KeyCode::F(2));
        press(&mut editor, KeyCode::Char('d'));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Se descartarán los cambios sin guardar."));
        press(&mut editor, KeyCode::Esc);
        assert_eq!(editor.workspace().notes.len(), 2);
        press(&mut editor, KeyCode::F(2));
        press(&mut editor, KeyCode::Char('d'));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert!(
            server
                .state()
                .entries
                .iter()
                .all(|entry| entry.id != copy.server())
        );
        assert_eq!(editor.workspace().notes.len(), 1);
        assert_eq!(editor.note().unwrap().id, original);
        assert_eq!(editor.note().unwrap().source.text(), text);
        assert!(editor.note().unwrap().modified);
    }

    #[test]
    fn q_closes_clean_notes_and_keeps_dirty_notes_until_a_real_save() {
        let (mut server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        let id = editor.note().unwrap().id;
        press(&mut editor, KeyCode::Char('q'));
        assert!(editor.note().is_none());
        assert_eq!(editor.workspace().tree.title(id), "Bienvenida");
        press(&mut editor, KeyCode::F(2));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        press(&mut editor, KeyCode::Enter);
        press(&mut editor, KeyCode::Char('q'));
        assert!(editor.palette.is_none());
        let text = editor.note().unwrap().source.text();
        assert!(text.starts_with('q'));
        press(&mut editor, KeyCode::Esc);
        press(&mut editor, KeyCode::Char('q'));
        assert!(editor.palette.is_some());
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Enter guardar y cerrar · Esc cancelar"));
        press(&mut editor, KeyCode::Enter);
        assert!(editor.palette.is_some());
        assert_eq!(editor.note().unwrap().source.text(), text);
        press(&mut editor, KeyCode::Esc);
        assert_eq!(editor.workspace().notes.len(), 1);
        assert_eq!(editor.note().unwrap().source.text(), text);

        // con el popup abierto, la confirmación del guardado cierra la pestaña
        press(&mut editor, KeyCode::Char('q'));
        press(&mut editor, KeyCode::Enter);
        server.sync(&mut editor);
        assert!(editor.note().is_none());
        assert!(!editor.has_changes());
        assert_eq!(server.markdown(id.server()), text);
    }

    /// Atiende la exportación como el hilo de red: lee del servidor lo que no está abierto.
    fn export(server: &fixtures::Server, update: Update) -> (std::path::PathBuf, usize) {
        let Update::Send(Job::Export(request)) = update else {
            panic!("se esperaba una exportación")
        };
        let files = request
            .notes
            .into_iter()
            .map(|(folders, title, content)| crate::export::File {
                folders,
                title,
                markdown: match content {
                    ExportContent::Text(text) => text,
                    ExportContent::Remote(id) => server.markdown(id),
                },
            })
            .collect::<Vec<_>>();
        let path =
            crate::export::write(&request.directory, request.folder.as_deref(), &files).unwrap();
        (path, files.len())
    }

    fn replace_destination(editor: &mut Editor, directory: &std::path::Path) {
        editor.handle(Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)));
        for _ in 0..1024 {
            press(editor, KeyCode::Backspace);
        }
        editor.handle(Event::Paste(directory.to_string_lossy().into_owned()));
    }

    #[test]
    fn e_exports_the_highlighted_folder_or_the_open_note_as_it_is_seen() {
        let (server, mut editor) = demo();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        let directory = tempfile::tempdir().unwrap();
        press(&mut editor, KeyCode::F(2));
        while editor
            .workspace()
            .tree
            .selected()
            .is_none_or(|entry| editor.workspace().tree.entry_title(entry) != "Universidad")
        {
            press(&mut editor, KeyCode::Down);
        }
        press(&mut editor, KeyCode::Char('e'));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(
            screen.contains("Carpeta · Universidad · 2 notas"),
            "{screen}"
        );
        let downloads = crate::export::default_directory();
        assert!(screen.contains(&*downloads.to_string_lossy()), "{screen}");
        replace_destination(&mut editor, directory.path());
        let update = editor.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let (path, notes) = export(&server, update);
        editor.exported(&path, notes);
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        assert!(fixtures::screen(&terminal).contains("Exportado (2 notas)"));
        let root = directory.path().join("Universidad");
        assert!(
            std::fs::read_to_string(root.join("Apuntes.md"))
                .unwrap()
                .starts_with("# Apuntes")
        );
        assert!(root.join("Proyecto").join("Ideas para Anotau.md").is_file());
        press(&mut editor, KeyCode::Enter);
        assert!(editor.palette.is_none());

        // desde Markdown se exporta la nota abierta con lo que tiene en pantalla
        press(&mut editor, KeyCode::Right);
        press(&mut editor, KeyCode::Enter);
        editor.handle(Event::Paste("Sin guardar\n".into()));
        press(&mut editor, KeyCode::Esc);
        press(&mut editor, KeyCode::Char('e'));
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let screen = fixtures::screen(&terminal);
        assert!(screen.contains("Nota · Bienvenida"), "{screen}");
        let update = editor.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let (path, _) = export(&server, update);
        // sin reescribir el destino: recuerda la última carpeta de la sesión
        assert_eq!(path, directory.path().join("Bienvenida.md"));
        assert!(
            std::fs::read_to_string(path)
                .unwrap()
                .starts_with("Sin guardar")
        );
    }
}
