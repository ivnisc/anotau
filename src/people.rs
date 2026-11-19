use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{List, ListItem, ListState},
};

fn away_style() -> Style {
    Style::new().fg(crate::theme::away())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct UserId(pub(crate) i64);

pub(crate) use anotau_protocol::{Contact, Presence};

pub(crate) fn indicator(presence: Presence) -> Span<'static> {
    let (symbol, style) = match presence {
        Presence::Active => ("●", Style::new().fg(crate::theme::active())),
        Presence::Away => ("◐", away_style()),
        Presence::Offline => ("○", Style::new().dim()),
    };
    Span::styled(symbol, style)
}

#[derive(Clone)]
pub(crate) struct Person {
    pub(crate) id: UserId,
    pub(crate) name: String,
    pub(crate) presence: Presence,
    pub(crate) contact: Contact,
}

pub(crate) struct People {
    pub(crate) entries: Vec<Person>,
    list: ListState,
}

impl People {
    pub(crate) fn empty() -> Self {
        Self {
            entries: Vec::new(),
            list: ListState::default(),
        }
    }
    pub(crate) fn replace(&mut self, mut entries: Vec<Person>) {
        let selected = self.selected().map(|person| person.id);
        // contactos aceptados primero; en cada grupo, activos, ausentes y desconectados
        entries.sort_by_cached_key(|person| {
            (
                person.contact != Contact::Accepted,
                match person.presence {
                    Presence::Active => 0,
                    Presence::Away => 1,
                    Presence::Offline => 2,
                },
                person.name.to_lowercase(),
                person.id,
            )
        });
        self.list.select(if entries.is_empty() {
            None
        } else {
            Some(
                selected
                    .and_then(|id| entries.iter().position(|person| person.id == id))
                    .unwrap_or_default(),
            )
        });
        self.entries = entries;
    }

    pub(crate) fn selected(&self) -> Option<&Person> {
        self.list
            .selected()
            .and_then(|index| self.entries.get(index))
    }

    pub(crate) fn navigate(&mut self, key: KeyCode) -> bool {
        crate::selection::navigate(&mut self.list, key, self.entries.len())
    }

    /// Posición «n de m» del usuario elegido.
    pub(crate) fn position(&self) -> Option<(usize, usize)> {
        self.list
            .selected()
            .map(|index| (index + 1, self.entries.len()))
    }

    pub(crate) fn draw(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        incoming: &[crate::inbox::Incoming],
    ) {
        let items = self.entries.iter().enumerate().map(|(index, person)| {
            let name_style = if self.list.selected() == Some(index) {
                Style::new().fg(crate::theme::accent())
            } else {
                Style::new()
            };
            let mut line = Line::from(vec![
                indicator(person.presence),
                Span::raw(" "),
                Span::styled(person.name.as_str(), name_style),
            ]);
            let count = incoming
                .iter()
                .filter(|offer| offer.sender == person.id)
                .count();
            if count > 0 {
                line.push_span(Span::styled(
                    format!(" ↓{count}"),
                    Style::new().fg(crate::theme::warning()),
                ));
            }
            let contact = match person.contact {
                Contact::Accepted => {
                    Some(Span::styled("★", Style::new().fg(crate::theme::accent())))
                }
                Contact::Pending | Contact::Incoming(_) => Some(Span::styled("?", away_style())),
                Contact::None => None,
                Contact::Rejected => Some(Span::styled("×", Style::new().dim())),
            };
            if let Some(contact) = contact {
                line.push_span(" ");
                line.push_span(contact);
            }
            ListItem::new(line)
        });
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol("› ")
                .highlight_style(Style::new().bold()),
            area,
            &mut self.list,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn directory() -> People {
        let mut people = People::empty();
        people.replace(
            [
                ("Zoe", Presence::Offline, Contact::None),
                ("Carla", Presence::Active, Contact::Accepted),
                ("Ana", Presence::Active, Contact::None),
                ("Diego", Presence::Away, Contact::Pending),
                ("Bruno", Presence::Offline, Contact::None),
                ("Elena", Presence::Offline, Contact::Accepted),
                ("Javier", Presence::Away, Contact::Accepted),
                ("María", Presence::Offline, Contact::None),
            ]
            .into_iter()
            .zip(1..)
            .map(|((name, presence, contact), id)| Person {
                id: UserId(id),
                name: name.into(),
                presence,
                contact,
            })
            .collect(),
        );
        people
    }

    #[test]
    fn presence_symbols_keep_their_colors_when_selected_without_text_labels() {
        use ratatui::{Terminal, backend::TestBackend, style::Modifier};
        let mut people = directory();
        let mut terminal = Terminal::new(TestBackend::new(36, 8)).unwrap();
        terminal
            .draw(|frame| people.draw(frame, frame.area(), &[]))
            .unwrap();
        for (y, expected, color) in [
            (0, "● Carla ★", Some(Color::Indexed(82))),
            (1, "◐ Javier ★", Some(Color::Indexed(208))),
            (2, "○ Elena ★", None),
            (3, "● Ana", Some(Color::Indexed(82))),
            (4, "◐ Diego ?", Some(Color::Indexed(208))),
            (5, "○ Bruno", None),
        ] {
            let row = (0..36)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>();
            assert!(row.contains(expected));
            assert!(
                !row.contains('·')
                    && !row.contains("activo")
                    && !row.contains("ausente")
                    && !row.contains("pendiente")
                    && !row.contains("desconectado")
            );
            let start = row.find(expected).unwrap();
            let status_x = u16::try_from(row[..start].chars().count()).unwrap();
            let cell = &terminal.backend().buffer()[(status_x, y)];
            if let Some(color) = color {
                assert_eq!(cell.fg, color, "{expected}");
            }
            assert!(!cell.modifier.contains(Modifier::UNDERLINED));
        }
    }

    #[test]
    fn directory_orders_contacts_first_then_others_by_presence_and_keeps_selection() {
        let mut people = directory();
        assert_eq!(
            people
                .entries
                .iter()
                .map(|person| person.name.as_str())
                .collect::<Vec<_>>(),
            [
                "Carla", "Javier", "Elena", "Ana", "Diego", "Bruno", "María", "Zoe"
            ]
        );
        people.navigate(KeyCode::Down);
        let selected = people.selected().unwrap().id;
        let mut updated = people.entries.clone();
        updated
            .iter_mut()
            .find(|person| person.id == selected)
            .unwrap()
            .presence = Presence::Offline;
        people.replace(updated);
        assert_eq!(people.selected().unwrap().id, selected);
        people.replace(Vec::new());
        assert!(people.selected().is_none());
        people.navigate(KeyCode::Down);
        assert!(people.selected().is_none());
    }
}
