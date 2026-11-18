use std::collections::{HashMap, HashSet};

use anotau_protocol as api;

use super::{Entry, EntryId, FolderId, Kind, NoteId, Tree};

impl NoteId {
    pub(crate) fn server(self) -> api::Id {
        self.entry
    }
}
impl FolderId {
    pub(crate) fn server(self) -> api::Id {
        self.entry
    }
}
impl EntryId {
    pub(crate) fn server(self) -> api::Id {
        match self {
            Self::Note(id) => id.entry,
            Self::Folder(id) => id.entry,
        }
    }
}

impl Tree {
    pub(crate) fn from_remote(
        space: api::Workspace,
        entries: Vec<api::Entry>,
    ) -> Result<Self, &'static str> {
        if space.id <= 0 || space.revision <= 0 {
            return Err("Identidad de espacio inválida.");
        }
        let mut tree = Self::new(super::WorkspaceId(space.id), "");
        tree.revision = space.revision;
        tree.name = space.title;
        let mut parents = Vec::with_capacity(entries.len());
        for entry in entries {
            if entry.id <= 0 || entry.workspace != space.id || entry.revision <= 0 {
                return Err("Identidad de entrada inválida.");
            }
            parents.push((entry.id, entry.parent));
            let value = Entry {
                title: entry.title,
                parent: entry.parent,
                revision: entry.revision,
                updated_at: entry.updated_at,
                kind: match entry.kind {
                    api::EntryKind::Folder => Kind::Folder {
                        children: Vec::new(),
                        expanded: false,
                    },
                    api::EntryKind::Note => Kind::Note,
                },
            };
            if tree.entries.insert(entry.id, value).is_some() {
                return Err("El servidor duplicó una identidad.");
            }
        }
        for (id, parent) in parents {
            if let Some(parent) = parent {
                let Some(Entry {
                    kind: Kind::Folder { children, .. },
                    ..
                }) = tree.entries.get_mut(&parent)
                else {
                    return Err("El servidor devolvió una carpeta inválida.");
                };
                children.push(id);
            } else {
                tree.roots.push(id);
            }
        }
        let mut pending = tree.roots.clone();
        let mut visited = HashSet::with_capacity(tree.entries.len());
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                return Err("El servidor devolvió un ciclo.");
            }
            if let Kind::Folder { children, .. } = &tree.entries[&id].kind {
                pending.extend(children);
            }
        }
        if visited.len() != tree.entries.len() {
            return Err("El árbol contiene entradas sin raíz.");
        }
        Ok(tree)
    }

    pub(crate) fn preserve_navigation(&mut self, previous: &Self) {
        let selected = previous.selected().map(EntryId::server);
        for (id, entry) in &mut self.entries {
            if let Some(old) = previous.entries.get(id) {
                if let (Kind::Folder { expanded, .. }, Kind::Folder { expanded: old, .. }) =
                    (&mut entry.kind, &old.kind)
                {
                    *expanded = *old;
                }
            }
        }
        if let Some(id) = selected.filter(|id| self.entries.contains_key(id)) {
            self.reveal(id);
        }
    }

    pub(crate) fn entry_revision(&self, id: EntryId) -> Option<i64> {
        self.key(id)
            .ok()
            .and_then(|key| self.entries.get(&key))
            .map(|entry| entry.revision)
    }
    pub(crate) fn note_version(&self, id: NoteId) -> (i64, i64) {
        let entry = &self.entries[&id.entry];
        (entry.revision, entry.updated_at)
    }
    pub(crate) fn note_id(&self, id: api::Id) -> Option<NoteId> {
        self.entries
            .get(&id)
            .filter(|entry| matches!(entry.kind, Kind::Note))
            .map(|_| NoteId {
                workspace: self.workspace,
                entry: id,
            })
    }
    pub(crate) fn first_note(&self) -> Option<NoteId> {
        self.entries
            .keys()
            .filter_map(|&id| self.note_id(id))
            .min_by_key(|id| id.entry)
    }
    pub(crate) fn reveal_server(&mut self, id: api::Id) {
        if self.entries.contains_key(&id) {
            self.reveal(id);
        }
    }
    pub(crate) fn confirm_note(&mut self, id: NoteId, revision: i64, at: i64) {
        if let Some(entry) = self.entries.get_mut(&id.entry) {
            entry.revision = revision;
            entry.updated_at = at;
        }
    }
}

pub(crate) fn group_entries(entries: Vec<api::Entry>) -> HashMap<api::Id, Vec<api::Entry>> {
    let mut grouped = HashMap::new();
    for entry in entries {
        grouped
            .entry(entry.workspace)
            .or_insert_with(Vec::new)
            .push(entry);
    }
    grouped
}
