use std::path::PathBuf;
use std::time::Instant;

use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, HighlightSpacing, List, ListItem, ListState, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Tabs,
};

use crate::captions;
use crate::db::{Db, SessionCombo};
use crate::layout::{Areas, ContentHeights, LayoutPrefs, Section};
use crate::mascot::{self, Mood, Reaction};
use crate::period::Period;
use crate::session::{self, BREAK, DISRUPTION, MEETING, Session};
use crate::summary::{DaySummary, summarize};
use crate::theme;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

#[cfg(test)]
mod tests;

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

/// After this long in a work session the cat gets drowsy, a nudge to take a
/// break. (A candidate for a user setting.)
const DROWSY_AFTER_MINUTES: i64 = 25;

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

/// How long nobody has to touch the computer during a work session before
/// coming back asks what that time was.
const AWAY_AFTER: chrono::TimeDelta = chrono::TimeDelta::minutes(15);

/// How far `[` and `]` move the active session's start.
const START_STEP: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

/// How many recent activity/project/task picks the start form offers.
const RECENT_PICKS: usize = 5;

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

/// A week or a month on screen: which one (`offset` periods from the
/// current one), its days, its sessions as loaded when it opened (draw
/// never queries the db) and how far it is scrolled.
struct PeriodView {
    period: Period,
    offset: i32,
    first: chrono::NaiveDate,
    last: chrono::NaiveDate,
    sessions: Vec<Session>,
    scroll: usize,
}

/// The command palette's state: what was typed (always starting with the
/// `/` that opened it) and the highlighted match.
struct PaletteForm {
    input: String,
    selection: usize,
}

/// A command the palette offers: its name (typed after `/`), what it does,
/// and what it runs.
struct Command {
    name: &'static str,
    description: &'static str,
    action: Action,
    /// The key as shown beside the command (none for the views, which have
    /// no key).
    key_label: &'static str,
}

/// What a command runs. Commands for existing actions run their
/// main-screen key, so a command can never drift from its key.
#[derive(Clone, Copy)]
enum Action {
    Key(KeyCode),
    View(Period),
}

const fn command(
    name: &'static str,
    description: &'static str,
    key: char,
    key_label: &'static str,
) -> Command {
    Command {
        name,
        description,
        action: Action::Key(KeyCode::Char(key)),
        key_label,
    }
}

const fn view(name: &'static str, description: &'static str, period: Period) -> Command {
    Command {
        name,
        description,
        action: Action::View(period),
        key_label: "",
    }
}

/// Everything the palette offers, in the order it lists them.
const COMMANDS: &[Command] = &[
    view("weekly", "this week, by day", Period::Week),
    view("monthly", "this month, by day", Period::Month),
    view("yearly", "this year's focus", Period::Year),
    command("start", "start a session", 's', "s"),
    command("end", "end the session", 'e', "e"),
    command("note", "add a note", 'n', "n"),
    command("interrupt", "count interruption", 'i', "i"),
    command("disrupt", "start a disruption", 'd', "d"),
    command("break", "take a break", 'b', "b"),
    command("meeting", "join a meeting", 'm', "m"),
    command("resume", "back to work", 'r', "r"),
    command("undo", "undo start or end", 'u', "u"),
    command("earlier", "start 5m earlier", '[', "["),
    command("later", "start 5m later", ']', "]"),
    command("reset", "reset section sizes", '=', "="),
    command("help", "show all keys", '?', "?"),
    command("quit", "quit Jiary", 'q', "q"),
];

/// The commands matching `query` (fuzzy, on the name), best first; all of
/// them, in order, for an empty query.
fn palette_matches(query: &str) -> Vec<&'static Command> {
    if query.is_empty() {
        return COMMANDS.iter().collect();
    }
    let matcher = SkimMatcherV2::default();
    let mut scored = COMMANDS
        .iter()
        .filter_map(|c| matcher.fuzzy_match(c.name, query).map(|score| (score, c)))
        .collect::<Vec<_>>();
    // Stable: ties keep the list's order.
    scored.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
    scored.into_iter().map(|(_, c)| c).collect()
}

struct AddNoteForm {
    input: String,
}

struct StartSessionForm {
    step: Step,
    /// Recent combinations, started with `1`–`5` while the activity input
    /// is empty.
    recent: Vec<SessionCombo>,
    activity: Autocomplete,
    project: Autocomplete,
    task: Autocomplete,
}

/// A free-text input with fuzzy suggestions: the start form's project and
/// task steps. What is shown in the input line is always what Enter submits.
struct Autocomplete {
    /// The field's value: what was typed, or the highlighted suggestion.
    input: String,
    /// What was typed; suggestions are filtered by this, not by `input`.
    query: String,
    /// The highlighted suggestion, if any. `None` means the typed text is
    /// the value and nothing in the list is highlighted.
    selection: Option<usize>,
    candidates: Vec<String>,
}

impl Autocomplete {
    fn new(candidates: Vec<String>) -> Self {
        Self {
            input: String::new(),
            query: String::new(),
            selection: None,
            candidates,
        }
    }

    fn suggestions(&self) -> Vec<String> {
        fuzzy_filter(&self.query, &self.candidates)
    }

    /// Tab/↓ select the next suggestion (Tab wraps, ↓ stops at the end), ↑
    /// the previous one and, past the top, returns to the typed text. Typing
    /// clears the selection and continues from what the input shows, so
    /// accepting a suggestion and then typing extends it.
    fn handle_key(&mut self, key: KeyCode) {
        let suggestions = self.suggestions();
        let selection = match (key, self.selection) {
            (KeyCode::Tab, Some(i)) => Some((i + 1) % suggestions.len()),
            (KeyCode::Down, Some(i)) => Some((i + 1).min(suggestions.len() - 1)),
            (KeyCode::Tab | KeyCode::Down, None) if !suggestions.is_empty() => Some(0),
            (KeyCode::Up, Some(i)) => i.checked_sub(1),
            (KeyCode::Char(c), _) => {
                self.input.push(c);
                self.query = self.input.clone();
                None
            }
            (KeyCode::Backspace, _) => {
                self.input.pop();
                self.query = self.input.clone();
                None
            }
            _ => return,
        };
        self.selection = selection;
        self.input = match selection {
            Some(i) => suggestions[i].clone(),
            None => self.query.clone(),
        };
    }

    /// The submitted value; blank input becomes `None` so the db stores
    /// NULL, not "".
    fn value(&self) -> Option<String> {
        let value = self.input.trim();
        (!value.is_empty()).then(|| value.to_string())
    }
}

#[derive(Clone, Copy)]
enum Step {
    Activity,
    Project,
    Task,
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

    /// Takes in the system idle time (`None` when the platform can't tell),
    /// polled every `idle::POLL` by `main`. Once nobody has touched the
    /// computer for `AWAY_AFTER`, the absence is remembered from when it
    /// started; on return (input again, or a key in the app) a work
    /// session still running from before then asks what that time was. A
    /// gap of `AWAY_AFTER` between polls means the computer slept, which
    /// counts as away too, from the last poll before it.
    pub fn observe_idle(&mut self, idle: Option<std::time::Duration>, now: chrono::DateTime<Utc>) {
        if let Some(last) = self.last_observed.replace(now)
            && now - last >= AWAY_AFTER
        {
            self.away_since = Some(self.away_since.map_or(last, |since| since.min(last)));
        }
        let idle = idle.and_then(|idle| chrono::TimeDelta::from_std(idle).ok());
        match idle {
            Some(idle) if idle >= AWAY_AFTER => {
                let since = now - idle;
                self.away_since = Some(self.away_since.map_or(since, |s| s.min(since)));
            }
            // Input again (or no way to tell, after a sleep): back.
            _ if self.away_since.is_some() => {
                self.came_back();
            }
            _ => {}
        }
    }

    /// Ends an absence: asks about it when a work session ran through it
    /// and the main screen is up (not to throw away a half-filled form).
    /// Returns whether it asked.
    fn came_back(&mut self) -> bool {
        let Some(since) = self.away_since.take() else {
            return false;
        };
        let asks = matches!(self.screen, Screen::Main)
            && self
                .active_session
                .as_ref()
                .is_some_and(|s| !session::is_detour(&s.activity) && s.started_at < since);
        if asks {
            self.screen = Screen::Away(since);
        }
        asks
    }

    /// The away question: `k` (or Esc) keeps the time as it was, `b` / `d`
    /// turn it into a break / disruption and pick the work back up, `e`
    /// ends the session when the absence began.
    fn handle_away(&mut self, key: KeyCode, since: chrono::DateTime<Utc>) {
        match key {
            KeyCode::Char('k') | KeyCode::Esc => {}
            KeyCode::Char('e') => {
                self.close_active_session_at(since);
            }
            KeyCode::Char(c @ ('b' | 'd')) => {
                let detour = if c == 'b' { BREAK } else { DISRUPTION };
                let Some(session) = self.active_session.as_ref() else {
                    return;
                };
                let (activity, project, task) = (
                    session.activity.clone(),
                    session.project.clone(),
                    session.task.clone(),
                );
                if !self.close_active_session_at(since) {
                    return;
                }
                match self.db.insert_finished_session(detour, since, Utc::now()) {
                    Ok(id) => self.undo.get_or_insert_default().created.push(id),
                    Err(e) => {
                        self.set_error(e);
                        return;
                    }
                }
                self.start_session(&activity, project, task);
                self.refresh_after_session_change();
            }
            _ => return,
        }
        self.screen = Screen::Main;
    }

    /// Like `close_active_session`, but the session ends at `at` (in the
    /// past), then the main screen refreshes.
    fn close_active_session_at(&mut self, at: chrono::DateTime<Utc>) -> bool {
        let Some(id) = self.active_session.as_ref().map(|s| s.id) else {
            return false;
        };
        if let Err(e) = self.db.complete_session_at(id, at) {
            self.set_error(e);
            return false;
        }
        // Already ended in the db, so this records the change (undo, the
        // rating window) without ending it again.
        let closed = self.close_active_session();
        self.refresh_after_session_change();
        closed
    }

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

    /// Handles keys while starting a session (activity → project → task).
    fn handle_start_session(&mut self, key: KeyCode) {
        if key == KeyCode::Esc {
            self.screen = Screen::Main;
            return;
        }

        // Read the step by copy so the borrow of self.screen drops before the
        // per-step handler takes &mut self.
        let step = match &self.screen {
            Screen::StartSession(form) => form.step,
            _ => return,
        };
        match step {
            Step::Activity => self.start_activity_key(key),
            Step::Project => self.start_project_key(key),
            Step::Task => self.start_task_key(key),
        }
    }

    fn start_activity_key(&mut self, key: KeyCode) {
        let Screen::StartSession(form) = &mut self.screen else {
            return;
        };
        match key {
            // Activity is required (the db column is NOT NULL), so Enter only
            // advances once the input holds one.
            KeyCode::Enter => {
                if form.activity.value().is_some() {
                    form.step = Step::Project;
                }
            }
            // With nothing typed, a digit starts that recent pick.
            KeyCode::Char(c @ '1'..='9')
                if form.activity.input.is_empty()
                    && let Some(combo) = form.recent.get(c as usize - '1' as usize) =>
            {
                let (activity, project, task) = combo.clone();
                self.begin(&activity, project, task);
            }
            _ => form.activity.handle_key(key),
        }
    }

    fn start_project_key(&mut self, key: KeyCode) {
        let Screen::StartSession(form) = &mut self.screen else {
            return;
        };
        match key {
            KeyCode::Enter => {
                let candidates = match form.project.value() {
                    None => Ok(self.tasks.clone()),
                    Some(project) => self.db.tasks_for_project(&project),
                };
                // `form` is dead from here, so self can be borrowed again.
                let candidates = candidates.unwrap_or_else(|e| {
                    self.set_error(e);
                    Vec::new()
                });
                if let Screen::StartSession(form) = &mut self.screen {
                    form.task = Autocomplete::new(candidates);
                    form.step = Step::Task;
                }
            }
            _ => form.project.handle_key(key),
        }
    }

    fn start_task_key(&mut self, key: KeyCode) {
        let Screen::StartSession(form) = &mut self.screen else {
            return;
        };
        if key != KeyCode::Enter {
            form.task.handle_key(key);
            return;
        }

        // Copy everything out of `form` before any call that needs &mut
        // self, so the screen borrow is dead.
        let Some(activity) = form.activity.value() else {
            return; // unreachable: the Activity step requires one
        };
        let project = form.project.value();
        let task = form.task.value();
        self.begin(&activity, project, task);
    }

    /// Submits the start form: ends the active session and starts this one.
    fn begin(&mut self, activity: &str, project: Option<String>, task: Option<String>) {
        // Auto-stop any currently active session. If that fails, abort:
        // creating anyway would leave two active sessions (the db trigger
        // rejects it, but failing early surfaces the real error instead of
        // the generic one).
        if !self.close_active_session() {
            return;
        }

        self.start_session(activity, project, task);

        self.refresh_after_session_change();
        self.refresh_suggestions();
        self.screen = Screen::Main;
    }

    /// Handles keys in the command palette: typing filters, ↑↓ pick, Tab
    /// completes the pick's name, Enter runs it, Esc (or Backspace on the
    /// bare `/`) closes. Returns true if the command quits the app.
    fn handle_palette(&mut self, key: KeyCode) -> bool {
        let Screen::Palette(form) = &mut self.screen else {
            return false;
        };
        let matches = palette_matches(&form.input[1..]);
        match key {
            KeyCode::Esc => self.screen = Screen::Main,
            KeyCode::Backspace if form.input == "/" => self.screen = Screen::Main,
            KeyCode::Backspace => {
                form.input.pop();
                form.selection = 0;
            }
            KeyCode::Char(c) => {
                form.input.push(c);
                form.selection = 0;
            }
            KeyCode::Up => form.selection = form.selection.saturating_sub(1),
            KeyCode::Down => {
                form.selection = (form.selection + 1).min(matches.len().saturating_sub(1));
            }
            KeyCode::Tab => {
                if let Some(command) = matches.get(form.selection) {
                    form.input = format!("/{}", command.name);
                    form.selection = 0;
                }
            }
            KeyCode::Enter => {
                if let Some(command) = matches.get(form.selection) {
                    self.screen = Screen::Main;
                    match command.action {
                        Action::Key(key) => return self.handle_main(key),
                        Action::View(period) => self.open_period(period, 0),
                    }
                }
            }
            _ => {}
        }
        false
    }

    // --- the longer views ----------------------------------------------------

    /// Opens the week or month `offset` periods from the current one,
    /// loading its sessions.
    fn open_period(&mut self, period: Period, offset: i32) {
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
    fn handle_period(&mut self, key: KeyCode) {
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
    fn period_lines(&self, view: &PeriodView, width: u16) -> Vec<Line<'static>> {
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
    fn draw_period(&self, frame: &mut Frame, view: &PeriodView) {
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

    /// Handles keys while adding a note to the active session.
    fn handle_add_note(&mut self, key: KeyCode) {
        let Screen::AddNote(form) = &mut self.screen else {
            return;
        };
        match key {
            KeyCode::Char(c) => form.input.push(c),
            KeyCode::Backspace => {
                form.input.pop();
            }
            KeyCode::Enter => {
                if !form.input.is_empty() {
                    let note = form.input.clone();
                    self.append_active_note(&note);
                    self.react(Reaction::Noted);
                    // Bring the new note into view.
                    self.notes_scroll = self.max_scroll(Section::Active);
                }
                self.screen = Screen::Main;
            }
            // Cancel: the typed note is discarded.
            KeyCode::Esc => self.screen = Screen::Main,
            _ => {}
        }
    }

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

    /// Handles a mouse event; only the main screen uses the mouse. A press
    /// focuses the section under the pointer. Dragging the line between two
    /// sections resizes; clicking a collapsible section's title text (press
    /// and release without moving) collapses or expands it. The wheel
    /// scrolls today's sessions.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.screen, Screen::Main) {
            return;
        }
        let [_, content, _] = chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        let areas = self.resolve_layout(content);
        let (column, row) = (mouse.column, mouse.row);

        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                // The section under the pointer; elsewhere, today's sessions.
                let pointer = ratatui::layout::Position::new(column, row);
                let target = if areas.active.contains(pointer) {
                    Section::Active
                } else {
                    Section::Today
                };
                let delta = if mouse.kind == MouseEventKind::ScrollUp {
                    -3
                } else {
                    3
                };
                self.scroll(target, delta);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let position = ratatui::layout::Position::new(column, row);
                if let Some(section) = [Section::Active, Section::Today, Section::Summary]
                    .into_iter()
                    .find(|&s| areas.get(s).contains(position))
                {
                    self.focus = section;
                }
                // A title can sit on a border row, so a press may be both;
                // the release (or a move) decides which it was.
                self.drag = self.border_at(&areas, row);
                self.title_press = [Section::Today, Section::Summary]
                    .into_iter()
                    .find(|&s| self.title_hit(s, areas.get(s), column, row));
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.title_press = None; // moved: not a click
                let Some((section, grabbed_row)) = self.drag else {
                    return;
                };
                self.focus = section;
                // Where the upper section's bottom border should now be.
                let border = row.saturating_sub(grabbed_row) as i32;
                let heights = self.content_heights(content.width);
                match section {
                    Section::Active => {
                        let height = border - areas.active.y as i32 + 1;
                        self.layout.set_active_height(height, content, heights);
                    }
                    _ => {
                        let height = content.bottom() as i32 - border - 1;
                        self.layout.set_summary_height(height, content, heights);
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let toggled = self.title_press.take();
                if let Some(section) = toggled {
                    self.layout.toggle_collapsed(section);
                }
                if self.drag.take().is_some() || toggled.is_some() {
                    self.save_layout();
                }
            }
            _ => {}
        }
    }

    /// The draggable line between two sections on `row`, as (the section
    /// whose height it sets, which of its rows: 0 or 1). The line is two
    /// rows (the upper section's bottom border and the lower one's top
    /// border) and either grabs it. The line under the active section sets
    /// the active section's height; the one under today's sessions sets the
    /// summary's, only while both are expanded (otherwise the summary just
    /// fills the space).
    fn border_at(&self, areas: &Areas, row: u16) -> Option<(Section, u16)> {
        let line = |upper: Rect, lower: Rect, lower_expanded: bool| {
            if upper.height > 0 && row + 1 == upper.bottom() {
                Some(0)
            } else if lower_expanded && lower.height > 0 && row == lower.y {
                Some(1)
            } else {
                None
            }
        };
        let today_expanded = !self.layout.today_collapsed;
        if let Some(grabbed) = line(areas.active, areas.today, today_expanded) {
            return Some((Section::Active, grabbed));
        }
        if today_expanded && !self.layout.summary_collapsed {
            return line(areas.today, areas.summary, true).map(|g| (Section::Summary, g));
        }
        None
    }

    /// The title of a section as drawn: in its top border when expanded, the
    /// whole row when collapsed. Shared with `title_hit`.
    fn section_title(&self, section: Section) -> String {
        let sessions = plural(self.today_sessions.len(), "session");
        match (section, self.layout.collapsed(section)) {
            (Section::Active, _) => " ACTIVE ".to_string(),
            (Section::Today, false) => format!(" ▾ TODAY · {sessions} "),
            (Section::Today, true) => {
                format!("▸ TODAY · {sessions} · {}", fmt_hms(self.total_today()))
            }
            (Section::Summary, false) => " ▾ SUMMARY ".to_string(),
            (Section::Summary, true) => format!("▸ {}", self.summary_one_liner()),
        }
    }

    /// Whether (`column`, `row`) is on `section`'s title text: right after
    /// the border's corner when expanded, from the left edge when collapsed.
    fn title_hit(&self, section: Section, area: Rect, column: u16, row: u16) -> bool {
        if area.height == 0 || row != area.y {
            return false;
        }
        let start = area.x + if self.layout.collapsed(section) { 0 } else { 1 };
        let width = self.section_title(section).chars().count() as u16;
        (start..start + width).contains(&column)
    }

    // --- main screen: geometry --------------------------------------------

    /// Records the terminal size. main.rs calls this every frame, before
    /// drawing, so handlers lay the main screen out exactly as drawn.
    pub fn set_size(&mut self, size: (u16, u16)) {
        self.size = size;
    }

    fn main_areas(&self) -> Areas {
        let [_, content, _] = chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        self.resolve_layout(content)
    }

    /// The main screen's sections. While a session can be rated, a row
    /// below the active section is kept for the rating bar: the sections
    /// below move down and the one filling the rest gives up the row.
    fn resolve_layout(&self, content: Rect) -> Areas {
        let heights = self.content_heights(content.width);
        if !self.rating_open() {
            return self.layout.resolve(content, heights);
        }
        let shorter = Rect {
            height: content.height.saturating_sub(1),
            ..content
        };
        let mut areas = self.layout.resolve(shorter, heights);
        areas.today.y += 1;
        areas.summary.y += 1;
        areas
    }

    /// Content-fitting heights of the active section and the summary,
    /// borders included.
    fn content_heights(&self, width: u16) -> ContentHeights {
        ContentHeights {
            active: self.active_content_height(width),
            summary: self.summary_lines(width.saturating_sub(2)).len() as u16 + 2,
        }
    }

    /// The active section's content-fitting height, borders included: the
    /// text column (info, then notes) or the cat column, whichever is
    /// taller. The cat column keeps its caption rows even without a caption,
    /// so the layout doesn't jump when one appears.
    fn active_content_height(&self, width: u16) -> u16 {
        let inner_width = width.saturating_sub(2);
        let notes = self.note_lines(text_width(inner_width)).len();
        let text = (self.active_lines().len() + notes) as u16;
        let cat = if cat_fits(inner_width) {
            CAT_BLOCK_HEIGHT
        } else {
            0
        };
        text.max(cat) + 2
    }

    /// Whether the summary (its interior `width` columns wide) shows its
    /// graphs: only when, with them, today's sessions keep
    /// `TODAY_ROOM_FOR_GRAPHS` rows, so on a short terminal the graphs give
    /// way rather than squeezing today's list or clipping the summary.
    fn summary_graphs_fit(&self, width: u16) -> bool {
        if (width.saturating_sub(SUMMARY_LABEL_WIDTH) as usize) < MIN_GRAPH_WIDTH {
            return false;
        }
        let [_, content, _] = chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        let active = (self.layout.active_height)
            .unwrap_or_else(|| self.active_content_height(content.width));
        let summary = self.summary_text_lines(width).len() as u16 + 2 + GRAPH_ROWS;
        let today = if self.layout.today_collapsed {
            crate::layout::COLLAPSED_HEIGHT
        } else {
            TODAY_ROOM_FOR_GRAPHS
        };
        content.height >= active + summary + today
    }

    /// The section the scroll keys act on: the focused one if it scrolls,
    /// else today's sessions.
    fn scroll_target(&self) -> Section {
        match self.focus {
            Section::Active => Section::Active,
            _ => Section::Today,
        }
    }

    fn scroll_position(&mut self, section: Section) -> &mut usize {
        match section {
            Section::Active => &mut self.notes_scroll,
            _ => &mut self.today_scroll,
        }
    }

    /// Scrollable rows of a section: the active session's notes, or
    /// today's sessions.
    fn scroll_lines(&self, section: Section) -> usize {
        match section {
            Section::Active => {
                let inner_width = self.main_areas().active.width.saturating_sub(2);
                self.note_lines(text_width(inner_width)).len()
            }
            _ => self.timeline_lines().len(),
        }
    }

    /// Rows of a section's scrollable part visible on screen.
    fn viewport(&self, section: Section) -> usize {
        let areas = self.main_areas();
        match section {
            Section::Active => {
                // The notes fill the text column below the session info.
                let inner = areas.active.height.saturating_sub(2) as usize;
                inner.saturating_sub(self.active_lines().len())
            }
            _ if self.layout.today_collapsed => 0,
            _ => areas.today.height.saturating_sub(2) as usize,
        }
    }

    fn max_scroll(&self, section: Section) -> usize {
        self.scroll_lines(section)
            .saturating_sub(self.viewport(section))
    }

    fn scroll(&mut self, section: Section, delta: isize) {
        let max = self.max_scroll(section);
        let position = self.scroll_position(section);
        *position = position.saturating_add_signed(delta).min(max);
    }

    fn resize_focused(&mut self, delta: i32) {
        let [_, content, _] = chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        let heights = self.content_heights(content.width);
        self.layout.resize(self.focus, delta, content, heights);
        self.save_layout();
    }

    /// Writes the layout file, if there is one. A failure only costs the
    /// layout, so it is reported without stopping anything.
    fn save_layout(&mut self) {
        let result = self.layout_path.as_ref().map(|path| self.layout.save(path));
        if let Some(Err(e)) = result {
            self.set_error(format!("Could not save layout: {e}"));
        }
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

    /// What the cat does for the current session: idle with none, asleep on
    /// a break, chatting in a meeting, gaming in a disruption, and writing
    /// in a work session (drowsy once it runs past `DROWSY_AFTER_MINUTES`).
    fn mood(&self) -> Mood {
        let Some(session) = &self.active_session else {
            return Mood::Idle;
        };
        match session.activity.as_str() {
            BREAK => Mood::Sleeping,
            MEETING => Mood::Meeting,
            DISRUPTION => Mood::Gaming,
            _ if Utc::now() - session.started_at
                >= chrono::Duration::minutes(DROWSY_AFTER_MINUTES) =>
            {
                Mood::Drowsy
            }
            _ => Mood::Working,
        }
    }

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

    /// The caption under the cat for the current mood and session
    /// activity, if its list has any.
    fn caption(&self) -> Option<&'static str> {
        if matches!(self.current_reaction(), Some((Reaction::Flinch, _))) {
            return Some(captions::FLINCH);
        }
        if self.rating_open() {
            return Some(captions::RATING);
        }
        let activity = self.active_session.as_ref().map(|s| s.activity.as_str());
        captions::caption(self.mood(), activity, self.caption_seed)
    }

    /// Picks another sentence for the caption: a different one when the
    /// list has more than one (a few random tries).
    fn resample_caption(&mut self) {
        let before = self.caption();
        for _ in 0..8 {
            self.caption_seed = fresh_seed();
            if self.caption() != before {
                break;
            }
        }
    }

    /// Starts a one-off reaction, replacing any still playing.
    fn react(&mut self, reaction: Reaction) {
        self.reaction = Some((reaction, Instant::now()));
    }

    /// The reaction still playing, with how far into it we are.
    fn current_reaction(&self) -> Option<(Reaction, u64)> {
        let (reaction, started) = self.reaction?;
        let elapsed = started.elapsed().as_millis() as u64;
        (elapsed < reaction.duration()).then_some((reaction, elapsed))
    }

    /// The cat's current frame: a reaction while one plays, else its mood
    /// (animated by the wall clock, so every app instance stays in step).
    fn cat_lines(&self) -> Vec<Line<'static>> {
        match self.current_reaction() {
            Some((reaction, elapsed)) => mascot::reaction_lines(reaction, elapsed),
            None => {
                let millis = Utc::now().timestamp_millis().max(0) as u64;
                mascot::mood_lines(self.mood(), millis)
            }
        }
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

    /// The start form is a centered fixed-size modal: a `Tabs` row for
    /// Activity·Project·Task with the current step selected, then the step's
    /// content: a wrapped input plus its suggestion `List`.
    fn draw_start_session(
        &self,
        frame: &mut Frame,
        area: ratatui::layout::Rect,
        form: &StartSessionForm,
        input: Option<&WrappedInput>,
    ) {
        let rows = input.map_or(0, WrappedInput::height);
        let (modal, tabs_area, input_area, list_area) = start_modal_layout(area, rows);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(Span::styled(" Start session ", theme::TITLE)));
        frame.render_widget(block, modal);

        let step_index = match form.step {
            Step::Activity => 0,
            Step::Project => 1,
            Step::Task => 2,
        };
        frame.render_widget(
            Tabs::new(["Activity", "Project", "Task"])
                .select(step_index)
                .highlight_style(Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
            tabs_area,
        );

        match form.step {
            Step::Activity if form.activity.input.is_empty() && !form.recent.is_empty() => {
                draw_recent(frame, list_area, &form.recent);
            }
            Step::Activity => draw_suggestions(frame, list_area, &form.activity),
            Step::Project => draw_suggestions(frame, list_area, &form.project),
            Step::Task => draw_suggestions(frame, list_area, &form.task),
        }
        if let Some(input) = input {
            frame.render_widget(input.paragraph(), input_area);
        }
    }

    /// The away question: a small modal saying since when, with the
    /// answers.
    fn draw_away(&self, frame: &mut Frame, area: Rect, since: chrono::DateTime<Utc>) {
        let (modal, top_area, body_area) = small_modal_layout(area, 4);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(Span::styled(" Welcome back ", theme::TITLE)));
        frame.render_widget(block, modal);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(
                    "Away since {} ({})",
                    fmt_time(&since),
                    fmt_duration(&since, &Utc::now())
                ),
                theme::DIMMED,
            ))),
            top_area,
        );
        let activity = self
            .active_session
            .as_ref()
            .map_or("the session", |s| s.activity.as_str());
        let key =
            |k: &str| Span::styled(format!("[{k}] "), Style::new().add_modifier(Modifier::BOLD));
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![key("k"), Span::raw(format!("count it as {activity}"))]),
                Line::from(vec![key("b"), Span::raw("it was a break")]),
                Line::from(vec![key("d"), Span::raw("it was a disruption")]),
                Line::from(vec![
                    key("e"),
                    Span::raw(format!("end it at {}", fmt_time(&since))),
                ]),
            ]),
            body_area,
        );
    }

    /// Add-note form: small centered modal with a dimmed project·task·activity
    /// context line (the full note history stays on the main screen) and a
    /// note input (cursor placed by main.rs).
    fn draw_add_note(
        &self,
        frame: &mut Frame,
        area: ratatui::layout::Rect,
        input: Option<&WrappedInput>,
    ) {
        let rows = input.map_or(1, WrappedInput::height);
        let (modal, context_area, input_area) = small_modal_layout(area, rows);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(Span::styled(" Add note ", theme::TITLE)));
        frame.render_widget(block, modal);

        if let Some(session) = &self.active_session {
            let parts = [
                session.project.as_deref().unwrap_or(""),
                session.task.as_deref().unwrap_or(""),
                &session.activity,
            ]
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ");
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(parts, theme::DIMMED))),
                context_area,
            );
        }
        if let Some(input) = input {
            frame.render_widget(input.paragraph(), input_area);
        }
    }
}

/// Fixed size of the modals; centered in the content area and shrunk to fit
/// on tiny terminals (see `centered_rect`). The start form needs room for the
/// suggestion list; end/note forms only hold a tabs/context line plus one input.
const MODAL_WIDTH: u16 = 40;
const START_MODAL_HEIGHT: u16 = 12;
const SMALL_MODAL_HEIGHT: u16 = 6;
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

/// The command palette: a modal like the start form's, with the matches
/// under the input as `name  description  key`, the pick highlighted (the
/// list scrolls to keep it in view), and above them how many match.
fn draw_palette(frame: &mut Frame, area: Rect, form: &PaletteForm, input: Option<&WrappedInput>) {
    let rows = input.map_or(1, WrappedInput::height);
    let (modal, count_area, input_area, list_area) = start_modal_layout(area, rows);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(Span::styled(" Commands ", theme::TITLE)));
    frame.render_widget(block, modal);

    let matches = palette_matches(&form.input[1..]);
    let count = match matches.len() {
        0 => "no matching command".to_string(),
        n if n == COMMANDS.len() => plural(n, "command"),
        n => format!("{n} of {}", plural(COMMANDS.len(), "command")),
    };
    frame.render_widget(
        Paragraph::new(Span::styled(count, theme::DIMMED)),
        count_area,
    );
    if let Some(input) = input {
        frame.render_widget(input.paragraph(), input_area);
    }

    // Indented two columns so names line up under the typed name (after
    // "› /"), with the highlight symbol under the "/".
    let list_area = Rect {
        x: list_area.x + 2,
        width: list_area.width.saturating_sub(2),
        ..list_area
    };
    // Name and key columns; the description takes what is left after them
    // and the highlight symbol's column.
    let name_width = COMMANDS.iter().map(|c| c.name.len()).max().unwrap_or(0) + 2;
    let width = (list_area.width as usize).saturating_sub(1);
    let items = matches
        .iter()
        .map(|c| {
            let description_width = width.saturating_sub(name_width + c.key_label.len() + 1);
            let description: String = c.description.chars().take(description_width).collect();
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:name_width$}", c.name), theme::VALUE),
                Span::styled(format!("{description:description_width$} "), theme::DIMMED),
                Span::styled(c.key_label, theme::DIMMED.add_modifier(Modifier::BOLD)),
            ]))
        })
        .collect::<Vec<_>>();
    frame.render_stateful_widget(
        picker_list(items),
        list_area,
        &mut ListState::default().with_selected((!matches.is_empty()).then_some(form.selection)),
    );
}

/// A picker `List` with the accent highlight symbol and style.
/// The symbol column is always reserved, so items don't shift right when
/// one gets selected.
fn picker_list<'a>(items: Vec<ListItem<'a>>) -> List<'a> {
    List::new(items)
        .highlight_symbol(theme::picker_symbol())
        .highlight_style(theme::PICKER_HIGHLIGHT)
        .highlight_spacing(HighlightSpacing::Always)
}

/// An autocomplete step's suggestion list, with the selected suggestion (if
/// any) highlighted. The input itself is drawn as a `WrappedInput`; the list
/// is indented one column so suggestion text lines up under the typed text
/// (after the `›` prompt) and the `>` marker under the prompt's space.
fn draw_suggestions(frame: &mut Frame, area: ratatui::layout::Rect, field: &Autocomplete) {
    let area = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    };
    let items = field
        .suggestions()
        .into_iter()
        .map(ListItem::new)
        .collect::<Vec<_>>();
    if !items.is_empty() {
        frame.render_stateful_widget(
            picker_list(items),
            area,
            &mut ListState::default().with_selected(field.selection),
        );
    }
}

/// The start form's recent picks, numbered for their keys, as
/// `activity · project · task`.
fn draw_recent(frame: &mut Frame, area: Rect, recent: &[SessionCombo]) {
    let lines = recent
        .iter()
        .enumerate()
        .map(|(i, (activity, project, task))| {
            let parts = [Some(activity.as_str()), project.as_deref(), task.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            Line::from(vec![
                Span::styled(
                    format!("[{}] ", i + 1),
                    Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(parts),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);
}

/// Columns the cat takes beside the session text: its canvas plus a gap on
/// each side.
const CAT_COLUMNS: u16 = mascot::WIDTH + 2;

/// Whether the cat fits beside at least this much session text in an active
/// section `inner_width` columns wide.
fn cat_fits(inner_width: u16) -> bool {
    const MIN_TEXT_WIDTH: u16 = 24;
    inner_width >= MIN_TEXT_WIDTH + CAT_COLUMNS
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

/// A scrollbar in `track` (a border column) when `total` rows don't fit in
/// `visible`; nothing otherwise.
fn draw_scrollbar(frame: &mut Frame, track: Rect, total: usize, visible: usize, offset: usize) {
    if total <= visible || track.height == 0 {
        return;
    }
    let mut state = ScrollbarState::new(total.saturating_sub(visible))
        .position(offset)
        .viewport_content_length(visible);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_style(Style::new().fg(theme::ACCENT)),
        track,
        &mut state,
    );
}

/// Rows kept under the cat for its caption (wrapped to the cat's width),
/// after a blank gap row so the caption doesn't crowd the cat.
const CAPTION_GAP: u16 = 1;
const CAPTION_ROWS: u16 = 2;

/// The cat column's height: the cat, the gap and the caption rows.
const CAT_BLOCK_HEIGHT: u16 = mascot::HEIGHT + CAPTION_GAP + CAPTION_ROWS;

/// Splits the active section's interior into the session text and, when it
/// fits, the cat on the right (top-aligned, one column in from the border)
/// with its caption rows below. The cat is left out rather than squeezing
/// the text below `MIN_TEXT_WIDTH` or being cut off vertically; the caption
/// is left out when only the cat fits.
fn split_for_cat(inner: Rect) -> (Rect, Option<Rect>, Option<Rect>) {
    if !cat_fits(inner.width) || inner.height < mascot::HEIGHT {
        return (inner, None, None);
    }
    let text = Rect {
        width: inner.width - CAT_COLUMNS,
        ..inner
    };
    let cat = Rect::new(
        inner.right() - mascot::WIDTH - 1,
        inner.y,
        mascot::WIDTH,
        mascot::HEIGHT,
    );
    let caption = (inner.height >= CAT_BLOCK_HEIGHT).then(|| Rect {
        y: cat.bottom() + CAPTION_GAP,
        height: CAPTION_ROWS,
        ..cat
    });
    (text, Some(cat), caption)
}

/// The caption wrapped to the cat's canvas `width`, each row centred under
/// the cat's own columns in `mood` (the scenes aren't centred on the canvas:
/// a laptop or a curled tail would pull it to one side), or as close as the
/// canvas allows for rows too wide to centre there.
fn caption_lines(caption: &str, mood: Mood, width: u16) -> Vec<Line<'static>> {
    let (left, right) = mood.columns();
    wrap_text(caption, width as usize)
        .iter()
        .map(|row| row.trim())
        .filter(|row| !row.is_empty())
        .map(|row| {
            let len = row.chars().count() as u16;
            let start = ((left + right + 1).saturating_sub(len) / 2).min(width.saturating_sub(len));
            Line::from(vec![
                Span::raw(" ".repeat(start as usize)),
                Span::styled(row.to_string(), theme::CAPTION),
            ])
        })
        .collect()
}

/// A random seed for picking captions, from std's randomly keyed hasher
/// (each `RandomState` gets new keys). Not the clock: on macOS it only
/// ticks in microseconds, so its nanoseconds are always a multiple of 1000
/// and would pick the same sentence from a list of 4 (or 8) all second.
fn fresh_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
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

/// A fixed-size rect centered in `area`, shrunk to fit when the area is too
/// small so tiny terminals don't break the layout.
fn centered_rect(width: u16, height: u16, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
    let y = area
        .y
        .saturating_add(area.height.saturating_sub(height) / 2);
    ratatui::layout::Rect::new(x, y, width.min(area.width), height.min(area.height))
}

/// A centered modal of the given size: (modal rect, content area inside the
/// border with a one-column padding).
fn modal_layout(
    content: ratatui::layout::Rect,
    width: u16,
    height: u16,
) -> (ratatui::layout::Rect, ratatui::layout::Rect) {
    let modal = centered_rect(width, height, content);
    let inner = Block::bordered().inner(modal);
    let padded = ratatui::layout::Rect::new(
        inner.x.saturating_add(1),
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    (modal, padded)
}

/// Layout of the start-session modal: (modal rect, tabs row, input rows,
/// list). The modal grows by one row per input row beyond the first, so the
/// list keeps its room. Shared by the
/// renderer and `cursor_position` so the two can never disagree.
fn start_modal_layout(
    content: ratatui::layout::Rect,
    input_rows: u16,
) -> (
    ratatui::layout::Rect,
    ratatui::layout::Rect,
    ratatui::layout::Rect,
    ratatui::layout::Rect,
) {
    let height = START_MODAL_HEIGHT + input_rows.saturating_sub(1);
    let (modal, inner) = modal_layout(content, MODAL_WIDTH, height);
    let [tabs_area, input_area, list_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(input_rows),
        Constraint::Min(0),
    ])
    .areas(inner);
    (modal, tabs_area, input_area, list_area)
}

/// Layout of the end/note modals: (modal rect, tabs/context row, body). The
/// body holds `body_rows` rows (the wrapped input, or the rating line) and
/// the modal grows to fit them. Shared by the renderers and
/// `cursor_position`.
fn small_modal_layout(
    content: ratatui::layout::Rect,
    body_rows: u16,
) -> (
    ratatui::layout::Rect,
    ratatui::layout::Rect,
    ratatui::layout::Rect,
) {
    let height = SMALL_MODAL_HEIGHT + body_rows.saturating_sub(1);
    let (modal, inner) = modal_layout(content, MODAL_WIDTH, height);
    let [top_area, body_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    (modal, top_area, body_area)
}

/// Text width inside any modal on `content` (border plus padding on each
/// side), accounting for `centered_rect` shrinking narrow terminals.
fn modal_inner_width(content: ratatui::layout::Rect) -> u16 {
    MODAL_WIDTH.min(content.width).saturating_sub(4)
}

/// A text input wrapped to fit its modal: an accent `›` prompt on the first
/// row, the value word-wrapped after it with later rows indented to line up,
/// a dimmed placeholder while empty, and the cursor right after the last
/// character.
struct WrappedInput {
    lines: Vec<Line<'static>>,
    /// Cursor (column, row), relative to the input area.
    cursor: (u16, u16),
}

impl WrappedInput {
    /// Wraps `value` after the prompt within `width` columns. When that
    /// takes more than `max_rows` rows, only the last ones are kept: the
    /// cursor is always on the last row, so it stays visible.
    fn new(value: &str, placeholder: &str, width: u16, max_rows: u16) -> Self {
        const PROMPT: &str = "› ";
        let indent = PROMPT.chars().count();
        let rows = wrap_text(value, (width as usize).saturating_sub(indent));
        let last = rows.len() - 1;
        let cursor_col = indent + rows[last].chars().count();

        let mut lines = rows
            .into_iter()
            .enumerate()
            .map(|(i, row)| match i {
                0 if value.is_empty() => Line::from(vec![
                    Span::styled(PROMPT, theme::PROMPT),
                    Span::styled(placeholder.to_string(), theme::PLACEHOLDER),
                ]),
                0 => Line::from(vec![Span::styled(PROMPT, theme::PROMPT), Span::raw(row)]),
                _ => Line::from(format!("{}{row}", " ".repeat(indent))),
            })
            .collect::<Vec<_>>();
        let hidden = lines.len().saturating_sub(max_rows as usize);
        lines.drain(..hidden);

        Self {
            lines,
            cursor: (cursor_col as u16, (last - hidden) as u16),
        }
    }

    fn height(&self) -> u16 {
        self.lines.len() as u16
    }

    fn paragraph(&self) -> Paragraph<'static> {
        Paragraph::new(self.lines.clone())
    }

    /// Absolute cursor position when drawn at `area`, kept inside it on
    /// terminals too narrow for the modal.
    fn cursor_in(&self, area: ratatui::layout::Rect) -> (u16, u16) {
        let (col, row) = self.cursor;
        (area.x + col.min(area.width.saturating_sub(1)), area.y + row)
    }
}

/// `label` beside the first row (as `theme::kv` draws it), the other rows
/// indented to line up under the first row's value.
fn hanging_lines(label: &str, rows: Vec<Vec<Span<'static>>>) -> Vec<Line<'static>> {
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
type Item = Vec<Span<'static>>;

/// Packs `items` into rows joined by a dimmed " · ", for display beside
/// `label` within `width` columns. An item only wraps (mid-item, via
/// `wrap_text`, losing its styling) when it is wider than a whole row.
fn wrap_items(items: Vec<Item>, width: u16, label: &str) -> Vec<Item> {
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
const SUMMARY_LABEL_WIDTH: u16 = 10;
/// The columns a day's total and focus take at the end of its row in the
/// longer views (" 10h 30m 2.5").
const DAY_TOTAL_WIDTH: usize = 12;
/// Narrower than this, the summary's graphs are left out.
const MIN_GRAPH_WIDTH: usize = 10;
/// Rows the graphs add to the summary: the focus line, the timeline, its
/// hours, the two share bars, and a blank row between each of the four
/// groups.
const GRAPH_ROWS: u16 = 5 + 3;
/// Rows today's sessions keep for the summary to show its graphs: about two
/// sessions.
const TODAY_ROOM_FOR_GRAPHS: u16 = 8;

/// A row of `halves.len() / 2` cells, two colors per cell: `▌` takes the
/// left half's color as foreground and the right one's as background, like
/// the cat's `▀` does for top and bottom. `None` leaves a half blank.
fn half_cells(halves: &[Option<Color>]) -> Item {
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
fn timeline_row(
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
fn focus_row(
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
fn hour_labels(start: chrono::DateTime<Utc>, end: chrono::DateTime<Utc>, width: usize) -> Item {
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
fn start_of_hour(at: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
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
fn share_row(shares: &[(Color, chrono::Duration)], width: usize) -> Item {
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

// --- summary pieces, shared by today's summary and the longer views -------

/// How a view colors projects (by name; `None`: no project).
type ProjectColors<'a> = &'a dyn Fn(Option<&str>) -> Color;

/// A project's name, bold in `color`, or a dimmed italic "(none)".
fn project_span(project: Option<&str>, color: Color) -> Span<'static> {
    match project {
        Some(name) => Span::styled(name.to_string(), theme::VALUE.fg(color)),
        None => Span::styled("(none)", theme::NONE),
    }
}

/// Each activity with its time, the name in its color, longest first.
fn activity_items(summary: &DaySummary) -> Vec<Item> {
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
fn project_items(summary: &DaySummary, colors: ProjectColors) -> Vec<Item> {
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
fn focus_items(summary: &DaySummary) -> Vec<Item> {
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

/// The period's days as a grid of squares, GitHub style: a row per
/// weekday, a column per week (headed by its Monday's date), each day
/// colored by its average focus on `theme::FOCUS_RAMP`; dark gray when
/// tracked but unrated, a faint texture when nothing was tracked, blank
/// outside the period and after `today`. Its legend is `focus_legend`.
fn focus_grid(
    days: &[(chrono::NaiveDate, DaySummary)],
    first: chrono::NaiveDate,
    last: chrono::NaiveDate,
    today: chrono::NaiveDate,
) -> Vec<Item> {
    use chrono::Datelike;
    const WEEKDAY: usize = 4; // "Mon "
    let monday = first - chrono::Duration::days(first.weekday().num_days_from_monday() as i64);
    let weeks = ((last - monday).num_days() / 7 + 1) as usize;
    let week = |w: usize| monday + chrono::Duration::weeks(w as i64);

    let mut header = vec![Span::raw(" ".repeat(WEEKDAY))];
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
        let mut row = vec![Span::styled(
            name,
            if today_row {
                theme::TITLE
            } else {
                theme::DIMMED
            },
        )];
        for w in 0..weeks {
            let day = week(w) + chrono::Duration::days(weekday);
            let summary = days.iter().find(|(d, _)| *d == day).map(|(_, s)| s);
            let cell = match summary {
                _ if day < first || day > last || day > today => Span::raw("  "),
                Some(s) if s.average_focus.is_some() => Span::styled(
                    "██",
                    Style::new().fg(theme::focus_gradient(s.average_focus.unwrap_or(2.0))),
                ),
                Some(s) if !s.tracked.is_zero() => {
                    Span::styled("██", Style::new().fg(theme::UNRATED))
                }
                _ => Span::styled("░░", Style::new().fg(theme::UNRATED)),
            };
            row.push(cell);
            if w + 1 < weeks {
                row.push(Span::raw(" "));
            }
        }
        rows.push(row);
    }
    rows
}

/// A day's (or month's) tracked time and focus average, at the end of its
/// row in the longer views (`DAY_TOTAL_WIDTH` columns).
fn day_total(summary: &DaySummary) -> Item {
    vec![
        Span::styled(format!(" {:>7}", fmt_hms(summary.tracked)), theme::VALUE),
        match summary.average_focus {
            Some(avg) => Span::styled(format!(" {avg:.1}"), theme::VALUE.fg(focus_color(avg))),
            None => Span::styled("   -", theme::DIMMED),
        },
    ]
}

/// A year's focus grid, compact enough for 53 weeks in 80 columns: a
/// column per week, the month names over them, and the weekdays two to a
/// row, each day half a row tall (`▀`: the top day in front, the bottom one
/// behind), so the squares stay square without gaps. Colors as in
/// `focus_grid`, with `theme::EMPTY_DAY` where nothing was tracked.
fn year_focus_grid(
    days: &[(chrono::NaiveDate, DaySummary)],
    first: chrono::NaiveDate,
    last: chrono::NaiveDate,
    today: chrono::NaiveDate,
) -> Vec<Item> {
    use chrono::Datelike;
    const WEEKDAY: usize = 4; // "Mon "
    let monday = first - chrono::Duration::days(first.weekday().num_days_from_monday() as i64);
    let weeks = ((last - monday).num_days() / 7 + 1) as usize;
    let date = |week: usize, weekday: u32| {
        monday + chrono::Duration::weeks(week as i64) + chrono::Duration::days(weekday as i64)
    };

    // Month names over the week each month starts in, where they fit.
    let mut header = vec![' '; WEEKDAY + weeks];
    let mut free = WEEKDAY;
    for month in first
        .iter_days()
        .take_while(|d| *d <= last)
        .filter(|d| d.day() == 1)
    {
        let column = WEEKDAY + ((month - monday).num_days() / 7) as usize;
        let name = month.format("%b").to_string();
        if column >= free && column + name.len() <= header.len() {
            header[column..column + name.len()]
                .iter_mut()
                .zip(name.chars())
                .for_each(|(cell, c)| *cell = c);
            free = column + name.len() + 1;
        }
    }
    let mut rows = vec![vec![Span::styled(
        header.into_iter().collect::<String>(),
        theme::DIMMED,
    )]];

    let color = |day: chrono::NaiveDate| -> Option<Color> {
        if day < first || day > last || day > today {
            return None;
        }
        let summary = days.iter().find(|(d, _)| *d == day).map(|(_, s)| s)?;
        Some(match summary.average_focus {
            Some(avg) => theme::focus_gradient(avg),
            None if !summary.tracked.is_zero() => theme::UNRATED,
            None => theme::EMPTY_DAY,
        })
    };
    for top in [0, 2, 4, 6] {
        let name = date(0, top).format("%a ").to_string();
        let mut row = vec![Span::styled(name, theme::DIMMED)];
        for week in 0..weeks {
            let upper = color(date(week, top));
            let lower = if top < 6 {
                color(date(week, top + 1))
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
fn focus_legend() -> Item {
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
fn item_width(row: &Item) -> usize {
    row.iter().map(Span::width).sum()
}

/// `row`, part of a block `block_width` wide, centered on `width` columns:
/// every row of the block gets the same left padding, so they stay aligned.
fn centered(row: Item, block_width: usize, width: u16) -> Line<'static> {
    let pad = (width as usize).saturating_sub(block_width) / 2;
    Line::from([vec![Span::raw(" ".repeat(pad))], row].concat())
}

/// The color of a focus average: its place on the focus grid's ramp, so
/// a number and its day's square always match.
fn focus_color(average: f64) -> Color {
    theme::focus_gradient(average)
}

/// Tracked time, then what it was made of (work, breaks, disruptions),
/// each part's time in the color the graphs give it; parts that didn't
/// happen are left out, keeping the row short.
fn time_split_items(summary: &DaySummary) -> Vec<Item> {
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

/// Each activity's share of the tracked time as one bar.
fn activity_bar(summary: &DaySummary, width: usize) -> Item {
    let shares = summary
        .per_activity
        .iter()
        .map(|(activity, d)| (theme::activity_color(activity), *d))
        .collect::<Vec<_>>();
    share_row(&shares, width)
}

/// Each project's share of the tracked time as one bar.
fn project_bar(summary: &DaySummary, width: usize, colors: ProjectColors) -> Item {
    let shares = summary
        .per_project
        .iter()
        .map(|(project, d)| (colors(project.as_deref()), *d))
        .collect::<Vec<_>>();
    share_row(&shares, width)
}

/// A duration in bold `color`.
fn colored_time(duration: chrono::Duration, color: Color) -> Span<'static> {
    Span::styled(fmt_hms(duration), theme::VALUE.fg(color))
}

/// A value, or a dimmed italic "(none)" when it is missing.
fn value_or_none(value: &Option<String>, style: Style) -> Span<'static> {
    match value {
        Some(value) => Span::styled(value.clone(), style),
        None => Span::styled("(none)", theme::NONE),
    }
}

/// An interruption count after `prefix`: bold red when there were any,
/// dimmed otherwise.
fn interruptions_span(count: i32, prefix: &str) -> Span<'static> {
    let style = if count > 0 {
        theme::VALUE.fg(theme::BAD)
    } else {
        theme::DIMMED
    };
    Span::styled(format!("{prefix}{count}"), style)
}

/// The "Previous notes:" / "Session notes:" headings.
const NOTES_HEADING: Style = theme::DIMMED.add_modifier(Modifier::BOLD);

/// The help screen: every key, in groups with an accent heading and a rule
/// under it. Two columns (sessions; sections and mouse) keep it short
/// enough for a 24-row terminal with room for spacing. A centered modal
/// sized to its content, shrunk on small terminals.
fn draw_help(frame: &mut Frame, content: Rect) {
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

/// Splits `text` into rows of at most `width` chars, breaking after the last
/// space that fits, or mid-word when a word is longer than a row. Every char
/// is kept, so the rows concatenate back to `text`. A trailing empty row is
/// added when the last one is full, so the cursor after it has a place to go.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let chars = text.chars().collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut start = 0;
    while chars.len() - start > width {
        let window = &chars[start..start + width];
        let fits_exactly = chars[start + width] == ' ';
        let end = match window.iter().rposition(|&c| c == ' ') {
            Some(space) if !fits_exactly => start + space + 1,
            _ => start + width,
        };
        rows.push(chars[start..end].iter().collect());
        start = end;
    }
    let last = chars[start..].iter().collect::<String>();
    let full = chars.len() - start == width;
    rows.push(last);
    if full {
        rows.push(String::new());
    }
    rows
}

/// "1 session", "3 sessions".
fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

fn fmt_time(dt: &chrono::DateTime<chrono::Utc>) -> String {
    dt.with_timezone(&chrono::Local).format("%H:%M").to_string()
}
fn fmt_duration(
    start: &chrono::DateTime<chrono::Utc>,
    end: &chrono::DateTime<chrono::Utc>,
) -> String {
    fmt_hms(*end - *start)
}
/// "1h 24m" or "9m" — per-session durations and the header total.
fn fmt_hms(dur: chrono::Duration) -> String {
    let h = dur.num_hours();
    let m = dur.num_minutes() % 60;
    if h > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}m", m)
    }
}
fn fuzzy_filter(query: &str, candidates: &[String]) -> Vec<String> {
    if query.is_empty() {
        return candidates.to_vec();
    }
    let matcher = SkimMatcherV2::default();
    let mut scored: Vec<(i64, &String)> = candidates
        .iter()
        .filter_map(|c| matcher.fuzzy_match(c, query).map(|s| (s, c)))
        .collect();
    scored.sort_by_key(|a| std::cmp::Reverse(a.0));
    scored.into_iter().take(5).map(|(_, c)| c.clone()).collect()
}
