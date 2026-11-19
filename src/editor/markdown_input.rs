use rat_text::TextPosition;

use super::buffer::NoteBuffer;
use crate::markdown::ListItem;

struct Marker<'a> {
    indent: usize,
    content_start: usize,
    delimiter: char,
    task: bool,
    content: &'a str,
}

fn indentation(text: &str) -> (usize, usize) {
    let bytes = text
        .bytes()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    (bytes, columns(&text[..bytes]))
}

fn columns(prefix: &str) -> usize {
    prefix.bytes().fold(0, |column, byte| {
        column + if byte == b'\t' { 4 - column % 4 } else { 1 }
    })
}

fn list_marker(line: &str) -> Option<Marker<'_>> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (indent, _) = indentation(line);
    let rest = &line[indent..];
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let marker_end = if digits == 0 { 1 } else { digits + 1 };
    let delimiter = char::from(*rest.as_bytes().get(marker_end - 1)?);
    if !((digits == 0 && matches!(delimiter, '-' | '+' | '*'))
        || ((1..=9).contains(&digits) && matches!(delimiter, '.' | ')')))
    {
        return None;
    }
    let remainder = rest.get(marker_end..)?;
    if !remainder.is_empty() && !remainder.starts_with([' ', '\t']) {
        return None;
    }
    let content = remainder.trim_start_matches([' ', '\t']);
    let content_start = line.len() - content.len();
    let task = ["[ ]", "[x]", "[X]"].iter().any(|prefix| {
        content
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
    });
    let content = if task {
        content[3..].trim_start_matches([' ', '\t'])
    } else {
        content
    };
    Some(Marker {
        indent,
        content_start,
        delimiter,
        task,
        content,
    })
}

impl NoteBuffer {
    pub(super) fn table_cell(&mut self, backward: bool) -> bool {
        let cursor = self.source.cursor();
        let Ok(byte) = self.source.try_byte_at(cursor).map(|range| range.start) else {
            return false;
        };
        let Ok(line_start) = self
            .source
            .try_byte_at((0, cursor.y))
            .map(|range| range.start)
        else {
            return false;
        };
        let Some(table) = self.tables.iter().find(|table| {
            table.range.start <= byte && byte <= table.range.end && line_start < table.range.end
        }) else {
            return false;
        };
        let target = if backward {
            table.cells.iter().rev().find(|cell| cell.end < byte)
        } else {
            table.cells.iter().find(|cell| cell.start > byte)
        };
        if let Some(cell) = target {
            let Ok(raw) = self.source.try_str_slice_byte(cell.clone()) else {
                return true;
            };
            let content = raw.trim();
            let start = cell.start + raw.len() - raw.trim_start().len();
            let end = start + content.len();
            if let (Ok(start), Ok(end)) = (
                self.source.try_byte_pos(start),
                self.source.try_byte_pos(end),
            ) {
                self.source.set_selection(start, end);
                self.source.scroll_cursor_to_visible();
            }
        }
        true
    }

    fn current_item(&self, marker: &Marker<'_>) -> Option<&ListItem> {
        let start = self
            .source
            .try_byte_at((0, self.source.cursor().y))
            .ok()?
            .start;
        self.items
            .iter()
            .rev()
            .find(|item| item.range.start >= start && item.range.start <= start + marker.indent)
    }

    pub(super) fn continue_list(&mut self) -> bool {
        let cursor = self.source.cursor();
        if self.source.has_selection()
            || self.source.try_line_width(cursor.y).ok() != Some(cursor.x)
        {
            return false;
        }
        if self.source.try_byte_at(cursor).ok().is_some_and(|range| {
            self.source
                .styles_at_match(
                    range.start.saturating_sub(1),
                    crate::markdown::BLOCK_CODE_HIGHLIGHT,
                )
                .is_some()
        }) {
            return false;
        }
        let Ok(line) = self.source.try_line_at(cursor.y) else {
            return false;
        };
        let Some(marker) = list_marker(&line) else {
            return false;
        };
        let Some(item) = self.current_item(&marker) else {
            return false;
        };
        let insertion = if marker.content.is_empty() {
            String::new()
        } else {
            let next = match item.number {
                Some(number) if number < 999_999_999 => {
                    format!("{}{} ", number + 1, marker.delimiter)
                }
                Some(_) => return false,
                None => format!("{} ", marker.delimiter),
            };
            format!(
                "{}{}{next}{}",
                self.source.newline,
                &line[..marker.indent],
                if marker.task { "[ ] " } else { "" }
            )
        };
        let empty = marker.content.is_empty();
        if empty {
            self.source.set_selection((0, cursor.y), cursor);
        }
        self.source.begin_undo_seq();
        self.source.insert_str(insertion);
        self.source.end_undo_seq();
        self.text_changed();
        true
    }

    pub(super) fn indent_list(&mut self, backward: bool) -> bool {
        if self.source.has_selection() {
            return false;
        }
        let cursor = self.source.cursor();
        let Ok(line) = self.source.try_line_at(cursor.y) else {
            return false;
        };
        let Some(marker) = list_marker(&line) else {
            return false;
        };
        let Some(item) = self.current_item(&marker) else {
            return false;
        };
        let range = item.range.clone();
        let (_, current_indent) = indentation(&line);
        let target = if backward {
            self.items
                .iter()
                .rev()
                .find(|parent| {
                    parent.range.start < range.start && parent.range.contains(&range.start)
                })
                .and_then(|parent| self.source.try_byte_pos(parent.range.start).ok())
                .and_then(|pos| self.source.try_line_at(pos.y).ok())
                .map_or(0, |line| indentation(&line).1)
        } else {
            let previous =
                self.items.iter().rev().find(|previous| {
                    previous.list == item.list && previous.range.start < range.start
                });
            let Some(previous) = previous else {
                return true;
            };
            let Ok(pos) = self.source.try_byte_pos(previous.range.start) else {
                return true;
            };
            let Ok(line) = self.source.try_line_at(pos.y) else {
                return true;
            };
            let Some(marker) = list_marker(&line) else {
                return true;
            };
            columns(&line[..marker.content_start])
        };
        if target == current_indent {
            return true;
        }
        let start = match self.source.try_byte_at((0, cursor.y)) {
            Ok(range) => range.start,
            Err(_) => return true,
        };
        let Ok(block) = self.source.try_str_slice_byte(start..range.end) else {
            return true;
        };
        let mut replacement = String::with_capacity(block.len());
        for line in block.split_inclusive('\n') {
            if line.trim().is_empty() {
                replacement.push_str(line);
                continue;
            }
            let (bytes, columns) = indentation(line);
            let shifted = columns
                .saturating_sub(current_indent)
                .saturating_add(target);
            replacement.extend(std::iter::repeat_n(' ', shifted));
            replacement.push_str(&line[bytes..]);
        }
        let x = cursor
            .x
            .saturating_sub(u32::try_from(marker.indent).unwrap_or(u32::MAX))
            .saturating_add(u32::try_from(target).unwrap_or(u32::MAX));
        let Ok(end) = self.source.try_byte_pos(range.end) else {
            return true;
        };
        self.source.begin_undo_seq();
        self.source.set_selection((0, cursor.y), end);
        self.source.insert_str(replacement);
        self.source.end_undo_seq();
        self.source
            .set_cursor(TextPosition::new(x, cursor.y), false);
        self.text_changed();
        true
    }

    pub(super) fn clamp_scroll_end(&mut self) -> bool {
        if self.source.vertical_offset() == 0 && self.source.sub_row_offset() == 0 {
            return false;
        }
        let Ok(end) = self.source.try_byte_pos(self.source.len_bytes()) else {
            return false;
        };
        let Some((_, row)) = self.source.pos_to_relative_screen(end) else {
            return false;
        };
        let Ok(row) = u16::try_from(row) else {
            return false;
        };
        let excess = self
            .source
            .inner
            .height
            .saturating_sub(row.saturating_add(1))
            .saturating_sub(2);
        excess > 0 && self.source.scroll_up(u32::from(excess))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::fixtures;
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    fn buffer(text: &str) -> NoteBuffer {
        let id = fixtures::Server::new().tree().first_note().unwrap();
        let mut note = NoteBuffer::new(id);
        note.loaded(anotau_protocol::Note {
            id: id.server(),
            markdown: text.into(),
            revision: 1,
            updated_at: 0,
        });
        note.source.focus.set(true);
        note
    }

    fn key(note: &mut NoteBuffer, code: KeyCode, modifiers: KeyModifiers) {
        note.handle_input(Event::Key(KeyEvent::new(code, modifiers)));
    }

    #[test]
    fn tabs_visit_cells_without_changing_markdown_or_splitting_unicode() {
        let text = "| A | B |\n|---|---|\n| e\u{301} | a\\|b |\n| 👩‍💻 | fin |";
        let mut note = buffer(text);
        note.source.set_cursor((2, 0), false);
        for content in ["B", "e\u{301}", "a\\|b", "👩‍💻", "fin", "fin"] {
            key(&mut note, KeyCode::Tab, KeyModifiers::NONE);
            assert_eq!(note.source.selected_text(), content);
        }
        key(&mut note, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(note.source.selected_text(), "👩‍💻");
        note.source.set_cursor((0, 1), false);
        key(&mut note, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(note.source.selected_text(), "e\u{301}");
        assert_eq!(note.source.text(), text);
        assert!(!note.modified);

        let mut literal = buffer("```\n| A | B |\n|---|---|\n```");
        literal.source.set_cursor((2, 1), false);
        assert!(!literal.table_cell(false));
    }

    #[test]
    fn enter_continues_semantic_numbers_and_unchecked_tasks_without_rewriting_existing_markers() {
        let mut note = buffer("7. uno\n99. dos");
        note.source.set_cursor((7, 1), false);
        key(&mut note, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(note.source.text(), "7. uno\n99. dos\n9. ");
        assert!(note.preview.to_string().contains("7. uno\n8. dos\n9."));
        key(&mut note, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(note.source.text(), "7. uno\n99. dos\n");

        let mut task = buffer("- [x] hecho");
        task.source.set_cursor((11, 0), false);
        key(&mut task, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(task.source.text(), "- [x] hecho\n- [ ] ");
        let mut literal = buffer("```\n- ejemplo\n```");
        literal.source.set_cursor((9, 1), false);
        key(&mut literal, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(literal.source.text(), "```\n- ejemplo\n\n```");
        for (source, expected) in [
            ("- ```\n  texto\n  ```", "- ```\n\n  texto\n  ```"),
            ("-     código", "-     código\n"),
            ("- `literal`", "- `literal`\n- "),
        ] {
            let mut note = buffer(source);
            let end = note
                .source
                .try_byte_pos(source.find('\n').unwrap_or(source.len()))
                .unwrap();
            note.source.set_cursor(end, false);
            key(&mut note, KeyCode::Enter, KeyModifiers::NONE);
            assert_eq!(note.source.text(), expected);
        }
    }
}
