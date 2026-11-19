use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};

pub(super) const SPLIT_WIDTH: u16 = 88;
const DEFAULT_TREE_MIN: u16 = 24;
const MIN_TREE: u16 = 18;
const MIN_DOCUMENT_PANE: u16 = 28;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Split {
    Tree,
    Document,
}

impl Split {
    fn bounds(self, width: u16) -> (u16, u16) {
        match self {
            Self::Tree => (MIN_TREE, width - SPLIT_WIDTH),
            Self::Document => (MIN_DOCUMENT_PANE, width - MIN_DOCUMENT_PANE),
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Tree => "Ancho del árbol",
            Self::Document => "Proporción de los paneles",
        }
    }
}

#[derive(Clone, Copy)]
struct Widths {
    tree: Option<u16>,
    source: (u16, u16),
}

#[derive(Clone, Copy)]
pub(super) struct Divider {
    pub(super) column: u16,
    pub(super) area: Rect,
}

impl Divider {
    fn hit(self, event: MouseEvent) -> bool {
        event.row > self.area.top()
            && event.row < self.area.bottom().saturating_sub(1)
            && (event.column == self.column || event.column.saturating_add(1) == self.column)
    }
}

#[derive(Clone, Copy)]
enum Adjustment {
    Drag { split: Split, offset: u16 },
    Keyboard { split: Split, original: Widths },
}

pub(super) struct Splits {
    widths: Widths,
    pub(super) tree: Option<Divider>,
    pub(super) document: Option<Divider>,
    adjustment: Option<Adjustment>,
}

impl Splits {
    pub(super) fn new() -> Self {
        Self {
            widths: Widths {
                tree: None,
                source: (52, 100),
            },
            tree: None,
            document: None,
            adjustment: None,
        }
    }

    pub(super) fn tree_fits(width: u16) -> bool {
        width >= DEFAULT_TREE_MIN + SPLIT_WIDTH
    }

    pub(super) fn tree_width(&self, width: u16) -> u16 {
        let (min, max) = Split::Tree.bounds(width);
        self.widths
            .tree
            .unwrap_or_else(|| max.min(30))
            .clamp(min, max)
    }

    pub(super) fn source_width(&self, width: u16) -> u16 {
        let (numerator, denominator) = self.widths.source;
        let denominator = u32::from(denominator);
        let columns = (u32::from(width) * u32::from(numerator) + denominator / 2) / denominator;
        let (min, max) = Split::Document.bounds(width);
        u16::try_from(columns).unwrap_or(u16::MAX).clamp(min, max)
    }

    fn divider(&self, split: Split) -> Option<Divider> {
        match split {
            Split::Tree => self.tree,
            Split::Document => self.document,
        }
    }

    fn resize(&mut self, split: Split, column: u16) -> bool {
        let Some(divider) = self.divider(split) else {
            return false;
        };
        let (min, max) = split.bounds(divider.area.width);
        let width = column.saturating_sub(divider.area.left()).clamp(min, max);
        if width == divider.column - divider.area.left() {
            return false;
        }
        match split {
            Split::Tree => self.widths.tree = Some(width),
            Split::Document => self.widths.source = (width, divider.area.width),
        }
        true
    }

    pub(super) fn active(&self) -> Option<Split> {
        self.adjustment.map(|adjustment| match adjustment {
            Adjustment::Drag { split, .. } | Adjustment::Keyboard { split, .. } => split,
        })
    }

    pub(super) fn keyboard_target(&self) -> Option<Split> {
        match self.adjustment {
            Some(Adjustment::Keyboard { split, .. }) => Some(split),
            _ => None,
        }
    }

    pub(super) fn start_keyboard(&mut self, split: Split) -> bool {
        if self.divider(split).is_none() {
            return false;
        }
        self.adjustment = Some(Adjustment::Keyboard {
            split,
            original: self.widths,
        });
        true
    }

    pub(super) fn keyboard(&mut self, key: KeyEvent) -> bool {
        let Some(Adjustment::Keyboard { split, original }) = self.adjustment else {
            return false;
        };
        if !key.modifiers.is_empty() || key.kind == KeyEventKind::Release {
            return false;
        }
        match key.code {
            KeyCode::Left | KeyCode::Right => {
                let Some(divider) = self.divider(split) else {
                    return false;
                };
                let column = if key.code == KeyCode::Left {
                    divider.column.saturating_sub(1)
                } else {
                    divider.column.saturating_add(1)
                };
                self.resize(split, column)
            }
            KeyCode::Enter | KeyCode::Esc if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Esc {
                    self.widths = original;
                }
                self.adjustment = None;
                true
            }
            _ => false,
        }
    }

    pub(super) fn stop_drag(&mut self) {
        if matches!(self.adjustment, Some(Adjustment::Drag { .. })) {
            self.adjustment = None;
        }
    }

    // Some indica que el separador consumió el evento y si hace falta redibujar.
    pub(super) fn mouse(&mut self, event: MouseEvent) -> Option<bool> {
        if let Some(Adjustment::Drag { split, offset }) = self.adjustment {
            match event.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    return Some(self.resize(split, event.column.saturating_add(offset)));
                }
                MouseEventKind::Up(_) | MouseEventKind::Moved => {
                    self.adjustment = None;
                    return Some(true);
                }
                MouseEventKind::Down(_) => self.adjustment = None,
                _ => return Some(false),
            }
        }
        if event.kind != MouseEventKind::Down(MouseButton::Left) || !event.modifiers.is_empty() {
            return None;
        }
        let (split, divider) = [Split::Tree, Split::Document]
            .into_iter()
            .find_map(|split| {
                let divider = self.divider(split)?;
                divider.hit(event).then_some((split, divider))
            })?;
        self.adjustment = Some(Adjustment::Drag {
            split,
            offset: divider.column - event.column,
        });
        Some(true)
    }
}
