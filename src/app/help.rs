use super::modal::modal_layout;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

/// The help screen: every key, in groups with an accent heading and a rule
/// under it. Two columns (sessions; sections and mouse) keep it short
/// enough for a 24-row terminal with room for spacing. A centered modal
/// sized to its content, shrunk on small terminals.
pub(super) fn draw_help(frame: &mut Frame, content: Rect) {
    type Group = (&'static str, &'static [(&'static str, &'static str)]);
    const LEFT: &[Group] = &[(
        "Sessions",
        &[
            ("/", "command palette"),
            ("s", "start a session"),
            ("e", "end the session"),
            ("n", "add a note"),
            ("i", "count an interruption"),
            ("d", "start a disruption"),
            ("b", "start a break"),
            ("m", "start a meeting"),
            ("r", "resume after d, b or m"),
            ("1 / 2 / 3", "rate focus, just after"),
            ("u", "undo last start or end"),
            ("[ / ]", "move start by 5 min"),
            ("q / Ctrl+C", "quit"),
        ],
    )];
    const RIGHT: &[Group] = &[
        (
            "Sections",
            &[
                ("Tab / Shift+Tab", "focus next / prev"),
                ("Space", "collapse or expand"),
                ("+ / -", "grow or shrink"),
                ("=", "reset sizes"),
                ("↑↓ j k PgUp PgDn", "scroll notes / today"),
                ("Home / End", "top / bottom"),
            ],
        ),
        (
            "Mouse",
            &[
                ("drag a border", "resize"),
                ("click a title", "collapse or expand"),
                ("wheel", "scroll under pointer"),
            ],
        ),
    ];

    fn key_width(groups: &[Group]) -> usize {
        let keys = groups.iter().flat_map(|(_, keys)| keys.iter());
        keys.map(|(key, _)| key.chars().count()).max().unwrap_or(0) + 2
    }

    /// Columns needed to show every key and description of `groups`.
    fn column_width(groups: &[Group]) -> u16 {
        let keys = groups.iter().flat_map(|(_, keys)| keys.iter());
        let desc = keys.map(|(_, d)| d.chars().count()).max().unwrap_or(0);
        (key_width(groups) + desc) as u16
    }

    /// One column: each group is a heading with a rule to the column's
    /// width, its keys, and a blank row before the next group.
    fn column(groups: &[Group], width: u16) -> Vec<Line<'static>> {
        let key_width = key_width(groups);
        let mut lines = Vec::new();
        for (heading, keys) in groups {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            let rule = "─".repeat((width as usize).saturating_sub(heading.chars().count() + 1));
            lines.push(Line::from(vec![
                Span::styled(format!("{heading} "), theme::TITLE),
                Span::styled(rule, theme::DIMMED),
            ]));
            for (key, desc) in *keys {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{key:<key_width$}"),
                        Style::new().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(desc.to_string(), theme::DIMMED),
                ]));
            }
        }
        lines
    }

    let rows = |groups: &[Group]| -> u16 {
        groups
            .iter()
            .map(|(_, keys)| keys.len() as u16 + 2)
            .sum::<u16>()
            - 1
    };
    let height = rows(LEFT).max(rows(RIGHT)) + 2;
    let (left_width, right_width) = (column_width(LEFT), column_width(RIGHT));
    // Border and padding (2 each side) plus a 3-column gutter.
    let width = left_width + 3 + right_width + 4;
    let (modal, inner) = modal_layout(content, width, height);
    frame.render_widget(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(Span::styled(" Keys ", theme::TITLE))),
        modal,
    );
    let [left, _, right] = Layout::horizontal([
        Constraint::Length(left_width),
        Constraint::Length(3),
        Constraint::Min(0),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(column(LEFT, left.width)), left);
    frame.render_widget(Paragraph::new(column(RIGHT, right.width)), right);
}
