//! Esquemas de color de la interfaz. `Terminal` hereda los colores del emulador; los demás
//! pintan también el fondo, para que su contraste no dependa del tema de la terminal.

use std::cell::Cell;

pub(crate) use anotau_protocol::Theme;
use ratatui::style::{Color, Style};

pub(crate) const ALL: [Theme; 7] = [
    Theme::Terminal,
    Theme::Nord,
    Theme::TokyoNight,
    Theme::Dracula,
    Theme::GruvboxDark,
    Theme::SolarizedDark,
    Theme::SolarizedLight,
];

/// Colores con significado; cada uso de la interfaz elige uno de ellos.
struct Colors {
    background: Color,
    foreground: Color,
    accent: Color,
    warning: Color,
    emphasis: Color,
    html: Color,
    active: Color,
    away: Color,
}

const fn rgb((red, green, blue): (u8, u8, u8)) -> Color {
    Color::Rgb(red, green, blue)
}

const TERMINAL: Colors = Colors {
    background: Color::Reset,
    foreground: Color::Reset,
    accent: Color::Cyan,
    warning: Color::Yellow,
    emphasis: Color::Green,
    html: Color::Magenta,
    active: Color::Indexed(82),
    away: Color::Indexed(208),
};

/// Paleta de un esquema: fondo, texto, acento, aviso, énfasis, HTML, activo y ausente.
const fn palette(values: [(u8, u8, u8); 8]) -> Colors {
    Colors {
        background: rgb(values[0]),
        foreground: rgb(values[1]),
        accent: rgb(values[2]),
        warning: rgb(values[3]),
        emphasis: rgb(values[4]),
        html: rgb(values[5]),
        active: rgb(values[6]),
        away: rgb(values[7]),
    }
}

const fn colors(theme: Theme) -> Colors {
    match theme {
        Theme::Terminal => TERMINAL,
        Theme::Nord => palette([
            (0x2E, 0x34, 0x40),
            (0xD8, 0xDE, 0xE9),
            (0x88, 0xC0, 0xD0),
            (0xEB, 0xCB, 0x8B),
            (0xA3, 0xBE, 0x8C),
            (0xB4, 0x8E, 0xAD),
            (0xA3, 0xBE, 0x8C),
            (0xD0, 0x87, 0x70),
        ]),
        Theme::TokyoNight => palette([
            (0x1A, 0x1B, 0x26),
            (0xC0, 0xCA, 0xF5),
            (0x7A, 0xA2, 0xF7),
            (0xE0, 0xAF, 0x68),
            (0x9E, 0xCE, 0x6A),
            (0xBB, 0x9A, 0xF7),
            (0x9E, 0xCE, 0x6A),
            (0xFF, 0x9E, 0x64),
        ]),
        Theme::Dracula => palette([
            (0x28, 0x2A, 0x36),
            (0xF8, 0xF8, 0xF2),
            (0x8B, 0xE9, 0xFD),
            (0xF1, 0xFA, 0x8C),
            (0x50, 0xFA, 0x7B),
            (0xFF, 0x79, 0xC6),
            (0x50, 0xFA, 0x7B),
            (0xFF, 0xB8, 0x6C),
        ]),
        Theme::GruvboxDark => palette([
            (0x28, 0x28, 0x28),
            (0xEB, 0xDB, 0xB2),
            (0x83, 0xA5, 0x98),
            (0xFA, 0xBD, 0x2F),
            (0xB8, 0xBB, 0x26),
            (0xD3, 0x86, 0x9B),
            (0xB8, 0xBB, 0x26),
            (0xFE, 0x80, 0x19),
        ]),
        Theme::SolarizedDark => palette([
            (0x00, 0x2B, 0x36),
            (0x83, 0x94, 0x96),
            (0x26, 0x8B, 0xD2),
            (0xB5, 0x89, 0x00),
            (0x85, 0x99, 0x00),
            (0xD3, 0x36, 0x82),
            (0x85, 0x99, 0x00),
            (0xCB, 0x4B, 0x16),
        ]),
        Theme::SolarizedLight => palette([
            (0xFD, 0xF6, 0xE3),
            (0x65, 0x7B, 0x83),
            (0x26, 0x8B, 0xD2),
            (0xB5, 0x89, 0x00),
            (0x85, 0x99, 0x00),
            (0xD3, 0x36, 0x82),
            (0x85, 0x99, 0x00),
            (0xCB, 0x4B, 0x16),
        ]),
    }
}

pub(crate) fn name(theme: Theme) -> &'static str {
    match theme {
        Theme::Terminal => "Terminal",
        Theme::Nord => "Nord",
        Theme::TokyoNight => "Tokyo Night",
        Theme::Dracula => "Dracula",
        Theme::GruvboxDark => "Gruvbox oscuro",
        Theme::SolarizedDark => "Solarized oscuro",
        Theme::SolarizedLight => "Solarized claro",
    }
}

// shortcut: el bucle de la TUI dibuja y aplica preferencias en un solo hilo, así que un valor
// por hilo evita pasar el tema por cada función de dibujo.
thread_local! {
    static CURRENT: Cell<Theme> = const { Cell::new(Theme::Terminal) };
}

/// Aplica el tema de la cuenta; devuelve si cambió.
pub(crate) fn set(theme: Theme) -> bool {
    CURRENT.replace(theme) != theme
}

fn current() -> Colors {
    colors(CURRENT.get())
}

/// Fondo y texto base; con `Terminal` no se pinta nada.
pub(crate) fn base() -> Style {
    let colors = current();
    Style::new().fg(colors.foreground).bg(colors.background)
}

pub(crate) fn accent() -> Color {
    current().accent
}

pub(crate) fn warning() -> Color {
    current().warning
}

pub(crate) fn emphasis() -> Color {
    current().emphasis
}

pub(crate) fn html() -> Color {
    current().html
}

pub(crate) fn active() -> Color {
    current().active
}

pub(crate) fn away() -> Color {
    current().away
}

/// Elemento elegido en listas y opciones.
pub(crate) fn selected() -> Style {
    Style::new().fg(accent()).bold()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_theme_inherits_and_presets_paint_background_and_text() {
        assert!(!set(Theme::Terminal));
        assert_eq!(base(), Style::new().fg(Color::Reset).bg(Color::Reset));
        assert_eq!(accent(), Color::Cyan);
        assert!(set(Theme::Nord));
        assert_eq!(base().bg, Some(Color::Rgb(0x2E, 0x34, 0x40)));
        assert_eq!(accent(), Color::Rgb(0x88, 0xC0, 0xD0));
        for theme in ALL.into_iter().skip(1) {
            set(theme);
            assert!(matches!(base().bg, Some(Color::Rgb(..))), "{}", name(theme));
            assert_ne!(base().fg, base().bg, "{}", name(theme));
        }
        set(Theme::Terminal);
    }
}
