use ratatui::{crossterm::event::KeyCode, widgets::ListState};

pub(crate) fn navigate(list: &mut ListState, key: KeyCode, count: usize) -> bool {
    let index = list
        .selected()
        .unwrap_or_default()
        .min(count.saturating_sub(1));
    let next = match key {
        KeyCode::Up => index.saturating_sub(1),
        KeyCode::Down => index.saturating_add(1).min(count.saturating_sub(1)),
        KeyCode::Home => 0,
        KeyCode::End => count.saturating_sub(1),
        _ => return false,
    };
    list.select((count > 0).then_some(next));
    true
}
