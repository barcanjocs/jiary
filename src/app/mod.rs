use std::path::PathBuf;
use std::time::Instant;

use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::db::Db;
use crate::layout::{LayoutPrefs, Section};
use crate::mascot::Reaction;
use crate::session::{self, BREAK, DISRUPTION, MEETING, Session};
use crate::summary::{DaySummary, summarize};
use crate::theme;
use cat::{CAT_COLUMNS, caption_lines, cat_fits, fresh_seed, split_for_cat};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use forms::{AddNoteForm, Autocomplete, RECENT_PICKS, StartSessionForm, Step};
use graphs::{activity_bar, focus_row, hour_labels, project_bar, start_of_hour, timeline_row};
use help::draw_help;
use items::{
    SUMMARY_LABEL_WIDTH, activity_items, focus_items, hanging_lines, interruptions_span,
    project_items, project_span, time_split_items, value_or_none, wrap_items,
};
use modal::{
    SMALL_MODAL_HEIGHT, START_MODAL_HEIGHT, WrappedInput, modal_inner_width, picker_list,
    small_modal_layout, start_modal_layout,
};
use palette::{PaletteForm, draw_palette};
use period_view::PeriodView;
use sections::draw_scrollbar;
use text::{fmt_duration, fmt_hms, fmt_time, wrap_text};

mod away;
mod cat;
mod forms;
mod graphs;
mod help;
mod items;
mod modal;
mod palette;
mod period_view;
mod sections;
#[cfg(test)]
mod tests;
mod text;

/// Suggested on the activity step until there is history to suggest from
/// (a fresh database). After that, suggestions are the activities actually
/// used, so built-ins that are never used drop out of the list.
const DEFAULT_ACTIVITIES: &[&str] = &[
    "Programming",
    "Reading",
    "Writing",
    MEETING,
    "Administration",
    DISRUPTION,
    BREAK,
    "Other",
];

pub struct App {
    db: Db,
    active_session: Option<Session>,
    today_sessions: Vec<Session>,

    /// Activity suggestions: most recently used first, or the defaults on a
    /// fresh database.
    activities: Vec<String>,
    projects: Vec<String>,
    /// Projects in the order they were first used, which picks their
    /// colors (`project_color`).
    project_order: Vec<String>,
    tasks: Vec<String>,
    screen: Screen,
    /// "Previous notes" for the active session's project/task. Cached because
    /// draw() runs ~10x/second; refreshed when the active session changes.
    previous_notes: Vec<String>,
    /// Last error that occurred, kept as plain data so the UI renders it in
    /// exactly one place (and a future auto-clear-on-success can too).
    error: Option<String>,

    /// Main-screen section sizes and collapsed state, saved to `layout_path`
    /// (when set) whenever they change.
    layout: LayoutPrefs,
    layout_path: Option<PathBuf>,
    /// The main-screen section that Space and +/- act on.
    focus: Section,
    /// First visible row of today's sessions.
    today_scroll: usize,
    /// First visible row of the active session's notes (below the pinned
    /// session info and cat).
    notes_scroll: usize,
    /// Terminal size as of the last frame. Key and mouse handlers lay the
    /// main screen out from it, so they see exactly what was drawn.
    size: (u16, u16),
    /// While a border is dragged with the mouse: the section whose height
    /// it sets (see `border_at`) and which of the border's two rows was
    /// grabbed (0: the upper section's bottom border, 1: the lower one's
    /// top border), so the border follows the pointer without jumping.
    drag: Option<(Section, u16)>,
    /// A press on a collapsible section's title, pending until release: it
    /// toggles only if the mouse didn't move (a click, not a drag).
    title_press: Option<Section>,
    /// The cat's current one-off reaction and when it started; it shows
    /// for the reaction's `duration`, then the cat goes back to its mood.
    reaction: Option<(Reaction, Instant)>,
    /// Picks the caption under the cat; renewed on every session change so
    /// the caption only changes then (or when the mood does).
    caption_seed: u64,
    /// The session that just ended and when, while `1`–`3` can still rate
    /// its focus (for `RATING_WINDOW`; only sessions that ask for focus).
    rating: Option<(i64, Instant)>,
    /// The last session change, for `u` to undo.
    undo: Option<Undo>,
    /// Since when nobody has touched the computer, while a long enough
    /// absence is under way (see `observe_idle`).
    away_since: Option<chrono::DateTime<Utc>>,
    /// When the system idle time was last observed, to notice sleep.
    last_observed: Option<chrono::DateTime<Utc>>,
}

/// How long after a session ends its focus can be rated. Short on purpose:
/// a rating given right away is the honest one.
const RATING_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// A session change, as the session it ended and those it created (either
/// may be missing: `e` only ends one, starting from nothing only creates
/// one; counting an absence as a break creates the break and the resumed
/// session).
#[derive(Clone, Debug, Default, PartialEq)]
struct Undo {
    ended: Option<i64>,
    created: Vec<i64>,
}

/// How far `[` and `]` move the active session's start.
const START_STEP: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

enum Screen {
    Main,
    Help,
    // Boxed: three autocompletes make it far larger than the other variants.
    StartSession(Box<StartSessionForm>),
    AddNote(AddNoteForm),
    /// The command palette (`/`).
    Palette(PaletteForm),
    /// A week or a month, day by day (`/weekly`, `/monthly`).
    Period(PeriodView),
    /// Back after `AWAY_AFTER` or more without touching the computer,
    /// during a work session: what was that time?
    Away(chrono::DateTime<Utc>),
}

impl App {
    /// `layout_path` is where the main-screen layout is loaded from and saved
    /// to; `None` keeps it in memory only (tests).
    pub fn new(db: Db, layout_path: Option<PathBuf>) -> Self {
        let mut app = Self {
            db,
            active_session: None,
            today_sessions: Vec::new(),
            activities: Vec::new(),
            projects: Vec::new(),
            project_order: Vec::new(),
            tasks: Vec::new(),
            screen: Screen::Main,
            previous_notes: Vec::new(),
            error: None,
            layout: LayoutPrefs::default(),
            layout_path,
            focus: Section::Active,
            today_scroll: 0,
            notes_scroll: 0,
            size: (0, 0),
            drag: None,
            title_press: None,
            reaction: None,
            caption_seed: fresh_seed(),
            rating: None,
            undo: None,
            away_since: None,
            last_observed: None,
        };

        if let Some(path) = &app.layout_path {
            match LayoutPrefs::load(path) {
                Ok(layout) => app.layout = layout,
                Err(e) => app.set_error(format!("Could not read layout: {e}")),
            }
        }

        // Attempt every load so partial data still shows; failures are recorded
        // for display instead of being swallowed.
        match app.db.get_active_session() {
            Ok(session) => app.active_session = session,
            Err(e) => app.set_error(e),
        }
        app.refresh_after_session_change();
        app.refresh_suggestions();

        app
    }

    /// Records an error for display. Every DB failure must go through this so
    /// there is a single point to render; successful operations call `clear_error`.
    fn set_error(&mut self, e: impl std::fmt::Display) {
        self.error = Some(e.to_string());
    }

    /// Clears the displayed error. Called after successful database operations,
    /// so the line reflects the outcome of the most recent one.
    fn clear_error(&mut self) {
        self.error = None;
    }

    /// Reloads the lists shown on the main screen. Called whenever the active
    /// session changes.
    fn refresh_after_session_change(&mut self) {
        self.caption_seed = fresh_seed();
        self.notes_scroll = 0;
        match self.db.sessions_for_today() {
            Ok(sessions) => self.today_sessions = sessions,
            Err(e) => self.set_error(e),
        }
        self.refresh_previous_notes();
    }

    /// Reloads the activity/project/task autocomplete candidates. Called at
    /// startup and after starting a session (which may introduce new values).
    fn refresh_suggestions(&mut self) {
        match self.db.recent_activities() {
            Ok(activities) if activities.is_empty() => {
                self.activities = DEFAULT_ACTIVITIES.iter().map(|a| a.to_string()).collect();
            }
            Ok(activities) => self.activities = activities,
            Err(e) => self.set_error(e),
        }
        match self.db.distinct_projects() {
            Ok(projects) => self.projects = projects,
            Err(e) => self.set_error(e),
        }
        match self.db.projects_by_first_use() {
            Ok(order) => self.project_order = order,
            Err(e) => self.set_error(e),
        }
        match self.db.distinct_tasks() {
            Ok(tasks) => self.tasks = tasks,
            Err(e) => self.set_error(e),
        }
    }

    /// Refreshes the cached "previous notes" for the active session. The list
    /// is static during a session (the query excludes the active session
    /// itself), so it only needs updating when the session changes.
    fn refresh_previous_notes(&mut self) {
        let key = self
            .active_session
            .as_ref()
            .map(|s| (s.project.clone(), s.task.clone()));
        let notes = match key {
            Some((Some(project), Some(task))) => match self.db.recent_notes(&project, &task) {
                Ok(notes) => notes,
                Err(e) => {
                    self.set_error(e);
                    Vec::new()
                }
            },
            _ => Vec::new(),
        };
        self.previous_notes = notes;
    }

    /// Closes the currently active session, if any, starting a session
    /// change for `u` to undo and, for a session that asks for focus, the
    /// rating window. Every session change starts here. Returns true on
    /// success (or when there was nothing to close) and clears the error
    /// line; on failure restores the session, records the error, and returns
    /// false so the caller can abort (e.g. not start a replacement session).
    fn close_active_session(&mut self) -> bool {
        match self.active_session.take() {
            None => {
                self.undo = Some(Undo::default());
                true
            }
            Some(session) => match self.db.complete_session(session.id, None) {
                Ok(()) => {
                    self.clear_error();
                    self.undo = Some(Undo {
                        ended: Some(session.id),
                        created: Vec::new(),
                    });
                    // Starting from nothing keeps an open window (say, a
                    // break right after `e`); ending another session
                    // replaces it.
                    self.rating = session::asks_focus(&session.activity)
                        .then(|| (session.id, Instant::now()));
                    true
                }
                Err(e) => {
                    self.set_error(e);
                    self.active_session = Some(session);
                    false
                }
            },
        }
    }

    /// Creates a new session in the db and makes it the active session. On
    /// failure records the error and leaves the active session unchanged.
    fn start_session(&mut self, activity: &str, project: Option<String>, task: Option<String>) {
        match self
            .db
            .create_session(activity, project.as_deref(), task.as_deref())
        {
            Ok(session) => {
                self.clear_error();
                self.undo.get_or_insert_default().created.push(session.id);
                self.active_session = Some(session);
            }
            Err(e) => self.set_error(e),
        }
    }

    /// How long `1`–`3` can still rate the session that just ended, while
    /// they can.
    fn rating_left(&self) -> Option<std::time::Duration> {
        let (_, ended) = self.rating?;
        Some(RATING_WINDOW.saturating_sub(ended.elapsed())).filter(|left| !left.is_zero())
    }

    /// Whether `1`–`3` still rate the session that just ended.
    fn rating_open(&self) -> bool {
        self.rating_left().is_some()
    }

    /// Rates the session that just ended, while the rating window is open.
    fn rate(&mut self, focus: i32) {
        let Some((id, _)) = self.rating.filter(|_| self.rating_open()) else {
            return;
        };
        match self.db.set_focus(id, focus) {
            Err(e) => self.set_error(e),
            Ok(()) => {
                self.clear_error();
                self.rating = None;
                self.react(Reaction::Rated(focus));
                self.reload_today();
            }
        }
    }

    /// Undoes the last session change: deletes the sessions it created and
    /// reopens the one it ended, so ending by mistake (or to add a last
    /// note) costs one key. Only the last change can be undone.
    fn undo(&mut self) {
        let Some(change) = self.undo.take() else {
            return;
        };
        for &id in &change.created {
            if let Err(e) = self.db.delete_session(id) {
                self.set_error(e);
                return;
            }
        }
        if let Some(id) = change.ended
            && let Err(e) = self.db.reopen_session(id)
        {
            self.set_error(e);
            return;
        }
        self.clear_error();
        self.rating = None;
        self.reaction = None;
        match self.db.get_active_session() {
            Ok(session) => self.active_session = session,
            Err(e) => self.set_error(e),
        }
        self.refresh_after_session_change();
    }

    /// Appends a note to the active session (db plus in-memory mirror). No-op
    /// if there is no active session.
    fn append_active_note(&mut self, note: &str) {
        let Some(id) = self.active_session.as_ref().map(|s| s.id) else {
            return;
        };
        match self.db.append_note(id, note) {
            Err(e) => self.set_error(e),
            Ok(()) => {
                self.clear_error();
                if let Some(session) = &mut self.active_session {
                    // Newline-terminated, same convention as the db.
                    session.notes = Some(match &session.notes {
                        None => format!("{note}\n"),
                        Some(existing) => format!("{existing}{note}\n"),
                    });
                }
            }
        }
    }

    /// Handles one key event. Returns true if the app should quit.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        // Some terminals (e.g. on Windows) also report releases and repeats;
        // act on presses only so each key fires once.
        if key.kind != KeyEventKind::Press {
            return false;
        }
        // Ctrl+C quits from any screen; other Ctrl combinations are ignored
        // rather than typed into input fields as plain letters.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return key.code == KeyCode::Char('c');
        }
        let key = key.code;
        // Back from an absence the idle poll hasn't seen end yet: ask about
        // it first; this key only brings the question up.
        if self.away_since.is_some() && self.came_back() {
            return false;
        }
        match &self.screen {
            Screen::Main => return self.handle_main(key),
            Screen::Help => self.screen = Screen::Main, // any key closes it
            Screen::StartSession(_) => self.handle_start_session(key),
            Screen::AddNote(_) => self.handle_add_note(key),
            Screen::Palette(_) => return self.handle_palette(key),
            Screen::Period(_) => self.handle_period(key),
            Screen::Away(since) => {
                let since = *since;
                self.handle_away(key, since);
            }
        }
        false
    }

    // --- away: catching sessions left running -------------------------------

    // --- moving the start ---------------------------------------------------

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
    fn reload_today(&mut self) {
        match self.db.sessions_for_today() {
            Ok(sessions) => self.today_sessions = sessions,
            Err(e) => self.set_error(e),
        }
    }
    /// Handles keys for the main screen. Returns true if the app should quit.
    fn handle_main(&mut self, key: KeyCode) -> bool {
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
            KeyCode::Char('s') => {
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
            KeyCode::Char('[') => self.move_start(-START_STEP),
            KeyCode::Char(']') => self.move_start(START_STEP),
            _ => {}
        };
        false
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

    // --- the longer views ----------------------------------------------------

    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();

        // Global chrome: header on top, status line at the bottom, screen
        // content in between. The rows are fixed (no conditional layout), so
        // the UI never jumps when an error appears or disappears.
        let [header, content, status] = chrome_layout(area);

        frame.render_widget(self.header_line(), header);

        let input = self.wrapped_input(content);
        let input = input.as_ref();
        match &self.screen {
            Screen::Main => self.draw_main(frame, content),
            Screen::Help => draw_help(frame, content),
            Screen::StartSession(form) => self.draw_start_session(frame, content, form, input),
            Screen::AddNote(_) => self.draw_add_note(frame, content, input),
            Screen::Palette(form) => draw_palette(frame, content, form, input),
            Screen::Period(view) => self.draw_period(frame, view),
            Screen::Away(since) => self.draw_away(frame, content, *since),
        }

        frame.render_widget(self.status_line(status.width), status);
    }

    /// Terminal cursor position for the current screen: `Some` on input lines
    /// (start form's project/task steps, add-note),
    /// right after the typed text. Pure function of the terminal size and form
    /// state — `Terminal::draw` discards closure return values, so main.rs
    /// calls this after drawing to show or hide the cursor.
    pub fn cursor_position(&self, size: (u16, u16)) -> Option<(u16, u16)> {
        let [_, content, _] = chrome_layout(ratatui::layout::Rect::new(0, 0, size.0, size.1));
        let input = self.wrapped_input(content)?;
        let input_area = match &self.screen {
            Screen::StartSession(_) | Screen::Palette(_) => {
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
            _ => None,
        }
    }

    /// The current screen's text input wrapped to its modal's width, with at
    /// most as many rows as the modal can grow to on `content`. Shared by
    /// `draw` and `cursor_position` so the two always agree.
    fn wrapped_input(&self, content: ratatui::layout::Rect) -> Option<WrappedInput> {
        let (value, placeholder) = self.text_input()?;
        let base_height = match self.screen {
            Screen::StartSession(_) | Screen::Palette(_) => START_MODAL_HEIGHT,
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
    fn header_line(&self) -> Line<'static> {
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
    fn total_today(&self) -> chrono::Duration {
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

    /// Bottom row: the red error when the last db operation failed, otherwise
    /// key hints for the current screen, as many as fit in `width`. The only
    /// place errors and key hints are rendered.
    fn status_line(&self, width: u16) -> Line<'static> {
        if let Some(err) = &self.error {
            return Line::from(Span::styled(format!("✗ {err}"), theme::ERROR));
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

    // --- main screen: mouse -----------------------------------------------

    // --- main screen: geometry --------------------------------------------

    /// Records the terminal size. main.rs calls this every frame, before
    /// drawing, so handlers lay the main screen out exactly as drawn.
    pub fn set_size(&mut self, size: (u16, u16)) {
        self.size = size;
    }

    // --- main screen: drawing ---------------------------------------------

    /// The three sections: the active session on top, today's sessions in
    /// the middle (scrollable), the day summary at the bottom. The middle
    /// and bottom ones collapse to a single line; all are resizable.
    fn draw_main(&self, frame: &mut Frame, area: Rect) {
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

    // --- the cat ------------------------------------------------------------

    /// A project's color on the main screen, gray for no project. Colors
    /// go by the order projects were first used, so each keeps its color
    /// from day to day, and are made distinct across today's projects (the
    /// main screen's view; see `theme::project_colors`).
    fn project_color(&self, project: Option<&str>) -> Color {
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
    fn project_rank(&self, project: &str) -> usize {
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
    fn active_lines(&self) -> Vec<Line<'static>> {
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
    fn timeline_lines(&self) -> Vec<Line<'static>> {
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

    fn day_summary(&self) -> DaySummary {
        summarize(
            &self.today_sessions,
            self.active_session.as_ref(),
            Utc::now(),
        )
    }

    /// Body of the summary, wrapped to `width`: time per activity and per
    /// project, focus and interruptions, and the day's span, with graphs of
    /// the day when they fit (`summary_graphs_fit`).
    fn summary_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.summary_body(width, self.summary_graphs_fit(width))
    }

    /// The summary's body without graphs.
    fn summary_text_lines(&self, width: u16) -> Vec<Line<'static>> {
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
    fn summary_one_liner(&self) -> String {
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

        let mut lines = vec![
            Line::from(vec![
                Span::styled(format!("{h:02}:{m:02}:{s:02}"), theme::TITLE),
                Span::styled(
                    format!("  since {}", fmt_time(&session.started_at)),
                    theme::DIMMED,
                ),
            ]),
            theme::kv(
                "Project",
                vec![self.project_span(session.project.as_deref())],
            ),
            theme::kv("Task", vec![value_or_none(&session.task, Style::new())]),
            theme::kv("Activity", vec![theme::activity(&session.activity)]),
        ];

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
    fn note_lines(&self, width: u16) -> Vec<Line<'static>> {
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

/// The status-line hint for the help screen; the main screen always shows it.
const HELP_HINT: (&str, &str) = ("?", "help");

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
fn text_width(inner_width: u16) -> u16 {
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

/// The global chrome: header row, content area, status row. Shared by `draw`
/// and `cursor_position`.
fn chrome_layout(area: ratatui::layout::Rect) -> [ratatui::layout::Rect; 3] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area)
}

// --- summary pieces, shared by today's summary and the longer views -------

/// The "Previous notes:" / "Session notes:" headings.
const NOTES_HEADING: Style = theme::DIMMED.add_modifier(Modifier::BOLD);
