use std::io::{self, IsTerminal, stdout};

use ratatui::{
    DefaultTerminal,
    crossterm::{
        event::{
            DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
            KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        },
        execute,
        terminal::{LeaveAlternateScreen, supports_keyboard_enhancement},
    },
};

pub(crate) struct Session {
    bracketed_paste: bool,
    enhanced_keyboard: bool,
    mouse_capture: bool,
}

impl Session {
    pub(crate) fn start() -> io::Result<(Self, DefaultTerminal)> {
        if !io::stdin().is_terminal() || !stdout().is_terminal() {
            return Err(io::Error::other("Anotau requiere una terminal interactiva"));
        }

        let mut session = Self {
            bracketed_paste: false,
            enhanced_keyboard: false,
            mouse_capture: false,
        };
        let terminal = ratatui::try_init()?;
        session.bracketed_paste = true;
        if let Err(error) = execute!(stdout(), EnableBracketedPaste) {
            if error.kind() != io::ErrorKind::Unsupported {
                return Err(error);
            }
            session.bracketed_paste = false;
        }
        if supports_keyboard_enhancement().unwrap_or(false) {
            let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES;
            session.enhanced_keyboard =
                execute!(stdout(), PushKeyboardEnhancementFlags(flags)).is_ok();
        }
        Ok((session, terminal))
    }

    pub(crate) fn toggle_mouse_capture(&mut self) -> io::Result<bool> {
        self.set_mouse_capture(!self.mouse_capture)
    }

    pub(crate) fn set_mouse_capture(&mut self, enabled: bool) -> io::Result<bool> {
        if self.mouse_capture == enabled {
            return Ok(enabled);
        }
        if !enabled {
            execute!(stdout(), DisableMouseCapture)?;
        } else if let Err(error) = execute!(stdout(), EnableMouseCapture) {
            let _ = execute!(stdout(), DisableMouseCapture);
            return Err(error);
        }
        self.mouse_capture = enabled;
        Ok(self.mouse_capture)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.mouse_capture {
            let _ = execute!(stdout(), DisableMouseCapture);
        }
        if self.enhanced_keyboard {
            let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
        }
        if self.bracketed_paste {
            let _ = execute!(stdout(), DisableBracketedPaste);
        }
        if ratatui::try_restore().is_err() {
            // intenta salir de la pantalla alternativa aunque falle el modo raw
            let _ = execute!(stdout(), LeaveAlternateScreen);
        }
    }
}
