//! Indicador de una escritura en curso dentro de su popup.
//!
//! El servidor local responde en milisegundos; un popup que se cierra antes de que el indicador
//! llegue a verse parece no haber hecho nada. Por eso el indicador dura al menos [`MINIMUM`]
//! (engaño benévolo) y, mientras tanto, el popup no acepta otra confirmación (antirrebote).

use std::time::{Duration, Instant};

use anotau_protocol::Command;
use ratatui::{
    style::Style,
    text::{Line, Span},
};

/// Duración mínima visible: suficiente para leer la acción sin frenar al usuario.
pub(crate) const MINIMUM: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(450)
};

/// Intervalo de redibujo mientras hay un indicador activo.
pub(crate) const FRAME: Duration = Duration::from_millis(80);

/// Celda braille con un punto ausente que gira en sentido horario.
const SPIN: [char; 8] = ['⣾', '⣷', '⣯', '⣟', '⡿', '⢿', '⣽', '⣻'];
/// Pista de transferencia: un punto lleno avanza hacia la derecha.
const TRACK: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Motion {
    Spin,
    Transfer,
}

pub(crate) struct Progress {
    started: Instant,
    motion: Motion,
    label: &'static str,
    /// Respuesta exitosa recibida: el popup se cierra en este instante.
    close_at: Option<Instant>,
}

impl Progress {
    pub(crate) fn start(command: &Command, now: Instant) -> Self {
        let (motion, label) = match command {
            Command::CreateEntry { .. } | Command::CreateWorkspace { .. } => {
                (Motion::Spin, "Creando…")
            }
            Command::CopyEntry { .. } | Command::CopyWorkspace { .. } => {
                (Motion::Spin, "Copiando…")
            }
            Command::MoveEntry { .. } => (Motion::Spin, "Moviendo…"),
            Command::DeleteEntry { .. } | Command::DeleteWorkspace { .. } => {
                (Motion::Spin, "Eliminando…")
            }
            Command::RenameEntry { .. } | Command::RenameWorkspace { .. } => {
                (Motion::Spin, "Renombrando…")
            }
            Command::Save { .. } => (Motion::Spin, "Guardando…"),
            Command::Preferences { .. } => (Motion::Spin, "Aplicando…"),
            Command::Contact { .. } => (Motion::Transfer, "Enviando solicitud…"),
            Command::ResolveContact { .. } => (Motion::Transfer, "Respondiendo…"),
            Command::Offer { .. } => (Motion::Transfer, "Enviando nota…"),
            Command::ResolveTransfer {
                destination: Some(_),
                ..
            } => (Motion::Transfer, "Recibiendo nota…"),
            Command::ResolveTransfer {
                destination: None, ..
            } => (Motion::Transfer, "Rechazando…"),
            Command::Suggest { .. } => (Motion::Transfer, "Enviando sugerencia…"),
        };
        Self {
            started: now,
            motion,
            label,
            close_at: None,
        }
    }

    /// Petición de cuenta que no pasa por la cola de escrituras, como borrarla o cambiar la
    /// contraseña; se anima igual y termina con su respuesta.
    pub(crate) fn account(label: &'static str, now: Instant) -> Self {
        Self {
            started: now,
            motion: Motion::Spin,
            label,
            close_at: None,
        }
    }

    /// Respuesta exitosa: el popup se cierra cuando el indicador cumplió su duración mínima.
    pub(crate) fn succeed(&mut self, now: Instant, minimum: Duration) {
        self.close_at = Some((self.started + minimum).max(now));
    }

    pub(crate) fn due(&self, now: Instant) -> bool {
        self.close_at.is_some_and(|at| now >= at)
    }

    pub(crate) fn line(&self, now: Instant) -> Line<'static> {
        let elapsed = now.saturating_duration_since(self.started);
        let step = usize::try_from(elapsed.as_millis() / FRAME.as_millis()).unwrap_or_default();
        let accent = Style::new().fg(crate::theme::accent());
        let indicator = match self.motion {
            Motion::Spin => SPIN[step % SPIN.len()].to_string(),
            // la pista avanza a la mitad de velocidad para que el recorrido se lea
            Motion::Transfer => (0..TRACK)
                .map(|cell| {
                    if cell == step / 2 % TRACK {
                        '▪'
                    } else {
                        '·'
                    }
                })
                .collect(),
        };
        Line::from(vec![
            Span::styled(indicator, accent),
            Span::raw(" "),
            Span::styled(self.label, Style::new().dim()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fast_success_stays_visible_for_the_minimum_and_a_slow_one_closes_at_once() {
        let now = Instant::now();
        let minimum = Duration::from_millis(450);
        let command = Command::CreateWorkspace {
            title: "Ramos".into(),
        };
        let mut fast = Progress::start(&command, now);
        fast.succeed(now + Duration::from_millis(20), minimum);
        assert!(!fast.due(now + Duration::from_millis(449)));
        assert!(fast.due(now + minimum));

        let mut slow = Progress::start(&command, now);
        slow.succeed(now + Duration::from_secs(2), minimum);
        assert!(slow.due(now + Duration::from_secs(2)));
    }
}
