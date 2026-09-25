//! Centralized palette and style helpers for the UI.
//!
//! Only the 16 named colors are used (no truecolor), so the app looks
//! decent on any terminal. All widgets should take their styles from here
//! instead of hardcoding them, so the look can be tweaked in one place.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

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
