//! All of the UI: `App`, the state the screens share, and the session
//! lifecycle every screen goes through (start, close, rate, undo, notes).
//! `handle_key` and `draw` dispatch on `Screen` to the per-screen files.

use std::path::PathBuf;
use std::time::Instant;

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;

use crate::db::Db;
use crate::layout::{LayoutPrefs, Section};
use crate::mascot::Reaction;
use crate::session::{self, BREAK, DISRUPTION, MEETING, Session};
use crate::settings::Settings;

use away::Away;
use cat::fresh_seed;
use chrome::chrome_layout;
use forms::{AddNoteForm, StartSessionForm};
use help::draw_help;
use mood::MoodForm;
use palette::{PaletteForm, draw_palette};
use period_view::PeriodView;
use rename::RenameForm;
use rewind::{PastDay, Rewind};
use settings_screen::SettingsForm;

mod away;
mod cat;
mod chrome;
mod forms;
mod graphs;
mod help;
mod items;
mod main_screen;
mod modal;
mod mood;
mod palette;
mod period_view;
mod pomodoro;
mod rename;
mod rewind;
#[cfg(test)]
mod screenshot;
mod sections;
mod settings_screen;
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
    /// Settings, saved to `settings_path` (when set) whenever they change.
    settings: Settings,
    settings_path: Option<PathBuf>,
    /// Where `heartbeat` notes that Jiary is running, and when it last did.
    seen_path: Option<PathBuf>,
    seen_written: Option<Instant>,
    /// A short message for the status line and when it was given; it shows
    /// for `NOTICE_TIME` (say, after turning sleep and mood off).
    notice: Option<(String, Instant)>,
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

/// Where the app keeps its files beside the db; `None` keeps that one in
/// memory only (tests).
#[derive(Clone, Debug, Default)]
pub struct Files {
    /// The main screen's layout (`layout.conf`).
    pub layout: Option<PathBuf>,
    /// The settings (`settings.conf`).
    pub settings: Option<PathBuf>,
    /// When Jiary was last running (`last_seen`), for a session left
    /// running while it was closed.
    pub seen: Option<PathBuf>,
}

/// How long after a session ends its focus can be rated. Short on purpose:
/// a rating given right away is the honest one.
const RATING_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a notice stays in the status line.
const NOTICE_TIME: std::time::Duration = std::time::Duration::from_secs(4);

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
    /// A week, month or year (`/weekly`, `/monthly`, `/yearly`).
    Period(PeriodView),
    /// Renaming or merging a label (`/rename`); boxed like the start form.
    Rename(Box<RenameForm>),
    /// Today's sleep and mood: before the day's first session, or `/mood`.
    Mood(MoodForm),
    /// `/settings`.
    Settings(SettingsForm),
    /// Every past day, searched (`/rewind`); boxed: it holds them all.
    Rewind(Box<Rewind>),
    /// A day opened from `/rewind`, with the list to go back to.
    PastDay(PastDay),
    /// Back after `AWAY_AFTER` or more without touching the computer
    /// during a work session, or opening Jiary on a session left running
    /// while it was closed: what was that time?
    Away(Away),
}

impl App {
    /// `files` says where the layout, the settings and when Jiary was last
    /// running are loaded from and saved to.
    pub fn new(db: Db, files: Files) -> Self {
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
            layout_path: files.layout,
            settings: Settings::default(),
            settings_path: files.settings,
            seen_path: files.seen,
            seen_written: None,
            notice: None,
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
        if let Some(path) = &app.settings_path {
            match Settings::load(path) {
                Ok(settings) => app.settings = settings,
                Err(e) => app.set_error(format!("Could not read settings: {e}")),
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
        app.ask_if_left_running(Utc::now());

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

    /// Shows `text` in the status line for a moment.
    fn notify(&mut self, text: &str) {
        self.notice = Some((text.to_string(), Instant::now()));
    }

    /// Saves the settings (when they have a file), right after a change.
    /// Returns false, with the error shown, when that fails.
    fn save_settings(&mut self) -> bool {
        if let Some(path) = &self.settings_path
            && let Err(e) = self.settings.save(path)
        {
            self.set_error(format!("Could not save settings: {e}"));
            return false;
        }
        true
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
            Screen::Rename(_) => self.handle_rename(key),
            Screen::Mood(_) => self.handle_mood(key),
            Screen::Settings(_) => self.handle_settings(key),
            Screen::Rewind(_) => self.handle_rewind(key),
            Screen::PastDay(_) => self.handle_past_day(key),
            Screen::Away(away) => {
                let since = away.since;
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
            Screen::Rename(form) => self.draw_rename(frame, content, form, input),
            Screen::Mood(form) => self.draw_mood(frame, content, form),
            Screen::Settings(form) => self.draw_settings(frame, content, form),
            Screen::Rewind(view) => self.draw_rewind(frame, content, view),
            Screen::PastDay(day) => self.draw_past_day(frame, day),
            Screen::Away(away) => self.draw_away(frame, content, *away),
        }

        frame.render_widget(self.status_line(status.width), status);
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

// --- summary pieces, shared by today's summary and the longer views -------
