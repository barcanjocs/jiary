//! The frame around every screen: the header row, the status row with its
//! key hints (or the last error), and where the terminal cursor goes.

use chrono::Utc;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::layout::Section;
use crate::theme;

use super::forms::Step;
use super::modal::{
    SMALL_MODAL_HEIGHT, START_MODAL_HEIGHT, WrappedInput, modal_inner_width, small_modal_layout,
    start_modal_layout,
};
use super::mood::MoodStep;
use super::rename::RenameStep;
use super::text::fmt_hms;
use super::{App, NOTICE_TIME, Screen};

/// The status-line hint for the help screen; the main screen always shows it.
const HELP_HINT: (&str, &str) = ("?", "help");

/// The global chrome: header row, content area, status row. Shared by `draw`
/// and `cursor_position`.
pub(super) fn chrome_layout(area: ratatui::layout::Rect) -> [ratatui::layout::Rect; 3] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area)
}

impl App {
    /// Terminal cursor position for the current screen: `Some` on input lines
    /// (start form's project/task steps, add-note),
    /// right after the typed text. Pure function of the terminal size and form
    /// state — `Terminal::draw` discards closure return values, so main.rs
    /// calls this after drawing to show or hide the cursor.
    pub fn cursor_position(&self, size: (u16, u16)) -> Option<(u16, u16)> {
        let [_, content, _] = chrome_layout(ratatui::layout::Rect::new(0, 0, size.0, size.1));
        if let Screen::Rewind(view) = &self.screen {
            let (input, area) = self.rewind_input(content, view);
            return Some(input.cursor_in(area));
        }
        let input = self.wrapped_input(content)?;
        let input_area = match &self.screen {
            Screen::StartSession(_) | Screen::Palette(_) | Screen::Rename(_) => {
                start_modal_layout(content, input.height()).2
            }
            _ => small_modal_layout(content, input.height()).2,
        };
        Some(input.cursor_in(input_area))
    }

    /// The current screen's text input, if it has one: (value, placeholder
    /// shown while it is empty). Inputs have no label: the tabs row (or the
    /// note modal's context line) already says what is being typed.
    fn text_input(&self) -> Option<(&str, &'static str)> {
        match &self.screen {
            Screen::StartSession(form) => match form.step {
                Step::Activity => Some((&form.activity.input, "type or pick an activity")),
                Step::Project => Some((&form.project.input, "type or pick a project (optional)")),
                Step::Task => Some((&form.task.input, "type or pick a task (optional)")),
            },
            Screen::AddNote(form) => Some((&form.input, "type a note")),
            Screen::Palette(form) => Some((&form.input, "")),
            Screen::Rename(form) => form
                .field()
                .map(|(field, placeholder)| (field.input.as_str(), placeholder)),
            _ => None,
        }
    }

    /// The current screen's text input wrapped to its modal's width, with at
    /// most as many rows as the modal can grow to on `content`. Shared by
    /// `draw` and `cursor_position` so the two always agree.
    pub(super) fn wrapped_input(&self, content: ratatui::layout::Rect) -> Option<WrappedInput> {
        if let Screen::Rewind(view) = &self.screen {
            return Some(self.rewind_input(content, view).0);
        }
        let (value, placeholder) = self.text_input()?;
        let base_height = match self.screen {
            Screen::StartSession(_) | Screen::Palette(_) | Screen::Rename(_) => START_MODAL_HEIGHT,
            _ => SMALL_MODAL_HEIGHT,
        };
        let max_rows = content.height.saturating_sub(base_height - 1).max(1);
        Some(WrappedInput::new(
            value,
            placeholder,
            modal_inner_width(content),
            max_rows,
        ))
    }

    /// Top row: name, date, and total time logged today. The total is computed
    /// from in-memory sessions only — draw runs ~10x/second and must never
    /// query the db.
    pub(super) fn header_line(&self) -> Line<'static> {
        let date = chrono::Local::now().format("%a %d %b %Y");
        Line::from(vec![
            Span::styled("JIARY", theme::TITLE),
            Span::styled(format!("  {date}"), theme::DIMMED),
            Span::styled("  ·  ", theme::DIMMED),
            Span::styled(fmt_hms(self.total_today()), theme::TIME),
            Span::styled(" today", theme::DIMMED),
        ])
    }

    /// Total time logged today: completed sessions plus the active session's
    /// elapsed time (the active one also appears in `today_sessions` without
    /// an end, so only completed ones are summed there).
    pub(super) fn total_today(&self) -> chrono::Duration {
        let mut total = chrono::Duration::zero();
        for session in &self.today_sessions {
            if let Some(end) = session.ended_at {
                total += end - session.started_at;
            }
        }
        if let Some(active) = &self.active_session {
            total += Utc::now() - active.started_at;
        }
        total
    }

    /// The notice still showing in the status line, on the main screen.
    fn current_notice(&self) -> Option<&str> {
        let (text, given) = self.notice.as_ref()?;
        (matches!(self.screen, Screen::Main) && given.elapsed() < NOTICE_TIME)
            .then_some(text.as_str())
    }

    /// Bottom row: the red error when the last db operation failed, otherwise
    /// key hints for the current screen, as many as fit in `width`. The only
    /// place errors and key hints are rendered.
    pub(super) fn status_line(&self, width: u16) -> Line<'static> {
        if let Some(err) = &self.error {
            return Line::from(Span::styled(format!("✗ {err}"), theme::ERROR));
        }
        if let Some(notice) = self.current_notice() {
            return Line::from(Span::styled(notice.to_string(), theme::PROMPT));
        }
        let hints: &[(&str, &str)] = match &self.screen {
            // Most useful first for the focused section. `[?] help` lists
            // everything, so it is always shown (see `HELP_HINT`).
            Screen::Main => match self.focus {
                Section::Active => &[
                    ("s", "start"),
                    ("e", "end"),
                    ("n", "note"),
                    ("i", "interrupt"),
                    // One hint for the three detour keys keeps [r] on an
                    // 80-column line; `?` help lists them separately.
                    ("d/b/m", "away"),
                    ("r", "resume"),
                    HELP_HINT,
                    ("Tab", "sections"),
                    ("q", "quit"),
                ],
                Section::Today => &[
                    ("↑↓", "scroll"),
                    ("Space", "collapse"),
                    ("+/-", "resize"),
                    ("s", "start"),
                    ("e", "end"),
                    HELP_HINT,
                    ("Tab", "next"),
                    ("q", "quit"),
                ],
                Section::Summary => &[
                    ("Space", "collapse"),
                    ("+/-", "resize"),
                    ("s", "start"),
                    ("e", "end"),
                    HELP_HINT,
                    ("Tab", "next"),
                    ("q", "quit"),
                ],
            },
            Screen::Help => &[("any key", "close")],
            Screen::StartSession(_) => &[("Tab", "accept"), ("Enter", "next"), ("Esc", "cancel")],
            Screen::Period(_) => &[
                ("←→", "previous/next"),
                ("w/m/y", "week/month/year"),
                ("↑↓", "scroll"),
                ("Esc", "back"),
            ],
            Screen::Palette(_) => &[
                ("↑↓", "select"),
                ("Tab", "complete"),
                ("Enter", "run"),
                ("Esc", "close"),
            ],
            Screen::AddNote(_) => &[("Enter", "save"), ("Esc", "cancel")],
            Screen::Rename(form) => match form.step {
                RenameStep::What => &[("↑↓", "select"), ("Enter", "next"), ("Esc", "cancel")],
                RenameStep::Confirm => &[("Enter", "apply"), ("Esc", "cancel")],
                _ => &[("Tab", "accept"), ("Enter", "next"), ("Esc", "cancel")],
            },
            Screen::Mood(form) => match (form.step, form.then_start) {
                (MoodStep::Sleep, true) => {
                    &[("1/2/3", "answer"), ("Enter", "keep"), ("Esc", "skip")]
                }
                (MoodStep::Sleep, false) => {
                    &[("1/2/3", "answer"), ("Enter", "keep"), ("Esc", "cancel")]
                }
                (MoodStep::Mood, true) => &[
                    ("1/2/3", "answer"),
                    ("Enter", "keep"),
                    ("Bksp", "back"),
                    ("Esc", "skip"),
                ],
                (MoodStep::Mood, false) => &[
                    ("1/2/3", "answer"),
                    ("Enter", "keep"),
                    ("Bksp", "back"),
                    ("Esc", "cancel"),
                ],
            },
            Screen::Settings(_) => &[("↑↓", "select"), ("Space/←→", "change"), ("Esc", "close")],
            Screen::Rewind(_) => &[
                ("type", "search"),
                ("↑↓", "pick"),
                ("Enter", "open day"),
                ("Esc", "clear/back"),
            ],
            Screen::PastDay(_) => &[("←→", "older/newer"), ("↑↓", "scroll"), ("Esc", "back")],
            Screen::Away(_) if !self.away_offers_detours() => &[("k", "keep"), ("e", "end")],
            Screen::Away(_) => &[
                ("k", "keep"),
                ("b", "break"),
                ("d", "disruption"),
                ("e", "end"),
            ],
        };
        // Hints are shown in order while they fit, keeping room for
        // `HELP_HINT` (always shown, wherever it sits in the list). Once a
        // hint doesn't fit, the remaining optional ones are dropped too, so
        // the most useful ones win.
        let hint_width =
            |(key, desc): &(&str, &str)| key.chars().count() + desc.chars().count() + 3;
        let help_left = |from: usize| -> usize {
            hints[from..]
                .iter()
                .filter(|&&h| h == HELP_HINT)
                .map(|h| hint_width(h) + 2)
                .sum()
        };
        let mut used = 0;
        let mut full = false;
        let mut shown = Vec::new();
        for (i, hint) in hints.iter().enumerate() {
            let needed = hint_width(hint) + if shown.is_empty() { 0 } else { 2 };
            let required = *hint == HELP_HINT;
            if !required && (full || used + needed + help_left(i + 1) > width as usize) {
                full = true;
                continue;
            }
            used += needed;
            shown.push(*hint);
        }

        let mut spans = Vec::new();
        for (key, desc) in shown {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                format!("[{key}]"),
                Style::new().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(format!(" {desc}"), theme::DIMMED));
        }
        Line::from(spans)
    }
}
