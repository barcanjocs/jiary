//! The main screen: its keys (`handle_main`) and its three sections, the
//! active session with its notes, today's sessions and the day summary.

use chrono::Utc;
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::layout::Section;
use crate::mascot::Reaction;
use crate::session::{self, BREAK, DISRUPTION, MEETING, Session};
use crate::summary::{DaySummary, summarize};
use crate::theme;

use super::cat::{CAT_COLUMNS, caption_lines, cat_fits, split_for_cat};
use super::forms::{AddNoteForm, Autocomplete, RECENT_PICKS, StartSessionForm, Step};
use super::graphs::{
    activity_bar, focus_row, hour_labels, project_bar, start_of_hour, timeline_row,
};
use super::items::{
    SUMMARY_LABEL_WIDTH, activity_items, focus_items, hanging_lines, interruptions_span,
    project_items, project_span, time_split_items, value_or_none, wrap_items,
};
use super::palette::PaletteForm;
use super::sections::draw_scrollbar;
use super::text::{fmt_duration, fmt_hms, fmt_time, wrap_text};
use super::{App, RATING_WINDOW, Screen};

/// How far `[` and `]` move the active session's start.
pub(super) const START_STEP: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

/// The row under the active section while a session can be rated: a bar
/// in the accent color, hard to miss, asking for the rating. Its filled
/// part drains from the right as the `left` of `RATING_WINDOW` runs out.
fn rating_bar(left: std::time::Duration, width: u16) -> Line<'static> {
    let width = width as usize;
    let seconds = left.as_secs_f32().ceil() as u32;
    let prompt = " How was your focus?  [1] bad  [2] ok  [3] good   [u] undo";
    let countdown = format!("{seconds}s ");
    let gap = width.saturating_sub(prompt.chars().count() + countdown.chars().count());
    let text: Vec<char> = format!("{prompt}{}{countdown}", " ".repeat(gap))
        .chars()
        .take(width)
        .collect();
    let filled = ((width as f32 * left.as_secs_f32() / RATING_WINDOW.as_secs_f32()).ceil()
        as usize)
        .min(text.len());
    let bold = Style::new().add_modifier(Modifier::BOLD);
    Line::from(vec![
        Span::styled(
            text[..filled].iter().collect::<String>(),
            bold.fg(theme::ON_ACCENT).bg(theme::ACCENT),
        ),
        Span::styled(
            text[filled..].iter().collect::<String>(),
            bold.fg(theme::ACCENT),
        ),
    ])
}

/// Width of the active section's text column (session info and notes):
/// what the cat leaves when it fits, otherwise the whole interior.
pub(super) fn text_width(inner_width: u16) -> u16 {
    if cat_fits(inner_width) {
        inner_width - CAT_COLUMNS
    } else {
        inner_width
    }
}

/// One note as `  • ` bullet rows, word-wrapped to `width` with
/// continuation rows indented under the text: dimmed, or for the session's
/// own (`current`) notes an accent bullet and plain text.
fn bullet_lines(note: &str, width: u16, current: bool) -> Vec<Line<'static>> {
    const BULLET: &str = "  • ";
    let indent = BULLET.chars().count();
    let mut rows = wrap_text(note, (width as usize).saturating_sub(indent));
    if rows.len() > 1 && rows.last().is_some_and(String::is_empty) {
        rows.pop(); // the cursor row wrap_text adds after a full row
    }
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| {
            let prefix = if i == 0 {
                BULLET.to_string()
            } else {
                " ".repeat(indent)
            };
            let (bullet, text) = if current {
                (Style::new().fg(theme::ACCENT), Style::new())
            } else {
                (theme::DIMMED, theme::DIMMED)
            };
            Line::from(vec![Span::styled(prefix, bullet), Span::styled(row, text)])
        })
        .collect()
}

/// The "Previous notes:" / "Session notes:" headings.
const NOTES_HEADING: Style = theme::DIMMED.add_modifier(Modifier::BOLD);

impl App {
    /// Moves the active session's start by `delta`, for a session started
    /// late (or early). It stays in the past, and doesn't run into the
    /// session before it: when that one ended as this one started (a
    /// switch), the boundary between them moves instead, leaving it at
    /// least a minute. After a gap the start stops at that session's end;
    /// from there the two touch, so pressing on moves both.
    fn move_start(&mut self, delta: chrono::TimeDelta) {
        let Some(session) = self.active_session.as_ref() else {
            return;
        };
        let (id, started) = (session.id, session.started_at);
        let previous = match self.db.previous_session(id) {
            Ok(previous) => previous,
            Err(e) => return self.set_error(e),
        };
        let mut start = (started + delta).min(Utc::now());
        let mut moved_end = None;
        if let Some(previous) = previous
            && let Some(end) = previous.ended_at
        {
            if (started - end).num_seconds().abs() <= 2 {
                start = start.max(previous.started_at + chrono::TimeDelta::minutes(1));
                moved_end = Some(previous.id);
            } else {
                start = start.max(end);
            }
        }
        if start == started {
            return;
        }
        let result = match moved_end {
            Some(previous) => self.db.set_ended_at(previous, start),
            None => Ok(()),
        }
        .and_then(|()| self.db.set_started_at(id, start));
        match result {
            Err(e) => self.set_error(e),
            Ok(()) => {
                self.clear_error();
                if let Some(session) = &mut self.active_session {
                    session.started_at = start;
                }
                self.reload_today();
            }
        }
    }

    /// Reloads today's sessions only (a rating or a moved start), keeping
    /// the caption and scroll positions.
    pub(super) fn reload_today(&mut self) {
        match self.db.sessions_for_today() {
            Ok(sessions) => self.today_sessions = sessions,
            Err(e) => self.set_error(e),
        }
    }

    /// Handles keys for the main screen. Returns true if the app should quit.
    pub(super) fn handle_main(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Char('q') => return true,
            KeyCode::Char('?') => self.screen = Screen::Help,
            KeyCode::Char('/') => {
                self.screen = Screen::Palette(PaletteForm {
                    input: "/".into(),
                    selection: 0,
                });
            }

            // Sections: focus, collapse, resize.
            KeyCode::Tab => self.focus = self.focus.next(),
            KeyCode::BackTab => self.focus = self.focus.prev(),
            KeyCode::Char(' ') => {
                self.layout.toggle_collapsed(self.focus);
                self.save_layout();
            }
            KeyCode::Char('+') => self.resize_focused(1),
            KeyCode::Char('-') => self.resize_focused(-1),
            KeyCode::Char('=') => {
                self.layout.reset_sizes();
                self.save_layout();
            }

            // Scroll the focused section: the active session's notes, or
            // today's sessions (also when the summary, which doesn't
            // scroll, has focus).
            KeyCode::Up | KeyCode::Char('k') => self.scroll(self.scroll_target(), -1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll(self.scroll_target(), 1),
            KeyCode::PageUp => {
                let target = self.scroll_target();
                self.scroll(target, -(self.viewport(target) as isize));
            }
            KeyCode::PageDown => {
                let target = self.scroll_target();
                self.scroll(target, self.viewport(target) as isize);
            }
            KeyCode::Home => *self.scroll_position(self.scroll_target()) = 0,
            KeyCode::End => {
                let target = self.scroll_target();
                *self.scroll_position(target) = self.max_scroll(target);
            }

            KeyCode::Char('i') => {
                if let Some(id) = self.active_session.as_ref().map(|s| s.id) {
                    match self.db.increment_interruptions(id) {
                        Err(e) => self.set_error(e),
                        Ok(()) => {
                            self.clear_error();
                            if let Some(session) = &mut self.active_session {
                                session.interruptions =
                                    Some(session.interruptions.unwrap_or(0) + 1);
                            }
                            self.resample_caption();
                            self.react(Reaction::Flinch);
                        }
                    }
                }
            }
            KeyCode::Char('r') => {
                // Resume ends a detour (disruption, break or meeting), or
                // picks work back up when nothing is running; it never ends
                // work.
                let should_proceed = match &self.active_session {
                    None => true,
                    Some(s) => session::is_detour(&s.activity),
                };

                if !should_proceed {
                    return false;
                }

                // Look up what to resume before closing the pause, so it
                // keeps running when there is nothing to resume.
                let combo = match self.db.latest_work_session() {
                    Ok(Some(combo)) => {
                        self.clear_error();
                        combo
                    }
                    Ok(None) => return false,
                    Err(e) => {
                        self.set_error(e);
                        return false;
                    }
                };

                if !self.close_active_session() {
                    return false;
                }

                let (activity, project, task) = combo;
                self.start_session(&activity, project, task);

                self.refresh_after_session_change();
            }

            // A disruption interrupts something, so it needs a session; a
            // break or a meeting can also start from nothing (say, right
            // after ending a session with `e`).
            KeyCode::Char('d') => self.detour(DISRUPTION, true),
            KeyCode::Char('b') => self.detour(BREAK, false),
            KeyCode::Char('m') => self.detour(MEETING, false),
            KeyCode::Char('s') => self.start_or_ask(),
            KeyCode::Char('n') => {
                if self.active_session.is_some() {
                    self.screen = Screen::AddNote(AddNoteForm {
                        input: String::new(),
                    });
                }
            }
            KeyCode::Char('e') if self.active_session.is_some() => {
                if self.close_active_session() {
                    self.refresh_after_session_change();
                }
            }
            KeyCode::Char(c @ '1'..='3') => self.rate(c as i32 - '0' as i32),
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('p') => self.toggle_pomodoro(),
            KeyCode::Char('[') => self.move_start(-START_STEP),
            KeyCode::Char(']') => self.move_start(START_STEP),
            _ => {}
        };
        false
    }

    /// Opens the start form, with the recent picks loaded.
    pub(super) fn open_start_form(&mut self) {
        let recent = match self.db.recent_combos(RECENT_PICKS) {
            Ok(recent) => recent,
            Err(e) => {
                self.set_error(e);
                Vec::new()
            }
        };
        self.screen = Screen::StartSession(Box::new(StartSessionForm {
            step: Step::Activity,
            recent,
            activity: Autocomplete::new(self.activities.clone()),
            project: Autocomplete::new(self.projects.clone()),
            task: Autocomplete::new(Vec::new()),
        }));
    }

    /// Ends the active session (if any) and starts a detour of `kind` with
    /// no project or task; `r` later resumes the work it interrupted. A
    /// no-op on top of the same kind of detour (switching between kinds is
    /// fine), and without a running session when `needs_session`.
    fn detour(&mut self, kind: &str, needs_session: bool) {
        let startable = match &self.active_session {
            Some(s) => s.activity != kind,
            None => !needs_session,
        };
        if startable && self.close_active_session() {
            self.start_session(kind, None, None);
            self.refresh_after_session_change();
        }
    }

    /// The three sections: the active session on top, today's sessions in
    /// the middle (scrollable), the day summary at the bottom. The middle
    /// and bottom ones collapse to a single line; all are resizable.
    pub(super) fn draw_main(&self, frame: &mut Frame, area: Rect) {
        let areas = self.resolve_layout(area);
        if let Some(left) = self.rating_left()
            && areas.active.bottom() < area.bottom()
        {
            let row = Rect::new(area.x, areas.active.bottom(), area.width, 1);
            frame.render_widget(rating_bar(left, row.width), row);
        }

        let block = self.section_block(Section::Active);
        let inner = block.inner(areas.active);
        frame.render_widget(block, areas.active);
        // Two columns: on the left the session info, pinned, with the notes
        // directly below it scrolling in the rest of the height; on the
        // right the cat and its caption, pinned.
        let (text_area, cat_area, caption_area) = split_for_cat(inner);
        let info = self.active_lines();
        let [info_area, notes_area] =
            Layout::vertical([Constraint::Length(info.len() as u16), Constraint::Min(0)])
                .areas(text_area);
        frame.render_widget(Paragraph::new(info), info_area);

        let notes = self.note_lines(notes_area.width);
        let visible = notes_area.height as usize;
        let offset = self.notes_scroll.min(notes.len().saturating_sub(visible));
        frame.render_widget(
            Paragraph::new(notes.clone()).scroll((offset as u16, 0)),
            notes_area,
        );
        // On the border, beside the notes rows.
        let track = Rect::new(
            areas.active.right().saturating_sub(1),
            notes_area.y,
            1,
            notes_area.height,
        );
        draw_scrollbar(frame, track, notes.len(), visible, offset);

        if let Some(cat_area) = cat_area {
            frame.render_widget(Paragraph::new(self.cat_lines()), cat_area);
        }
        if let (Some(area), Some(caption)) = (caption_area, self.caption()) {
            frame.render_widget(
                Paragraph::new(caption_lines(caption, self.mood(), area.width)),
                area,
            );
        }

        if self.layout.today_collapsed {
            self.draw_collapsed(frame, areas.today, Section::Today);
        } else {
            self.draw_today(frame, areas.today);
        }

        if self.layout.summary_collapsed {
            self.draw_collapsed(frame, areas.summary, Section::Summary);
        } else {
            let block = self.section_block(Section::Summary);
            let inner = block.inner(areas.summary);
            frame.render_widget(block, areas.summary);
            frame.render_widget(Paragraph::new(self.summary_lines(inner.width)), inner);
        }
    }

    /// A section's rounded border with its title: accent while focused,
    /// dimmed otherwise.
    fn section_block(&self, section: Section) -> Block<'static> {
        let title = self.section_title(section);
        let border = if self.focus == section {
            Style::new().fg(theme::ACCENT)
        } else {
            theme::DIMMED
        };
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(border)
            .title(Line::from(Span::styled(title, theme::TITLE)))
    }

    /// A collapsed section: one unbordered `▸` line, accent while focused.
    fn draw_collapsed(&self, frame: &mut Frame, area: Rect, section: Section) {
        let style = if self.focus == section {
            theme::TITLE
        } else {
            theme::DIMMED.add_modifier(Modifier::BOLD)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(self.section_title(section), style)),
            area,
        );
    }

    /// Today's sessions, scrolled to `today_scroll` (clamped to the content), with
    /// a scrollbar on the right border when they don't all fit.
    fn draw_today(&self, frame: &mut Frame, area: Rect) {
        let block = self.section_block(Section::Today);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines = self.timeline_lines();
        let total = lines.len();
        let visible = inner.height as usize;
        let offset = self.today_scroll.min(total.saturating_sub(visible));
        frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), inner);
        draw_scrollbar(frame, area.inner(Margin::new(0, 1)), total, visible, offset);
    }

    /// A project's color on the main screen, gray for no project. Colors
    /// go by the order projects were first used, so each keeps its color
    /// from day to day, and are made distinct across today's projects (the
    /// main screen's view; see `theme::project_colors`).
    pub(super) fn project_color(&self, project: Option<&str>) -> Color {
        let Some(project) = project else {
            return theme::NO_PROJECT;
        };
        let mut today = self
            .today_sessions
            .iter()
            .chain(&self.active_session)
            .filter_map(|s| s.project.as_deref())
            .collect::<Vec<_>>();
        if !today.contains(&project) {
            today.push(project);
        }
        today.sort_unstable();
        today.dedup();
        let ranks = today
            .iter()
            .map(|&name| self.project_rank(name))
            .collect::<Vec<_>>();
        let colors = theme::project_colors(&ranks);
        let i = today.iter().position(|&name| name == project).unwrap_or(0);
        colors[i]
    }

    /// Where `project` comes in the order projects were first used; one
    /// not loaded yet (fresh, mid-refresh) comes after the rest.
    pub(super) fn project_rank(&self, project: &str) -> usize {
        self.project_order
            .iter()
            .position(|p| p == project)
            .unwrap_or(self.project_order.len())
    }

    /// A project's name, bold in its color, or a dimmed italic "(none)".
    fn project_span(&self, project: Option<&str>) -> Span<'static> {
        project_span(project, self.project_color(project))
    }

    /// The pinned part of the active section: the running session's info,
    /// or a hint.
    pub(super) fn active_lines(&self) -> Vec<Line<'static>> {
        match &self.active_session {
            Some(session) => self.active_info_lines(session),
            None => vec![Line::from(Span::styled(
                "No active session. Press s to start one.",
                theme::DIMMED,
            ))],
        }
    }

    /// Body of today's sessions: one item per session, newest first,
    /// separated by blank rows.
    pub(super) fn timeline_lines(&self) -> Vec<Line<'static>> {
        if self.today_sessions.is_empty() {
            return vec![Line::from(Span::styled(
                "No sessions yet today.",
                theme::DIMMED,
            ))];
        }
        let mut lines = Vec::new();
        for session in &self.today_sessions {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.extend(self.timeline_item_lines(session));
        }
        lines
    }

    pub(super) fn day_summary(&self) -> DaySummary {
        summarize(
            &self.today_sessions,
            self.active_session.as_ref(),
            Utc::now(),
        )
    }

    /// Body of the summary, wrapped to `width`: time per activity and per
    /// project, focus and interruptions, and the day's span, with graphs of
    /// the day when they fit (`summary_graphs_fit`).
    pub(super) fn summary_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.summary_body(width, self.summary_graphs_fit(width))
    }

    /// The summary's body without graphs.
    pub(super) fn summary_text_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.summary_body(width, false)
    }

    fn summary_body(&self, width: u16, graphs: bool) -> Vec<Line<'static>> {
        let summary = self.day_summary();
        let Some((start, end)) = summary.span else {
            return vec![Line::from(Span::styled(
                "Nothing tracked yet today.",
                theme::DIMMED,
            ))];
        };

        let time = |d: chrono::Duration| Span::styled(fmt_hms(d), theme::VALUE);
        let activities = activity_items(&summary);
        let projects = project_items(&summary, &|p| self.project_color(p));
        let focus = focus_items(&summary);
        let mut day = vec![vec![Span::styled(
            format!("{} – {}", fmt_time(&start), fmt_time(&end)),
            theme::VALUE,
        )]];
        day.extend(time_split_items(&summary));
        day.push(vec![Span::raw("gaps "), time(summary.untracked())]);

        // Two pictures lead their rows when there is room for them: the day
        // as a timeline in activity colors, and each activity's share of
        // the tracked time as one stacked bar.
        let graph_width = width.saturating_sub(SUMMARY_LABEL_WIDTH) as usize;
        let mut day_rows = Vec::new();
        let mut activity_rows = Vec::new();
        let mut project_rows = Vec::new();
        if graphs {
            // From the hour the day began, so the hours line up from the
            // left edge; the minutes before the first start show blank.
            let from = start_of_hour(start);
            day_rows.push(timeline_row(&summary, from, end, graph_width));
            day_rows.push(hour_labels(from, end, graph_width));
            activity_rows.push(activity_bar(&summary, graph_width));
            project_rows.push(project_bar(&summary, graph_width, &|p| {
                self.project_color(p)
            }));
            // The hours under the timeline already show the span.
            day.remove(0);
        }

        let mut lines = Vec::new();
        if graphs {
            // The focus line sits on top of the timeline, under no label.
            let from = start_of_hour(start);
            let indent = Span::raw(" ".repeat(SUMMARY_LABEL_WIDTH as usize));
            let focus = focus_row(&summary, from, end, graph_width);
            lines.push(Line::from([vec![indent], focus].concat()));
        }
        for (i, (label, graph_rows, items)) in [
            ("Day     ", day_rows, day),
            ("Activity", activity_rows, activities),
            ("Project ", project_rows, projects),
            ("Focus   ", Vec::new(), focus),
        ]
        .into_iter()
        .enumerate()
        {
            // With the graphs there is room to breathe: a blank row
            // between groups.
            if graphs && i > 0 {
                lines.push(Line::default());
            }
            let rows = [graph_rows, wrap_items(items, width, label)].concat();
            lines.extend(hanging_lines(label, rows));
        }
        lines
    }

    /// The collapsed summary: tracked time and the top activity.
    pub(super) fn summary_one_liner(&self) -> String {
        let summary = self.day_summary();
        match summary.per_activity.first() {
            Some((activity, d)) => format!(
                "SUMMARY · tracked {} · {activity} {}",
                fmt_hms(summary.tracked),
                fmt_hms(*d)
            ),
            None => "SUMMARY · nothing tracked yet".to_string(),
        }
    }

    /// The running session's info: bold timer, dimmed label/value rows,
    /// interruptions colored when > 0.
    fn active_info_lines(&self, session: &Session) -> Vec<Line<'static>> {
        let elapsed = Utc::now() - session.started_at;
        let (h, m, s) = (
            elapsed.num_hours(),
            elapsed.num_minutes() % 60,
            elapsed.num_seconds() % 60,
        );

        let mut lines = vec![Line::from(vec![
            Span::styled(format!("{h:02}:{m:02}:{s:02}"), self.timer_style()),
            Span::styled(
                format!("  since {}", fmt_time(&session.started_at)),
                theme::DIMMED,
            ),
        ])];
        // The Pomodoro's bar, right under the timer it measures.
        lines.extend(self.pomodoro_line());
        lines.extend([
            theme::kv(
                "Project",
                vec![self.project_span(session.project.as_deref())],
            ),
            theme::kv("Task", vec![value_or_none(&session.task, Style::new())]),
            theme::kv("Activity", vec![theme::activity(&session.activity)]),
        ]);

        let interruptions = session.interruptions.unwrap_or(0);
        lines.push(theme::kv(
            "Interruptions",
            vec![interruptions_span(interruptions, "")],
        ));
        lines
    }

    /// The scrollable part of the active section: previous notes for the
    /// project/task, then the session's own, as dimmed bullets.
    /// Notes are wrapped to `width` (the text column), so scrolling and the
    /// section's height count rows on screen, not notes.
    pub(super) fn note_lines(&self, width: u16) -> Vec<Line<'static>> {
        let Some(session) = &self.active_session else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        // Cached; refreshed whenever the active session changes.
        if !self.previous_notes.is_empty() {
            lines.push(Line::from(Span::styled("Previous notes:", NOTES_HEADING)));
            // Each entry is one session's notes: newline-separated, one
            // bullet per line.
            for note in self.previous_notes.iter().flat_map(|notes| notes.lines()) {
                lines.extend(bullet_lines(note, width, false));
            }
        }

        // Current session's own notes, brighter than the previous ones.
        if let Some(notes) = &session.notes
            && !notes.is_empty()
        {
            lines.push(Line::from(Span::styled("Session notes:", NOTES_HEADING)));
            for note in notes.lines() {
                lines.extend(bullet_lines(note, width, true));
            }
        }

        lines
    }

    /// One timeline item: time range, duration, focus badge and interruption
    /// count on the first line, project·task·activity below it, then notes.
    fn timeline_item_lines(&self, session: &Session) -> Vec<Line<'static>> {
        let start = fmt_time(&session.started_at);
        let (end, duration) = match &session.ended_at {
            Some(end) => (
                Span::styled(fmt_time(end), theme::VALUE),
                fmt_duration(&session.started_at, end),
            ),
            // The active session appears here too, still running.
            None => (
                Span::styled("now", theme::TIME),
                fmt_hms(Utc::now() - session.started_at),
            ),
        };

        let interruptions = session.interruptions.unwrap_or(0);
        let mut first = vec![
            Span::styled(format!("{start} – "), theme::VALUE),
            end,
            Span::styled(format!(" ({duration}) "), theme::TIME),
        ];
        // Breaks and disruptions are never rated, so they get no badge.
        if session::asks_focus(&session.activity) {
            first.extend([theme::focus_badge(session.focus), Span::raw(" ")]);
        }
        first.push(interruptions_span(interruptions, "I:"));
        let mut lines = vec![Line::from(first)];

        // Project and activity in their colors, task plain.
        let mut parts = Vec::new();
        for part in [
            session
                .project
                .as_deref()
                .map(|p| self.project_span(Some(p))),
            session.task.as_deref().map(|t| Span::raw(t.to_string())),
            Some(theme::activity(&session.activity)),
        ]
        .into_iter()
        .flatten()
        .filter(|span| !span.content.is_empty())
        {
            if !parts.is_empty() {
                parts.push(Span::styled(" · ", theme::DIMMED));
            }
            parts.push(part);
        }
        lines.push(Line::from(parts));

        if let Some(notes) = &session.notes {
            for line in notes.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  • {line}"),
                    theme::DIMMED,
                )));
            }
        }

        lines
    }
}
