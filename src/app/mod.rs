use std::path::PathBuf;
use std::time::Instant;

use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::db::Db;
use crate::layout::{LayoutPrefs, Section};
use crate::mascot::Reaction;
use crate::session::{self, BREAK, DISRUPTION, MEETING, Session};
use crate::theme;
use cat::fresh_seed;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use forms::{AddNoteForm, StartSessionForm, Step};
use help::draw_help;
use modal::{
    SMALL_MODAL_HEIGHT, START_MODAL_HEIGHT, WrappedInput, modal_inner_width, picker_list,
    small_modal_layout, start_modal_layout,
};
use palette::{PaletteForm, draw_palette};
use period_view::PeriodView;
use text::fmt_hms;

mod away;
mod cat;
mod forms;
mod graphs;
mod help;
mod items;
mod main_screen;
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

    // --- the cat ------------------------------------------------------------
}

/// The status-line hint for the help screen; the main screen always shows it.
const HELP_HINT: (&str, &str) = ("?", "help");

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
