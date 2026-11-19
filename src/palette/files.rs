use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Line,
    widgets::{ListItem, Paragraph, Wrap},
};

use super::{Action, FileRequest, Mode, Palette};
use crate::workspace::{EntryId, FileAction, Tree};

impl Palette {
    pub(crate) fn file(
        action: FileAction,
        source: EntryId,
        tree: &Tree,
        has_unsaved_changes: bool,
    ) -> Self {
        let destinations = if matches!(action, FileAction::Move | FileAction::Copy) {
            tree.destinations(source)
        } else {
            Vec::new()
        };
        let current = tree.parent_folder(source);
        let selected = destinations
            .iter()
            .position(|(folder, _)| *folder == current)
            .unwrap_or_default();
        let mut palette = Self {
            mode: Mode::File {
                action,
                source,
                path: tree.entry_path(source),
                destinations,
                has_unsaved_changes,
                contents: tree.contents(source),
            },
            opening_key: Some(action.key()),
            ..Self::new()
        };
        if matches!(action, FileAction::Rename) {
            palette.input.set_text(tree.entry_title(source));
            palette.input.select_all();
        }
        palette.list.select(Some(selected));
        palette
    }

    pub(super) fn handle_file(&mut self, event: &Event) -> Option<Action> {
        let Mode::File {
            action,
            destinations,
            ..
        } = &self.mode
        else {
            return None;
        };
        if matches!(action, FileAction::Rename) {
            return None;
        }
        if let Event::Key(key) = event
            && key.modifiers == KeyModifiers::NONE
            && matches!(key.code, KeyCode::Up | KeyCode::Down)
            && !destinations.is_empty()
        {
            crate::selection::navigate(&mut self.list, key.code, destinations.len());
            self.error = None;
        }
        Some(Action::KeepOpen)
    }

    pub(super) fn confirm_file(&self) -> Action {
        let Mode::File {
            action,
            source,
            destinations,
            contents,
            ..
        } = &self.mode
        else {
            return Action::KeepOpen;
        };
        if matches!(action, FileAction::Delete) && *contents > 0 {
            return Action::KeepOpen;
        }
        let request = match action {
            FileAction::Rename => FileRequest::Rename {
                source: *source,
                title: self.input.text().into(),
            },
            FileAction::Delete => FileRequest::Delete(*source),
            FileAction::Close => {
                let EntryId::Note(id) = *source else {
                    return Action::KeepOpen;
                };
                FileRequest::SaveAndClose(id)
            }
            FileAction::Move | FileAction::Copy => {
                let Some((destination, _)) = self
                    .list
                    .selected()
                    .and_then(|index| destinations.get(index))
                else {
                    return Action::KeepOpen;
                };
                if matches!(action, FileAction::Move) {
                    FileRequest::Move {
                        source: *source,
                        destination: *destination,
                    }
                } else {
                    FileRequest::Copy {
                        source: *source,
                        destination: *destination,
                    }
                }
            }
        };
        Action::File(request)
    }

    pub(super) fn file_presentation(&self) -> (&'static str, u16, &'static str, &'static str) {
        let Mode::File {
            action, contents, ..
        } = self.mode
        else {
            unreachable!("formulario de archivo")
        };
        match action {
            FileAction::Delete if contents > 0 => ("Eliminar", 7, "", "Esc volver"),
            FileAction::Move => ("Mover", 10, "", "↑↓ destino · Enter mover · Esc cancelar"),
            FileAction::Copy => ("Copiar", 10, "", "↑↓ destino · Enter copiar · Esc cancelar"),
            FileAction::Delete => ("Eliminar", 7, "", "Enter eliminar · Esc cancelar"),
            FileAction::Rename => ("Renombrar", 7, "Nombre", "Enter renombrar · Esc cancelar"),
            FileAction::Close => (
                "Cerrar nota",
                7,
                "",
                "Enter guardar y cerrar · Esc cancelar",
            ),
        }
    }

    pub(super) fn draw_file(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Mode::File {
            action,
            path,
            destinations,
            has_unsaved_changes,
            contents,
            ..
        } = &self.mode
        else {
            return;
        };
        let show_drafts = matches!(action, FileAction::Copy) && *has_unsaved_changes;
        let [area, warning] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(u16::from(show_drafts)),
        ])
        .areas(area);
        match action {
            FileAction::Move | FileAction::Copy => {
                let items = destinations
                    .iter()
                    .map(|(_, path)| ListItem::new(path.as_str()));
                let list = crate::popup::list(items);
                frame.render_stateful_widget(list, area, &mut self.list);
            }
            FileAction::Rename => {
                frame.render_widget(
                    Paragraph::new(path.as_str()).wrap(Wrap { trim: true }),
                    area,
                );
            }
            FileAction::Delete if *contents > 0 => {
                let items = if *contents == 1 {
                    "elemento"
                } else {
                    "elementos"
                };
                frame.render_widget(
                    Paragraph::new(format!(
                        "La carpeta no está vacía: contiene {contents} {items}. Mueve o elimina \
                         su contenido antes de eliminarla."
                    ))
                    .style(Style::new().fg(crate::theme::warning()))
                    .wrap(Wrap { trim: true }),
                    area,
                );
            }
            FileAction::Delete => {
                let message = if *has_unsaved_changes {
                    "Se descartarán los cambios sin guardar."
                } else {
                    "Confirma la eliminación de esta entrada."
                };
                frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: true }), area);
            }
            FileAction::Close => {
                frame.render_widget(
                    Paragraph::new("Hay cambios pendientes. ¿Guardar y cerrar esta nota?")
                        .wrap(Wrap { trim: true }),
                    area,
                );
            }
        }
        if show_drafts {
            frame.render_widget(
                Line::styled(
                    "La copia incluye cambios sin guardar.",
                    Style::new().fg(crate::theme::warning()),
                ),
                warning,
            );
        }
    }
}
