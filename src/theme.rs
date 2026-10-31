//! Centralized palette and style helpers for the UI.
//!
//! Only the 16 named colors are used (no truecolor), so the app looks
//! decent on any terminal. All widgets should take their styles from here
//! instead of hardcoding them, so the look can be tweaked in one place.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

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
/// Secondary text: labels, key hints, notes, past data.
pub const MUTED: Color = Color::Gray;
/// Good outcome (focus 3).
pub const GOOD: Color = Color::Green;
/// Neutral outcome (focus 2).
pub const OK: Color = Color::Yellow;
/// Bad outcome (focus 1), errors, interruption counts.
pub const BAD: Color = Color::Red;

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

/// A "label: value" line with a dimmed label and plain value.
pub fn kv(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}  "), DIMMED),
        Span::raw(value.into()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kv_pairs_dimmed_label_with_plain_value() {
        let line = kv("Project:", "alpha".to_string());
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content, "Project:  ");
        assert_eq!(line.spans[0].style, DIMMED);
        assert_eq!(line.spans[1].content, "alpha");
        assert_eq!(line.spans[1].style, Style::new());
    }
}
