//! Centralized palette and style helpers for the UI.
//!
//! Only the 16 named colors are used (no truecolor), so the app looks
//! decent on any terminal. All widgets should take their styles from here
//! instead of hardcoding them, so the look can be tweaked in one place.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::session::{BREAK, DISRUPTION, MEETING};

/// The cat mascot, a blue kitten in the app's accent hue: fur, lighter
/// muzzle and belly, pink inner ears and nose, eyes and their shine.
/// Readable on dark and light backgrounds alike.
pub const CAT_FUR: Color = ACCENT;
pub const CAT_LIGHT: Color = Color::LightCyan;
pub const CAT_PINK: Color = Color::LightRed;
pub const CAT_EYE: Color = Color::Black;
pub const CAT_SHINE: Color = Color::White;
/// The friend it meets with: a pink cat.
pub const CAT_FRIEND: Color = Color::Magenta;
pub const CAT_FRIEND_LIGHT: Color = Color::LightMagenta;
/// Props: laptop body, screen, code on it and keys; notebook paper, ink
/// and spine, pen; game controller and buttons.
pub const CAT_LAPTOP: Color = Color::Gray;
pub const CAT_SCREEN: Color = Color::Black;
pub const CAT_CODE: Color = Color::Green;
pub const CAT_KEYS: Color = Color::DarkGray;
pub const CAT_PAPER: Color = Color::White;
pub const CAT_INK: Color = Color::Gray;
pub const CAT_PEN: Color = Color::Yellow;
pub const CAT_CONTROLLER: Color = Color::DarkGray;
pub const CAT_BUTTON_A: Color = Color::Red;
pub const CAT_BUTTON_B: Color = Color::Green;
/// Symbols drawn as text: `z` and `…`, the flinch's `!`, a note's `✓`,
/// hearts and music notes.
pub const CAT_TEXT: Color = MUTED;
pub const CAT_ALERT: Color = Color::Yellow;
pub const CAT_CHECK: Color = GOOD;
pub const CAT_HEART: Color = CAT_PINK;
pub const CAT_MUSIC: Color = ACCENT;

/// Accent: panel titles, timer, active tab, selected items.
pub const ACCENT: Color = Color::Cyan;
/// Text on an accent background (the rating bar).
pub const ON_ACCENT: Color = Color::Black;
/// Secondary text: labels, key hints, notes, past data.
pub const MUTED: Color = Color::Gray;
/// Good outcome (focus 3).
pub const GOOD: Color = Color::Green;
/// Neutral outcome (focus 2).
pub const OK: Color = Color::Yellow;
/// Bad outcome (focus 1), errors, interruption counts.
pub const BAD: Color = Color::Red;

/// Activities, each in its own color wherever it shows (the active
/// session, today's list, the summary): work in light blue, meetings in the
/// cat's friend's magenta, breaks in a restful green, disruptions in red.
pub const WORK: Color = Color::LightBlue;
pub const MEETING_COLOR: Color = CAT_FRIEND;
pub const BREAK_COLOR: Color = Color::LightGreen;
pub const DISRUPTION_COLOR: Color = Color::LightRed;

/// The color of `activity` (see `WORK`).
pub fn activity_color(activity: &str) -> Color {
    match activity {
        MEETING => MEETING_COLOR,
        BREAK => BREAK_COLOR,
        DISRUPTION => DISRUPTION_COLOR,
        _ => WORK,
    }
}

/// An activity's name, bold in its color.
pub fn activity(activity: &str) -> Span<'static> {
    Span::styled(
        activity.to_string(),
        Style::new()
            .fg(activity_color(activity))
            .add_modifier(Modifier::BOLD),
    )
}

/// A focus rating as a small badge: ` F1 `–` F3 ` in black on its outcome
/// color, or a dimmed ` F- ` when unrated.
pub fn focus_badge(focus: Option<i32>) -> Span<'static> {
    let color = match focus {
        Some(1) => BAD,
        Some(2) => OK,
        Some(3) => GOOD,
        _ => return Span::styled(" F- ", DIMMED),
    };
    Span::styled(
        format!(" F{} ", focus.unwrap_or_default()),
        Style::new()
            .fg(ON_ACCENT)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

/// A value that should stand out next to its dimmed label: bold.
pub const VALUE: Style = Style::new().add_modifier(Modifier::BOLD);
/// A missing value, like "(none)": dimmed italic.
pub const NONE: Style = Style::new().fg(MUTED).add_modifier(Modifier::ITALIC);
/// Times and durations worth spotting: bold accent.
pub const TIME: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);

/// Bold accent — panel titles, active tab, the running timer.
pub const TITLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
/// Dimmed gray — labels, key hints, notes, completed sessions.
pub const DIMMED: Style = Style::new().fg(MUTED);
/// Bold red — the error line.
pub const ERROR: Style = Style::new().fg(BAD).add_modifier(Modifier::BOLD);
/// Bold accent `›` before a text input.
pub const PROMPT: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
/// Dimmed italic hint shown in an empty text input.
pub const PLACEHOLDER: Style = Style::new().fg(MUTED).add_modifier(Modifier::ITALIC);
/// Faded italic caption under the cat.
pub const CAPTION: Style = Style::new().fg(MUTED).add_modifier(Modifier::ITALIC);
/// Highlight symbol for selected items in picker `List`s: a bold accent `>`.
pub fn picker_symbol() -> Span<'static> {
    Span::styled(">", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
}
pub const PICKER_HIGHLIGHT: Style = Style::new().fg(ACCENT);

/// A "label: value" line with a dimmed label and a styled value.
pub fn kv(label: &str, value: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("{label}  "), DIMMED)];
    spans.extend(value);
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kv_pairs_dimmed_label_with_its_value() {
        let line = kv("Project:", vec![Span::raw("alpha")]);
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content, "Project:  ");
        assert_eq!(line.spans[0].style, DIMMED);
        assert_eq!(line.spans[1].content, "alpha");
        assert_eq!(line.spans[1].style, Style::new());
    }
}
