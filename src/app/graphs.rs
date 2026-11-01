//! The summary's graphs: the timeline and focus line, the share bars and
//! the focus grids, drawn with half-cell blocks.

use chrono::Utc;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::summary::DaySummary;
use crate::theme;

use super::items::{Item, ProjectColors};

/// Narrower than this, the summary's graphs are left out.
pub(super) const MIN_GRAPH_WIDTH: usize = 10;

/// Rows the graphs add to the summary: the focus line, the timeline, its
/// hours, the two share bars, and a blank row between each of the four
/// groups.
pub(super) const GRAPH_ROWS: u16 = 5 + 3;

/// A row of `halves.len() / 2` cells, two colors per cell: `▌` takes the
/// left half's color as foreground and the right one's as background, like
/// the cat's `▀` does for top and bottom. `None` leaves a half blank.
pub(super) fn half_cells(halves: &[Option<Color>]) -> Item {
    halves
        .chunks(2)
        .map(|pair| {
            let (left, right) = (pair[0], pair.get(1).copied().flatten());
            match (left, right) {
                (None, None) => Span::raw(" "),
                (Some(l), None) => Span::styled("▌", Style::new().fg(l)),
                (None, Some(r)) => Span::styled("▐", Style::new().fg(r)),
                (Some(l), Some(r)) if l == r => Span::styled("█", Style::new().fg(l)),
                (Some(l), Some(r)) => Span::styled("▌", Style::new().fg(l).bg(r)),
            }
        })
        .collect()
}

/// The day from `start` to `end` across `width` cells, each half cell in
/// the color of what was going on at its middle, gaps blank.
pub(super) fn timeline_row(
    summary: &DaySummary,
    start: chrono::DateTime<Utc>,
    end: chrono::DateTime<Utc>,
    width: usize,
) -> Item {
    let halves = width * 2;
    let span = (end - start).num_milliseconds().max(1) as f64;
    let colors = (0..halves)
        .map(|i| {
            let offset = span * (i as f64 + 0.5) / halves as f64;
            let at = start + chrono::Duration::milliseconds(offset as i64);
            summary
                .block_at(at)
                .map(|block| theme::activity_color(&block.activity))
        })
        .collect::<Vec<_>>();
    half_cells(&colors)
}

/// A thick line over the timeline (`▀`, the top half of its row, so it
/// lifts off the bar by the bottom half) in the focus rating of what ran
/// at each cell's middle: red, yellow, green, or dimmed for a session that
/// could be rated but wasn't. Blank over breaks, disruptions and gaps,
/// which are never rated.
pub(super) fn focus_row(
    summary: &DaySummary,
    start: chrono::DateTime<Utc>,
    end: chrono::DateTime<Utc>,
    width: usize,
) -> Item {
    let span = (end - start).num_milliseconds().max(1) as f64;
    (0..width)
        .map(|i| {
            let offset = span * (i as f64 + 0.5) / width as f64;
            let at = start + chrono::Duration::milliseconds(offset as i64);
            let color = match summary.block_at(at) {
                Some(block) if block.rateable() => match block.focus {
                    Some(1) => theme::BAD,
                    Some(2) => theme::OK,
                    Some(3) => theme::GOOD,
                    _ => theme::UNRATED,
                },
                _ => return Span::raw(" "),
            };
            Span::styled("▀", Style::new().fg(color))
        })
        .collect()
}

/// The hours (local time) under the timeline, each at the cell where it
/// starts, dropped where they would run into the one before.
pub(super) fn hour_labels(
    start: chrono::DateTime<Utc>,
    end: chrono::DateTime<Utc>,
    width: usize,
) -> Item {
    use chrono::TimeZone;
    let span = (end - start).num_milliseconds().max(1) as f64;
    let mut hour = start_of_hour(start).with_timezone(&chrono::Local);
    if hour.with_timezone(&Utc) < start {
        hour += chrono::Duration::hours(1);
    }
    let mut row = vec![' '; width];
    let mut free_from = 0;
    while hour.with_timezone(&Utc) <= end {
        let offset = (hour.with_timezone(&Utc) - start).num_milliseconds() as f64;
        let x = (offset / span * width as f64) as usize;
        let label = hour.format("%H").to_string();
        if x >= free_from && x + label.len() <= width {
            for (i, c) in label.chars().enumerate() {
                row[x + i] = c;
            }
            free_from = x + label.len() + 1;
        }
        hour = chrono::Local
            .from_local_datetime(&(hour.naive_local() + chrono::Duration::hours(1)))
            .earliest()
            .unwrap_or(hour + chrono::Duration::hours(1));
    }
    vec![Span::styled(
        row.into_iter().collect::<String>(),
        theme::DIMMED,
    )]
}

/// The start of the (local) hour `at` falls in.
pub(super) fn start_of_hour(at: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    use chrono::Timelike;
    let local = at.with_timezone(&chrono::Local);
    local
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .map_or(at, |t| t.with_timezone(&Utc))
}

/// Shares of the tracked time (an activity's or a project's, as its
/// color and time) as one bar `width` cells wide, in the order given.
/// Shares are rounded to half cells, so a sliver of a few minutes may not
/// show.
pub(super) fn share_row(shares: &[(Color, chrono::Duration)], width: usize) -> Item {
    let halves = width * 2;
    let total = shares
        .iter()
        .map(|(_, d)| d.num_milliseconds())
        .sum::<i64>()
        .max(1) as f64;
    let mut colors = Vec::with_capacity(halves);
    let mut cumulative = 0;
    for &(color, d) in shares {
        cumulative += d.num_milliseconds();
        let until = (cumulative as f64 / total * halves as f64).round() as usize;
        colors.resize(until.max(colors.len()), Some(color));
    }
    colors.resize(halves, None);
    half_cells(&colors)
}

/// What a day's square in a grid shows: a value on the 1–3 ramp (the
/// day's focus average, or its sleep or mood), something that could have
/// been rated but wasn't, or nothing at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DayCell {
    Rated(f64),
    Unrated,
    Empty,
}

/// A day's focus cell: its average, unrated when it was tracked without a
/// rating, empty when nothing was tracked.
pub(super) fn focus_cell(summary: Option<&DaySummary>) -> DayCell {
    match summary {
        Some(s) if s.average_focus.is_some() => DayCell::Rated(s.average_focus.unwrap_or(2.0)),
        Some(s) if !s.tracked.is_zero() => DayCell::Unrated,
        _ => DayCell::Empty,
    }
}

/// The period's days as a grid of squares, GitHub style: a row per
/// weekday (labelled when `weekdays`), a column per week (headed by its
/// Monday's date), each day as `cell` says: colored on
/// `theme::FOCUS_RAMP` when rated, dark gray when unrated, a faint texture
/// when empty, blank outside the period and after `today`. Its legend is
/// `focus_legend`.
pub(super) fn focus_grid(
    first: chrono::NaiveDate,
    last: chrono::NaiveDate,
    today: chrono::NaiveDate,
    cell: &dyn Fn(chrono::NaiveDate) -> DayCell,
    weekdays: bool,
) -> Vec<Item> {
    use chrono::Datelike;
    let weekday_width = if weekdays { 4 } else { 0 }; // "Mon "
    let monday = first - chrono::Duration::days(first.weekday().num_days_from_monday() as i64);
    let weeks = ((last - monday).num_days() / 7 + 1) as usize;
    let week = |w: usize| monday + chrono::Duration::weeks(w as i64);

    let mut header = vec![Span::raw(" ".repeat(weekday_width))];
    // Spaced like the squares below: two columns, a space between.
    let dates = (0..weeks)
        .map(|w| format!("{:<2}", week(w).day()))
        .collect::<Vec<_>>()
        .join(" ");
    header.push(Span::styled(dates, theme::DIMMED));
    let mut rows = vec![header];
    for weekday in 0..7 {
        let name = (monday + chrono::Duration::days(weekday))
            .format("%a ")
            .to_string();
        let today_row = today.weekday().num_days_from_monday() as i64 == weekday
            && (first..=last).contains(&today);
        let style = if today_row {
            theme::TITLE
        } else {
            theme::DIMMED
        };
        let mut row = match weekdays {
            true => vec![Span::styled(name, style)],
            false => Vec::new(),
        };
        for w in 0..weeks {
            let day = week(w) + chrono::Duration::days(weekday);
            let square = match cell(day) {
                _ if day < first || day > last || day > today => Span::raw("  "),
                DayCell::Rated(value) => {
                    Span::styled("██", Style::new().fg(theme::focus_gradient(value)))
                }
                DayCell::Unrated => Span::styled("██", Style::new().fg(theme::UNRATED)),
                DayCell::Empty => Span::styled("░░", Style::new().fg(theme::UNRATED)),
            };
            row.push(square);
            if w + 1 < weeks {
                row.push(Span::raw(" "));
            }
        }
        rows.push(row);
    }
    rows
}

/// A year's focus grid, compact enough for 80 columns: a column per week
/// of each month, the weeks split where a month ends (so each month is a
/// block of its own and the week it shares with the next shows its days
/// in both, blank where the other month's are; a blank column after a
/// month that ends on a Sunday), the month names centered over their
/// blocks, and the weekdays two to a row, labelled with both ("Mo Tu"),
/// each day half a row tall (`▀`: the top day in front, the bottom one
/// behind), so the squares stay square without gaps. Colors as in
/// `focus_grid`, with `theme::EMPTY_DAY` for an empty day. `title` goes
/// in the header row, over the weekdays.
pub(super) fn year_focus_grid(
    first: chrono::NaiveDate,
    last: chrono::NaiveDate,
    today: chrono::NaiveDate,
    cell: &dyn Fn(chrono::NaiveDate) -> DayCell,
    title: &str,
) -> Vec<Item> {
    use chrono::Datelike;
    const WEEKDAY: usize = 6; // "Mo Tu "

    // The columns: each month's weeks, Monday to Sunday, holding only that
    // month's days (`None` for the other month's and outside the period).
    let mut columns: Vec<[Option<chrono::NaiveDate>; 7]> = Vec::new();
    // Each month's name, the column its block starts at and its width.
    let mut labels = Vec::new();
    for month_start in first
        .iter_days()
        .take_while(|d| *d <= last)
        .filter(|d| d.day() == 1 || *d == first)
    {
        let in_month = |day: chrono::NaiveDate| {
            day.month() == month_start.month() && day.year() == month_start.year()
        };
        let start = columns.len();
        let mut monday = month_start
            - chrono::Duration::days(month_start.weekday().num_days_from_monday() as i64);
        while in_month(monday) || in_month(monday + chrono::Duration::days(6)) {
            columns.push(std::array::from_fn(|weekday| {
                let day = monday + chrono::Duration::days(weekday as i64);
                (in_month(day) && day >= first && day <= last).then_some(day)
            }));
            monday += chrono::Duration::weeks(1);
        }
        labels.push((
            month_start.format("%b").to_string(),
            start,
            columns.len() - start,
        ));
        // A month ending on a Sunday has no split week to set it apart from
        // the next one, so a blank column does.
        if columns.last().is_some_and(|week| week[6].is_some()) {
            columns.push([None; 7]);
        }
    }
    if columns
        .last()
        .is_some_and(|week| week.iter().all(Option::is_none))
    {
        columns.pop();
    }
    // Each name centered over its block (blocks are at least 4 wide).
    let mut header = vec![' '; WEEKDAY + columns.len()];
    for (at, c) in header.iter_mut().zip(title.chars().take(WEEKDAY - 1)) {
        *at = c;
    }
    for (name, start, width) in labels {
        let at = WEEKDAY + start + width.saturating_sub(name.len()) / 2;
        for (cell, c) in header.iter_mut().skip(at).zip(name.chars()) {
            *cell = c;
        }
    }
    let header = header.into_iter().collect::<String>();
    let mut rows = vec![vec![Span::styled(header, theme::DIMMED)]];

    let color = |day: Option<chrono::NaiveDate>| -> Option<Color> {
        let day = day.filter(|day| *day <= today)?;
        Some(match cell(day) {
            DayCell::Rated(value) => theme::focus_gradient(value),
            DayCell::Unrated => theme::UNRATED,
            DayCell::Empty => theme::EMPTY_DAY,
        })
    };
    for top in [0, 2, 4, 6] {
        // Both days the row holds, top then bottom: "Mo Tu", …, "Su".
        let short = |weekday: usize| {
            chrono::Weekday::try_from(weekday as u8)
                .map(|day| day.to_string()[..2].to_string())
                .unwrap_or_default()
        };
        let name = match top {
            6 => short(top),
            _ => format!("{} {}", short(top), short(top + 1)),
        };
        let mut row = vec![Span::styled(format!("{name:WEEKDAY$}"), theme::DIMMED)];
        for column in &columns {
            let upper = color(column[top]);
            let lower = if top < 6 {
                color(column[top + 1])
            } else {
                None
            };
            row.push(match (upper, lower) {
                (None, None) => Span::raw(" "),
                (Some(u), None) => Span::styled("▀", Style::new().fg(u)),
                (None, Some(l)) => Span::styled("▄", Style::new().fg(l)),
                (Some(u), Some(l)) if u == l => Span::styled("█", Style::new().fg(u)),
                (Some(u), Some(l)) => Span::styled("▀", Style::new().fg(u).bg(l)),
            });
        }
        rows.push(row);
    }
    rows
}

/// The focus grid's legend: 1, the ramp, 3.
pub(super) fn focus_legend() -> Item {
    let mut legend = vec![Span::styled("1 ", theme::DIMMED)];
    legend.extend(
        theme::FOCUS_RAMP
            .iter()
            .map(|&color| Span::styled("█", Style::new().fg(color))),
    );
    legend.push(Span::styled(" 3", theme::DIMMED));
    legend
}

/// A row's width in cells.
pub(super) fn item_width(row: &Item) -> usize {
    row.iter().map(Span::width).sum()
}

/// `row`, part of a block `block_width` wide, centered on `width` columns:
/// every row of the block gets the same left padding, so they stay aligned.
pub(super) fn centered(row: Item, block_width: usize, width: u16) -> Line<'static> {
    let pad = (width as usize).saturating_sub(block_width) / 2;
    Line::from([vec![Span::raw(" ".repeat(pad))], row].concat())
}

/// The color of a focus average: its place on the focus grid's ramp, so
/// a number and its day's square always match.
pub(super) fn focus_color(average: f64) -> Color {
    theme::focus_gradient(average)
}

/// Each activity's share of the tracked time as one bar.
pub(super) fn activity_bar(summary: &DaySummary, width: usize) -> Item {
    let shares = summary
        .per_activity
        .iter()
        .map(|(activity, d)| (theme::activity_color(activity), *d))
        .collect::<Vec<_>>();
    share_row(&shares, width)
}

/// Each project's share of the tracked time as one bar.
pub(super) fn project_bar(summary: &DaySummary, width: usize, colors: ProjectColors) -> Item {
    let shares = summary
        .per_project
        .iter()
        .map(|(project, d)| (colors(project.as_deref()), *d))
        .collect::<Vec<_>>();
    share_row(&shares, width)
}
