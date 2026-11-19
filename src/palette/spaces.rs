use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEventKind},
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use super::{Action, Mode, Palette};
use crate::workspace::WorkspaceId;

const NEW_SPACE: &str = "Nuevo espacio";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpaceAction {
    Browse,
    Create,
    Rename,
    Copy,
    Delete,
}

pub(crate) struct SpaceRequest {
    pub(crate) action: SpaceAction,
    pub(crate) id: Option<WorkspaceId>,
    pub(crate) title: String,
}

impl SpaceAction {
    pub(super) fn presentation(self) -> (&'static str, u16, &'static str, &'static str) {
        match self {
            Self::Browse => (
                "Espacios",
                12,
                "",
                "Flechas · Enter abrir · N nuevo · R renombrar · C copiar · D borrar",
            ),
            Self::Create => (
                "Crear espacio",
                7,
                "Nombre del espacio",
                "Enter crear · Esc volver",
            ),
            Self::Rename => (
                "Renombrar espacio",
                7,
                "Nombre del espacio",
                "Enter renombrar · Esc volver",
            ),
            Self::Copy => (
                "Copiar espacio",
                7,
                "Nombre de la copia",
                "Enter copiar · Esc volver",
            ),
            Self::Delete => ("Eliminar espacio", 8, "", "Enter eliminar · Esc volver"),
        }
    }
}

impl Palette {
    pub(crate) fn spaces(entries: Vec<(WorkspaceId, String)>, selected: usize) -> Self {
        let mut popup = Self {
            mode: Mode::Spaces {
                entries,
                action: SpaceAction::Browse,
                columns: 1,
            },
            opening_key: Some('w'),
            ..Self::new()
        };
        popup.list.select(Some(selected));
        popup
    }

    pub(super) fn handle_spaces(&mut self, event: &Event) -> Option<Action> {
        let Mode::Spaces {
            entries,
            action,
            columns,
        } = &self.mode
        else {
            return None;
        };
        if *action != SpaceAction::Browse {
            return (*action == SpaceAction::Delete).then_some(Action::KeepOpen);
        }
        if let Event::Key(key) = event
            && key.modifiers.is_empty()
        {
            let index = self.list.selected().unwrap_or_default();
            let count = entries.len() + 1;
            match key.code {
                KeyCode::Left => self.list.select(Some(index.saturating_sub(1))),
                KeyCode::Right => self.list.select(Some((index + 1).min(count - 1))),
                KeyCode::Up if index >= *columns => self.list.select(Some(index - columns)),
                KeyCode::Down if (index / columns + 1) * columns < count => {
                    self.list.select(Some((index + columns).min(count - 1)));
                }
                KeyCode::Home => self.list.select(Some(0)),
                KeyCode::End => self.list.select(Some(count - 1)),
                KeyCode::Char(ch) if key.kind == KeyEventKind::Press => {
                    let action = match ch {
                        'n' => SpaceAction::Create,
                        'r' => SpaceAction::Rename,
                        'c' => SpaceAction::Copy,
                        'd' => SpaceAction::Delete,
                        _ => return Some(Action::KeepOpen),
                    };
                    if index == entries.len() && action != SpaceAction::Create {
                        return Some(Action::KeepOpen);
                    }
                    self.begin_space_form(action);
                    self.opening_key = Some(ch);
                }
                _ => {}
            }
        }
        Some(Action::KeepOpen)
    }

    fn begin_space_form(&mut self, next: SpaceAction) {
        let Mode::Spaces {
            entries, action, ..
        } = &mut self.mode
        else {
            return;
        };
        *action = next;
        self.input.set_text(if next == SpaceAction::Create {
            ""
        } else {
            self.list
                .selected()
                .and_then(|index| entries.get(index))
                .map_or("", |(_, title)| title.as_str())
        });
        self.input.select_all();
        self.error = None;
    }

    pub(super) fn confirm_space(&mut self) -> Action {
        let Mode::Spaces {
            entries, action, ..
        } = &self.mode
        else {
            return Action::KeepOpen;
        };
        if *action == SpaceAction::Browse && self.list.selected() == Some(entries.len()) {
            self.begin_space_form(SpaceAction::Create);
            return Action::KeepOpen;
        }
        Action::Space(SpaceRequest {
            action: *action,
            id: self
                .list
                .selected()
                .filter(|_| *action != SpaceAction::Create)
                .and_then(|index| entries.get(index))
                .map(|(id, _)| *id),
            title: self.input.text().into(),
        })
    }

    pub(super) fn spaces_back(&mut self) -> bool {
        if let Mode::Spaces { action, .. } = &mut self.mode
            && *action != SpaceAction::Browse
        {
            *action = SpaceAction::Browse;
            self.error = None;
            return true;
        }
        false
    }

    pub(super) fn draw_spaces(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Mode::Spaces {
            entries,
            action,
            columns,
        } = &mut self.mode
        else {
            return;
        };
        if *action == SpaceAction::Create {
            return;
        }
        if *action == SpaceAction::Browse {
            if area.is_empty() {
                return;
            }
            let names = entries
                .iter()
                .map(|(_, name)| name.as_str())
                .chain([NEW_SPACE]);
            let cell_width = names
                .clone()
                .map(|name| Span::raw(name).width() + 4)
                .max()
                .unwrap_or(1);
            *columns = (usize::from(area.width) / cell_width)
                .max(1)
                .min(entries.len() + 1);
            let selected = self.list.selected().unwrap_or_default().min(entries.len());
            let row = selected / *columns;
            let offset = self.list.offset_mut();
            *offset = (*offset).min(row).max(
                row.saturating_add(1)
                    .saturating_sub(usize::from(area.height)),
            );
            let width = usize::from(area.width) / *columns;
            for (index, name) in names
                .enumerate()
                .skip(*offset * *columns)
                .take(usize::from(area.height) * *columns)
            {
                let marker = if index == entries.len() {
                    "> "
                } else if index == selected {
                    "› "
                } else {
                    "  "
                };
                let style = if index == selected {
                    crate::theme::selected()
                } else {
                    Style::new()
                };
                let cell = Rect::new(
                    area.x + u16::try_from(index % *columns * width).unwrap_or_default(),
                    area.y + u16::try_from(index / *columns - *offset).unwrap_or_default(),
                    u16::try_from(width).unwrap_or(area.width),
                    1,
                );
                frame.render_widget(
                    Line::from(vec![Span::styled(marker, style), Span::styled(name, style)]),
                    cell,
                );
            }
        } else {
            let title = self
                .list
                .selected()
                .and_then(|index| entries.get(index))
                .map_or("", |(_, name)| name.as_str());
            let message = if *action == SpaceAction::Delete {
                format!(
                    "Eliminar «{title}» y todo su contenido. Los cambios sin guardar impedirán \
                     la eliminación."
                )
            } else {
                format!("Espacio · {title}")
            };
            frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: true }), area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyEvent, KeyModifiers},
    };

    fn press(palette: &mut Palette, code: KeyCode) -> Action {
        palette.handle(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn rows(palette: &mut Palette, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| palette.draw(frame, None)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .chunks(usize::from(width))
            .map(|cells| cells.iter().map(ratatui::buffer::Cell::symbol).collect())
            .collect()
    }

    #[test]
    fn workspace_grid_wraps_navigates_and_keeps_the_last_action_visible_on_resize() {
        let entries = (0..18)
            .map(|id| (WorkspaceId(id), format!("Espacio{id:02}")))
            .collect();
        let mut palette = Palette::spaces(entries, 0);
        let wide = rows(&mut palette, 100, 24);
        assert_eq!(
            wide.iter().position(|row| row.contains("Espacio00")),
            wide.iter().position(|row| row.contains("Espacio01"))
        );
        press(&mut palette, KeyCode::Right);
        assert_eq!(palette.list.selected(), Some(1));
        press(&mut palette, KeyCode::Down);
        let Mode::Spaces { columns, .. } = palette.mode else {
            panic!("espacios")
        };
        assert_eq!(palette.list.selected(), Some(1 + columns));
        press(&mut palette, KeyCode::Home);
        let narrow = rows(&mut palette, 22, 12);
        assert_eq!(
            narrow
                .iter()
                .position(|row| row.contains("Espacio01"))
                .unwrap(),
            narrow
                .iter()
                .position(|row| row.contains("Espacio00"))
                .unwrap()
                + 1
        );
        press(&mut palette, KeyCode::End);
        assert!(
            rows(&mut palette, 22, 12)
                .join("\n")
                .contains("> Nuevo espacio")
        );
        assert!(
            rows(&mut palette, 100, 24)
                .join("\n")
                .contains("> Nuevo espacio")
        );
        assert_eq!(palette.list.selected(), Some(18));
        rows(&mut palette, 1, 1);
    }
}
