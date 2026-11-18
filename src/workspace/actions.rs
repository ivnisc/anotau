use super::{Entry, EntryId, FileAction, FolderId, Kind, NoteId, Tree};

impl FileAction {
    pub(crate) fn from_key(key: char) -> Option<Self> {
        match key {
            'm' => Some(Self::Move),
            'c' => Some(Self::Copy),
            'd' => Some(Self::Delete),
            'r' => Some(Self::Rename),
            _ => None,
        }
    }

    pub(crate) fn key(self) -> char {
        match self {
            Self::Move => 'm',
            Self::Copy => 'c',
            Self::Delete => 'd',
            Self::Rename => 'r',
            Self::Close => 'q',
        }
    }
}

impl Tree {
    pub(crate) fn selected(&self) -> Option<EntryId> {
        let row = self.list.selected()?.checked_sub(1)?;
        let key = self.visible().get(row)?.entry;
        Some(match self.entries[&key].kind {
            Kind::Note => EntryId::Note(NoteId {
                workspace: self.workspace,
                entry: key,
            }),
            Kind::Folder { .. } => EntryId::Folder(FolderId {
                workspace: self.workspace,
                entry: key,
            }),
        })
    }

    pub(super) fn key(&self, source: EntryId) -> Result<i64, &'static str> {
        let (workspace, key) = match source {
            EntryId::Note(NoteId { workspace, entry })
            | EntryId::Folder(FolderId { workspace, entry }) => (workspace, entry),
        };
        if workspace != self.workspace || !self.entries.contains_key(&key) {
            return Err("La entrada ya no está disponible.");
        }
        Ok(key)
    }

    /// Notas y carpetas que cuelgan directamente de la entrada.
    pub(crate) fn contents(&self, source: EntryId) -> usize {
        self.key(source).map_or(0, |key| {
            self.entries
                .values()
                .filter(|entry| entry.parent == Some(key))
                .count()
        })
    }

    pub(crate) fn entry_title(&self, source: EntryId) -> &str {
        match source {
            EntryId::Note(id) => &self.entries[&id.entry].title,
            EntryId::Folder(id) => &self.entries[&id.entry].title,
        }
    }

    pub(crate) fn entry_path(&self, source: EntryId) -> String {
        match self.key(source) {
            Ok(key) => format!(
                "{} / {}",
                self.folder_path(self.entries[&key].parent),
                self.entries[&key].title
            ),
            Err(error) => error.into(),
        }
    }

    pub(crate) fn parent_folder(&self, source: EntryId) -> Option<FolderId> {
        self.key(source)
            .ok()
            .and_then(|key| self.entries[&key].parent)
            .map(|entry| FolderId {
                workspace: self.workspace,
                entry,
            })
    }

    fn within(&self, mut key: i64, ancestor: i64) -> bool {
        loop {
            if key == ancestor {
                return true;
            }
            match self.entries[&key].parent {
                Some(parent) => key = parent,
                None => return false,
            }
        }
    }

    pub(crate) fn contains_note(&self, source: EntryId, id: NoteId) -> bool {
        id.workspace == self.workspace
            && self.entries.contains_key(&id.entry)
            && self.key(source).is_ok_and(|key| self.within(id.entry, key))
    }

    /// Notas a exportar desde la entrada: carpetas intermedias dentro de ella, título e id.
    /// Una nota se exporta sola; una carpeta, con todo su contenido en orden alfabético.
    pub(crate) fn export_notes(&self, source: EntryId) -> Vec<(Vec<String>, String, NoteId)> {
        let Ok(root) = self.key(source) else {
            return Vec::new();
        };
        let note = |entry: i64| NoteId {
            workspace: self.workspace,
            entry,
        };
        if let EntryId::Note(_) = source {
            return vec![(Vec::new(), self.entries[&root].title.clone(), note(root))];
        }
        let mut notes = self
            .entries
            .iter()
            .filter(|&(&key, entry)| matches!(entry.kind, Kind::Note) && self.within(key, root))
            .map(|(&key, entry)| {
                let mut folders = Vec::new();
                let mut parent = entry.parent;
                while let Some(folder) = parent.filter(|&folder| folder != root) {
                    folders.push(self.entries[&folder].title.clone());
                    parent = self.entries[&folder].parent;
                }
                folders.reverse();
                (folders, entry.title.clone(), note(key))
            })
            .collect::<Vec<_>>();
        notes.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        notes
    }

    pub(crate) fn destinations(&self, source: EntryId) -> Vec<(Option<FolderId>, String)> {
        let Ok(source) = self.key(source) else {
            return Vec::new();
        };
        self.folders()
            .into_iter()
            .filter(|(folder, _)| folder.is_none_or(|folder| !self.within(folder.entry, source)))
            .collect()
    }

    pub(crate) fn folders(&self) -> Vec<(Option<FolderId>, String)> {
        let mut folders = self
            .entries
            .iter()
            .filter(|(_, entry)| matches!(entry.kind, Kind::Folder { .. }))
            .map(|(&key, _)| {
                (
                    Some(FolderId {
                        workspace: self.workspace,
                        entry: key,
                    }),
                    self.folder_path(Some(key)),
                )
            })
            .collect::<Vec<_>>();
        folders.sort_by(|left, right| left.1.cmp(&right.1));
        folders.insert(0, (None, self.name.clone()));
        folders
    }

    pub(super) fn reveal(&mut self, key: i64) {
        let mut parent = self.entries[&key].parent;
        while let Some(folder) = parent {
            let entry = self.entry_mut(folder);
            if let Kind::Folder { expanded, .. } = &mut entry.kind {
                *expanded = true;
            }
            parent = entry.parent;
        }
        self.list.select(
            self.visible()
                .iter()
                .position(|row| row.entry == key)
                .map(|row| row + 1),
        );
    }

    fn entry_mut(&mut self, key: i64) -> &mut Entry {
        self.rows.take();
        self.entries
            .get_mut(&key)
            .unwrap_or_else(|| unreachable!("entrada interna inexistente"))
    }
}
