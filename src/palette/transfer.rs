use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode},
    layout::Rect,
    text::Line,
    widgets::{ListItem, Paragraph, Wrap},
};

use anotau_protocol::Resolution;

use super::receive::Receive;
use super::{Action, Mode, Palette};
use crate::{
    inbox::TransferId,
    people::{Contact, Person, UserId},
    workspace::{FolderId, NoteId, WorkspaceId},
};

pub(super) enum Transfer {
    Receive(Receive),
    Recipient {
        note: NoteId,
        title: String,
        people: Vec<Person>,
    },
    Note {
        person: Person,
        notes: Vec<(NoteId, String)>,
    },
    Contact(Person),
    RespondContact {
        person: Person,
        id: i64,
        choice: Resolution,
    },
    Confirm {
        note: NoteId,
        title: String,
        person: Person,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum TransferRequest {
    Contact(UserId),
    ResolveContact {
        id: i64,
        resolution: Resolution,
    },
    Copy {
        note: NoteId,
        recipient: UserId,
    },
    Accept {
        request: TransferId,
        workspace: WorkspaceId,
        parent: Option<FolderId>,
    },
    Reject(TransferId),
}

impl Palette {
    pub(crate) fn transfer_note(note: NoteId, title: String, people: Vec<Person>) -> Self {
        Self {
            mode: Mode::Transfer(Transfer::Recipient {
                note,
                title,
                people,
            }),
            opening_key: Some('t'),
            ..Self::new()
        }
    }

    pub(crate) fn transfer_person(person: Person, notes: Vec<(NoteId, String)>) -> Self {
        let stage = if let Contact::Incoming(id) = person.contact {
            Transfer::RespondContact {
                person,
                id,
                choice: Resolution::Accept,
            }
        } else if person.contact == Contact::Accepted {
            Transfer::Note { person, notes }
        } else {
            Transfer::Contact(person)
        };
        Self {
            mode: Mode::Transfer(stage),
            opening_key: Some('t'),
            ..Self::new()
        }
    }

    pub(super) fn handle_transfer(&mut self, event: &Event) -> Option<Action> {
        let Mode::Transfer(stage) = &mut self.mode else {
            return None;
        };
        let count = match stage {
            Transfer::Recipient { people, .. } => people.len(),
            Transfer::Note { notes, .. } => notes.len(),
            Transfer::Receive(receive) => receive.count(),
            _ => 0,
        };
        if let Event::Key(key) = event
            && key.modifiers.is_empty()
        {
            if let Transfer::RespondContact { choice, .. } = stage {
                match key.code {
                    KeyCode::Left => *choice = Resolution::Accept,
                    KeyCode::Right => *choice = Resolution::Reject,
                    _ => {}
                }
            }
            if let Transfer::Receive(receive) = stage {
                receive.choose(key.code);
            }
            if matches!(key.code, KeyCode::Up | KeyCode::Down) {
                crate::selection::navigate(&mut self.list, key.code, count);
            }
        }
        Some(Action::KeepOpen)
    }

    pub(super) fn confirm_transfer(&mut self) -> Action {
        if let Mode::Transfer(Transfer::Receive(receive)) = &mut self.mode {
            self.error = None;
            return receive.confirm(&mut self.list);
        }
        let Mode::Transfer(stage) = &mut self.mode else {
            return Action::KeepOpen;
        };
        let index = self.list.selected().unwrap_or_default();
        let next = match stage {
            Transfer::Receive(_) => unreachable!("recepción procesada antes del envío"),
            Transfer::Recipient {
                note,
                title,
                people,
            } => {
                if index >= people.len() {
                    return Action::KeepOpen;
                }
                let person = people.swap_remove(index);
                if person.contact == Contact::Accepted {
                    Transfer::Confirm {
                        note: *note,
                        title: std::mem::take(title),
                        person,
                    }
                } else if let Contact::Incoming(id) = person.contact {
                    Transfer::RespondContact {
                        person,
                        id,
                        choice: Resolution::Accept,
                    }
                } else {
                    Transfer::Contact(person)
                }
            }
            Transfer::Note { person, notes } => {
                if index >= notes.len() {
                    return Action::KeepOpen;
                }
                let (note, title) = notes.swap_remove(index);
                Transfer::Confirm {
                    note,
                    title,
                    person: Person {
                        name: std::mem::take(&mut person.name),
                        ..*person
                    },
                }
            }
            Transfer::RespondContact { id, choice, .. } => {
                return Action::Transfer(TransferRequest::ResolveContact {
                    id: *id,
                    resolution: *choice,
                });
            }
            Transfer::Contact(person) if person.contact == Contact::Rejected => {
                self.error = Some("La solicitud de contacto fue rechazada.".into());
                return Action::KeepOpen;
            }
            Transfer::Contact(person) if person.contact == Contact::Pending => {
                self.error = Some("Ya hay una solicitud pendiente. Espera su respuesta.".into());
                return Action::KeepOpen;
            }
            Transfer::Contact(person) => {
                return Action::Transfer(TransferRequest::Contact(person.id));
            }
            Transfer::Confirm { note, person, .. } => {
                return Action::Transfer(TransferRequest::Copy {
                    note: *note,
                    recipient: person.id,
                });
            }
        };
        self.mode = Mode::Transfer(next);
        self.list.select(Some(0));
        Action::KeepOpen
    }

    pub(super) fn transfer_presentation(&self) -> (&'static str, u16, &'static str, &'static str) {
        let Mode::Transfer(stage) = &self.mode else {
            unreachable!("formulario de transferencia")
        };
        match stage {
            Transfer::Receive(receive) => receive.presentation(),
            Transfer::Recipient { .. } => (
                "Transferir nota",
                12,
                "Destinatario",
                "↑↓ destinatario · Enter elegir · Esc cancelar",
            ),
            Transfer::Note { .. } => (
                "Transferir nota",
                12,
                "Nota",
                "↑↓ nota · Enter elegir · Esc cancelar",
            ),
            Transfer::RespondContact { .. } => (
                "Responder contacto",
                9,
                "Consentimiento previo",
                "← aceptar · → rechazar · Enter confirmar · Esc cancelar",
            ),
            Transfer::Contact(person) if person.contact == Contact::Rejected => {
                ("Contacto rechazado", 9, "Estado", "Esc volver")
            }
            Transfer::Contact(_) => (
                "Solicitar contacto",
                9,
                "Consentimiento previo",
                "Enter solicitar contacto · Esc cancelar",
            ),
            Transfer::Confirm { .. } => (
                "Transferir nota",
                9,
                "Enviar copia",
                "Enter solicitar envío · Esc cancelar",
            ),
        }
    }

    pub(super) fn draw_transfer(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Mode::Transfer(stage) = &self.mode else {
            return;
        };
        if let Transfer::Receive(receive) = stage {
            receive.draw(frame, area, &mut self.list);
            return;
        }
        let items: Vec<ListItem<'_>> = match stage {
            Transfer::Recipient { people, .. } => people
                .iter()
                .map(|person| ListItem::new(person.name.as_str()))
                .collect(),
            Transfer::Note { notes, .. } => notes
                .iter()
                .map(|(_, title)| ListItem::new(title.as_str()))
                .collect(),
            _ => Vec::new(),
        };
        let message = match stage {
            Transfer::Recipient { title, people, .. } if people.is_empty() => {
                format!("Nota · {title}\n\nNo hay destinatarios disponibles.")
            }
            Transfer::Note { notes, person } if notes.is_empty() => {
                format!("Destino · {}\n\nNo hay notas en este espacio.", person.name)
            }
            Transfer::RespondContact { person, choice, .. } => {
                let selected = usize::from(*choice == Resolution::Reject);
                let text = vec![
                    Line::from(format!("Solicitud de {}", person.name)),
                    Line::default(),
                    crate::popup::choice(
                        &["Aceptar", "Rechazar"],
                        selected,
                        crate::theme::selected(),
                    ),
                ];
                frame.render_widget(Paragraph::new(text), area);
                return;
            }
            Transfer::Contact(person) => format!(
                "Usuario · {}\n\n{}",
                person.name,
                if person.contact == Contact::Rejected {
                    "Este usuario rechazó la solicitud de contacto."
                } else if person.contact == Contact::Pending {
                    "Solicitud de contacto pendiente de aceptación."
                } else {
                    "Se requiere aceptar el contacto antes de enviar archivos. Cada copia \
                     también necesita aprobación."
                }
            ),
            Transfer::Confirm { title, person, .. } => format!(
                "Nota · {title}\nDestinatario · {}\n\n\
                 El destinatario deberá aceptar la copia. El original se conserva.",
                person.name
            ),
            _ => String::new(),
        };
        if items.is_empty() {
            frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: true }), area);
        } else {
            frame.render_stateful_widget(crate::popup::list(items), area, &mut self.list);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{editor::fixtures::Server, people::Presence};

    #[test]
    fn transitions_preserve_invalid_selection_and_keep_confirm_for_retries() {
        let tree = Server::new().tree();
        let note = tree.first_note().unwrap();
        let person = Person {
            id: UserId(17),
            name: "Carla".into(),
            presence: Presence::Active,
            contact: Contact::Accepted,
        };
        for mut palette in [
            Palette::transfer_note(note, "Título".into(), vec![person.clone()]),
            Palette::transfer_person(person, vec![(note, "Título".into())]),
        ] {
            palette.list.select(Some(99));
            assert!(matches!(palette.confirm_transfer(), Action::KeepOpen));
            assert!(
                matches!(
                    &palette.mode,
                    Mode::Transfer(Transfer::Recipient { title, people, .. })
                        if title == "Título" && people[0].name == "Carla"
                ) || matches!(
                    &palette.mode,
                    Mode::Transfer(Transfer::Note { person, notes })
                        if person.name == "Carla" && notes[0].1 == "Título"
                )
            );
            palette.list.select(Some(0));
            assert!(matches!(palette.confirm_transfer(), Action::KeepOpen));
            for _ in 0..2 {
                assert!(matches!(
                    palette.confirm_transfer(),
                    Action::Transfer(TransferRequest::Copy { note: id, recipient: UserId(17) })
                        if id == note
                ));
                assert!(matches!(
                    &palette.mode,
                    Mode::Transfer(Transfer::Confirm { title, person, .. })
                        if title == "Título" && person.name == "Carla"
                ));
                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 20)).unwrap();
                terminal.draw(|frame| palette.draw(frame, None)).unwrap();
            }
        }
    }
}
