mod actions;
mod remote;
pub(crate) use remote::group_entries;

use std::{cell::OnceCell, collections::HashMap};

use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::Rect,
    text::{Line, Span},
    widgets::{ListItem, ListState},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct WorkspaceId(pub(crate) i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NoteId {
    workspace: WorkspaceId,
    entry: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FolderId {
    workspace: WorkspaceId,
    entry: i64,
}

struct Entry {
    title: String,
    revision: i64,
    updated_at: i64,
    parent: Option<i64>,
    kind: Kind,
}

enum Kind {
    Folder { children: Vec<i64>, expanded: bool },
    Note,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryId {
    Note(NoteId),
    Folder(FolderId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileAction {
    Move,
    Copy,
    Delete,
    Rename,
    Close,
}

#[derive(Clone, Copy)]
struct Row {
    entry: i64,
    depth: usize,
}

pub(crate) enum Navigation {
    Changed,
    Open(NoteId),
    Idle,
}

pub(crate) struct Tree {
    workspace: WorkspaceId,
    pub(crate) name: String,
    pub(crate) revision: i64,
    entries: HashMap<i64, Entry>,
    roots: Vec<i64>,
    rows: OnceCell<Vec<Row>>,
    list: ListState,
}

impl Tree {
    pub(crate) fn new(workspace: WorkspaceId, name: &str) -> Self {
        Self {
            workspace,
            name: name.into(),
            revision: 1,
            entries: HashMap::new(),
            roots: Vec::new(),
            rows: OnceCell::new(),
            list: ListState::default().with_selected(Some(0)),
        }
    }

    pub(crate) fn id(&self) -> WorkspaceId {
        self.workspace
    }

    pub(crate) fn has_note(&self, id: NoteId) -> bool {
        id.workspace == self.workspace
            && self
                .entries
                .get(&id.entry)
                .is_some_and(|entry| matches!(entry.kind, Kind::Note))
    }

    pub(crate) fn notes(&self) -> Vec<(NoteId, String)> {
        let mut notes = self
            .entries
            .iter()
            .filter(|(_, value)| matches!(value.kind, Kind::Note))
            .map(|(&entry, value)| {
                let id = NoteId {
                    workspace: self.workspace,
                    entry,
                };
                (id, format!("{} / {}", self.note_folder(id), value.title))
            })
            .collect::<Vec<_>>();
        notes.sort_by(|a, b| a.1.cmp(&b.1));
        notes
    }

    pub(crate) fn title(&self, id: NoteId) -> &str {
        &self.note_entry(id).title
    }

    fn note_entry(&self, id: NoteId) -> &Entry {
        debug_assert_eq!(id.workspace, self.workspace, "nota de otro workspace");
        &self.entries[&id.entry]
    }

    pub(crate) fn note_folder(&self, id: NoteId) -> String {
        self.folder_path(self.note_entry(id).parent)
    }

    pub(crate) fn destination(&self) -> (Option<FolderId>, String) {
        let rows = self.visible();
        let selected = self.list.selected().and_then(|row| row.checked_sub(1));
        let parent = selected.and_then(|row| rows.get(row)).and_then(|row| {
            let entry = &self.entries[&row.entry];
            match entry.kind {
                Kind::Folder { .. } => Some(row.entry),
                Kind::Note => entry.parent,
            }
        });
        let id = parent.map(|entry| FolderId {
            workspace: self.workspace,
            entry,
        });
        (id, self.folder_path(parent))
    }

    fn folder_path(&self, mut folder: Option<i64>) -> String {
        let mut parts = Vec::new();
        while let Some(index) = folder {
            let entry = &self.entries[&index];
            parts.push(entry.title.as_str());
            folder = entry.parent;
        }
        parts.push(self.name.as_str());
        parts.reverse();
        parts.join(" / ")
    }

    fn visible(&self) -> &[Row] {
        self.rows.get_or_init(|| {
            let mut rows = Vec::new();
            let mut pending: Vec<_> = self.roots.iter().rev().map(|&entry| (entry, 0)).collect();
            while let Some((entry, depth)) = pending.pop() {
                rows.push(Row { entry, depth });
                if let Kind::Folder {
                    children,
                    expanded: true,
                } = &self.entries[&entry].kind
                {
                    pending.extend(children.iter().rev().map(|&child| (child, depth + 1)));
                }
            }
            rows
        })
    }

    pub(crate) fn navigate(&mut self, key: KeyCode) -> Navigation {
        let rows = self.visible();
        let count = rows.len();
        let index = self.list.selected().unwrap_or_default().min(count);
        let selected = index.checked_sub(1).and_then(|row| rows.get(row)).copied();
        let parent = if key == KeyCode::Left {
            selected
                .and_then(|row| self.entries[&row.entry].parent)
                .and_then(|parent| rows.iter().position(|row| row.entry == parent))
        } else {
            None
        };
        match key {
            KeyCode::Up => self.list.select(Some(index.saturating_sub(1))),
            KeyCode::Down => self.list.select(Some((index + 1).min(count))),
            KeyCode::Home => self.list.select(Some(0)),
            KeyCode::End => self.list.select(Some(count)),
            KeyCode::Left | KeyCode::Right | KeyCode::Enter => {
                let Some(row) = selected else {
                    return Navigation::Idle;
                };
                let Some(entry) = self.entries.get_mut(&row.entry) else {
                    return Navigation::Idle;
                };
                match (&mut entry.kind, key) {
                    (Kind::Folder { expanded, .. }, KeyCode::Enter) => *expanded = !*expanded,
                    (Kind::Folder { expanded, .. }, KeyCode::Right) if !*expanded => {
                        *expanded = true;
                    }
                    (Kind::Folder { children, .. }, KeyCode::Right) if !children.is_empty() => {
                        self.list.select(Some(index + 1));
                    }
                    (Kind::Folder { expanded, .. }, KeyCode::Left) if *expanded => {
                        *expanded = false;
                    }
                    (_, KeyCode::Left) => {
                        self.list.select(Some(parent.map_or(0, |row| row + 1)));
                    }
                    (Kind::Note, KeyCode::Enter) => {
                        return Navigation::Open(NoteId {
                            workspace: self.workspace,
                            entry: row.entry,
                        });
                    }
                    _ => return Navigation::Idle,
                }
            }
            _ => return Navigation::Idle,
        }
        if matches!(key, KeyCode::Left | KeyCode::Right | KeyCode::Enter) {
            self.rows.take();
        }
        Navigation::Changed
    }

    pub(crate) fn draw(&mut self, frame: &mut Frame<'_>, area: Rect, active: Option<NoteId>) {
        self.visible();
        let rows = self.rows.get().expect("filas inicializadas");
        let items = std::iter::once(ListItem::new("  Raíz")).chain(rows.iter().map(|row| {
            let entry = &self.entries[&row.entry];
            let marker = match entry.kind {
                Kind::Note if active.is_some_and(|id| id.entry == row.entry) => "● ",
                Kind::Note => "  ",
                Kind::Folder { expanded: true, .. } => "▾ ",
                Kind::Folder { .. } => "▸ ",
            };
            ListItem::new(Line::from(vec![
                Span::raw("  ".repeat(row.depth.min(usize::from(area.width) / 2))),
                Span::raw(marker),
                Span::raw(entry.title.as_str()),
            ]))
        }));
        let list = crate::popup::list(items);
        frame.render_stateful_widget(list, area, &mut self.list);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anotau_protocol as api;

    #[test]
    fn visible_rows_are_cached_and_invalidated_without_recursive_traversal() {
        let space = api::Workspace {
            id: 1,
            title: "Profundo".into(),
            revision: 1,
        };
        let entries = (1..=4096)
            .map(|id| api::Entry {
                id,
                workspace: 1,
                parent: (id > 1).then(|| id - 1),
                title: "Nivel".into(),
                kind: api::EntryKind::Folder,
                revision: 1,
                updated_at: 0,
            })
            .collect();
        let mut tree = Tree::from_remote(space, entries).unwrap();
        assert_eq!(tree.visible().len(), 1);
        tree.reveal_server(4096);
        assert_eq!(tree.visible().len(), 4096);
        assert_eq!(tree.visible().last().unwrap().depth, 4095);
        let rows = tree.visible().as_ptr();
        assert_eq!(tree.visible().as_ptr(), rows);
        tree.navigate(KeyCode::Home);
        tree.navigate(KeyCode::Down);
        tree.navigate(KeyCode::Enter);
        assert_eq!(tree.visible().len(), 1);
        tree.navigate(KeyCode::Enter);
        assert_eq!(tree.visible().len(), 4096);
    }
}
