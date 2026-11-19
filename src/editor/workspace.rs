use super::buffer::NoteBuffer;
use crate::workspace::{NoteId, Tree};

pub(super) struct Workspace {
    pub(super) tree: Tree,
    pub(super) notes: Vec<NoteBuffer>,
    pub(super) active: Option<Selection>,
}

pub(super) enum Selection {
    Open(usize),
    Temporary(Box<NoteBuffer>),
}

impl Workspace {
    pub(super) fn new(tree: Tree) -> Self {
        Self {
            tree,
            notes: Vec::new(),
            active: None,
        }
    }
    pub(super) fn buffers(&self) -> impl Iterator<Item = &NoteBuffer> {
        self.notes
            .iter()
            .chain(self.active.iter().filter_map(|selection| match selection {
                Selection::Temporary(note) => Some(note.as_ref()),
                Selection::Open(_) => None,
            }))
    }

    pub(super) fn buffers_mut(&mut self) -> impl Iterator<Item = &mut NoteBuffer> {
        self.notes
            .iter_mut()
            .chain(
                self.active
                    .iter_mut()
                    .filter_map(|selection| match selection {
                        Selection::Temporary(note) => Some(note.as_mut()),
                        Selection::Open(_) => None,
                    }),
            )
    }

    pub(super) fn note(&self) -> Option<&NoteBuffer> {
        match self.active.as_ref()? {
            Selection::Open(index) => Some(&self.notes[*index]),
            Selection::Temporary(note) => Some(note),
        }
    }

    pub(super) fn note_mut(&mut self) -> Option<&mut NoteBuffer> {
        match self.active.as_mut()? {
            Selection::Open(index) => Some(&mut self.notes[*index]),
            Selection::Temporary(note) => Some(note),
        }
    }

    pub(super) fn preview_note(&mut self, id: NoteId) {
        if let Some(index) = self.notes.iter().position(|note| note.id == id) {
            self.activate(index);
        } else if !matches!(&self.active, Some(Selection::Temporary(note)) if note.id == id) {
            self.active = Some(Selection::Temporary(Box::new(NoteBuffer::new(id))));
        }
    }

    pub(super) fn keep_note(&mut self) {
        self.active = match self.active.take() {
            Some(Selection::Temporary(note)) => {
                self.notes.push(*note);
                Some(Selection::Open(self.notes.len() - 1))
            }
            current => current,
        };
    }

    pub(super) fn activate(&mut self, index: usize) {
        if let Some(note) = self.note() {
            note.source.focus.set(false);
        }
        self.active = Some(Selection::Open(index));
    }

    pub(super) fn active_tab(&self) -> Option<usize> {
        match self.active {
            Some(Selection::Open(index)) => Some(index),
            _ => None,
        }
    }

    pub(super) fn close_note(&mut self, id: NoteId) {
        let removed = self.notes.iter().position(|note| note.id == id);
        if let Some(index) = removed {
            self.notes.remove(index);
        }
        self.active = match self.active.take() {
            Some(Selection::Open(_)) if self.notes.is_empty() => None,
            Some(Selection::Open(index)) => {
                let index = if removed.is_some_and(|removed| removed < index) {
                    index - 1
                } else {
                    index
                };
                Some(Selection::Open(index.min(self.notes.len() - 1)))
            }
            Some(Selection::Temporary(note)) if note.id == id => {
                self.notes.len().checked_sub(1).map(Selection::Open)
            }
            current => current,
        };
    }
}
