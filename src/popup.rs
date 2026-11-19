use rat_text::{
    HasScreenCursor,
    clipboard::LocalClipboard,
    text_input::{TextInput, TextInputState},
    undo_buffer::UndoVec,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Padding, StatefulWidget},
};

/// Campo para una contraseña: no pasa por el portapapeles ni queda en el historial de deshacer.
pub(crate) fn secret_input() -> TextInputState {
    let mut input = TextInputState::new();
    input.set_clipboard(None::<LocalClipboard>);
    input.set_undo_buffer(None::<UndoVec>);
    input
}

/// Etiqueta y campo de una línea en `area` (dos filas). El campo enfocado resalta su etiqueta
/// y recibe el cursor; `secret` lo muestra como asteriscos.
pub(crate) fn field(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    input: &mut TextInputState,
    focused: bool,
    secret: bool,
) {
    let [label_area, input_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    let style = if focused {
        crate::theme::selected()
    } else {
        Style::new()
    };
    frame.render_widget(Span::styled(label, style), label_area);
    let widget = TextInput::new()
        .style(Style::new().underlined())
        .select_style(Style::new().reversed());
    if secret { widget.passwd() } else { widget }.render(input_area, frame.buffer_mut(), input);
    if focused
        && let Some(cursor) = input.screen_cursor()
        && input_area.contains(cursor.into())
    {
        frame.set_cursor_position(cursor);
    }
}

/// Opciones en una línea, como `[Aceptar]   Rechazar `: la elegida va entre corchetes con
/// `highlight`; las demás llevan espacios en su lugar para que nada se desplace al cambiar.
pub(crate) fn choice(options: &[&str], selected: usize, highlight: Style) -> Line<'static> {
    let mut line = Line::default();
    for (index, option) in options.iter().enumerate() {
        if index > 0 {
            line.push_span(Span::raw("  "));
        }
        line.push_span(if index == selected {
            Span::styled(format!("[{option}]"), highlight)
        } else {
            Span::raw(format!(" {option} "))
        });
    }
    line
}

/// Lista con la marca y el estilo comunes de selección.
pub(crate) fn list<'a, T: Into<ListItem<'a>>>(items: impl IntoIterator<Item = T>) -> List<'a> {
    List::new(items)
        .highlight_symbol("› ")
        .highlight_style(crate::theme::selected())
}

pub(crate) fn draw(frame: &mut Frame<'_>, title: &'static str, width: u16, height: u16) -> Rect {
    let area = frame
        .area()
        .centered(Constraint::Max(width), Constraint::Max(height));
    let block = Block::bordered()
        .title(format!(" {title} "))
        .style(crate::theme::base())
        .border_style(Style::new().fg(crate::theme::accent()))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    inner
}
