//! The summary's styled items (activities, projects, focus, time split)
//! and how they wrap beside their labels. Shared by the main screen's
//! summary and the longer views.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::summary::DaySummary;
use crate::theme;

use super::graphs::focus_color;
use super::text::{fmt_hms, plural, wrap_text};

/// `label` beside the first row (as `theme::kv` draws it), the other rows
/// indented to line up under the first row's value.
pub(super) fn hanging_lines(label: &str, rows: Vec<Vec<Span<'static>>>) -> Vec<Line<'static>> {
    let indent = " ".repeat(label.chars().count() + 2);
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| match i {
            0 => theme::kv(label, row),
            _ => Line::from([vec![Span::raw(indent.clone())], row].concat()),
        })
        .collect()
}

/// A styled summary item: its spans.
pub(super) type Item = Vec<Span<'static>>;

/// Packs `items` into rows joined by a dimmed " · ", for display beside
/// `label` within `width` columns. An item only wraps (mid-item, via
/// `wrap_text`, losing its styling) when it is wider than a whole row.
pub(super) fn wrap_items(items: Vec<Item>, width: u16, label: &str) -> Vec<Item> {
    let width = (width as usize)
        .saturating_sub(label.chars().count() + 2)
        .max(1);
    let row_len = |row: &Item| row.iter().map(Span::width).sum::<usize>();
    let mut rows = Vec::new();
    let mut row: Item = Vec::new();
    for item in items {
        let len = row_len(&item);
        if !row.is_empty() && row_len(&row) + 3 + len <= width {
            row.push(Span::styled(" · ", theme::DIMMED));
            row.extend(item);
            continue;
        }
        if !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
        if len <= width {
            row = item;
            continue;
        }
        let text: String = item.iter().map(|span| span.content.as_ref()).collect();
        let mut pieces = wrap_text(&text, width);
        if pieces.len() > 1 && pieces.last().is_some_and(String::is_empty) {
            pieces.pop(); // the cursor row wrap_text adds after a full row
        }
        row = vec![Span::raw(pieces.pop().unwrap_or_default())];
        rows.extend(pieces.into_iter().map(|piece| vec![Span::raw(piece)]));
    }
    rows.push(row);
    rows
}

/// Columns the summary's labels take ("Activity" and its gap), before the
/// values and graphs.
pub(super) const SUMMARY_LABEL_WIDTH: u16 = 10;

/// The columns a day's total and focus take at the end of its row in the
/// longer views (" 10h 30m 2.5").
pub(super) const DAY_TOTAL_WIDTH: usize = 13;

/// How a view colors projects (by name; `None`: no project).
pub(super) type ProjectColors<'a> = &'a dyn Fn(Option<&str>) -> Color;

/// A project's name, bold in `color`, or a dimmed italic "(none)".
pub(super) fn project_span(project: Option<&str>, color: Color) -> Span<'static> {
    match project {
        Some(name) => Span::styled(name.to_string(), theme::VALUE.fg(color)),
        None => Span::styled("(none)", theme::NONE),
    }
}

/// Each activity with its time, the name in its color, longest first.
pub(super) fn activity_items(summary: &DaySummary) -> Vec<Item> {
    summary
        .per_activity
        .iter()
        .map(|(activity, d)| {
            vec![
                theme::activity(activity),
                Span::raw(" "),
                Span::styled(fmt_hms(*d), theme::VALUE),
            ]
        })
        .collect()
}

/// Each project with its time, the name in its color, longest first.
pub(super) fn project_items(summary: &DaySummary, colors: ProjectColors) -> Vec<Item> {
    summary
        .per_project
        .iter()
        .map(|(project, d)| {
            let project = project.as_deref();
            vec![
                project_span(project, colors(project)),
                Span::raw(" "),
                Span::styled(fmt_hms(*d), theme::VALUE),
            ]
        })
        .collect()
}

/// The focus average (in its outcome color), interruptions and the
/// disruption count; nothing to report dimmed.
pub(super) fn focus_items(summary: &DaySummary) -> Vec<Item> {
    let dimmed = |text: &str| vec![Span::styled(text.to_string(), theme::DIMMED)];
    vec![
        match summary.average_focus {
            Some(avg) => vec![
                Span::raw("avg "),
                Span::styled(format!("{avg:.1}"), theme::VALUE.fg(focus_color(avg))),
                Span::styled(format!(" ({} rated)", summary.rated), theme::DIMMED),
            ],
            None => vec![Span::styled("not rated", theme::NONE)],
        },
        match summary.interruptions {
            0 => dimmed("0 interruptions"),
            n => vec![
                interruptions_span(n, ""),
                Span::raw(if n == 1 {
                    " interruption"
                } else {
                    " interruptions"
                }),
            ],
        },
        match summary.disruptions {
            0 => dimmed("no disruptions"),
            // Their time is with the tracked time's split.
            n => vec![Span::styled(
                plural(n, "disruption"),
                theme::VALUE.fg(theme::DISRUPTION_COLOR),
            )],
        },
    ]
}

/// A day's (or month's) tracked time and focus average, at the end of its
/// row in the longer views (`DAY_TOTAL_WIDTH` columns).
pub(super) fn day_total(summary: &DaySummary) -> Item {
    vec![
        Span::styled(format!(" {:>8}", fmt_hms(summary.tracked)), theme::VALUE),
        match summary.average_focus {
            Some(avg) => Span::styled(format!(" {avg:.1}"), theme::VALUE.fg(focus_color(avg))),
            None => Span::styled("   -", theme::DIMMED),
        },
    ]
}

/// Tracked time, then what it was made of (work, breaks, disruptions),
/// each part's time in the color the graphs give it; parts that didn't
/// happen are left out, keeping the row short.
pub(super) fn time_split_items(summary: &DaySummary) -> Vec<Item> {
    let mut items = vec![
        vec![
            Span::raw("tracked "),
            Span::styled(fmt_hms(summary.tracked), theme::VALUE),
        ],
        vec![
            Span::raw("work "),
            colored_time(summary.work_time(), theme::WORK),
        ],
    ];
    for (word, duration, color) in [
        ("breaks ", summary.break_time, theme::BREAK_COLOR),
        (
            "disrupted ",
            summary.disruption_time,
            theme::DISRUPTION_COLOR,
        ),
    ] {
        if !duration.is_zero() {
            items.push(vec![Span::raw(word), colored_time(duration, color)]);
        }
    }
    items
}

/// A duration in bold `color`.
fn colored_time(duration: chrono::Duration, color: Color) -> Span<'static> {
    Span::styled(fmt_hms(duration), theme::VALUE.fg(color))
}

/// A value, or a dimmed italic "(none)" when it is missing.
pub(super) fn value_or_none(value: &Option<String>, style: Style) -> Span<'static> {
    match value {
        Some(value) => Span::styled(value.clone(), style),
        None => Span::styled("(none)", theme::NONE),
    }
}

/// An interruption count after `prefix`: bold red when there were any,
/// dimmed otherwise.
pub(super) fn interruptions_span(count: i32, prefix: &str) -> Span<'static> {
    let style = if count > 0 {
        theme::VALUE.fg(theme::BAD)
    } else {
        theme::DIMMED
    };
    Span::styled(format!("{prefix}{count}"), style)
}
