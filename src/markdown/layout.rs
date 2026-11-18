use ratatui::{
    style::Style,
    text::{Line, Span, Text},
};
use std::{ops::Range, sync::Arc};

#[derive(Clone)]
pub(crate) struct Link {
    pub(crate) columns: Range<usize>,
    pub(crate) destination: Arc<str>,
}

#[derive(Clone, Default)]
pub(crate) struct LineInfo {
    pub(crate) block: Option<usize>,
    pub(crate) links: Vec<Link>,
    /// Separador `---`: su trazo se estira al ancho disponible al componer.
    pub(crate) rule: bool,
}

pub(crate) fn window<'a>(line: &'a Line<'_>, offset: usize, width: u16) -> Line<'a> {
    let end = offset.saturating_add(usize::from(width));
    let mut column = 0usize;
    let mut spans = Vec::new();
    for grapheme in line.styled_graphemes(Style::new()) {
        let next = column.saturating_add(Span::raw(grapheme.symbol).width());
        if next > offset && column < end {
            spans.push(if column < offset || next > end {
                Span::styled(
                    " ".repeat(next.min(end) - column.max(offset)),
                    grapheme.style,
                )
            } else {
                Span::styled(grapheme.symbol, grapheme.style)
            });
        }
        column = next;
        if column >= end {
            break;
        }
    }
    Line::from(spans).style(line.style)
}

/// Conserva el prefijo de la línea (citas, sangría) y extiende el trazo hasta `width`.
fn stretch_rule(line: &Line<'static>, width: u16) -> Line<'static> {
    let Some((stroke, prefix)) = line.spans.split_last() else {
        return line.clone();
    };
    let used: usize = prefix.iter().map(Span::width).sum();
    let mut spans = prefix.to_vec();
    spans.push(Span::styled(
        "─".repeat(usize::from(width).saturating_sub(used).max(1)),
        stroke.style,
    ));
    Line::from(spans).style(line.style)
}

pub(super) fn wrap(
    text: &Text<'static>,
    info: &[LineInfo],
    width: u16,
) -> (Text<'static>, Vec<LineInfo>, Vec<usize>) {
    let mut lines = Vec::new();
    let mut metadata = Vec::new();
    let mut rows = Vec::new();
    for (line, info) in text.lines.iter().zip(info) {
        rows.push(lines.len());
        if info.rule && width > 0 {
            lines.push(stretch_rule(line, width));
            metadata.push(info.clone());
            continue;
        }
        if info.block.is_some() || width == 0 || line.width() <= usize::from(width) {
            lines.push(line.clone());
            metadata.push(info.clone());
            continue;
        }
        let graphemes = line.styled_graphemes(Style::new()).collect::<Vec<_>>();
        let mut start = 0;
        let mut source_column = 0;
        while start < graphemes.len() {
            let mut end = start;
            let mut columns = 0;
            let mut space = None;
            while let Some(grapheme) = graphemes.get(end) {
                let next = Span::raw(grapheme.symbol).width();
                if columns + next > usize::from(width) && end > start {
                    break;
                }
                columns += next;
                if grapheme.is_whitespace() {
                    space = Some(end + 1);
                }
                end += 1;
            }
            if end < graphemes.len()
                && let Some(boundary) = space
            {
                end = boundary;
            }
            let mut result = Line::default();
            for grapheme in &graphemes[start..end] {
                if let Some(last) = result.spans.last_mut()
                    && last.style == grapheme.style
                {
                    last.content.to_mut().push_str(grapheme.symbol);
                } else {
                    result.push_span(Span::styled(grapheme.symbol.to_owned(), grapheme.style));
                }
            }
            let columns = result.width();
            let regions = info
                .links
                .iter()
                .filter_map(|link| {
                    let left = link.columns.start.max(source_column);
                    let right = link.columns.end.min(source_column + columns);
                    (left < right).then(|| Link {
                        columns: left - source_column..right - source_column,
                        destination: Arc::clone(&link.destination),
                    })
                })
                .collect();
            lines.push(result);
            metadata.push(LineInfo {
                block: None,
                links: regions,
                rule: false,
            });
            source_column += columns;
            start = end;
        }
    }
    (Text::from(lines), metadata, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    #[test]
    fn window_borrows_visible_graphemes_and_blanks_partial_wide_characters() {
        let line = Line::from(vec![Span::styled("界e\u{301}👩‍💻fin", Style::new().bold())]);
        assert_eq!(window(&line, 1, 4).to_string(), " e\u{301}👩‍💻");
        assert_eq!(window(&line, 0, 1).to_string(), " ");
        let view = window(&line, 2, 1);
        assert_eq!(view.to_string(), "e\u{301}");
        assert!(matches!(view.spans[0].content, Cow::Borrowed(_)));
        assert!(
            view.spans[0]
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
    }
}
