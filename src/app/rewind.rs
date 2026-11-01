//! `/rewind`: every day with sessions, newest first, searched as you type
//! (see search.rs for the query), with the year's focus grid on top lit on
//! the matching days. Enter opens a day, read-only, like the main screen
//! for that day; ←→ step through the listed days from there.
//!
//! Everything is loaded once when it opens: the sessions, each day's
//! summary, and the hours the rows share. Searching filters that in
//! memory, so draw never queries the db.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, TimeZone, Utc};
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::search::Query;
use crate::session::Session;
use crate::summary::{DaySummary, summarize};
use crate::theme;

use super::graphs::{DayCell, centered, focus_cell, item_width, timeline_row, year_focus_grid};
use super::items::{DAY_TOTAL_WIDTH, day_total, project_span};
use super::main_screen::summary_body_of;
use super::modal::WrappedInput;
use super::sections::draw_scrollbar;
use super::text::fmt_time;
use super::{App, Screen};

/// Columns before a day's timeline: the selection marker and the date.
const DATE_WIDTH: usize = 18;

/// Rows the year grid takes, with the blank row under it.
const GRID_ROWS: u16 = 6;

/// A day with sessions: its date, its sessions (indices into
/// `Rewind::sessions`, earliest first) and its summary.
pub(super) struct RewindDay {
    pub(super) date: chrono::NaiveDate,
    pub(super) sessions: Vec<usize>,
    pub(super) summary: DaySummary,
}

/// A day the query matches (an index into `Rewind::days`) and its
/// matching sessions, each with the note line that matched; no sessions
/// when the query has no words for them.
pub(super) struct Found {
    pub(super) day: usize,
    pub(super) sessions: Vec<(usize, Option<String>)>,
}

/// The `/rewind` screen's state.
pub(super) struct Rewind {
    pub(super) sessions: Vec<Session>,
    /// Every day with sessions, newest first.
    pub(super) days: Vec<RewindDay>,
    /// The hours (local, from–to) every row's timeline spans.
    pub(super) hours: (i64, i64),
    pub(super) input: String,
    pub(super) query: Query,
    /// The listed days, newest first, and their dates (for the grid).
    pub(super) found: Vec<Found>,
    pub(super) matched: HashSet<chrono::NaiveDate>,
    /// The picked day, an index into `found`.
    pub(super) selection: usize,
    /// The list's first visible row.
    pub(super) scroll: usize,
}

/// A day opened from `/rewind`: the list to go back to (the day is its
/// selection) and how far the day is scrolled.
pub(super) struct PastDay {
    pub(super) rewind: Box<Rewind>,
    pub(super) scroll: usize,
}

/// The parts of the `/rewind` screen: the box, the grid (when there is
/// room), the list and the input row.
struct RewindAreas {
    frame: Rect,
    grid: Option<Rect>,
    list: Rect,
    input: Rect,
}

fn rewind_areas(content: Rect) -> RewindAreas {
    let inner = Block::bordered().inner(content);
    // The grid only when the list keeps a few rows of its own.
    let grid_rows = if inner.height >= GRID_ROWS + 8 {
        GRID_ROWS
    } else {
        0
    };
    let [grid, list, _, input] = Layout::vertical([
        Constraint::Length(grid_rows),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    RewindAreas {
        frame: content,
        grid: (grid_rows > 0).then_some(grid),
        list,
        input,
    }
}

/// UTC at `hour` o'clock (local) on `day`; hours past midnight run into
/// the next day.
fn at_hour(day: chrono::NaiveDate, hour: i64) -> Option<chrono::DateTime<Utc>> {
    let midnight = chrono::Local
        .from_local_datetime(&day.and_time(chrono::NaiveTime::MIN))
        .earliest()?;
    Some((midnight + chrono::Duration::hours(hour)).with_timezone(&Utc))
}

impl Rewind {
    /// Groups `sessions` (earliest first) by local day, newest day first,
    /// summing each up; `active` stands in for the running session's row.
    fn new(sessions: Vec<Session>, active: Option<&Session>) -> Self {
        let now = Utc::now();
        let local_day = |at: chrono::DateTime<Utc>| at.with_timezone(&chrono::Local).date_naive();
        let mut by_day: Vec<(chrono::NaiveDate, Vec<usize>)> = Vec::new();
        for (i, session) in sessions.iter().enumerate() {
            let date = local_day(session.started_at);
            match by_day.last_mut() {
                Some((last, indices)) if *last == date => indices.push(i),
                _ => by_day.push((date, vec![i])),
            }
        }
        let days = by_day
            .into_iter()
            .rev()
            .map(|(date, indices)| {
                let day_sessions = indices.iter().map(|&i| &sessions[i]);
                let running = active.filter(|a| local_day(a.started_at) == date);
                RewindDay {
                    date,
                    summary: summarize(day_sessions, running, now),
                    sessions: indices,
                }
            })
            .collect::<Vec<_>>();
        // The hours every row shares: from the earliest start's hour to
        // the latest end's, as times of day (9–17 with nothing).
        let mut minutes = days.iter().flat_map(|day| {
            let midnight = at_hour(day.date, 0);
            day.summary.blocks.iter().filter_map(move |block| {
                let midnight = midnight?;
                let start = (block.start - midnight).num_minutes();
                let end = (block.end - midnight).num_minutes().min(24 * 60);
                Some((start, end))
            })
        });
        let hours = match minutes.next() {
            None => (9, 17),
            Some(first) => {
                let (start, end) = minutes.fold(first, |(s, e), (bs, be)| (s.min(bs), e.max(be)));
                let from = (start / 60).clamp(0, 23);
                (from, ((end + 59) / 60).clamp(from + 1, 24))
            }
        };
        let mut rewind = Self {
            sessions,
            days,
            hours,
            input: String::new(),
            query: Query::default(),
            found: Vec::new(),
            matched: HashSet::new(),
            selection: 0,
            scroll: 0,
        };
        rewind.search();
        rewind
    }

    /// Runs the query in `input`: the days it matches, newest first, with
    /// their matching sessions; back to the newest of them.
    fn search(&mut self) {
        self.query = Query::parse(&self.input);
        let text = self.query.has_text();
        self.found = self
            .days
            .iter()
            .enumerate()
            .filter(|(_, day)| self.query.matches_date(day.date))
            .filter_map(|(d, day)| {
                if !text {
                    return Some(Found {
                        day: d,
                        sessions: Vec::new(),
                    });
                }
                let sessions = day
                    .sessions
                    .iter()
                    .filter_map(|&i| {
                        let found = self.query.match_session(&self.sessions[i])?;
                        Some((i, found.note))
                    })
                    .collect::<Vec<_>>();
                (!sessions.is_empty()).then_some(Found { day: d, sessions })
            })
            .collect();
        self.matched = self.found.iter().map(|f| self.days[f.day].date).collect();
        self.selection = 0;
        self.scroll = 0;
    }

    /// The picked day, if any are listed.
    pub(super) fn selected_day(&self) -> Option<&RewindDay> {
        let found = self.found.get(self.selection)?;
        Some(&self.days[found.day])
    }
}

impl App {
    /// `/rewind`: loads every session and opens the list.
    pub(super) fn open_rewind(&mut self) {
        match self.db.all_sessions() {
            Ok(sessions) => {
                self.clear_error();
                let rewind = Rewind::new(sessions, self.active_session.as_ref());
                self.screen = Screen::Rewind(Box::new(rewind));
            }
            Err(e) => self.set_error(e),
        }
    }

    /// The list, `width` columns wide, and the row each listed day starts
    /// at: a row per day (marker, date, timeline, total and focus), and
    /// under it, when the query has words, its matching sessions with the
    /// note line that matched.
    pub(super) fn rewind_lines(
        &self,
        view: &Rewind,
        width: u16,
    ) -> (Vec<Line<'static>>, Vec<usize>) {
        let width = width as usize;
        let graph_width = width.saturating_sub(DATE_WIDTH + DAY_TOTAL_WIDTH);
        let this_year = chrono::Local::now().year();
        let mut lines = Vec::new();
        let mut starts = Vec::new();
        for (i, found) in view.found.iter().enumerate() {
            let day = &view.days[found.day];
            starts.push(lines.len());
            let picked = i == view.selection;
            let date = match day.date.year() == this_year {
                true => day.date.format("%a %-d %b").to_string(),
                false => day.date.format("%a %-d %b %Y").to_string(),
            };
            let mut row = vec![
                Span::styled(if picked { "› " } else { "  " }, theme::PROMPT),
                Span::styled(
                    format!("{date:w$}", w = DATE_WIDTH - 2),
                    if picked { theme::TITLE } else { theme::VALUE },
                ),
            ];
            match (
                at_hour(day.date, view.hours.0),
                at_hour(day.date, view.hours.1),
            ) {
                (Some(from), Some(to)) => {
                    row.extend(timeline_row(&day.summary, from, to, graph_width))
                }
                _ => row.push(Span::raw(" ".repeat(graph_width))),
            }
            row.extend(day_total(&day.summary));
            lines.push(Line::from(row));

            let day_sessions = day
                .sessions
                .iter()
                .map(|&i| &view.sessions[i])
                .collect::<Vec<_>>();
            let colors = self.project_colors_of(&day_sessions);
            for (s, note) in &found.sessions {
                let session = &view.sessions[*s];
                let end = session
                    .ended_at
                    .as_ref()
                    .map_or("now".to_string(), fmt_time);
                let mut row = vec![Span::styled(
                    format!("    {} – {end}  ", fmt_time(&session.started_at)),
                    theme::DIMMED,
                )];
                let parts = [
                    session
                        .project
                        .as_deref()
                        .map(|p| project_span(Some(p), colors(Some(p)))),
                    session.task.as_deref().map(|t| Span::raw(t.to_string())),
                    Some(theme::activity(&session.activity)),
                ];
                for (n, part) in parts.into_iter().flatten().enumerate() {
                    if n > 0 {
                        row.push(Span::styled(" · ", theme::DIMMED));
                    }
                    row.push(part);
                }
                lines.push(Line::from(row));
                if let Some(note) = note {
                    let room = width.saturating_sub(8);
                    let mut text: String = note.chars().take(room).collect();
                    if note.chars().count() > room {
                        text.pop();
                        text.push('…');
                    }
                    lines.push(Line::from(Span::styled(
                        format!("      • {text}"),
                        theme::DIMMED,
                    )));
                }
            }
        }
        (lines, starts)
    }

    /// Scrolls the list so the picked day's rows (as many as fit) show.
    fn rewind_follow(&mut self) {
        let Screen::Rewind(view) = &self.screen else {
            return;
        };
        let [_, content, _] =
            super::chrome::chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        let list = rewind_areas(content).list;
        let (lines, starts) = self.rewind_lines(view, list.width);
        let Some(&start) = starts.get(view.selection) else {
            return;
        };
        let end = starts
            .get(view.selection + 1)
            .copied()
            .unwrap_or(lines.len());
        let height = list.height as usize;
        let scroll = view
            .scroll
            .min(start)
            .max(end.saturating_sub(height).min(start));
        if let Screen::Rewind(view) = &mut self.screen {
            view.scroll = scroll;
        }
    }

    /// Moves the pick by `delta` days (down the list: older), within it.
    pub(super) fn rewind_move(&mut self, delta: isize) {
        if let Screen::Rewind(view) = &mut self.screen {
            let last = view.found.len().saturating_sub(1);
            view.selection = view.selection.saturating_add_signed(delta).min(last);
        }
        self.rewind_follow();
    }

    /// Keys in the list: typing searches, ↑↓ / PgUp PgDn / Home End move
    /// the pick, Enter opens it, Esc clears the query, then closes.
    pub(super) fn handle_rewind(&mut self, key: KeyCode) {
        let Screen::Rewind(view) = &mut self.screen else {
            return;
        };
        match key {
            KeyCode::Esc if view.input.is_empty() => self.screen = Screen::Main,
            KeyCode::Esc => {
                view.input.clear();
                view.search();
            }
            KeyCode::Backspace => {
                view.input.pop();
                view.search();
            }
            KeyCode::Char(c) => {
                view.input.push(c);
                view.search();
            }
            KeyCode::Up => self.rewind_move(-1),
            KeyCode::Down => self.rewind_move(1),
            KeyCode::PageUp => self.rewind_move(-10),
            KeyCode::PageDown => self.rewind_move(10),
            KeyCode::Home => self.rewind_move(isize::MIN / 2),
            KeyCode::End => self.rewind_move(isize::MAX / 2),
            KeyCode::Enter if !view.found.is_empty() => {
                let Screen::Rewind(view) = std::mem::replace(&mut self.screen, Screen::Main) else {
                    return;
                };
                self.screen = Screen::PastDay(PastDay {
                    rewind: view,
                    scroll: 0,
                });
            }
            _ => {}
        }
    }

    /// The year grid above the list: the picked day's year (this one when
    /// nothing is listed), each day lit with its focus where the query
    /// matches it.
    fn rewind_grid(&self, view: &Rewind) -> Vec<super::items::Item> {
        let today = chrono::Local::now().date_naive();
        let year = view
            .selected_day()
            .map_or(today.year(), |day| day.date.year());
        let (Some(first), Some(last)) = (
            chrono::NaiveDate::from_ymd_opt(year, 1, 1),
            chrono::NaiveDate::from_ymd_opt(year, 12, 31),
        ) else {
            return Vec::new();
        };
        let by_date = view
            .days
            .iter()
            .map(|day| (day.date, &day.summary))
            .collect::<HashMap<_, _>>();
        let cell = |date: chrono::NaiveDate| match view.matched.contains(&date) {
            true => focus_cell(by_date.get(&date).copied()),
            false => DayCell::Empty,
        };
        year_focus_grid(first, last, today, &cell, &year.to_string())
    }

    /// The list in an accent box titled with how many days it shows, the
    /// grid on top, the query input at the bottom.
    pub(super) fn draw_rewind(&self, frame: &mut Frame, content: Rect, view: &Rewind) {
        let areas = rewind_areas(content);
        let days = view.days.len();
        let title = match view.query.is_empty() {
            true => format!(" REWIND · {} ", super::text::plural(days, "day")),
            false => format!(
                " REWIND · {} of {} ",
                view.found.len(),
                super::text::plural(days, "day")
            ),
        };
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(theme::ACCENT))
                .title(Line::from(Span::styled(title, theme::TITLE))),
            areas.frame,
        );
        if let Some(grid_area) = areas.grid {
            let grid = self.rewind_grid(view);
            let block_width = grid.iter().map(item_width).max().unwrap_or(0);
            let rows = grid
                .into_iter()
                .map(|row| centered(row, block_width, grid_area.width))
                .collect::<Vec<_>>();
            frame.render_widget(Paragraph::new(rows), grid_area);
        }

        let (lines, _) = self.rewind_lines(view, areas.list.width);
        if lines.is_empty() {
            let text = match days {
                0 => "Nothing tracked yet.",
                _ => "No day matches.",
            };
            frame.render_widget(
                Paragraph::new(Span::styled(text, theme::DIMMED)),
                areas.list,
            );
        }
        let visible = areas.list.height as usize;
        let offset = view.scroll.min(lines.len().saturating_sub(visible));
        let total = lines.len();
        frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), areas.list);
        let track = Rect::new(
            areas.frame.right().saturating_sub(1),
            areas.list.y,
            1,
            areas.list.height,
        );
        draw_scrollbar(frame, track, total, visible, offset);

        if let Some(input) = self.wrapped_input(content) {
            frame.render_widget(input.paragraph(), areas.input);
        }
    }

    /// Where the query input goes and how wide it is, for the chrome's
    /// cursor.
    pub(super) fn rewind_input(&self, content: Rect, view: &Rewind) -> (WrappedInput, Rect) {
        let area = rewind_areas(content).input;
        let input = WrappedInput::new(
            &view.input,
            "search: words, a: p: t: n: d: \"phrases\"",
            area.width,
            1,
        );
        (input, area)
    }

    // --- a past day ----------------------------------------------------------

    /// A past day's lines, `width` wide: its summary with the graphs, then
    /// its sessions, earliest first, each with all its notes.
    pub(super) fn past_day_lines(&self, view: &Rewind, width: u16) -> Vec<Line<'static>> {
        let Some(day) = view.selected_day() else {
            return Vec::new();
        };
        let sessions = day
            .sessions
            .iter()
            .map(|&i| &view.sessions[i])
            .collect::<Vec<_>>();
        let colors = self.project_colors_of(&sessions);
        let mut lines = summary_body_of(&day.summary, width, true, &colors);
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            super::text::plural(sessions.len(), "session"),
            theme::TITLE,
        )));
        for session in sessions {
            lines.push(Line::default());
            lines.extend(self.timeline_item_lines(session, &colors));
        }
        lines
    }

    fn past_day_area(&self) -> (Rect, Rect) {
        let [_, content, _] =
            super::chrome::chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        (content, Block::bordered().inner(content))
    }

    /// Scrolls the day by `delta` rows, within it.
    pub(super) fn past_day_scroll(&mut self, delta: isize) {
        let (_, inner) = self.past_day_area();
        let Screen::PastDay(day) = &self.screen else {
            return;
        };
        let max = self
            .past_day_lines(&day.rewind, inner.width)
            .len()
            .saturating_sub(inner.height as usize);
        if let Screen::PastDay(day) = &mut self.screen {
            day.scroll = day.scroll.saturating_add_signed(delta).min(max);
        }
    }

    /// Keys in a past day: ←→ (h l) the older / newer listed day, the
    /// scroll keys scroll, Esc or q go back to the list.
    pub(super) fn handle_past_day(&mut self, key: KeyCode) {
        let page = self.past_day_area().1.height.saturating_sub(1) as isize;
        let Screen::PastDay(day) = &mut self.screen else {
            return;
        };
        match key {
            KeyCode::Esc | KeyCode::Char('q') => {
                let Screen::PastDay(day) = std::mem::replace(&mut self.screen, Screen::Main) else {
                    return;
                };
                self.screen = Screen::Rewind(day.rewind);
                self.rewind_follow();
            }
            KeyCode::Left | KeyCode::Char('h') => {
                let last = day.rewind.found.len().saturating_sub(1);
                day.rewind.selection = (day.rewind.selection + 1).min(last);
                day.scroll = 0;
            }
            KeyCode::Right | KeyCode::Char('l') => {
                day.rewind.selection = day.rewind.selection.saturating_sub(1);
                day.scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => self.past_day_scroll(-1),
            KeyCode::Down | KeyCode::Char('j') => self.past_day_scroll(1),
            KeyCode::PageUp => self.past_day_scroll(-page),
            KeyCode::PageDown => self.past_day_scroll(page),
            KeyCode::Home => self.past_day_scroll(isize::MIN / 2),
            KeyCode::End => self.past_day_scroll(isize::MAX / 2),
            _ => {}
        }
    }

    /// The day in an accent box titled with its date, scrolled, with a
    /// scrollbar when it doesn't all fit.
    pub(super) fn draw_past_day(&self, frame: &mut Frame, day: &PastDay) {
        let (area, inner) = self.past_day_area();
        let title = day.rewind.selected_day().map_or(String::new(), |d| {
            d.date.format("%A %-d %B %Y").to_string().to_uppercase()
        });
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(theme::ACCENT))
                .title(Line::from(Span::styled(format!(" {title} "), theme::TITLE))),
            area,
        );
        let lines = self.past_day_lines(&day.rewind, inner.width);
        let visible = inner.height as usize;
        let offset = day.scroll.min(lines.len().saturating_sub(visible));
        let total = lines.len();
        frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), inner);
        let track = Rect::new(area.right().saturating_sub(1), inner.y, 1, inner.height);
        draw_scrollbar(frame, track, total, visible, offset);
    }
}
