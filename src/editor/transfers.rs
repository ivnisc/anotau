use super::{Editor, Update};
use crate::palette::{Destination, Palette};

impl Editor {
    pub(super) fn review_incoming(&mut self) -> Update {
        let Some(person) = self.people.selected() else {
            return Update::Idle;
        };
        if matches!(person.contact, crate::people::Contact::Incoming(_)) {
            self.palette = Some(Palette::transfer_person(
                person.clone(),
                self.workspace().tree.notes(),
            ));
            return Update::Draw;
        }
        let offers: Vec<_> = self
            .incoming
            .iter()
            .filter(|offer| offer.sender == person.id)
            .cloned()
            .collect();
        if offers.is_empty() {
            self.command_notice = Some("Este usuario no tiene archivos pendientes de recepción.");
        } else {
            self.palette = Some(Palette::receive(
                person.name.clone(),
                offers,
                self.workspaces
                    .iter()
                    .map(|workspace| Destination {
                        id: workspace.tree.id(),
                        name: workspace.tree.name.clone(),
                        folders: workspace.tree.folders(),
                    })
                    .collect(),
            ));
        }
        Update::Draw
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        editor::{
            Sidebar,
            fixtures::{self, demo},
        },
        inbox::TransferId,
        palette::TransferRequest,
        workspace::WorkspaceId,
    };
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers},
    };

    fn press(editor: &mut Editor, code: KeyCode) {
        editor.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn screen(editor: &mut Editor, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        fixtures::screen(&terminal)
    }

    fn open_offer_from_carla(editor: &mut Editor) {
        editor.sidebar = Sidebar::Users;
        editor.people.navigate(KeyCode::Home);
        assert_eq!(editor.people.selected().unwrap().name, "Carla");
        press(editor, KeyCode::Enter);
    }

    #[test]
    fn acceptance_waits_for_a_destination_and_cancelling_creates_nothing() {
        let (mut server, mut editor) = demo();
        let entries = server.state().entries.len();
        editor.sidebar = Sidebar::Users;
        editor.people.navigate(KeyCode::Down);
        assert!(screen(&mut editor, 120, 30).contains("Carla ↓1"));
        for confirm in [false, true] {
            open_offer_from_carla(&mut editor);
            assert!(screen(&mut editor, 120, 30).contains("Apuntes compartidos.md"));
            press(&mut editor, KeyCode::Enter);
            assert!(screen(&mut editor, 120, 30).contains("1. Elegir espacio"));
            press(&mut editor, KeyCode::Down);
            press(&mut editor, KeyCode::Enter);
            assert!(screen(&mut editor, 120, 30).contains("Raíz de Matemática"));
            screen(&mut editor, 18, 6);
            if !confirm {
                for _ in 0..3 {
                    press(&mut editor, KeyCode::Esc);
                }
                assert!(editor.palette.is_none());
                server.sync(&mut editor);
                assert_eq!(server.state().entries.len(), entries);
                assert_eq!(editor.incoming.len(), 1);
                continue;
            }
            press(&mut editor, KeyCode::Enter);
            server.sync(&mut editor);
        }
        // la copia llega a la raíz de Matemática sin cambiar el espacio abierto
        assert!(editor.palette.is_none());
        assert!(editor.incoming.is_empty());
        assert_eq!(editor.workspace, 0);
        let copy = server.entry("Apuntes compartidos.md");
        assert_eq!(copy.workspace, server.state().workspaces[1].id);
        assert_eq!(copy.parent, None);
        assert_eq!(server.markdown(copy.id), "Compartido");
    }

    #[test]
    fn stale_request_or_destination_is_reported_without_mutation() {
        let (mut server, mut editor) = demo();
        let request = editor.incoming[0].id;
        for (request, workspace) in [(TransferId(99), WorkspaceId(1)), (request, WorkspaceId(99))] {
            open_offer_from_carla(&mut editor);
            editor.apply_transfer(TransferRequest::Accept {
                request,
                workspace,
                parent: None,
            });
            server.sync(&mut editor);
            assert!(screen(&mut editor, 120, 30).contains("ya no existe"));
            press(&mut editor, KeyCode::Esc);
            press(&mut editor, KeyCode::Esc);
        }
        assert_eq!(editor.incoming.len(), 1);
        assert_eq!(server.state().directory.incoming.len(), 1);
    }
}
