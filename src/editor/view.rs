use std::borrow::Cow;

use rat_text::{HasScreenCursor, text_area::TextArea};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Clear, Padding, Paragraph, StatefulWidget},
};

use crate::{markdown, palette::OPEN_HINT};

use super::{
    Editor, Pane, Sidebar, TAB_SHORTCUTS,
    split::{Divider, SPLIT_WIDTH, Split, Splits},
};

impl Editor {
    pub(crate) fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        frame.render_widget(Block::new().style(crate::theme::base()), area);
        let [body, footer] = if area.height >= 4 {
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area)
        } else {
            [area, Rect::ZERO]
        };
        self.splits.tree = None;
        self.splits.document = None;
        self.preview_area = Rect::ZERO;
        self.room_for_sidebar = Splits::tree_fits(body.width);
        let document_area = if self.room_for_sidebar && self.sidebar != Sidebar::Hidden {
            let tree_width = self.splits.tree_width(body.width);
            let [tree, document] =
                Layout::horizontal([Constraint::Length(tree_width), Constraint::Fill(1)])
                    .areas(body);
            self.draw_sidebar(frame, tree);
            self.splits.tree = Some(Divider {
                column: document.left(),
                area: body,
            });
            Some(document)
        } else if self.sidebar.focused() {
            self.draw_sidebar(frame, body);
            None
        } else {
            Some(body)
        };
        if let Some(document) = document_area {
            self.draw_document(frame, document);
        }

        self.draw_dividers(frame);
        self.draw_footer(frame, footer);
        if let Some(palette) = &mut self.palette {
            let status = self
                .progress
                .as_ref()
                .map(|progress| progress.line(std::time::Instant::now()));
            palette.draw(frame, status);
        }
    }

    /// Una línea discreta: modo o aviso vigente a la izquierda, teclas de ayuda a la derecha.
    fn draw_footer(&self, frame: &mut Frame<'_>, area: Rect) {
        if area.is_empty() {
            return;
        }
        let status = if let Some(split) = self.splits.keyboard_target() {
            Span::styled(
                format!(
                    " {} · ← → ajustar · Enter aplicar · Esc cancelar",
                    split.label()
                ),
                Style::new().fg(crate::theme::accent()),
            )
        } else if let Some(message) = self
            .remote_error()
            .or(self.command_notice)
            .or(self.note().and_then(|note| note.notice))
        {
            Span::styled(
                format!(" {message}"),
                Style::new().fg(crate::theme::warning()),
            )
        } else if self.pane == Pane::Writing && !self.sidebar.focused() {
            Span::styled(" Escritura · Esc acciones", Style::new().dim())
        } else {
            Span::styled(" Acciones", Style::new().dim())
        };
        let keys = if area.width >= 72 {
            format!("{OPEN_HINT} comandos · F3 configuración · F4 ayuda ")
        } else {
            "F1 · F3 · F4 ".into()
        };
        let inbox = (!self.sidebar_shown())
            .then(|| self.inbox_label())
            .flatten();
        let mut right = Line::default().right_aligned();
        if let Some(inbox) = inbox {
            right.extend(inbox.spans);
            right.push_span(Span::raw(" "));
        }
        right.push_span(Span::styled(keys, Style::new().dim()));
        // las teclas se dibujan después y tapan la cola de un aviso largo
        frame.render_widget(Line::from(status), area);
        frame.render_widget(right, area);
    }

    /// Aviso de archivos recibidos sin resolver, como una notificación: remitente y cuántos más.
    fn inbox_label(&self) -> Option<Line<'static>> {
        let mut senders = Vec::new();
        for offer in &self.incoming {
            if !senders.contains(&offer.sender) {
                senders.push(offer.sender);
            }
        }
        let first = *senders.first()?;
        let name = self
            .people
            .entries
            .iter()
            .find(|person| person.id == first)
            .map_or("usuario", |person| person.name.as_str());
        let accent = Style::new().fg(crate::theme::accent());
        let mut line = Line::from(vec![
            Span::styled(" ◆ ", accent),
            Span::styled(name.to_owned(), accent.bold()),
        ]);
        if senders.len() > 1 {
            line.push_span(Span::styled(format!(" +{}", senders.len() - 1), accent));
        }
        line.push_span(Span::raw(" "));
        Some(line)
    }

    fn sidebar_shown(&self) -> bool {
        self.sidebar.focused() || (self.sidebar == Sidebar::Visible && self.room_for_sidebar)
    }

    fn draw_document(&mut self, frame: &mut Frame<'_>, document: Rect) {
        if document.width >= SPLIT_WIDTH {
            let source_width = self.splits.source_width(document.width);
            let [source, preview] =
                Layout::horizontal([Constraint::Length(source_width), Constraint::Fill(1)])
                    .areas(document);
            self.draw_source(frame, source);
            self.draw_preview(frame, preview, false);
            self.splits.document = Some(Divider {
                column: preview.left(),
                area: document,
            });
        } else {
            match self.pane {
                Pane::Source | Pane::Writing => self.draw_source(frame, document),
                Pane::Preview => self.draw_preview(frame, document, true),
            }
        }
    }

    fn draw_dividers(&self, frame: &mut Frame<'_>) {
        let active = self.splits.active();
        for (split, divider) in [
            (Split::Tree, self.splits.tree),
            (Split::Document, self.splits.document),
        ] {
            if let Some(divider) = divider
                && divider.area.height > 2
                && (self.mouse_capture || active == Some(split))
            {
                let style = if active == Some(split) {
                    Style::new().fg(crate::theme::accent()).bold()
                } else {
                    Style::new().dim()
                };
                let row = divider.area.top() + divider.area.height / 2;
                frame.render_widget(
                    Span::styled("↔", style),
                    Rect::new(divider.column, row, 1, 1),
                );
            }
        }
    }

    fn draw_sidebar(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let [tree, users] = if area.height < 7 {
            if self.sidebar == Sidebar::Users {
                [Rect::ZERO, area]
            } else {
                [area, Rect::ZERO]
            }
        } else {
            let user_height = (frame.area().height / 3).max(3).min(area.height - 4);
            Layout::vertical([Constraint::Fill(1), Constraint::Length(user_height)]).areas(area)
        };
        self.draw_tree(frame, tree);
        let mut block = pane_block(" Usuarios ", self.sidebar == Sidebar::Users);
        if self.away {
            block = block.title_top(
                Line::from(vec![
                    crate::people::indicator(crate::people::Presence::Away),
                    Span::raw(" "),
                ])
                .right_aligned(),
            );
        }
        if let Some(inbox) = self.inbox_label() {
            block = block.title_bottom(inbox.left_aligned());
        }
        let inner = block.inner(users);
        if let Some((position, total)) = self.people.position() {
            block = block.title_bottom(position_label(position, total));
        }
        frame.render_widget(block, users);
        self.people.draw(frame, inner, &self.incoming);
    }

    fn draw_tree(&mut self, frame: &mut Frame<'_>, area: Rect) {
        // solo el espacio actual; el pie dice cuál de cuántos, y Tab cambia de espacio
        let block = pane_block(" Notas ", self.sidebar == Sidebar::Focused)
            .title_bottom(position_label(self.workspace + 1, self.workspaces.len()));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [selector, tree] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
        let name = Span::styled(
            format!("›{}", self.workspace().tree.name),
            crate::theme::selected(),
        );
        frame.render_widget(fit_tab(name, usize::from(selector.width)), selector);
        let workspace = self.workspace();
        let active = workspace
            .active_tab()
            .map(|index| workspace.notes[index].id);
        self.workspaces[self.workspace]
            .tree
            .draw(frame, tree, active);
    }

    fn draw_source(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let active = self.pane != Pane::Preview && !self.sidebar.focused();
        let (heading, temporary): (Cow<'_, str>, bool) = match &self.workspace().active {
            Some(super::Selection::Temporary(note)) => (
                format!(" Markdown · {} ", self.workspace().tree.title(note.id)).into(),
                true,
            ),
            _ => (" Markdown ".into(), false),
        };
        // cambiar de nota solo funciona en navegación: al escribir, las flechas mueven el cursor
        let switch = (self.workspace().notes.len() > 1 && self.pane == Pane::Source)
            .then_some(NOTE_SWITCH_HINT)
            .filter(|hint| usize::from(area.width) >= Span::raw(*hint).width() + 24);
        let reserved = switch.map_or(0, |hint| Span::raw(hint).width());
        let mut block = pane_block(&heading, active).title_bottom(
            self.tabs(
                area.width
                    .saturating_sub(u16::try_from(reserved).unwrap_or(u16::MAX)),
            ),
        );
        if let Some(hint) = switch {
            block = block.title_bottom(Line::styled(hint, Style::new().dim()).left_aligned());
        }
        let show_cursor = self.palette.is_none() && self.splits.keyboard_target().is_none();
        let Some(note) = self.note_mut() else {
            frame.render_widget(
                Paragraph::new("Abre una nota desde el árbol").block(block),
                area,
            );
            return;
        };
        if area.width >= 28 {
            let status = if temporary && !note.loading() {
                " Vista temporal ".into()
            } else {
                note.status()
            };
            block = block.title_top(Line::styled(status, Style::new().dim()).right_aligned());
        }
        let inner = block.inner(area);
        if inner.is_empty() {
            frame.render_widget(block, area);
            return;
        }

        let widget = TextArea::new()
            .block(block)
            .text_style(markdown::editor_styles())
            .select_style(Style::new().reversed());
        (&widget).render(area, frame.buffer_mut(), &mut note.source);
        if note.clamp_scroll_end() {
            frame.render_widget(Clear, inner);
            frame.render_widget(Block::new().style(crate::theme::base()), inner);
            (&widget).render(area, frame.buffer_mut(), &mut note.source);
        }

        if show_cursor
            && let Some(cursor) = note.source.screen_cursor()
            && area.contains(cursor.into())
        {
            frame.set_cursor_position(cursor);
        }
    }

    fn draw_preview(&mut self, frame: &mut Frame<'_>, area: Rect, show_tabs: bool) {
        let active = self.pane == Pane::Preview && !self.sidebar.focused();
        let mut block = pane_block(" Vista previa ", active);
        if show_tabs {
            block = block.title_bottom(self.tabs(area.width));
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.preview_area = inner;
        if inner.is_empty() {
            return;
        }

        let follow_source = self.pane == Pane::Writing && !self.sidebar.focused();
        let Some(note) = self.note_mut() else { return };
        note.preview_width_changed(inner.width);
        if follow_source {
            note.sync_preview(inner);
        }
        let row = note.preview_offset(inner);
        let height = usize::from(inner.height);
        for (index, line) in note.preview.lines.iter().enumerate().skip(row).take(height) {
            let horizontal = note.horizontal_offset(index, inner.width);
            let line_area = Rect::new(
                inner.x,
                inner.y + u16::try_from(index - row).unwrap_or_default(),
                inner.width,
                1,
            );
            frame.render_widget(
                crate::markdown::window(line, horizontal, inner.width),
                line_area,
            );
            if let Some((selected_row, selected)) = note.selected_link
                && let Some(link) = note
                    .preview_info
                    .get(selected_row)
                    .and_then(|info| info.links.get(selected))
            {
                for candidate in &note.preview_info[index].links {
                    if std::sync::Arc::ptr_eq(&link.destination, &candidate.destination) {
                        let left = candidate
                            .columns
                            .start
                            .saturating_sub(horizontal)
                            .min(usize::from(inner.width));
                        let right = candidate
                            .columns
                            .end
                            .saturating_sub(horizontal)
                            .min(usize::from(inner.width));
                        frame.buffer_mut().set_style(
                            Rect::new(
                                inner.x + u16::try_from(left).unwrap_or_default(),
                                line_area.y,
                                u16::try_from(right - left).unwrap_or_default(),
                                1,
                            ),
                            Style::new().reversed(),
                        );
                    }
                }
            }
        }
    }

    pub(super) fn tabs(&self, width: u16) -> Line<'static> {
        let width = usize::from(width.saturating_sub(2));
        let workspace = self.workspace();
        let active = workspace.active_tab();
        let mut spans = workspace
            .notes
            .iter()
            .enumerate()
            .map(|(index, note)| {
                let mark = if Some(index) == active { "› " } else { "" };
                let dirty = if note.modified { "*" } else { "" };
                let title = workspace.tree.title(note.id);
                let style = if Some(index) == active {
                    crate::theme::selected()
                } else {
                    Style::new().dim()
                };
                let label = if index < TAB_SHORTCUTS.len() {
                    format!("{mark}[{} {dirty}{title}] ", index + 1)
                } else {
                    format!("{mark}[{dirty}{title}] ")
                };
                Span::styled(label, style)
            })
            .collect::<Vec<_>>();
        if spans.iter().map(Span::width).sum::<usize>() > width {
            let active = spans.remove(active.unwrap_or(0));
            let count = spans.len();
            let badge_width = if count > 0 {
                Span::raw(format!("+{count} ")).width()
            } else {
                0
            };
            let show_badge = count > 0 && width >= badge_width + 6;
            let budget = width.saturating_sub(if show_badge { badge_width } else { 0 });
            let mut remaining = budget.saturating_sub(active.width());
            spans.retain(|span| {
                if span.width() > remaining {
                    return false;
                }
                remaining -= span.width();
                true
            });
            if show_badge {
                spans.push(Span::styled(
                    format!("+{} ", count - spans.len()),
                    Style::new().dim(),
                ));
            }
            spans.push(fit_tab(active, budget));
        }
        Line::from(spans).right_aligned()
    }
}

const NOTE_SWITCH_HINT: &str = " ↑↓ · 1–9 cambiar nota ";

fn pane_block(title: &str, active: bool) -> Block<'static> {
    let marker = if active { " ›" } else { "" };
    let border = if active {
        Style::new().fg(crate::theme::accent())
    } else {
        Style::new().dim()
    };
    Block::bordered()
        .title(format!("{marker}{title}"))
        .border_style(border)
        .padding(Padding::horizontal(1))
}

/// Pie de panel «n de m» con la posición del elemento elegido, abajo a la derecha.
fn position_label(position: usize, total: usize) -> Line<'static> {
    Line::styled(format!(" {position} de {total} "), Style::new().dim()).right_aligned()
}

fn fit_tab(span: Span<'_>, width: usize) -> Span<'_> {
    if span.width() <= width {
        return span;
    }
    let mut clipped = String::new();
    let mut remaining = width.saturating_sub(usize::from(width >= 2));
    for grapheme in span.styled_graphemes(Style::new()) {
        let columns = Span::raw(grapheme.symbol).width();
        if columns > remaining {
            break;
        }
        clipped.push_str(grapheme.symbol);
        remaining -= columns;
    }
    if width >= 2 {
        clipped.push('…');
    }
    Span::styled(clipped, span.style)
}
