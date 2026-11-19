//! Exportar con E: la carpeta de destino es editable y empieza en Descargas.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Paragraph, Wrap},
};

use super::{Action, Mode, Palette};
use crate::workspace::EntryId;

pub(super) struct Export {
    source: EntryId,
    /// Qué se exporta, como «Carpeta · Ramos · 3 notas».
    label: String,
    /// Ruta escrita y cuántas notas, cuando la exportación terminó.
    done: Option<String>,
}

impl Export {
    pub(super) fn done(&self) -> bool {
        self.done.is_some()
    }

    pub(super) fn hint(&self) -> &'static str {
        if self.done() {
            "Enter cerrar"
        } else {
            "Enter exportar  ·  Esc cancelar"
        }
    }
}

impl Palette {
    pub(crate) fn export(source: EntryId, label: String, directory: &str) -> Self {
        let mut palette = Self {
            mode: Mode::Export(Box::new(Export {
                source,
                label,
                done: None,
            })),
            opening_key: Some('e'),
            ..Self::new()
        };
        palette.input.set_text(directory);
        palette
    }

    /// Respuesta de la exportación: el popup muestra dónde quedó.
    pub(crate) fn exported(&mut self, path: &std::path::Path, notes: usize) {
        if let Mode::Export(export) = &mut self.mode {
            let count = if notes == 1 {
                "1 nota".to_owned()
            } else {
                format!("{notes} notas")
            };
            export.done = Some(format!("Exportado ({count}) en {}", path.display()));
            self.error = None;
        }
    }

    pub(super) fn handle_export(&self) -> Option<Action> {
        match &self.mode {
            Mode::Export(export) if export.done() => Some(Action::KeepOpen),
            _ => None,
        }
    }

    pub(super) fn confirm_export(&mut self) -> Action {
        let Mode::Export(export) = &self.mode else {
            return Action::KeepOpen;
        };
        if export.done() {
            return Action::Close;
        }
        if self.input.text().trim().is_empty() {
            self.error = Some("Escribe la carpeta de destino.".into());
            return Action::KeepOpen;
        }
        Action::Export {
            source: export.source,
            directory: self.input.text().trim().to_owned(),
        }
    }

    pub(super) fn draw_export(&self, frame: &mut Frame<'_>, area: Rect) {
        let Mode::Export(export) = &self.mode else {
            return;
        };
        let lines = match &export.done {
            Some(message) => vec![Line::styled(
                message.clone(),
                Style::new().fg(crate::theme::active()),
            )],
            None => vec![
                Line::styled(export.label.clone(), Style::new().dim()),
                Line::default(),
                Line::styled(
                    "Se guarda como Markdown (.md); si el nombre existe, se añade (2).",
                    Style::new().dim(),
                ),
            ],
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
    }
}
