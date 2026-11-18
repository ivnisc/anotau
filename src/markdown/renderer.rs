use std::{borrow::Cow, collections::HashMap, sync::Arc};

use ego_tree::NodeRef;
use ratatui::{
    style::Style,
    text::{Line, Span, Text},
};
use scraper::{ElementRef, Html, Node};

use super::{
    ALIGNMENT_MARKER, LineInfo, Link, Preview, RULE, TASK_CHECKED, TASK_OPEN, code_style,
    heading_style, link_style,
};

pub(super) fn render(html: &str) -> Preview {
    let dom = Html::parse_fragment(html);
    let mut renderer = Renderer::default();
    for node in dom.root_element().children() {
        renderer.node(node, Style::new(), 0);
    }
    Preview {
        text: Text::from(renderer.finish()),
        info: renderer.info,
        anchors: renderer.anchors,
    }
}

type HtmlNode<'a> = NodeRef<'a, Node>;

#[derive(Default)]
struct Renderer {
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    space: bool,
    indent: usize,
    quotes: usize,
    info: Vec<LineInfo>,
    current_links: Vec<Link>,
    link: Option<Arc<str>>,
    block: Option<usize>,
    column: usize,
    anchors: HashMap<String, usize>,
    anchor_suffixes: HashMap<String, usize>,
    rule: bool,
}

impl Renderer {
    fn node(&mut self, node: HtmlNode<'_>, style: Style, depth: usize) {
        if depth >= 128 {
            self.text("…", style);
            return;
        }

        if let Some(text) = node.value().as_text() {
            self.text(text, style);
            return;
        }
        let Some(element) = ElementRef::wrap(node) else {
            return;
        };

        let name = element.value().name();
        if is_omitted(name) {
            return;
        }
        if let Some(mode) = element.attr("data-anotau-math") {
            let block = mode == "block";
            if block {
                self.paragraph();
            }
            let previous = self.block;
            if block {
                self.block = Some(self.lines.len());
            }
            self.children(element, style.patch(code_style()), depth);
            if block {
                self.finish_block(previous);
            } else {
                self.block = previous;
            }
            return;
        }
        match name {
            "br" => self.newline(),
            "hr" => {
                self.paragraph();
                self.push("─".into(), RULE);
                self.rule = true;
                self.paragraph();
            }
            "pre" => self.code_block(element),
            "table" => self.table(element, depth + 1),
            "ul" | "ol" => self.list(element, style, depth + 1),
            "blockquote" => {
                self.paragraph();
                self.quotes += 1;
                self.children(element, style.italic(), depth);
                self.paragraph();
                self.quotes -= 1;
            }
            "img" => {
                let link = self.link.take();
                self.link = element.attr("src").map(Arc::from);
                self.reference(
                    element.attr("alt").unwrap_or(""),
                    element.attr("src"),
                    link_style(),
                );
                self.link = link;
            }
            "a" => {
                let link = self.link.take();
                self.link = element.attr("href").map(Arc::from);
                self.children(element, style.patch(link_style()), depth);
                self.reference("", element.attr("href"), RULE);
                self.link = link;
            }
            "strong" | "b" => self.children(element, style.bold(), depth),
            "em" | "i" => self.children(element, style.italic(), depth),
            "s" | "del" | "strike" => self.children(element, style.crossed_out(), depth),
            "code" => {
                if std::mem::take(&mut self.space) && !self.current.is_empty() {
                    self.push(" ".into(), style);
                }
                self.push(
                    code_text(element).replace(['\n', '\r', '\t'], " ").into(),
                    style.patch(code_style()),
                );
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.heading(element, style, depth),
            "p" | "div" | "section" | "article" | "details" | "summary" => {
                self.paragraph();
                self.children(element, style, depth);
                self.paragraph();
            }
            "span" | "li" => self.children(element, style, depth),
            _ => {
                self.text(&format!("<{name}>"), RULE);
                self.children(element, style, depth);
                if element.has_children() {
                    self.text(&format!("</{name}>"), RULE);
                }
            }
        }
    }

    fn heading(&mut self, element: ElementRef<'_>, style: Style, depth: usize) {
        let name = element.value().name();
        self.paragraph();
        let base = element.attr("id").map_or_else(
            || {
                code_text(element)
                    .trim()
                    .to_lowercase()
                    .chars()
                    .filter(|ch| {
                        ch.is_alphanumeric() || matches!(ch, '-' | '_') || ch.is_whitespace()
                    })
                    .map(|ch| if ch.is_whitespace() { '-' } else { ch })
                    .collect::<String>()
            },
            str::to_owned,
        );
        let suffix = self.anchor_suffixes.entry(base.clone()).or_default();
        let mut anchor = if *suffix == 0 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        while self.anchors.contains_key(&anchor) {
            *suffix += 1;
            anchor = format!("{base}-{suffix}");
        }
        *suffix += 1;
        self.anchors.insert(anchor, self.lines.len());
        self.children(element, style.patch(heading_style()), depth);
        if name == "h2" {
            let width = self.column.min(40);
            self.newline();
            self.push("─".repeat(width).into(), heading_style());
        }
        self.paragraph();
    }

    fn children(&mut self, element: ElementRef<'_>, style: Style, depth: usize) {
        for child in element.children() {
            self.node(child, style, depth + 1);
        }
    }

    fn code_block(&mut self, element: ElementRef<'_>) {
        self.paragraph();
        let previous = self.block;
        self.block = Some(self.lines.len());
        if let Some(language) = element
            .child_elements()
            .next()
            .and_then(|code| code.attr("class"))
            .and_then(|class| class.strip_prefix("language-"))
        {
            self.text(language, RULE);
            self.newline();
        }
        for line in code_text(element).lines() {
            self.push(line.replace('\t', "    ").into(), code_style());
            self.newline();
        }
        self.finish_block(previous);
    }

    fn list(&mut self, element: ElementRef<'_>, style: Style, depth: usize) {
        let ordered = element.value().name() == "ol";
        let mut number = element
            .attr("start")
            .and_then(|start| start.parse::<i64>().ok())
            .unwrap_or(1);
        if self.indent == 0 {
            self.paragraph();
        } else if !self.current.is_empty() {
            self.newline();
        }
        for item in element
            .child_elements()
            .filter(|item| item.value().name() == "li")
        {
            let (marker, marker_style): (Cow<'static, str>, Style) = match task_state(item) {
                Some(true) => ("☑ ".into(), Style::new().fg(crate::theme::emphasis())),
                Some(false) => ("☐ ".into(), Style::new()),
                None if ordered => (format!("{number}. ").into(), RULE),
                None => ("• ".into(), RULE),
            };
            let marker = Span::styled(marker, marker_style);
            let width = marker.width();
            self.push(marker.content, marker_style);
            self.indent += width;
            for child in item.children() {
                match ElementRef::wrap(child) {
                    Some(paragraph) if paragraph.value().name() == "p" => {
                        self.children(paragraph, style, depth);
                        self.newline();
                    }
                    _ => self.node(child, style, depth),
                }
            }
            if !self.current.is_empty() {
                self.newline();
            }
            self.indent -= width;
            number = number.saturating_add(1);
        }
        if self.indent == 0 {
            self.paragraph();
        }
    }

    fn table(&mut self, element: ElementRef<'_>, depth: usize) {
        let alignments = element
            .prev_sibling()
            .and_then(|node| node.value().as_comment())
            .and_then(|comment| comment.strip_prefix(ALIGNMENT_MARKER))
            .unwrap_or("");
        let mut rows = Vec::new();
        for group in element.child_elements() {
            match group.value().name() {
                "thead" | "tbody" | "tfoot" => rows.extend(group.child_elements()),
                "tr" => rows.push(group),
                _ => {}
            }
        }

        let mut widths = Vec::<usize>::new();
        let rows: Vec<_> = rows
            .into_iter()
            .map(|row| {
                let cells: Vec<_> = row
                    .child_elements()
                    .filter(|cell| matches!(cell.value().name(), "td" | "th"))
                    .map(|cell| render_cell(cell, depth))
                    .collect();
                if cells.len() > widths.len() {
                    widths.resize(cells.len(), 0);
                }
                for (cell, width) in cells.iter().zip(&mut widths) {
                    *width = (*width).max(cell.0.width());
                }
                let header = row.child_elements().any(|cell| cell.value().name() == "th");
                (cells, header)
            })
            .collect();

        if widths.is_empty() {
            return;
        }
        self.paragraph();
        let previous = self.block;
        self.block = Some(self.lines.len());
        for (cells, header) in rows {
            let mut cells = cells.into_iter();
            self.push("│".into(), RULE);
            for (index, width) in widths.iter().copied().enumerate() {
                let cell = cells.next();
                let space = width.saturating_sub(cell.as_ref().map_or(0, |cell| cell.0.width()));
                let left = match alignments.as_bytes().get(index) {
                    Some(b'c') => space / 2,
                    Some(b'r') => space,
                    _ => 0,
                };
                self.push(" ".repeat(left + 1).into(), Style::new());
                if let Some((cell, links)) = cell {
                    self.current_links.extend(links.into_iter().map(|mut link| {
                        link.columns =
                            link.columns.start + self.column..link.columns.end + self.column;
                        link
                    }));
                    self.column += cell.width();
                    self.current.extend(cell.spans);
                }
                self.push(" ".repeat(space - left + 1).into(), Style::new());
                self.push("│".into(), RULE);
            }
            self.newline();
            if header {
                let separator = widths
                    .iter()
                    .map(|width| "─".repeat(width + 2))
                    .collect::<Vec<_>>()
                    .join("┼");
                self.push(format!("├{separator}┤").into(), RULE);
                self.newline();
            }
        }
        self.finish_block(previous);
    }

    fn finish_block(&mut self, previous: Option<usize>) {
        if !self.current.is_empty() {
            self.newline();
        }
        self.block = previous;
        self.paragraph();
    }

    fn reference(&mut self, label: &str, destination: Option<&str>, style: Style) {
        if !label.is_empty() {
            self.text(label, style);
        }
        if let Some(destination) = destination.filter(|destination| !destination.is_empty()) {
            if self.current.is_empty() {
                self.text(destination, style);
            } else {
                self.text(&format!(" ({destination})"), style);
            }
        }
    }

    fn text(&mut self, text: &str, style: Style) {
        let mut run = String::new();
        for ch in text.chars() {
            if ch.is_ascii_whitespace() {
                self.space = true;
            } else {
                if self.space && (!self.current.is_empty() || !run.is_empty()) {
                    run.push(' ');
                }
                self.space = false;
                run.push(if ch.is_control() { '�' } else { ch });
            }
        }
        if !run.is_empty() {
            self.push(run.into(), style);
        }
    }

    fn push(&mut self, text: Cow<'static, str>, style: Style) {
        if self.current.is_empty() {
            if self.quotes > 0 {
                self.current
                    .push(Span::styled("│ ".repeat(self.quotes), RULE));
            }
            if self.indent > 0 {
                self.current.push(Span::raw(" ".repeat(self.indent)));
            }
            self.column = self.current.iter().map(Span::width).sum();
        }
        let end = self.column + Span::raw(text.as_ref()).width();
        if let Some(destination) = &self.link {
            if let Some(previous) = self.current_links.last_mut()
                && previous.columns.end == self.column
                && Arc::ptr_eq(&previous.destination, destination)
            {
                previous.columns.end = end;
            } else {
                self.current_links.push(Link {
                    columns: self.column..end,
                    destination: Arc::clone(destination),
                });
            }
        }
        self.column = end;
        self.current.push(Span::styled(text, style));
    }

    fn newline(&mut self) {
        self.lines
            .push(Line::from(std::mem::take(&mut self.current)));
        self.info.push(LineInfo {
            block: self.block,
            links: std::mem::take(&mut self.current_links),
            rule: std::mem::take(&mut self.rule),
        });
        self.column = 0;
        self.space = false;
    }

    fn paragraph(&mut self) {
        if !self.current.is_empty() {
            self.newline();
        }
        if self.lines.last().is_some_and(|line| line.width() > 0) {
            self.newline();
        }
        self.space = false;
    }

    fn finish(&mut self) -> Vec<Line<'static>> {
        if !self.current.is_empty() {
            self.newline();
        }
        while self.lines.last().is_some_and(|line| line.width() == 0) {
            self.lines.pop();
            self.info.pop();
        }
        std::mem::take(&mut self.lines)
    }
}

fn render_cell(cell: ElementRef<'_>, depth: usize) -> (Line<'static>, Vec<Link>) {
    let mut contents = Renderer::default();
    let style = if cell.value().name() == "th" {
        Style::new().bold()
    } else {
        Style::new()
    };
    contents.children(cell, style, depth);
    let mut spans = Vec::new();
    let mut regions = Vec::new();
    let mut column = 0;
    let cell_lines = contents.finish();
    for (line, info) in cell_lines.into_iter().zip(contents.info) {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
            column += 1;
        }
        regions.extend(info.links.into_iter().map(|mut link| {
            link.columns = link.columns.start + column..link.columns.end + column;
            link
        }));
        column += line.width();
        spans.extend(line.spans);
    }
    (Line::from(spans), regions)
}

fn task_state(item: ElementRef<'_>) -> Option<bool> {
    let paragraph = item
        .child_elements()
        .next()
        .filter(|child| child.value().name() == "p");
    item.children()
        .chain(paragraph.into_iter().flat_map(|child| child.children()))
        .filter_map(|child| child.value().as_comment())
        .find_map(|comment| match &**comment {
            TASK_CHECKED => Some(true),
            TASK_OPEN => Some(false),
            _ => None,
        })
}

fn code_text(element: ElementRef<'_>) -> String {
    let mut result = String::new();
    let mut pending: Vec<_> = element.children().rev().collect();
    while let Some(node) = pending.pop() {
        if let Some(text) = node.value().as_text() {
            result.extend(text.chars().map(|ch| {
                if ch.is_control() && !matches!(ch, '\t' | '\n' | '\r') {
                    '�'
                } else {
                    ch
                }
            }));
        } else if let Some(child) = ElementRef::wrap(node) {
            match child.value().name() {
                "br" => result.push('\n'),
                name if !is_omitted(name) => pending.extend(child.children().rev()),
                _ => {}
            }
        }
    }
    result
}

fn is_omitted(name: &str) -> bool {
    matches!(
        name,
        "script"
            | "style"
            | "iframe"
            | "object"
            | "embed"
            | "form"
            | "input"
            | "button"
            | "textarea"
            | "select"
            | "template"
            | "svg"
            | "math"
    )
}
