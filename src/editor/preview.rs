use super::{Editor, Pane, buffer::NoteBuffer};
use crate::Update;
use ratatui::{
    crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    layout::Rect,
};
use std::sync::Arc;

pub(super) struct Horizontal {
    width: usize,
    offset: usize,
}

impl NoteBuffer {
    pub(super) fn refresh_horizontal(&mut self) {
        let mut blocks = std::collections::HashMap::<usize, Horizontal>::new();
        for (line, info) in self.preview.lines.iter().zip(&self.preview_info) {
            if let Some(id) = info.block {
                let block = blocks.entry(id).or_insert_with(|| Horizontal {
                    width: 0,
                    offset: self.horizontal.get(&id).map_or(0, |block| block.offset),
                });
                block.width = block.width.max(line.width());
            }
        }
        self.horizontal = blocks;
        self.active_block = self
            .active_block
            .filter(|id| self.horizontal.contains_key(id));
    }

    pub(super) fn horizontal_offset(&self, row: usize, width: u16) -> usize {
        self.preview_info
            .get(row)
            .and_then(|info| info.block)
            .and_then(|id| self.horizontal.get(&id))
            .map_or(0, |block| {
                block
                    .offset
                    .min(block.width.saturating_sub(usize::from(width)))
            })
    }

    pub(super) fn scroll_horizontal(
        &mut self,
        key: KeyCode,
        area: Rect,
        pointer: Option<u16>,
    ) -> Update {
        let top = self.preview_offset(area);
        let visible =
            &self.preview_info[top..(top + usize::from(area.height)).min(self.preview_info.len())];
        let target = if let Some(row) = pointer {
            self.preview_info
                .get(top + usize::from(row - area.y))
                .and_then(|info| info.block)
        } else if let Some(id) = self
            .active_block
            .filter(|id| visible.iter().any(|info| info.block == Some(*id)))
        {
            Some(id)
        } else {
            visible
                .iter()
                .filter_map(|info| info.block)
                .find(|id| self.horizontal[id].width > usize::from(area.width))
        };
        let Some(id) = target else {
            return Update::Idle;
        };
        self.active_block = Some(id);
        let Some(block) = self.horizontal.get_mut(&id) else {
            return Update::Idle;
        };
        let maximum = block.width.saturating_sub(usize::from(area.width));
        let current = block.offset.min(maximum);
        let next = if key == KeyCode::Left {
            current.saturating_sub(4)
        } else {
            current.saturating_add(4).min(maximum)
        };
        if current == next {
            return Update::Idle;
        }
        block.offset = next;
        self.selected_link = None;
        Update::Draw
    }

    fn link_at(&self, area: Rect, column: u16, row: u16) -> Option<(usize, usize)> {
        if !area.contains((column, row).into()) {
            return None;
        }
        let top = self.preview_offset(area);
        let row = top + usize::from(row - area.y);
        let info = self.preview_info.get(row)?;
        let column = usize::from(column - area.x) + self.horizontal_offset(row, area.width);
        info.links
            .iter()
            .position(|link| link.columns.contains(&column))
            .map(|index| (row, index))
    }

    fn cycle_link(&mut self, backward: bool, area: Rect) -> Update {
        let mut links = Vec::new();
        let mut previous = None;
        for (row, info) in self.preview_info.iter().enumerate() {
            for (index, link) in info.links.iter().enumerate() {
                if !previous.is_some_and(|previous| Arc::ptr_eq(previous, &link.destination)) {
                    links.push((row, index));
                    previous = Some(&link.destination);
                }
            }
        }
        if links.is_empty() {
            return Update::Idle;
        }
        let selected = self
            .selected_link
            .and_then(|(row, index)| self.preview_info.get(row)?.links.get(index));
        let current = selected.and_then(|selected| {
            links.iter().position(|&(row, index)| {
                Arc::ptr_eq(
                    &selected.destination,
                    &self.preview_info[row].links[index].destination,
                )
            })
        });
        let index = match current {
            Some(index) if backward => (index + links.len() - 1) % links.len(),
            Some(index) => (index + 1) % links.len(),
            None if backward => links.len() - 1,
            None => 0,
        };
        let (row, index) = links[index];
        self.selected_link = Some((row, index));
        let mut top = self.preview_offset(area);
        if row < top {
            top = row;
        } else if row >= top + usize::from(area.height) {
            top = row + 1 - usize::from(area.height);
        }
        let info = &self.preview_info[row];
        let link = &info.links[index];
        let column = self.horizontal_offset(row, area.width);
        self.active_block = info.block;
        if let Some(id) = info.block
            && (link.columns.start < column || link.columns.end > column + usize::from(area.width))
        {
            self.horizontal
                .get_mut(&id)
                .expect("bloque del enlace")
                .offset = link.columns.start;
        }
        self.preview_scroll = top;
        Update::Draw
    }

    fn activate_link(&mut self) -> Update {
        let Some(link) = self
            .selected_link
            .and_then(|(row, index)| self.preview_info.get(row)?.links.get(index))
        else {
            return Update::Idle;
        };
        if let Some(anchor) = link.destination.strip_prefix('#') {
            let row = crate::links::fragment(anchor).and_then(|anchor| {
                if anchor.is_empty() {
                    Some(0)
                } else {
                    self.preview_anchors.get(&anchor).copied()
                }
            });
            if let Some(row) = row {
                self.preview_scroll = row;
                self.active_block = None;
                self.selected_link = None;
            } else {
                self.notice = Some("No se encontró ese encabezado en esta nota.");
            }
            Update::Draw
        } else {
            Update::OpenLink(Arc::clone(&link.destination))
        }
    }
}

impl Editor {
    pub(super) fn preview_key(&mut self, key: KeyEvent) -> Option<Update> {
        if self.pane != Pane::Preview || self.sidebar.focused() {
            return None;
        }
        let area = self.preview_area;
        match (key.code, key.modifiers) {
            (KeyCode::Tab | KeyCode::BackTab, KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Some(self.note_mut().map_or(Update::Idle, |note| {
                    note.cycle_link(
                        key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT),
                        area,
                    )
                }))
            }
            (KeyCode::Enter, KeyModifiers::NONE) => Some(if key.kind == KeyEventKind::Press {
                self.note_mut()
                    .map_or(Update::Idle, NoteBuffer::activate_link)
            } else {
                Update::Idle
            }),
            _ => None,
        }
    }

    pub(super) fn preview_mouse(&mut self, mouse: MouseEvent) -> Option<Update> {
        let area = self.preview_area;
        if !area.contains((mouse.column, mouse.row).into()) {
            return None;
        }
        let key = match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) if mouse.modifiers.is_empty() => {
                self.focus(Pane::Preview);
                let Some(note) = self.note_mut() else {
                    return Some(Update::Draw);
                };
                let row = note.preview_offset(area) + usize::from(mouse.row - area.y);
                note.active_block = note.preview_info.get(row).and_then(|info| info.block);
                note.selected_link = note.link_at(area, mouse.column, mouse.row);
                return Some(if note.selected_link.is_some() {
                    note.activate_link()
                } else {
                    Update::Draw
                });
            }
            MouseEventKind::ScrollLeft => KeyCode::Left,
            MouseEventKind::ScrollRight => KeyCode::Right,
            MouseEventKind::ScrollUp if mouse.modifiers == KeyModifiers::SHIFT => KeyCode::Left,
            MouseEventKind::ScrollDown if mouse.modifiers == KeyModifiers::SHIFT => KeyCode::Right,
            MouseEventKind::ScrollUp => KeyCode::Up,
            MouseEventKind::ScrollDown => KeyCode::Down,
            _ => return None,
        };
        let update = self.note_mut().map_or(Update::Idle, |note| {
            if matches!(key, KeyCode::Left | KeyCode::Right) {
                note.scroll_horizontal(key, area, Some(mouse.row))
            } else {
                note.scroll_preview(key, area)
            }
        });
        if matches!(update, Update::Draw) {
            self.focus(Pane::Preview);
        }
        Some(update)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ratatui::{Terminal, backend::TestBackend, crossterm::event::Event};

    fn editor(text: &str) -> Editor {
        let (_server, mut editor) = crate::editor::fixtures::demo();
        let note = editor.note_mut().unwrap();
        note.source.set_text(text);
        note.text_changed();
        editor.focus(Pane::Preview);
        editor.mouse_capture = true;
        editor
    }

    fn press(editor: &mut Editor, code: KeyCode, modifiers: KeyModifiers) -> Update {
        editor.handle(Event::Key(KeyEvent::new(code, modifiers)))
    }

    fn row(terminal: &Terminal<TestBackend>, area: Rect) -> String {
        (area.left()..area.right())
            .map(|x| terminal.backend().buffer()[(x, area.y)].symbol())
            .collect()
    }

    #[test]
    fn horizontal_table_scrolling_works_with_keyboard_and_mouse_without_shifting_prose() {
        let mut editor = editor(
            "Párrafo con muchas palabras para comprobar el margen del panel y su ajuste.\n\n| A | B |\n|---|---|\n| dato | abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 |\n\nCierre.",
        );
        let mut terminal = Terminal::new(TestBackend::new(120, 16)).unwrap();
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let paragraph = row(&terminal, editor.preview_area);
        let table_row = editor
            .note()
            .unwrap()
            .preview
            .lines
            .iter()
            .position(|line| line.to_string().contains("dato"))
            .unwrap();
        editor.note_mut().unwrap().preview_scroll = table_row;
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let mut table_area = editor.preview_area;
        table_area.y +=
            u16::try_from(table_row - editor.note().unwrap().preview_offset(table_area)).unwrap();
        let before = row(&terminal, table_area);
        press(&mut editor, KeyCode::Right, KeyModifiers::SHIFT);
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        assert_ne!(before, row(&terminal, table_area));
        let column = editor
            .note()
            .unwrap()
            .horizontal_offset(table_row, editor.preview_area.width);
        let area = editor.preview_area;
        editor.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollRight,
            column: area.x + 1,
            row: table_area.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(
            editor
                .note()
                .unwrap()
                .horizontal_offset(table_row, area.width)
                > column
        );
        editor.note_mut().unwrap().preview_scroll = 0;
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        assert_eq!(paragraph, row(&terminal, editor.preview_area));
    }

    #[test]
    fn links_are_activated_by_keyboard_or_click_with_wrapping_and_table_offsets() {
        let mut editor = editor(
            "界 [e\u{301} enlace](https://example.org/long-address)\n\n[Ir al destino](#destino)\n\n| Imagen |\n|---|\n| ![remota](https://example.org/image.png) |\n\n## Destino\n\nFinal.",
        );
        let original = editor.note().unwrap().source.text();
        let mut terminal = Terminal::new(TestBackend::new(100, 18)).unwrap();
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        press(&mut editor, KeyCode::Tab, KeyModifiers::NONE);
        assert!(matches!(
            press(&mut editor, KeyCode::Enter, KeyModifiers::NONE),
            Update::OpenLink(url) if url.as_ref() == "https://example.org/long-address"
        ));
        press(&mut editor, KeyCode::Tab, KeyModifiers::NONE);
        assert!(matches!(
            press(&mut editor, KeyCode::Enter, KeyModifiers::NONE),
            Update::Draw
        ));
        assert!(editor.note().unwrap().preview_scroll > 0);
        press(&mut editor, KeyCode::BackTab, KeyModifiers::SHIFT);
        terminal.draw(|frame| editor.draw(frame)).unwrap();
        let note = editor.note().unwrap();
        let (target_row, target_index) = note.selected_link.unwrap();
        let target = &note.preview_info[target_row].links[target_index];
        assert_eq!(target.destination.as_ref(), "https://example.org/image.png");
        let top = note.preview_offset(editor.preview_area);
        let left = note.horizontal_offset(target_row, editor.preview_area.width);
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: editor.preview_area.x + u16::try_from(target.columns.start - left).unwrap(),
            row: editor.preview_area.y + u16::try_from(target_row - top).unwrap(),
            modifiers: KeyModifiers::NONE,
        };
        assert!(matches!(
            editor.handle(Event::Mouse(mouse)),
            Update::OpenLink(url) if url.as_ref() == "https://example.org/image.png"
        ));
        assert_eq!(editor.note().unwrap().source.text(), original);
    }

    #[test]
    fn encoded_unicode_anchors_and_explicit_html_ids_jump_to_their_heading() {
        for (link, heading, anchor) in [
            ("#caf%C3%A9", "## Café", "café"),
            (
                "#con%20espacio",
                "<h2 id=\"con espacio\">Título</h2>",
                "con espacio",
            ),
        ] {
            let mut editor = editor(&format!("[Ir]({link})\n\n{heading}\n\nFinal."));
            let mut terminal = Terminal::new(TestBackend::new(100, 18)).unwrap();
            terminal.draw(|frame| editor.draw(frame)).unwrap();
            press(&mut editor, KeyCode::Tab, KeyModifiers::NONE);
            assert!(matches!(
                press(&mut editor, KeyCode::Enter, KeyModifiers::NONE),
                Update::Draw
            ));
            let note = editor.note().unwrap();
            assert_eq!(note.preview_scroll, note.preview_anchors[anchor]);
            assert!(note.notice.is_none());
        }
    }
}
