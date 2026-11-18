mod layout;
mod math;
mod renderer;

pub(crate) use layout::window;
pub(crate) use layout::{LineInfo, Link};

use std::{collections::HashMap, ops::Range};

use pulldown_cmark::{Alignment, Event, Options, Parser, Tag, TagEnd, html};
use ratatui::{style::Style, text::Text};

const RULE: Style = Style::new().dim();
const ALIGNMENT_MARKER: &str = "anotau-table-alignment:";
const TASK_CHECKED: &str = "anotau-task:checked";
const TASK_OPEN: &str = "anotau-task:open";
pub(crate) const CODE_HIGHLIGHT: usize = 2;
pub(crate) const BLOCK_CODE_HIGHLIGHT: usize = 5;

fn code_style() -> Style {
    Style::new().fg(crate::theme::warning())
}

fn heading_style() -> Style {
    Style::new().fg(crate::theme::accent()).bold()
}

fn link_style() -> Style {
    Style::new().fg(crate::theme::accent()).underlined()
}

pub(crate) fn editor_styles() -> [Style; 6] {
    [
        heading_style(),
        Style::new().fg(crate::theme::emphasis()),
        code_style(),
        link_style(),
        Style::new().fg(crate::theme::html()),
        code_style(),
    ]
}

pub(crate) struct Document {
    pub(crate) highlights: Vec<(Range<usize>, usize)>,
    pub(crate) tables: Vec<Table>,
    pub(crate) items: Vec<ListItem>,
    pub(crate) view: Preview,
}

#[derive(Default)]
pub(crate) struct Preview {
    pub(crate) text: Text<'static>,
    pub(crate) info: Vec<LineInfo>,
    pub(crate) anchors: HashMap<String, usize>,
}

impl Preview {
    pub(crate) fn wrap(&self, width: u16) -> Self {
        let (text, info, rows) = layout::wrap(&self.text, &self.info, width);
        let anchors = self
            .anchors
            .iter()
            .map(|(name, &row)| (name.clone(), rows.get(row).copied().unwrap_or_default()))
            .collect();
        Self {
            text,
            info,
            anchors,
        }
    }
}

pub(crate) struct Table {
    pub(crate) range: Range<usize>,
    pub(crate) cells: Vec<Range<usize>>,
}

pub(crate) struct ListItem {
    pub(crate) range: Range<usize>,
    pub(crate) number: Option<u64>,
    pub(crate) list: usize,
}

pub(crate) fn render(mut markdown: String) -> Document {
    let source_len = markdown.len();
    // pulldown-cmark requiere un espacio o salto después de una casilla.
    if !markdown.ends_with('\n') {
        markdown.push('\n');
    }
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_MATH;
    let mut highlights = Vec::new();
    let mut intermediate = String::new();
    let mut tables: Vec<Table> = Vec::new();
    let mut items = Vec::new();
    let mut lists = Vec::new();
    let events = Parser::new_ext(&markdown, options)
        .into_offset_iter()
        .flat_map(|(event, range)| {
            match &event {
                Event::Start(Tag::Table(_)) => tables.push(Table {
                    range: range.start..range.end.min(source_len),
                    cells: Vec::new(),
                }),
                Event::Start(Tag::TableCell) => {
                    if let Some(table) = tables.last_mut() {
                        table
                            .cells
                            .push(range.start.min(source_len)..range.end.min(source_len));
                    }
                }
                Event::Start(Tag::List(number)) => lists.push((*number, range.start)),
                Event::End(TagEnd::List(_)) => {
                    lists.pop();
                }
                Event::Start(Tag::Item) => {
                    if let Some((number, list)) = lists.last_mut() {
                        let end = range.end.min(source_len);
                        let raw = &markdown[range.start..end];
                        let leading =
                            raw.len() - raw.trim_start_matches([' ', '\t', '\r', '\n']).len();
                        items.push(ListItem {
                            range: range.start + leading..end,
                            number: *number,
                            list: *list,
                        });
                        if let Some(number) = number {
                            *number = number.saturating_add(1);
                        }
                    }
                }
                _ => {}
            }
            let style = match &event {
                Event::Start(Tag::Heading { .. }) => Some(0),
                Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough) => Some(1),
                Event::Start(Tag::CodeBlock(_)) => Some(BLOCK_CODE_HIGHLIGHT),
                Event::Code(_) => Some(CODE_HIGHLIGHT),
                Event::Start(Tag::Link { .. } | Tag::Image { .. }) => Some(3),
                Event::Html(_) | Event::InlineHtml(_) => Some(4),
                _ => None,
            };
            if let Some(style) = style {
                highlights.push((range.start..range.end.min(source_len), style));
            }

            // conserva la alineación del Markdown sin aplicar estilos CSS de la nota
            let marker = if let Event::Start(Tag::Table(alignments)) = &event {
                let columns: String = alignments
                    .iter()
                    .map(|alignment| match alignment {
                        Alignment::Center => 'c',
                        Alignment::Right => 'r',
                        Alignment::Left | Alignment::None => 'l',
                    })
                    .collect();
                Some(Event::Html(
                    format!("<!--{ALIGNMENT_MARKER}{columns}-->").into(),
                ))
            } else {
                None
            };

            marker.into_iter().chain([preview_event(event)])
        });
    html::push_html(&mut intermediate, events);

    Document {
        highlights,
        tables,
        items,
        view: renderer::render(&intermediate),
    }
}

fn preview_event(event: Event<'_>) -> Event<'_> {
    match event {
        Event::InlineMath(source) => Event::Html(math::html(&source, false).into()),
        Event::DisplayMath(source) => Event::Html(math::html(&source, true).into()),
        Event::TaskListMarker(true) => Event::Html("<!--anotau-task:checked-->".into()),
        Event::TaskListMarker(false) => Event::Html("<!--anotau-task:open-->".into()),
        Event::Html(raw) => Event::Html(unmark_raw_html(raw)),
        Event::InlineHtml(raw) => Event::InlineHtml(unmark_raw_html(raw)),
        other => other,
    }
}

fn unmark_raw_html(raw: pulldown_cmark::CowStr<'_>) -> pulldown_cmark::CowStr<'_> {
    if [ALIGNMENT_MARKER, TASK_CHECKED, TASK_OPEN]
        .iter()
        .any(|marker| raw.contains(marker))
    {
        raw.replace(ALIGNMENT_MARKER, "")
            .replace(TASK_CHECKED, "")
            .replace(TASK_OPEN, "")
            .into()
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    #[test]
    fn welcome_audit_preserves_literal_code_and_omits_active_html() {
        let document = render(crate::editor::fixtures::WELCOME.into());
        let preview = document.view.text.to_string();
        assert!(preview.contains("7. Esta lista comienza en siete."));
        assert!(preview.contains("<strong>Esto debe permanecer literal.</strong>"));
        assert!(preview.contains("Repositorio de anotau (https://www.github.com/ivnisc/anotau)"));
        assert!(preview.contains("👩‍💻"));
        assert!(preview.contains("A anotar!"));
        assert!(!preview.contains("NO_MOSTRAR_SCRIPT"));
        assert!(!preview.contains("NO_MOSTRAR_CSS"));
        assert!(!preview.contains("Este comentario no debería mostrarse"));
    }

    #[test]
    fn renders_formatting_and_keeps_code_literal() {
        let document = render(
            concat!(
                "# Café\n\n**Fuerte** y <em>HTML &amp; texto</em> ~~viejo~~\n\n",
                "`<b>dos  espacios</b>`\n\n",
                "```html\n<strong>literal</strong>\n```\n\n",
                "<pre>  literal<script>inactivo</script></pre>\n\n",
                "&lt;u&gt;escapado&lt;/u&gt;\n\n",
                "![Mapa](mapa.png) <img src=\"ruta.png\" alt=\"Ruta\">\n",
            )
            .into(),
        );
        let preview = document.view.text.to_string();
        assert!(preview.contains("Fuerte y HTML & texto viejo"));
        assert!(preview.contains("<b>dos  espacios</b>"));
        assert!(preview.contains("html\n<strong>literal</strong>"));
        assert!(preview.contains("  literal"));
        assert!(!preview.contains("inactivo"));
        assert!(preview.contains("<u>escapado</u>"));
        assert!(preview.contains("Mapa (mapa.png) Ruta (ruta.png)"));
        assert!(
            document
                .view
                .text
                .lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| {
                    span.content.trim() == "viejo"
                        && span.style.add_modifier.contains(Modifier::CROSSED_OUT)
                })
        );
        assert!(
            document
                .highlights
                .iter()
                .any(|(range, style)| *style == 0 && range.start == 0)
        );
    }

    #[test]
    fn preserves_tasks_and_table_alignment_without_using_html_css() {
        let preview = render(concat!(
            "- [x] Listo\n  - [ ] Pendiente\n\n",
            "| A | B |\n| :-- | --: |\n| 界 | larga |\n\n",
            "<table><tr><td style=\"text-align:right\">X</td></tr><tr><td>larga</td></tr></table>",
        ).into())
        .view.text
        .to_string();
        assert!(preview.contains("☑ Listo\n  ☐ Pendiente"), "{preview}");
        assert!(preview.contains("│ A  │     B │"), "{preview}");
        assert!(preview.contains("│ 界 │ larga │"), "{preview}");
        assert!(preview.contains("│ X     │"), "{preview}");
    }
}
