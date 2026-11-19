use std::time::{Duration, Instant};

use ratatui::crossterm::event::{Event, KeyEventKind, MouseEventKind};

const AWAY: Duration = Duration::from_secs(5 * 60);
const EXPIRE: Duration = Duration::from_secs(15 * 60);

pub(crate) struct Activity {
    last: Instant,
}

impl Activity {
    pub(crate) fn new(now: Instant) -> Self {
        Self { last: now }
    }

    pub(crate) fn last_activity(&self) -> Instant {
        self.last
    }

    pub(crate) fn away(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last) >= AWAY
    }

    pub(crate) fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last) >= EXPIRE
    }

    pub(crate) fn wait(&self, now: Instant) -> Duration {
        let deadline = if self.away(now) { EXPIRE } else { AWAY };
        deadline
            .saturating_sub(now.saturating_duration_since(self.last))
            .min(Duration::from_secs(60))
    }

    pub(crate) fn observe(&mut self, event: &Event, now: Instant) {
        if match event {
            Event::Key(key) => key.kind != KeyEventKind::Release,
            Event::Paste(_) => true,
            Event::Mouse(mouse) => {
                !matches!(mouse.kind, MouseEventKind::Moved | MouseEventKind::Up(_))
            }
            _ => false,
        } {
            self.last = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn idle_deadlines_ignore_redraws_and_resume_on_input() {
        let now = Instant::now();
        let mut activity = Activity::new(now);
        activity.observe(&Event::Resize(80, 24), now + AWAY);
        assert!(activity.away(now + AWAY));
        assert!(!activity.expired(now + Duration::from_secs(15 * 60 - 1)));
        assert!(activity.expired(now + EXPIRE));
        assert_eq!(activity.wait(now + EXPIRE), Duration::ZERO);
        activity.observe(
            &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            now + AWAY,
        );
        assert!(!activity.away(now + AWAY));
        assert!(!activity.expired(now + EXPIRE));
    }
}
