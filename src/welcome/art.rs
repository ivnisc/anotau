use std::io;

use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
};

pub(super) fn logo() -> io::Result<Text<'static>> {
    parse(include_str!("../../assets/welcome_frame.txt")).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "El logo contiene ANSI no admitido",
        )
    })
}

fn parse(art: &'static str) -> Option<Text<'static>> {
    let mut color = Style::new();
    let mut lines = Vec::new();
    for line in art.lines() {
        let mut spans = Vec::new();
        let mut remaining = line;
        loop {
            let next = remaining.split_once('\x1b');
            let literal = next.map_or(remaining, |(text, _)| text);
            if literal.chars().any(char::is_control) {
                return None;
            }
            if !literal.is_empty() {
                spans.push(Span::styled(literal, color));
            }
            let Some((_, escape)) = next else {
                break;
            };
            let (parameters, rest) = escape.strip_prefix('[')?.split_once('m')?;
            color = sgr(parameters)?;
            remaining = rest;
        }
        lines.push(Line::from(spans));
    }
    Some(Text::from(lines))
}

fn sgr(parameters: &str) -> Option<Style> {
    if parameters == "0" {
        return Some(Style::new());
    }
    let mut rgb = parameters.strip_prefix("38;2;")?.split(';');
    let color = Color::Rgb(
        rgb.next()?.parse().ok()?,
        rgb.next()?.parse().ok()?,
        rgb.next()?.parse().ok()?,
    );
    rgb.next().is_none().then_some(Style::new().fg(color))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_rgb_colors_and_rejects_other_terminal_sequences() {
        let art = logo().unwrap();
        assert_eq!((art.width(), art.height()), (40, 17));
        assert!(
            art.lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| { matches!(span.style.fg, Some(Color::Rgb(..))) })
        );
        let sample = parse("\x1b[38;2;12;34;56mX\x1b[0mY").unwrap();
        assert_eq!(
            sample.lines[0].spans[0].style.fg,
            Some(Color::Rgb(12, 34, 56))
        );
        assert_eq!(sample.lines[0].spans[1].style.fg, None);
        assert!(parse("\x1b[2J").is_none());
        assert!(parse("\x1b]8;;https://example.invalid").is_none());
    }
}
