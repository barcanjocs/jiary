//! The Pomodoro: a target session length, and what Jiary does once a work
//! session reaches it.
//!
//! The only mode so far is `Soft`, which never interrupts: a thin bar
//! under the timer fills up toward the target, then the timer turns green
//! and the cat has a coffee ("break's earned, if you want one"). Past the
//! target it just stays that way. Detours get none of it. `p` turns it on
//! and off, and `/settings` sets the target.
//!
//! Everything asks `pomodoro()` where the running session stands, so a
//! stricter mode would branch on `PomodoroMode` there and in the few
//! places that call it.

use chrono::Utc;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::session;
use crate::settings::PomodoroMode;
use crate::theme;

use super::App;

/// Columns the bar under the timer takes, before its target label.
const BAR_WIDTH: usize = 18;

/// Where the running session stands against the target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pomodoro {
    pub(super) mode: PomodoroMode,
    /// Elapsed time over the target: 1 or more once reached.
    pub(super) progress: f64,
}

impl Pomodoro {
    pub(super) fn reached(self) -> bool {
        self.progress >= 1.0
    }
}

impl App {
    /// The running session against the target: `None` with the Pomodoro
    /// off, nothing running, or a detour (breaks, disruptions and meetings
    /// have no target).
    pub(super) fn pomodoro(&self) -> Option<Pomodoro> {
        let mode = self.settings.pomodoro;
        let session = self.active_session.as_ref()?;
        if mode == PomodoroMode::Off || session::is_detour(&session.activity) {
            return None;
        }
        let elapsed = (Utc::now() - session.started_at).num_seconds().max(0) as f64;
        let target = self.settings.target_minutes as f64 * 60.0;
        Some(Pomodoro {
            mode,
            progress: elapsed / target,
        })
    }

    /// `p`: turns the Pomodoro off, or back on (soft) with the target set
    /// in `/settings`. Mid-session, the cue just appears or goes against
    /// the time so far.
    pub(super) fn toggle_pomodoro(&mut self) {
        self.settings.pomodoro = match self.settings.pomodoro {
            PomodoroMode::Off => PomodoroMode::Soft,
            _ => PomodoroMode::Off,
        };
        if self.save_settings() {
            let text = match self.settings.pomodoro {
                PomodoroMode::Off => "Pomodoro off".to_string(),
                mode => format!(
                    "Pomodoro on ({}): {}m target; /settings changes it",
                    mode.name(),
                    self.settings.target_minutes
                ),
            };
            self.notify(&text);
        }
    }

    /// The timer's style: the accent, or green once the target is reached.
    pub(super) fn timer_style(&self) -> Style {
        match self.pomodoro() {
            Some(pomodoro) if pomodoro.reached() => theme::TITLE.fg(theme::GOOD),
            _ => theme::TITLE,
        }
    }

    /// The row under the timer: a thin bar filling up toward the target,
    /// green and full once reached, then the target. None without a
    /// Pomodoro for this session.
    pub(super) fn pomodoro_line(&self) -> Option<Line<'static>> {
        let pomodoro = self.pomodoro()?;
        let filled = ((pomodoro.progress.min(1.0) * BAR_WIDTH as f64) as usize).min(BAR_WIDTH);
        let color = if pomodoro.reached() {
            theme::GOOD
        } else {
            theme::ACCENT
        };
        Some(Line::from(vec![
            Span::styled("━".repeat(filled), Style::new().fg(color)),
            Span::styled(
                "─".repeat(BAR_WIDTH - filled),
                Style::new().fg(theme::UNRATED),
            ),
            Span::styled(format!(" {}m", self.settings.target_minutes), theme::DIMMED),
        ]))
    }
}
