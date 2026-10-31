use super::graphs::{
    activity_bar, centered, focus_grid, focus_legend, hour_labels, item_width, project_bar,
    timeline_row, year_focus_grid,
};
use super::items::{
    DAY_TOTAL_WIDTH, SUMMARY_LABEL_WIDTH, activity_items, day_total, focus_items, hanging_lines,
    project_items, time_split_items, wrap_items,
};
use super::sections::draw_scrollbar;
use super::text::{fmt_hms, plural};
use super::{App, Screen, chrome_layout};
use crate::period::Period;
use crate::session::Session;
use crate::summary::summarize;
use crate::theme;
use chrono::Utc;
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

/// A week or a month on screen: which one (`offset` periods from the
/// current one), its days, its sessions as loaded when it opened (draw
/// never queries the db) and how far it is scrolled.
pub(super) struct PeriodView {
    pub(super) period: Period,
    pub(super) offset: i32,
    pub(super) first: chrono::NaiveDate,
    pub(super) last: chrono::NaiveDate,
    pub(super) sessions: Vec<Session>,
    pub(super) scroll: usize,
}

impl App {
    /// Opens the week or month `offset` periods from the current one,
    /// loading its sessions.
    pub(super) fn open_period(&mut self, period: Period, offset: i32) {
        let today = chrono::Local::now().date_naive();
        let (first, last) = period.days(offset, today);
        let sessions = match self.db.sessions_between(first, last) {
            Ok(sessions) => {
                self.clear_error();
                sessions
            }
            Err(e) => {
                self.set_error(e);
                Vec::new()
            }
        };
        self.screen = Screen::Period(PeriodView {
            period,
            offset,
            first,
            last,
            sessions,
            scroll: 0,
        });
    }

    /// Keys in a week, month or year: ←→ (h l) page to the period before
    /// or after (not past the current one), w, m and y switch to this week,
    /// month or year, the scroll keys scroll, Esc or q go back.
    pub(super) fn handle_period(&mut self, key: KeyCode) {
        let Screen::Period(view) = &self.screen else {
            return;
        };
        let (period, offset) = (view.period, view.offset);
        let max_scroll = self.period_max_scroll(view);
        match key {
            KeyCode::Esc | KeyCode::Char('q') => self.screen = Screen::Main,
            KeyCode::Left | KeyCode::Char('h') => self.open_period(period, offset - 1),
            KeyCode::Right | KeyCode::Char('l') if offset < 0 => {
                self.open_period(period, offset + 1);
            }
            KeyCode::Char('w') => self.open_period(Period::Week, 0),
            KeyCode::Char('m') => self.open_period(Period::Month, 0),
            KeyCode::Char('y') => self.open_period(Period::Year, 0),
            _ => {
                let Screen::Period(view) = &mut self.screen else {
                    return;
                };
                let page = self.size.1.saturating_sub(6) as usize;
                view.scroll = match key {
                    KeyCode::Up | KeyCode::Char('k') => view.scroll.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => view.scroll + 1,
                    KeyCode::PageUp => view.scroll.saturating_sub(page),
                    KeyCode::PageDown => view.scroll + page,
                    KeyCode::Home => 0,
                    KeyCode::End => max_scroll,
                    _ => return,
                }
                .min(max_scroll);
            }
        }
    }

    /// The period view's frame: (bordered area, its interior).
    fn period_areas(&self) -> (Rect, Rect) {
        let [_, content, _] = chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        (content, Block::bordered().inner(content))
    }

    fn period_max_scroll(&self, view: &PeriodView) -> usize {
        let (_, inner) = self.period_areas();
        self.period_lines(view, inner.width)
            .len()
            .saturating_sub(inner.height as usize)
    }

    /// The week or month: a row per day (its timeline on an axis shared by
    /// all the days, its tracked time and focus), the hours under them,
    /// then the period's totals like today's summary.
    pub(super) fn period_lines(&self, view: &PeriodView, width: u16) -> Vec<Line<'static>> {
        use chrono::{Datelike, TimeZone};
        let now = Utc::now();
        let today = chrono::Local::now().date_naive();
        let local_day = |at: chrono::DateTime<Utc>| at.with_timezone(&chrono::Local).date_naive();
        // The in-memory copy of the running session stands in for its row.
        let active = |within: &dyn Fn(chrono::NaiveDate) -> bool| {
            self.active_session
                .as_ref()
                .filter(|s| within(local_day(s.started_at)))
        };
        // Grouped by day once: a year has 365 of them.
        let mut by_day: std::collections::BTreeMap<chrono::NaiveDate, Vec<&Session>> =
            Default::default();
        for session in &view.sessions {
            by_day
                .entry(local_day(session.started_at))
                .or_default()
                .push(session);
        }
        let days = view
            .first
            .iter_days()
            .take_while(|day| *day <= view.last)
            .map(|day| {
                let sessions = by_day.get(&day).into_iter().flatten().copied();
                (day, summarize(sessions, active(&|d| d == day), now))
            })
            .collect::<Vec<_>>();
        let total = summarize(
            &view.sessions,
            active(&|d| (view.first..=view.last).contains(&d)),
            now,
        );

        // The hours the days share: from the earliest start's hour to the
        // latest end's, as times of day (9–17 when nothing was tracked).
        let midnight = |day: chrono::NaiveDate| {
            chrono::Local
                .from_local_datetime(&day.and_time(chrono::NaiveTime::MIN))
                .earliest()
                .map(|t| t.with_timezone(&Utc))
        };
        let mut minutes = days.iter().flat_map(|(day, summary)| {
            let midnight = midnight(*day);
            summary.blocks.iter().filter_map(move |block| {
                let midnight = midnight?;
                let start = (block.start - midnight).num_minutes();
                let end = (block.end - midnight).num_minutes().min(24 * 60);
                Some((start, end))
            })
        });
        let (from_hour, to_hour) = match minutes.next() {
            None => (9, 17),
            Some(first) => {
                let (start, end) = minutes.fold(first, |(s, e), (bs, be)| (s.min(bs), e.max(be)));
                let from = (start / 60).clamp(0, 23);
                let to = ((end + 59) / 60).clamp(from + 1, 24);
                (from, to)
            }
        };
        let at_hour = |day: chrono::NaiveDate, hour: i64| {
            midnight(day).map(|m| m + chrono::Duration::hours(hour))
        };

        let graph_width =
            (width as usize).saturating_sub(SUMMARY_LABEL_WIDTH as usize + DAY_TOTAL_WIDTH);
        let label_width = SUMMARY_LABEL_WIDTH as usize;
        let mut lines = Vec::new();
        if view.period == Period::Year {
            // A year leads with its focus grid, the view's point, then a
            // row per month instead of per day.
            let grid = year_focus_grid(&days, view.first, view.last, today);
            let block_width = grid.iter().map(item_width).max().unwrap_or(0);
            lines.extend(
                grid.into_iter()
                    .map(|row| centered(row, block_width, width)),
            );
            lines.push(Line::default());
            let legend = focus_legend();
            let legend_width = item_width(&legend);
            lines.push(centered(legend, legend_width, width));
            lines.push(Line::default());

            let months = (1..=12)
                .filter_map(|month| view.first.with_month(month))
                .map(|first| {
                    let in_month = |d: chrono::NaiveDate| d.month() == first.month();
                    let sessions = by_day
                        .iter()
                        .filter(|(day, _)| in_month(**day))
                        .flat_map(|(_, sessions)| sessions.iter().copied());
                    (first, summarize(sessions, active(&in_month), now))
                })
                .collect::<Vec<_>>();
            // Bars scaled to the busiest month, so their lengths compare.
            let busiest = months
                .iter()
                .map(|(_, s)| s.tracked.num_seconds())
                .max()
                .unwrap_or(0)
                .max(1);
            for (first, summary) in &months {
                let tracked = !summary.tracked.is_zero();
                let this_month = first.year() == today.year() && first.month() == today.month();
                let label_style = if this_month {
                    theme::TITLE
                } else if tracked {
                    theme::VALUE
                } else {
                    theme::DIMMED
                };
                let mut row = vec![Span::styled(
                    format!("{:label_width$}", first.format("%b").to_string()),
                    label_style,
                )];
                // At least a cell for any time tracked; none for none.
                let length = match tracked {
                    true => ((graph_width as i64 * summary.tracked.num_seconds() / busiest)
                        as usize)
                        .max(1),
                    false => 0,
                };
                row.extend(activity_bar(summary, length));
                row.push(Span::raw(" ".repeat(graph_width.saturating_sub(length))));
                if tracked {
                    row.extend(day_total(summary));
                }
                lines.push(Line::from(row));
            }
            lines.push(Line::default());
        }
        for (day, summary) in days.iter().filter(|_| view.period != Period::Year) {
            let tracked = !summary.tracked.is_zero();
            let label_style = if *day == today {
                theme::TITLE
            } else if tracked {
                theme::VALUE
            } else {
                theme::DIMMED
            };
            let mut row = vec![Span::styled(
                format!("{:label_width$}", day.format("%a %-d").to_string()),
                label_style,
            )];
            match (at_hour(*day, from_hour), at_hour(*day, to_hour)) {
                (Some(from), Some(to)) if tracked => {
                    row.extend(timeline_row(summary, from, to, graph_width));
                }
                _ => row.push(Span::raw(" ".repeat(graph_width))),
            }
            if tracked {
                row.extend(day_total(summary));
            }
            lines.push(Line::from(row));
            // A month reads in weeks: a blank row after each Sunday.
            if view.period == Period::Month
                && day.weekday() == chrono::Weekday::Sun
                && *day != view.last
            {
                lines.push(Line::default());
            }
        }
        if view.period != Period::Year
            && let (Some(from), Some(to)) =
                (at_hour(view.first, from_hour), at_hour(view.first, to_hour))
        {
            let mut hours = vec![Span::raw(" ".repeat(label_width))];
            hours.extend(hour_labels(from, to, graph_width));
            lines.push(Line::from(hours));
            lines.push(Line::default());
        }

        if total.tracked.is_zero() {
            let what = match view.period {
                Period::Week => "this week",
                Period::Month => "this month",
                Period::Year => "this year",
            };
            lines.push(Line::from(Span::styled(
                format!("Nothing tracked {what}."),
                theme::DIMMED,
            )));
            return lines;
        }

        // The period's own project colors: distinct among its projects.
        let mut projects = view
            .sessions
            .iter()
            .filter_map(|s| s.project.clone())
            .collect::<Vec<_>>();
        projects.sort_unstable();
        projects.dedup();
        let ranks = projects
            .iter()
            .map(|p| self.project_rank(p))
            .collect::<Vec<_>>();
        let palette = theme::project_colors(&ranks);
        let colors = |project: Option<&str>| match project {
            None => theme::NO_PROJECT,
            Some(name) => projects
                .iter()
                .position(|p| p == name)
                .map_or(theme::NO_PROJECT, |i| palette[i]),
        };

        let active_days = days.iter().filter(|(_, s)| !s.tracked.is_zero()).count();
        let mut totals = time_split_items(&total);
        totals.push(vec![Span::styled(plural(active_days, "day"), theme::VALUE)]);
        totals.push(vec![
            Span::styled(
                fmt_hms(total.tracked / active_days.max(1) as i32),
                theme::VALUE,
            ),
            Span::raw(" a day"),
        ]);
        // Each group: graph rows before its items, and after them.
        for (i, (label, before, items, after)) in [
            ("Total   ", Vec::new(), totals, Vec::new()),
            (
                "Activity",
                vec![activity_bar(&total, graph_width)],
                activity_items(&total),
                Vec::new(),
            ),
            (
                "Project ",
                vec![project_bar(&total, graph_width, &colors)],
                project_items(&total, &colors),
                Vec::new(),
            ),
            ("Focus   ", Vec::new(), focus_items(&total), Vec::new()),
        ]
        .into_iter()
        .enumerate()
        {
            if i > 0 {
                lines.push(Line::default());
            }
            let rows = [before, wrap_items(items, width, label), after].concat();
            lines.extend(hanging_lines(label, rows));
        }

        // Under the focus text, the focus grid, centered as a block, and
        // a row further down its legend, centered on its own (a year's
        // leads the view instead).
        if view.period == Period::Year {
            return lines;
        }
        let grid = focus_grid(&days, view.first, view.last, today);
        let block_width = grid.iter().map(item_width).max().unwrap_or(0);
        lines.push(Line::default());
        lines.extend(
            grid.into_iter()
                .map(|row| centered(row, block_width, width)),
        );
        lines.push(Line::default());
        let legend = focus_legend();
        let legend_width = item_width(&legend);
        lines.push(centered(legend, legend_width, width));
        lines
    }

    /// The week or month in a bordered box titled with its dates, scrolled
    /// to `scroll` (clamped), with a scrollbar when it doesn't all fit.
    pub(super) fn draw_period(&self, frame: &mut Frame, view: &PeriodView) {
        let (area, inner) = self.period_areas();
        let title = view.period.title(view.first, view.last);
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(theme::ACCENT))
                .title(Line::from(Span::styled(format!(" {title} "), theme::TITLE))),
            area,
        );
        let lines = self.period_lines(view, inner.width);
        let visible = inner.height as usize;
        let offset = view.scroll.min(lines.len().saturating_sub(visible));
        let total = lines.len();
        frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), inner);
        let track = Rect::new(area.right().saturating_sub(1), inner.y, 1, inner.height);
        draw_scrollbar(frame, track, total, visible, offset);
    }
}
