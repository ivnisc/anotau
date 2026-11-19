use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::Rect,
    text::Line,
    widgets::{ListItem, ListState, Paragraph, Wrap},
};

use anotau_protocol::Resolution;

use super::{Action, Mode, Palette, Transfer, TransferRequest};
use crate::{
    inbox::Incoming,
    workspace::{FolderId, WorkspaceId},
};

pub(crate) struct Destination {
    pub(crate) id: WorkspaceId,
    pub(crate) name: String,
    pub(crate) folders: Vec<(Option<FolderId>, String)>,
}

pub(super) struct Receive {
    sender: String,
    offers: Vec<Incoming>,
    offer: usize,
    destinations: Vec<Destination>,
    stage: Stage,
}

enum Stage {
    Inbox,
    Review(Resolution),
    Workspace,
    Folder(usize),
}

impl Palette {
    pub(crate) fn receive(
        sender: String,
        offers: Vec<Incoming>,
        destinations: Vec<Destination>,
    ) -> Self {
        let stage = if offers.len() == 1 {
            Stage::Review(Resolution::Accept)
        } else {
            Stage::Inbox
        };
        Self {
            mode: Mode::Transfer(Transfer::Receive(Receive {
                sender,
                offers,
                offer: 0,
                destinations,
                stage,
            })),
            ..Self::new()
        }
    }

    pub(super) fn receive_back(&mut self) -> bool {
        let Mode::Transfer(Transfer::Receive(receive)) = &mut self.mode else {
            return false;
        };
        let (stage, selected) = match receive.stage {
            Stage::Folder(space) => (Stage::Workspace, space),
            Stage::Workspace => (Stage::Review(Resolution::Accept), 0),
            Stage::Review(_) if receive.offers.len() > 1 => (Stage::Inbox, receive.offer),
            _ => return false,
        };
        receive.stage = stage;
        self.list.select(Some(selected));
        self.error = None;
        true
    }
}

impl Receive {
    pub(super) fn count(&self) -> usize {
        match self.stage {
            Stage::Inbox => self.offers.len(),
            Stage::Review(_) => 0,
            Stage::Workspace => self.destinations.len(),
            Stage::Folder(space) => self.destinations[space].folders.len(),
        }
    }

    pub(super) fn choose(&mut self, key: KeyCode) {
        if let Stage::Review(choice) = &mut self.stage {
            match key {
                KeyCode::Left => *choice = Resolution::Accept,
                KeyCode::Right => *choice = Resolution::Reject,
                _ => {}
            }
        }
    }

    pub(super) fn confirm(&mut self, list: &mut ListState) -> Action {
        let index = list.selected().unwrap_or_default();
        match self.stage {
            Stage::Inbox if index < self.offers.len() => {
                self.offer = index;
                self.stage = Stage::Review(Resolution::Accept);
            }
            Stage::Review(Resolution::Accept) => self.stage = Stage::Workspace,
            Stage::Review(Resolution::Reject) => {
                if let Some(offer) = self.offers.get(self.offer) {
                    return Action::Transfer(TransferRequest::Reject(offer.id));
                }
            }
            Stage::Workspace if index < self.destinations.len() => {
                self.stage = Stage::Folder(index);
            }
            Stage::Folder(space) => {
                if let Some(offer) = self.offers.get(self.offer)
                    && let Some((parent, _)) = self.destinations[space].folders.get(index)
                {
                    return Action::Transfer(TransferRequest::Accept {
                        request: offer.id,
                        workspace: self.destinations[space].id,
                        parent: *parent,
                    });
                }
            }
            _ => {}
        }
        list.select(Some(0));
        Action::KeepOpen
    }

    pub(super) fn presentation(&self) -> (&'static str, u16, &'static str, &'static str) {
        let (title, label, footer) = match self.stage {
            Stage::Inbox => (
                "Archivos recibidos",
                "Solicitudes pendientes",
                "↑↓ archivo · Enter revisar · Esc cancelar",
            ),
            Stage::Review(_) => (
                "Recibir archivo",
                "Solicitud pendiente",
                "←→ aceptar/rechazar · Enter seguir · Esc cancelar",
            ),
            Stage::Workspace => (
                "Recibir archivo",
                "1. Elegir espacio",
                "↑↓ espacio · Enter elegir · Esc volver",
            ),
            Stage::Folder(_) => (
                "Recibir archivo",
                "2. Elegir carpeta",
                "↑↓ destino · Enter aceptar aquí · Esc volver",
            ),
        };
        (title, 13, label, footer)
    }

    pub(super) fn draw(&self, frame: &mut Frame<'_>, area: Rect, list: &mut ListState) {
        if let Stage::Review(choice) = self.stage {
            let title = self
                .offers
                .get(self.offer)
                .map_or("Sin solicitudes", |offer| offer.title.as_str());
            let selected = usize::from(choice == Resolution::Reject);
            let text = vec![
                Line::from(format!("De: {}", self.sender)),
                Line::from(format!("Archivo: {title}")),
                Line::default(),
                crate::popup::choice(&["Aceptar", "Rechazar"], selected, crate::theme::selected()),
                Line::default(),
                Line::from("Aceptar permite elegir destino antes de confirmar."),
            ];
            frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
            return;
        }
        let items: Vec<ListItem<'_>> = match self.stage {
            Stage::Inbox => self
                .offers
                .iter()
                .map(|offer| ListItem::new(offer.title.as_str()))
                .collect(),
            Stage::Workspace => self
                .destinations
                .iter()
                .map(|space| ListItem::new(space.name.as_str()))
                .collect(),
            Stage::Folder(space) => self.destinations[space]
                .folders
                .iter()
                .map(|(id, path)| {
                    if id.is_none() {
                        ListItem::new(format!("Raíz de {path}"))
                    } else {
                        ListItem::new(path.as_str())
                    }
                })
                .collect(),
            Stage::Review(_) => unreachable!("confirmación renderizada antes de la lista"),
        };
        frame.render_stateful_widget(crate::popup::list(items), area, list);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{inbox::TransferId, people::UserId};

    fn popup() -> Palette {
        let space = anotau_protocol::Workspace {
            id: 92,
            title: "Matemática".into(),
            revision: 1,
        };
        let algebra = anotau_protocol::Entry {
            id: 93,
            workspace: 92,
            parent: None,
            title: "Álgebra".into(),
            kind: anotau_protocol::EntryKind::Folder,
            revision: 1,
            updated_at: 0,
        };
        let tree = crate::workspace::Tree::from_remote(space, vec![algebra]).unwrap();
        Palette::receive(
            "Carla".into(),
            vec![Incoming {
                id: TransferId(41),
                sender: UserId(7),
                title: "Apuntes.md".into(),
            }],
            vec![Destination {
                id: WorkspaceId(92),
                name: "Matemática".into(),
                folders: tree.folders(),
            }],
        )
    }

    #[test]
    fn acceptance_waits_for_destination_and_uses_ids_including_root() {
        for index in 0..2 {
            let mut palette = popup();
            let Mode::Transfer(Transfer::Receive(receive)) = &palette.mode else {
                panic!("recepción");
            };
            let expected = receive.destinations[0].folders[index].0;
            assert!(matches!(palette.confirm_transfer(), Action::KeepOpen));
            assert!(matches!(palette.confirm_transfer(), Action::KeepOpen));
            palette.list.select(Some(index));
            assert!(matches!(
                palette.confirm_transfer(),
                Action::Transfer(TransferRequest::Accept {
                    request: TransferId(41),
                    workspace: WorkspaceId(92),
                    parent,
                }) if parent == expected
            ));
            assert!(palette.receive_back());
            assert_eq!(palette.list.selected(), Some(0));
            assert!(palette.receive_back());
            assert!(!palette.receive_back());
        }
    }
}
