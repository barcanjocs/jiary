use std::path::PathBuf;
use std::time::Instant;

use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, HighlightSpacing, List, ListItem, ListState, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Tabs,
};

use crate::captions;
use crate::db::Db;
use crate::layout::{Areas, ContentHeights, LayoutPrefs, Section};
use crate::mascot::{self, Mood, Reaction};
use crate::session::{self, BREAK, DISRUPTION, MEETING, Session};
use crate::summary::{DaySummary, summarize};
use crate::theme;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

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
}

/// After this long in a work session the cat gets drowsy, a nudge to take a
/// break. (A candidate for a user setting.)
const DROWSY_AFTER_MINUTES: i64 = 25;

/// How long after a session ends its focus can be rated. Short on purpose:
/// a rating given right away is the honest one.
const RATING_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// A session change, as the sessions it ended and started (either may be
/// missing: `e` only ends one, starting from nothing only starts one).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Undo {
    ended: Option<i64>,
    started: Option<i64>,
}

enum Screen {
    Main,
    Help,
    // Boxed: three autocompletes make it far larger than the other variants.
    StartSession(Box<StartSessionForm>),
    AddNote(AddNoteForm),
}

struct AddNoteForm {
    input: String,
}

struct StartSessionForm {
    step: Step,
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
                        started: None,
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
                self.undo.get_or_insert_default().started = Some(session.id);
                self.active_session = Some(session);
            }
            Err(e) => self.set_error(e),
        }
    }

    /// Whether `1`–`3` still rate the session that just ended.
    fn rating_open(&self) -> bool {
        self.rating
            .is_some_and(|(_, ended)| ended.elapsed() < RATING_WINDOW)
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
                match self.db.sessions_for_today() {
                    Ok(sessions) => self.today_sessions = sessions,
                    Err(e) => self.set_error(e),
                }
            }
        }
    }

    /// Undoes the last session change: deletes the session it started and
    /// reopens the one it ended, so ending by mistake (or to add a last
    /// note) costs one key. Only the last change can be undone.
    fn undo(&mut self) {
        let Some(change) = self.undo.take() else {
            return;
        };
        if let Some(id) = change.started
            && let Err(e) = self.db.delete_session(id)
        {
            self.set_error(e);
            return;
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
        match &self.screen {
            Screen::Main => return self.handle_main(key),
            Screen::Help => self.screen = Screen::Main, // any key closes it
            Screen::StartSession(_) => self.handle_start_session(key),
            Screen::AddNote(_) => self.handle_add_note(key),
        }
        false
    }
    /// Handles keys for the main screen. Returns true if the app should quit.
    fn handle_main(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Char('q') => return true,
            KeyCode::Char('?') => self.screen = Screen::Help,

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
                self.screen = Screen::StartSession(Box::new(StartSessionForm {
                    step: Step::Activity,
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

        // Auto-stop any currently active session. If that fails, abort:
        // creating anyway would leave two active sessions (the db trigger
        // rejects it, but failing early surfaces the real error instead of
        // the generic one).
        if !self.close_active_session() {
            return;
        }

        self.start_session(&activity, project, task);

        self.refresh_after_session_change();
        self.refresh_suggestions();
        self.screen = Screen::Main;
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
            Screen::StartSession(_) => start_modal_layout(content, input.height()).2,
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
            _ => None,
        }
    }

    /// The current screen's text input wrapped to its modal's width, with at
    /// most as many rows as the modal can grow to on `content`. Shared by
    /// `draw` and `cursor_position` so the two always agree.
    fn wrapped_input(&self, content: ratatui::layout::Rect) -> Option<WrappedInput> {
        let (value, placeholder) = self.text_input()?;
        let base_height = match self.screen {
            Screen::StartSession(_) => START_MODAL_HEIGHT,
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
            Span::styled(
                fmt_hms(self.total_today()),
                Style::new().add_modifier(Modifier::BOLD),
            ),
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
            Screen::AddNote(_) => &[("Enter", "save"), ("Esc", "cancel")],
        };
        // Right after a session ends, rating it and undoing come first,
        // with the seconds left to rate.
        let countdown;
        let mut hints = hints.to_vec();
        if matches!(self.screen, Screen::Main)
            && let Some((_, ended)) = self.rating.filter(|_| self.rating_open())
        {
            let left = RATING_WINDOW.saturating_sub(ended.elapsed()).as_secs_f32();
            countdown = format!("rate bad/ok/good {}s", left.ceil() as u32);
            hints.splice(0..0, [("1/2/3", countdown.as_str()), ("u", "undo")]);
        }
        let hints = &hints[..];
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

    fn resolve_layout(&self, content: Rect) -> Areas {
        self.layout
            .resolve(content, self.content_heights(content.width))
    }

    /// Content-fitting heights of the active section and the summary,
    /// borders included.
    fn content_heights(&self, width: u16) -> ContentHeights {
        let inner_width = width.saturating_sub(2);
        // The text column (info, then notes) or the cat column, whichever is
        // taller. The cat column keeps its caption rows even without a
        // caption, so the layout doesn't jump when one appears.
        let notes = self.note_lines(text_width(inner_width)).len();
        let text = (self.active_lines().len() + notes) as u16;
        let cat = if cat_fits(inner_width) {
            CAT_BLOCK_HEIGHT
        } else {
            0
        };
        let active = text.max(cat);
        ContentHeights {
            active: active + 2,
            summary: self.summary_lines(width.saturating_sub(2)).len() as u16 + 2,
        }
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

    /// The caption under the cat for the current mood and session
    /// activity, if its list has any.
    fn caption(&self) -> Option<&'static str> {
        let activity = self.active_session.as_ref().map(|s| s.activity.as_str());
        captions::caption(self.mood(), activity, self.caption_seed)
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
    /// project, focus and interruptions, and the day's span.
    fn summary_lines(&self, width: u16) -> Vec<Line<'static>> {
        let summary = self.day_summary();
        let Some((start, end)) = summary.span else {
            return vec![Line::from(Span::styled(
                "Nothing tracked yet today.",
                theme::DIMMED,
            ))];
        };

        let activities = summary
            .per_activity
            .iter()
            .map(|(activity, d)| format!("{activity} {}", fmt_hms(*d)))
            .collect::<Vec<_>>();
        let projects = summary
            .per_project
            .iter()
            .map(|(project, d)| {
                format!("{} {}", project.as_deref().unwrap_or("(none)"), fmt_hms(*d))
            })
            .collect::<Vec<_>>();
        let focus = vec![
            match summary.average_focus {
                Some(avg) => format!("avg {avg:.1} ({} rated)", summary.rated),
                None => "not rated".to_string(),
            },
            plural(summary.interruptions as usize, "interruption"),
            match summary.disruptions {
                0 => "no disruptions".to_string(),
                n => format!(
                    "{} ({})",
                    plural(n, "disruption"),
                    fmt_hms(summary.disruption_time)
                ),
            },
        ];
        let day = vec![
            format!("{} – {}", fmt_time(&start), fmt_time(&end)),
            format!("tracked {}", fmt_hms(summary.tracked)),
            if summary.break_time.is_zero() {
                "no breaks".to_string()
            } else {
                format!("breaks {}", fmt_hms(summary.break_time))
            },
            format!("gaps {}", fmt_hms(summary.untracked())),
        ];

        let mut lines = Vec::new();
        for (label, items) in [
            ("Activity", activities),
            ("Project ", projects),
            ("Focus   ", focus),
            ("Day     ", day),
        ] {
            lines.extend(hanging_lines(label, wrap_items(&items, width, label)));
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
            Line::from(Span::styled(
                format!("{h:02}:{m:02}:{s:02}"),
                Style::new().add_modifier(Modifier::BOLD),
            )),
            theme::kv("Project", session.project.as_deref().unwrap_or("(none)")),
            theme::kv("Task", session.task.as_deref().unwrap_or("(none)")),
            theme::kv("Activity", session.activity.as_str()),
        ];

        let interruptions = session.interruptions.unwrap_or(0);
        lines.push(Line::from(vec![
            Span::styled("Interruptions  ", theme::DIMMED),
            Span::styled(
                interruptions.to_string(),
                if interruptions > 0 {
                    Style::new().fg(theme::BAD)
                } else {
                    Style::new()
                },
            ),
        ]));
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
            lines.push(Line::from(Span::styled("Previous notes:", theme::DIMMED)));
            // Each entry is one session's notes: newline-separated, one
            // bullet per line.
            for note in self.previous_notes.iter().flat_map(|notes| notes.lines()) {
                lines.extend(bullet_lines(note, width));
            }
        }

        // Current session's own notes.
        if let Some(notes) = &session.notes
            && !notes.is_empty()
        {
            lines.push(Line::from(Span::styled("Session notes:", theme::DIMMED)));
            for note in notes.lines() {
                lines.extend(bullet_lines(note, width));
            }
        }

        lines
    }

    /// One timeline item: time range, duration, focus badge and interruption
    /// count on the first line, project·task·activity below it, then notes.
    fn timeline_item_lines(&self, session: &Session) -> Vec<Line<'static>> {
        let start = fmt_time(&session.started_at);
        let (range, duration) = match &session.ended_at {
            Some(end) => (
                format!("{start} – {}", fmt_time(end)),
                fmt_duration(&session.started_at, end),
            ),
            // The active session appears here too, still running.
            None => (
                format!("{start} – now"),
                fmt_hms(Utc::now() - session.started_at),
            ),
        };

        let focus = match session.focus {
            Some(1) => Span::styled(" F1", Style::new().fg(theme::BAD)),
            Some(2) => Span::styled(" F2", Style::new().fg(theme::OK)),
            Some(3) => Span::styled(" F3", Style::new().fg(theme::GOOD)),
            _ => Span::styled(" F-", theme::DIMMED),
        };

        let interruptions = session.interruptions.unwrap_or(0);
        let interruptions_span = Span::styled(
            format!(" I:{interruptions}"),
            if interruptions > 0 {
                Style::new().fg(theme::BAD)
            } else {
                theme::DIMMED
            },
        );

        let mut lines = vec![Line::from(vec![
            Span::styled(range, Style::new().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(" ({duration})"),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            focus,
            interruptions_span,
        ])];

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
        if !parts.is_empty() {
            // Bright: default style.
            lines.push(Line::from(Span::raw(parts)));
        }

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
            Step::Activity => draw_suggestions(frame, list_area, &form.activity),
            Step::Project => draw_suggestions(frame, list_area, &form.project),
            Step::Task => draw_suggestions(frame, list_area, &form.task),
        }
        if let Some(input) = input {
            frame.render_widget(input.paragraph(), input_area);
        }
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

/// One note as dimmed `  • ` bullet rows, word-wrapped to `width` with
/// continuation rows indented under the text.
fn bullet_lines(note: &str, width: u16) -> Vec<Line<'static>> {
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
            Line::from(Span::styled(format!("{prefix}{row}"), theme::DIMMED))
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

/// A seed for picking captions: the clock's nanoseconds, plenty random for
/// choosing a sentence.
fn fresh_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as u64 ^ d.as_secs())
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
fn hanging_lines(label: &str, rows: Vec<String>) -> Vec<Line<'static>> {
    let indent = " ".repeat(label.chars().count() + 2);
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| match i {
            0 => theme::kv(label, row),
            _ => Line::from(format!("{indent}{row}")),
        })
        .collect()
}

/// Packs `items` into rows joined by " · ", for display beside `label`
/// within `width` columns. An item only wraps (mid-item, via `wrap_text`)
/// when it is wider than a whole row.
fn wrap_items(items: &[String], width: u16, label: &str) -> Vec<String> {
    let width = (width as usize)
        .saturating_sub(label.chars().count() + 2)
        .max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    for item in items {
        let len = item.chars().count();
        if !row.is_empty() && row.chars().count() + 3 + len <= width {
            row.push_str(" · ");
            row.push_str(item);
            continue;
        }
        if !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
        let mut pieces = wrap_text(item, width);
        if pieces.len() > 1 && pieces.last().is_some_and(String::is_empty) {
            pieces.pop(); // the cursor row wrap_text adds after a full row
        }
        row = pieces.pop().unwrap_or_default();
        rows.extend(pieces);
    }
    rows.push(row);
    rows
}

/// The help screen: every key, in groups with an accent heading and a rule
/// under it. Two columns (sessions; sections and mouse) keep it short
/// enough for a 24-row terminal with room for spacing. A centered modal
/// sized to its content, shrunk on small terminals.
fn draw_help(frame: &mut Frame, content: Rect) {
    type Group = (&'static str, &'static [(&'static str, &'static str)]);
    const LEFT: &[Group] = &[(
        "Sessions",
        &[
            ("s", "start a session"),
            ("e", "end the session"),
            ("n", "add a note"),
            ("i", "count an interruption"),
            ("d", "start a disruption"),
            ("b", "start a break"),
            ("m", "start a meeting"),
            ("r", "resume after d, b or m"),
            ("1 / 2 / 3", "rate focus, just after"),
            ("u", "undo the last start/end"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use chrono::TimeZone;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// An [`App`] backed by a throwaway db. The `app` field is declared first so
    /// its `Db` connection drops before the directory is removed.
    struct TestApp {
        app: App,
        dir: std::path::PathBuf,
    }

    impl TestApp {
        fn new() -> Self {
            // Distinct prefix from db.rs's tests: both counters start at 0 in
            // the same process, and a shared dir would let one test delete the
            // other's live database.
            let dir = std::env::temp_dir().join(format!(
                "jiary-app-test-{}-{}",
                std::process::id(),
                TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let db = Db::open_at(&dir.join("jiary.db")).unwrap();
            let mut app = App::new(db, None);
            app.set_size((RENDER_WIDTH, RENDER_HEIGHT));
            Self { app, dir }
        }

        /// Puts the app on the Project step of the start-session form, with
        /// `typed` already typed and nothing highlighted.
        fn on_project_step(mut self, projects: Vec<String>, typed: &str) -> Self {
            let mut project = Autocomplete::new(projects);
            project.input = typed.to_string();
            project.query = typed.to_string();
            // An activity was picked on the previous step (it is required).
            let mut activity = Autocomplete::new(Vec::new());
            activity.input = "Reading".to_string();
            self.app.screen = Screen::StartSession(Box::new(StartSessionForm {
                activity,
                project,
                ..Self::empty_form(Step::Project)
            }));
            self
        }

        /// Puts the app on an arbitrary screen (render tests of the chrome).
        fn on_screen(mut self, screen: Screen) -> Self {
            self.app.screen = screen;
            self
        }

        /// A start-session form at the given step, everything else empty.
        fn empty_form(step: Step) -> StartSessionForm {
            StartSessionForm {
                step,
                activity: Autocomplete::new(Vec::new()),
                project: Autocomplete::new(Vec::new()),
                task: Autocomplete::new(Vec::new()),
            }
        }

        /// A start-session screen at the given step, everything else empty.
        fn start_form_at(step: Step) -> Screen {
            Screen::StartSession(Box::new(Self::empty_form(step)))
        }

        /// Opens the start-session form the way `s` does, on the Activity
        /// step with the app's activity suggestions.
        fn on_activity_step(mut self) -> Self {
            self.key(KeyCode::Char('s'));
            self
        }

        fn key(&mut self, code: KeyCode) {
            self.app.handle_key(press(code));
        }

        fn typed(&mut self, text: &str) {
            for c in text.chars() {
                self.key(KeyCode::Char(c));
            }
        }

        fn form(&self) -> &StartSessionForm {
            match &self.app.screen {
                Screen::StartSession(f) => f,
                _ => panic!("expected start-session screen"),
            }
        }
    }

    impl Drop for TestApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// A plain key press, as the terminal reports it.
    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    // --- key events ---------------------------------------------------

    #[test]
    fn ctrl_c_quits_from_any_screen() {
        let mut t = TestApp::new().on_project_step(vec![], "");
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(t.app.handle_key(ctrl_c));
    }

    #[test]
    fn key_releases_and_ctrl_combinations_are_ignored() {
        let mut t = TestApp::new().on_project_step(vec![], "");
        let mut release = press(KeyCode::Char('x'));
        release.kind = KeyEventKind::Release;
        assert!(!t.app.handle_key(release));
        assert!(
            !t.app
                .handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
        );
        assert_eq!(t.form().project.input, "");
        // Shift is how uppercase arrives, so it must still type.
        t.app
            .handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT));
        assert_eq!(t.form().project.input, "X");
    }

    // --- fuzzy_filter -------------------------------------------------

    #[test]
    fn fuzzy_filter_empty_query_returns_all_in_order() {
        let c = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(fuzzy_filter("", &c), c);
    }

    #[test]
    fn fuzzy_filter_matches_subsequences() {
        let c = vec!["programming".into(), "reading".into()];
        // 'pg' is a subsequence of "programming" but not of "reading".
        assert_eq!(fuzzy_filter("pg", &c), vec!["programming".to_string()]);
    }

    #[test]
    fn fuzzy_filter_ranks_contiguous_match_higher() {
        let c = vec!["xaybz".into(), "abc".into()];
        assert_eq!(fuzzy_filter("ab", &c)[0], "abc");
    }

    #[test]
    fn fuzzy_filter_caps_results_at_five() {
        let c: Vec<String> = (0..10).map(|i| format!("item{}", i)).collect();
        assert_eq!(fuzzy_filter("item", &c).len(), 5);
    }

    // --- autocomplete -------------------------------------------------

    fn ab() -> Vec<String> {
        vec!["alpha".into(), "beta".into()]
    }

    #[test]
    fn tab_accepts_best_match_first() {
        // Regression (61d9181): the first Tab must accept the top suggestion,
        // not skip past it.
        let mut t = TestApp::new().on_project_step(ab(), "b");
        t.key(KeyCode::Tab);
        assert_eq!(t.form().project.input, "beta");
        assert_eq!(t.form().project.selection, Some(0));
    }

    #[test]
    fn tab_cycles_and_wraps_around() {
        let mut t = TestApp::new().on_project_step(ab(), "");
        let mut seen = Vec::new();
        for _ in 0..3 {
            t.key(KeyCode::Tab);
            seen.push(t.form().project.input.clone());
        }
        assert_eq!(seen, ["alpha", "beta", "alpha"]);
    }

    #[test]
    fn arrows_select_and_up_past_top_restores_typed_text() {
        let mut t = TestApp::new().on_project_step(ab(), "");
        t.key(KeyCode::Down);
        t.key(KeyCode::Down);
        t.key(KeyCode::Down); // stops at the end
        assert_eq!(t.form().project.input, "beta");
        t.key(KeyCode::Up);
        assert_eq!(t.form().project.input, "alpha");
        t.key(KeyCode::Up);
        assert_eq!(t.form().project.input, "");
        assert_eq!(t.form().project.selection, None);
    }

    #[test]
    fn enter_submits_the_highlighted_suggestion() {
        let mut t = TestApp::new().on_project_step(ab(), "");
        t.key(KeyCode::Down);
        t.key(KeyCode::Down);
        t.key(KeyCode::Enter); // project → task step
        t.key(KeyCode::Enter); // start the session
        let active = t.app.active_session.as_ref().unwrap();
        assert_eq!(active.project.as_deref(), Some("beta"));
    }

    #[test]
    fn typing_after_accepting_continues_from_the_suggestion() {
        let mut t = TestApp::new().on_project_step(ab(), "");
        t.key(KeyCode::Tab);
        t.typed("2");
        let f = &t.form().project;
        assert_eq!(f.input, "alpha2");
        assert_eq!(f.query, "alpha2"); // suggestions filter on what is shown
        assert_eq!(f.selection, None);
    }

    #[test]
    fn backspace_clears_selection() {
        let mut t = TestApp::new().on_project_step(ab(), "al");
        t.key(KeyCode::Tab);
        t.key(KeyCode::Backspace);
        let f = &t.form().project;
        assert_eq!(f.input, "alph");
        assert_eq!(f.query, "alph");
        assert_eq!(f.selection, None);
    }

    #[test]
    fn tab_is_noop_when_no_suggestions() {
        let mut t = TestApp::new().on_project_step(ab(), "zzz");
        t.key(KeyCode::Tab);
        assert_eq!(t.form().project.input, "zzz");
        assert_eq!(t.form().project.selection, None);
    }

    #[test]
    fn blank_fields_are_stored_as_none() {
        let mut t = TestApp::new().on_project_step(ab(), "   ");
        t.key(KeyCode::Enter);
        t.key(KeyCode::Enter);
        let active = t.app.active_session.as_ref().unwrap();
        assert_eq!(active.project, None);
        assert_eq!(active.task, None);
    }

    // --- render harness ---------------------------------------------------

    /// Fixed size for render tests: wide enough for every status line, tall
    /// enough that the chrome rows (0 and last) never overlap the content.
    const RENDER_WIDTH: u16 = 80;
    const RENDER_HEIGHT: u16 = 24;

    /// Draws the app into a fixed-size [`TestBackend`] and returns the
    /// resulting buffer so tests can assert on rendered lines.
    fn render(app: &App) -> Buffer {
        let backend = TestBackend::new(RENDER_WIDTH, RENDER_HEIGHT);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// Text of one buffer row, trailing padding trimmed.
    fn line(buf: &Buffer, y: u16) -> String {
        let width = buf.area.width as usize;
        buf.content()
            .iter()
            .skip(y as usize * width)
            .take(width)
            .map(|c| c.symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// Column (char count, not byte offset — rows contain multi-byte chars
    /// like `–` and `·`) of the first occurrence of `needle` in a buffer row.
    fn col_of(buf: &Buffer, y: u16, needle: &str) -> u16 {
        let row = line(buf, y);
        let byte_idx = row.find(needle).unwrap();
        row[..byte_idx].chars().count() as u16
    }

    /// Row text starting at char column `x` (rows contain multi-byte chars,
    /// so byte-index slicing is not safe).
    fn line_from(buf: &Buffer, y: u16, x: u16) -> String {
        line(buf, y).chars().skip(x as usize).collect()
    }

    /// A running session that started `elapsed` ago, so the panel's timer is
    /// deterministic (truncated h/m/s can't shift within the test's runtime).
    fn running_session(elapsed: chrono::Duration, interruptions: i32) -> Session {
        Session {
            id: 1,
            started_at: Utc::now() - elapsed,
            ended_at: None,
            project: Some("alpha".into()),
            task: Some("beta".into()),
            activity: "Programming".into(),
            notes: None,
            focus: None,
            interruptions: Some(interruptions),
        }
    }

    /// A completed session with a known local-time range.
    fn finished_session(
        start: chrono::DateTime<chrono::Local>,
        end: chrono::DateTime<chrono::Local>,
        focus: Option<i32>,
    ) -> Session {
        Session {
            id: 1,
            started_at: start.with_timezone(&Utc),
            ended_at: Some(end.with_timezone(&Utc)),
            project: Some("alpha".into()),
            task: Some("beta".into()),
            activity: "Programming".into(),
            notes: None,
            focus,
            interruptions: Some(0),
        }
    }

    // --- chrome: header ---------------------------------------------------

    #[test]
    fn header_shows_name_date_and_zero_total() {
        let t = TestApp::new();
        let buf = render(&t.app);
        let date = chrono::Local::now().format("%a %d %b %Y");
        assert_eq!(line(&buf, 0), format!("JIARY  {date}  ·  0m today"));
    }

    #[test]
    fn header_total_includes_completed_sessions() {
        let mut t = TestApp::new();
        // A completed 60-minute session. The header only reads in-memory
        // state (draw must never query the db), so pushing it directly is
        // the right level of test setup.
        let now = Utc::now();
        t.app.today_sessions.push(Session {
            id: 1,
            started_at: now - chrono::Duration::minutes(90),
            ended_at: Some(now - chrono::Duration::minutes(30)),
            project: None,
            task: None,
            activity: "Programming".into(),
            notes: None,
            focus: None,
            interruptions: None,
        });
        let buf = render(&t.app);
        assert!(line(&buf, 0).contains("1h 0m today"));
    }

    // --- chrome: status line ----------------------------------------------

    #[test]
    fn status_line_shows_main_screen_hints() {
        let t = TestApp::new();
        let buf = render(&t.app);
        assert_eq!(
            line(&buf, RENDER_HEIGHT - 1),
            "[s] start  [e] end  [n] note  [i] interrupt  [d/b/m] away  [r] resume  [?] help"
        );
    }

    #[test]
    fn status_line_drops_hints_that_dont_fit_but_keeps_help() {
        let t = TestApp::new();
        let text = |width| {
            t.app
                .status_line(width)
                .spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        assert_eq!(text(40), "[s] start  [e] end  [n] note  [?] help");
        // Narrower than the help hint itself (a pty can report 0 columns).
        assert_eq!(text(5), "[?] help");
        assert_eq!(text(0), "[?] help");
        assert!(text(120).ends_with("[?] help  [Tab] sections  [q] quit")); // everything
    }

    #[test]
    fn status_line_shows_start_form_hints() {
        let t = TestApp::new().on_project_step(vec![], "");
        let buf = render(&t.app);
        assert_eq!(
            line(&buf, RENDER_HEIGHT - 1),
            "[Tab] accept  [Enter] next  [Esc] cancel"
        );
    }

    #[test]
    fn status_line_leads_with_rating_and_undo_while_the_window_is_open() {
        let mut t = TestApp::new();
        t.app.start_session("Reading", None, None);
        t.key(KeyCode::Char('e'));
        let buf = render(&t.app);
        assert!(
            line(&buf, RENDER_HEIGHT - 1)
                .starts_with("[1/2/3] rate bad/ok/good 5s  [u] undo  [s] start"),
            "{}",
            line(&buf, RENDER_HEIGHT - 1)
        );

        // Once it closes, the usual hints are back.
        t.app.rating = Some((1, Instant::now() - RATING_WINDOW));
        let buf = render(&t.app);
        assert!(line(&buf, RENDER_HEIGHT - 1).starts_with("[s] start"));
    }

    #[test]
    fn status_line_shows_add_note_hints() {
        let t = TestApp::new().on_screen(Screen::AddNote(AddNoteForm {
            input: String::new(),
        }));
        let buf = render(&t.app);
        assert_eq!(line(&buf, RENDER_HEIGHT - 1), "[Enter] save  [Esc] cancel");
    }

    #[test]
    fn error_line_replaces_hints_in_red() {
        let mut t = TestApp::new();
        t.app.set_error("disk full");
        let buf = render(&t.app);
        assert_eq!(line(&buf, RENDER_HEIGHT - 1), "✗ disk full");
        let cell = &buf[(0, RENDER_HEIGHT - 1)];
        assert_eq!(cell.fg, Color::Red);
        assert!(cell.modifier.contains(Modifier::BOLD));
    }

    // --- main screen ------------------------------------------------------

    #[test]
    fn main_screen_no_active_shows_a_dimmed_hint_in_the_active_section() {
        let t = TestApp::new();
        let buf = render(&t.app);
        assert!(line(&buf, 1).starts_with("╭ ACTIVE "));
        assert!(line(&buf, 2).starts_with("│No active session. Press s to start one.  "));
        assert_eq!(buf[(1, 2)].fg, Color::Gray);
        // As tall as the idling cat and its caption rows beside the hint:
        // border, 8 rows (cat, gap, caption), border.
        assert!(line(&buf, 10).starts_with("╰"));
    }

    #[test]
    fn main_screen_active_panel_has_accent_border_and_title() {
        let mut t = TestApp::new();
        t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 0));
        let buf = render(&t.app);
        assert_eq!(buf[(0, 1)].symbol(), "╭");
        assert_eq!(buf[(0, 1)].fg, Color::Cyan);
        // The title sits on the top border, left-aligned inside it.
        assert!(line(&buf, 1).starts_with("╭ ACTIVE "));
        let cell = &buf[(2, 1)];
        assert_eq!(cell.fg, Color::Cyan);
        assert!(cell.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn main_screen_active_panel_shows_timer_and_kv_rows() {
        let mut t = TestApp::new();
        let elapsed = chrono::Duration::hours(5)
            + chrono::Duration::minutes(23)
            + chrono::Duration::seconds(45);
        t.app.active_session = Some(running_session(elapsed, 0));
        let buf = render(&t.app);
        // The panel's inner area starts right at the border (no padding).
        assert!(line(&buf, 2).starts_with("│05:23:45"));
        assert!(buf[(1, 2)].modifier.contains(Modifier::BOLD));
        assert!(line(&buf, 3).starts_with("│Project  alpha"));
        assert_eq!(buf[(1, 3)].fg, Color::Gray); // dimmed label
        assert_eq!(buf[(10, 3)].fg, Color::Reset); // plain value
        assert!(line(&buf, 4).starts_with("│Task  beta"));
        assert!(line(&buf, 5).starts_with("│Activity  Programming"));
        assert!(line(&buf, 6).starts_with("│Interruptions  0"));
    }

    #[test]
    fn main_screen_active_panel_colors_interruptions_when_positive() {
        let mut t = TestApp::new();
        t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 2));
        let buf = render(&t.app);
        assert!(line(&buf, 6).starts_with("│Interruptions  2"));
        let col = col_of(&buf, 6, "2");
        assert_eq!(buf[(col, 6)].fg, Color::Red);
    }

    #[test]
    fn previous_notes_render_one_bullet_per_line() {
        let mut t = TestApp::new();
        let start = |t: &mut TestApp| {
            t.app
                .start_session("Reading", Some("alpha".into()), Some("beta".into()));
            t.app.refresh_after_session_change();
        };
        start(&mut t);
        t.app.append_active_note("first");
        t.app.append_active_note("second");
        t.app.close_active_session();
        start(&mut t);

        let buf = render(&t.app);
        let rows: Vec<String> = (0..RENDER_HEIGHT).map(|y| line(&buf, y)).collect();
        let at = rows
            .iter()
            .position(|r| r.starts_with("│Previous notes:"))
            .expect("previous notes shown");
        assert!(rows[at + 1].starts_with("│  • first"));
        assert!(rows[at + 2].starts_with("│  • second"));
    }

    #[test]
    fn timeline_item_shows_range_duration_and_parts() {
        let mut t = TestApp::new();
        let start = chrono::Local
            .with_ymd_and_hms(2026, 9, 24, 10, 0, 0)
            .unwrap();
        let end = chrono::Local
            .with_ymd_and_hms(2026, 9, 24, 11, 30, 0)
            .unwrap();
        t.app
            .today_sessions
            .push(finished_session(start, end, None));
        let buf = render(&t.app);
        // Below the 10-row active section (the cat, a gap and its caption); inner text starts at column 1.
        assert!(line(&buf, 11).starts_with("╭ ▾ TODAY · 1 session "));
        assert!(line_from(&buf, 12, 1).starts_with("10:00 – 11:30 (1h 30m) F- I:0 "));
        assert!(buf[(1, 12)].modifier.contains(Modifier::BOLD)); // time range
        let dur_col = col_of(&buf, 12, "1h 30m");
        assert!(buf[(dur_col, 12)].modifier.contains(Modifier::BOLD)); // duration
        assert!(line_from(&buf, 13, 1).starts_with("alpha · beta · Programming "));
    }

    #[test]
    fn timeline_focus_badge_uses_semantic_colors() {
        let mut t = TestApp::new();
        for focus in [Some(1), Some(2), Some(3), None] {
            let start = chrono::Local
                .with_ymd_and_hms(2026, 9, 24, 10, 0, 0)
                .unwrap();
            let end = chrono::Local
                .with_ymd_and_hms(2026, 9, 24, 10, 30, 0)
                .unwrap();
            t.app
                .today_sessions
                .push(finished_session(start, end, focus));
        }
        // With the summary collapsed and the active section shrunk to its
        // 3-row minimum, all four items fit.
        t.app.layout.summary_collapsed = true;
        t.app.layout.active_height = Some(3);
        let buf = render(&t.app);
        // Items stack three rows apart (range, parts, spacing), from row 5.
        let expectations = [
            ("F1", Color::Red),
            ("F2", Color::Yellow),
            ("F3", Color::Green),
            ("F-", Color::Gray),
        ];
        for (i, (badge, color)) in expectations.iter().enumerate() {
            let y = 5 + 3 * i as u16;
            let col = col_of(&buf, y, badge);
            assert_eq!(buf[(col, y)].fg, *color);
        }
    }

    // --- forms ------------------------------------------------------------

    /// The start modal is 40×12 centered in the content area: x=20, y=6;
    /// tabs row at y=7, body from y=8. End/note modals are 40×6: x=20, y=9;
    /// tabs/context row at y=10, input row at y=11. All inner text starts at
    /// the padded column 22.

    #[test]
    fn start_modal_shows_title_and_selected_tab() {
        let t = TestApp::new().on_activity_step();
        let buf = render(&t.app);
        // The modal border is default color; only the title carries the accent.
        assert_eq!(buf[(20, 6)].symbol(), "╭");
        assert_eq!(buf[(20, 6)].fg, Color::Reset);
        assert!(line_from(&buf, 6, 20).starts_with("╭ Start session "));
        let title = &buf[(22, 6)];
        assert_eq!(title.fg, Color::Cyan);
        assert!(title.modifier.contains(Modifier::BOLD));
        // Tabs row: the selected step is highlighted, the others are plain.
        assert!(line_from(&buf, 7, 22).starts_with(" Activity │ Project │ Task"));
        assert_eq!(buf[(23, 7)].fg, Color::Cyan); // "Activity" selected
        assert!(buf[(23, 7)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(34, 7)].fg, Color::Reset); // "Project"
        assert_eq!(buf[(44, 7)].fg, Color::Reset); // "Task"
    }

    #[test]
    fn start_modal_activity_step_shows_input_and_suggestions() {
        let mut t = TestApp::new().on_activity_step();
        for _ in 0..3 {
            t.key(KeyCode::Down);
        }
        let buf = render(&t.app);
        // Input row at y=8 shows the selection after the prompt;
        // suggestions from y=9, their text aligned with the typed text
        // (column 24) and the bold accent `>` under the prompt's space.
        assert!(line_from(&buf, 8, 22).starts_with("› Writing"));
        assert!(line_from(&buf, 9, 22).starts_with("  Programming"));
        assert_eq!(buf[(24, 9)].fg, Color::Reset); // unselected item text
        assert!(line_from(&buf, 11, 22).starts_with(" >Writing"));
        assert_eq!(buf[(23, 11)].fg, Color::Cyan);
        assert!(buf[(23, 11)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(24, 11)].fg, Color::Cyan);
    }

    // --- custom activities ------------------------------------------------

    fn activity_suggestions(t: &TestApp) -> Vec<String> {
        match &t.app.screen {
            Screen::StartSession(form) => form.activity.suggestions(),
            _ => panic!("expected start-session screen"),
        }
    }

    #[test]
    fn fresh_database_suggests_the_default_activities() {
        let t = TestApp::new().on_activity_step();
        assert_eq!(activity_suggestions(&t), DEFAULT_ACTIVITIES);
    }

    #[test]
    fn typed_activity_is_stored_and_replaces_the_defaults() {
        let mut t = TestApp::new().on_activity_step();
        t.typed("Gardening");
        t.key(KeyCode::Enter); // → project
        t.key(KeyCode::Enter); // → task
        t.key(KeyCode::Enter); // start
        assert_eq!(activity(&t), Some("Gardening"));

        // With history, only activities actually used are suggested.
        t.key(KeyCode::Char('s'));
        assert_eq!(activity_suggestions(&t), ["Gardening"]);
    }

    #[test]
    fn activities_are_suggested_most_recent_first() {
        let mut t = TestApp::new();
        t.app.start_session("Reading", None, None);
        t.app.close_active_session();
        t.app.start_session("Writing", None, None);
        t.app.refresh_suggestions();
        // One Tab repeats the most recent activity.
        let mut t = t.on_activity_step();
        t.key(KeyCode::Tab);
        assert_eq!(t.form().activity.value().as_deref(), Some("Writing"));
        assert_eq!(activity_suggestions(&t), ["Writing", "Reading"]);
    }

    #[test]
    fn enter_on_an_empty_activity_stays_on_the_step() {
        let mut t = TestApp::new().on_activity_step();
        t.key(KeyCode::Enter);
        assert!(matches!(t.form().step, Step::Activity));
        t.typed("   ");
        t.key(KeyCode::Enter);
        assert!(matches!(t.form().step, Step::Activity));
    }

    #[test]
    fn start_modal_project_step_shows_input_and_suggestions() {
        let mut t = TestApp::new().on_project_step(vec!["alpha".into(), "alpine".into()], "a");
        t.key(KeyCode::Down);
        t.key(KeyCode::Down);
        let buf = render(&t.app);
        // The Project tab is now selected.
        assert_eq!(buf[(34, 7)].fg, Color::Cyan);
        assert!(buf[(34, 7)].modifier.contains(Modifier::BOLD));
        // Input row: bold accent prompt + the highlighted suggestion.
        assert!(line_from(&buf, 8, 22).starts_with("› alpine"));
        assert_eq!(buf[(22, 8)].fg, Color::Cyan);
        assert!(buf[(22, 8)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(24, 8)].fg, Color::Reset);
        // Suggestions below, aligned with the value; the selection is
        // highlighted.
        assert!(line_from(&buf, 9, 24).starts_with("alpha"));
        assert!(line_from(&buf, 10, 23).starts_with(">alpine"));
        assert_eq!(buf[(23, 10)].fg, Color::Cyan);
    }

    #[test]
    fn empty_input_shows_a_dimmed_placeholder_not_a_label() {
        let t = TestApp::new().on_activity_step();
        let buf = render(&t.app);
        // The tabs row names the field; the input row doesn't repeat it.
        assert_eq!(
            line_from(&buf, 8, 22),
            "› type or pick an activity           │"
        );
        assert_eq!(buf[(24, 8)].fg, Color::Gray);
        assert!(buf[(24, 8)].modifier.contains(Modifier::ITALIC));
        // The cursor sits at the start of the placeholder.
        assert_eq!(t.app.cursor_position((80, 24)), Some((24, 8)));
    }

    #[test]
    fn cursor_position_tracks_input_lines() {
        // 80×24 terminal: the start modal's input row is y=8, the note
        // modal's is y=11; values start at column 24, after "› ".
        let t = TestApp::new().on_project_step(vec![], "");
        assert_eq!(t.app.cursor_position((80, 24)), Some((24, 8)));

        let t = TestApp::new().on_project_step(vec![], "ab");
        assert_eq!(t.app.cursor_position((80, 24)), Some((26, 8)));

        let t = TestApp::new().on_screen(TestApp::start_form_at(Step::Task));
        assert_eq!(t.app.cursor_position((80, 24)), Some((24, 8)));

        let t = TestApp::new().on_screen(TestApp::start_form_at(Step::Activity));
        assert_eq!(t.app.cursor_position((80, 24)), Some((24, 8)));

        let t = TestApp::new().on_screen(Screen::AddNote(AddNoteForm { input: "hi".into() }));
        assert_eq!(t.app.cursor_position((80, 24)), Some((26, 11))); // "› hi"

        let t = TestApp::new();
        assert_eq!(t.app.cursor_position((80, 24)), None); // main screen
    }

    // --- mascot -------------------------------------------------------------

    #[test]
    fn the_cat_sits_on_the_right_of_the_active_section() {
        let mut t = TestApp::new();
        t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 0));
        let buf = render(&t.app);
        // Inner area is columns 1–78; the 14-wide cat ends one column before
        // the border, so its left edge is column 64. The left ear (same in
        // every frame): a lower half, then fur over the pink inner ear.
        assert_eq!(buf[(65, 2)].symbol(), "▄");
        assert_eq!(buf[(65, 2)].fg, Color::Cyan);
        assert_eq!(buf[(66, 2)].symbol(), "▀");
        assert_eq!(buf[(66, 2)].bg, Color::LightRed);
        // The session text keeps the left side.
        assert!(line(&buf, 3).starts_with("│Project  alpha"));
    }

    #[test]
    fn the_cat_hides_when_the_section_is_too_small() {
        // Shrunk below the cat's height.
        let mut t = TestApp::new();
        t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 0));
        t.app.layout.active_height = Some(5);
        let buf = render(&t.app);
        assert!((1..79).all(|x| (2..5).all(|y| buf[(x, y)].fg != Color::Cyan)));

        // Too narrow to keep 24 columns of text beside it.
        let mut t = TestApp::new();
        t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 0));
        t.app.set_size((40, 24));
        let backend = TestBackend::new(40, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| t.app.draw(frame)).unwrap();
        let buf = terminal.backend().buffer();
        assert!((1..39).all(|x| (2..7).all(|y| buf[(x, y)].fg != Color::Cyan)));
    }

    /// The caption rows under the cat (rows 8–9, after a gap row; the cat's
    /// columns 54–77),
    /// each trimmed.
    fn caption_rows(buf: &Buffer) -> Vec<String> {
        (8..10)
            .map(|y| {
                let row: String = line_from(buf, y, 54).chars().take(24).collect();
                row.trim().to_string()
            })
            .collect()
    }

    #[test]
    fn a_caption_shows_under_the_cat_in_faded_italic() {
        let mut t = TestApp::new();
        t.app.active_session = Some(running_session(chrono::Duration::minutes(1), 0));
        t.app.caption_seed = 0; // "working on world domination..."
        let buf = render(&t.app);
        let rows = caption_rows(&buf);
        // Wrapped to the cat's width.
        assert_eq!(rows.join(" "), "working on world domination...");
        assert!(rows.iter().all(|r| !r.is_empty()), "{rows:?}");
        let x = 54
            + line_from(&buf, 8, 54)
                .chars()
                .position(|c| c != ' ')
                .unwrap() as u16;
        assert_eq!(buf[(x, 8)].fg, Color::Gray);
        assert!(buf[(x, 8)].modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn the_caption_centres_under_the_cat_not_the_canvas() {
        // Working: the laptop is on the left of the canvas and the cat's
        // body on columns 10–20, so each row centres on column 54 + 15.
        let mut t = TestApp::new();
        t.app.active_session = Some(running_session(chrono::Duration::minutes(1), 0));
        t.app.caption_seed = 0; // "working on world domination..."
        let buf = render(&t.app);
        for y in 8..10 {
            let row: String = line_from(&buf, y, 54).chars().take(24).collect();
            let first = row.chars().position(|c| c != ' ').unwrap();
            let last = row.chars().count() - 1 - row.chars().rev().position(|c| c != ' ').unwrap();
            assert!((29..=31).contains(&(first + last)), "{row:?}");
        }
    }

    #[test]
    fn caption_rows_too_wide_to_centre_stay_on_the_canvas() {
        let lines = caption_lines("don't be evil, don't be evil", Mood::Idle, 24);
        let widths: Vec<usize> = lines.iter().map(Line::width).collect();
        assert!(widths.iter().all(|&w| w <= 24), "{widths:?}");
        assert_eq!(widths[0], 24, "pushed back to the canvas's left edge");
    }

    #[test]
    fn idling_shows_its_caption_on_one_row() {
        let mut t = TestApp::new();
        t.app.caption_seed = 0;
        assert_eq!(caption_rows(&render(&t.app)), ["what's next, boss?", ""]);
    }

    #[test]
    fn the_cats_mood_follows_the_session() {
        let mut t = TestApp::new();
        assert_eq!(t.app.mood(), Mood::Idle);
        for (activity, mood) in [
            ("Reading", Mood::Working),
            (BREAK, Mood::Sleeping),
            (MEETING, Mood::Meeting),
            (DISRUPTION, Mood::Gaming),
        ] {
            let mut session = running_session(chrono::Duration::minutes(5), 0);
            session.activity = activity.into();
            t.app.active_session = Some(session);
            assert_eq!(t.app.mood(), mood, "{activity}");
        }
        // A long work session gets drowsy; a long meeting doesn't.
        let long = chrono::Duration::minutes(DROWSY_AFTER_MINUTES);
        t.app.active_session = Some(running_session(long, 0));
        assert_eq!(t.app.mood(), Mood::Drowsy);
        let mut meeting = running_session(long, 0);
        meeting.activity = MEETING.into();
        t.app.active_session = Some(meeting);
        assert_eq!(t.app.mood(), Mood::Meeting);
    }

    #[test]
    fn actions_trigger_short_reactions() {
        let reaction = |t: &TestApp| t.app.current_reaction().map(|(r, _)| r);
        let mut t = TestApp::new();
        t.app.start_session("Writing", None, None);
        assert_eq!(reaction(&t), None);

        t.key(KeyCode::Char('i'));
        assert_eq!(reaction(&t), Some(Reaction::Flinch));

        t.key(KeyCode::Char('n'));
        t.typed("idea");
        t.key(KeyCode::Enter);
        assert_eq!(reaction(&t), Some(Reaction::Noted));

        for code in [KeyCode::Char('e'), KeyCode::Enter, KeyCode::Char('3')] {
            t.key(code);
        }
        assert_eq!(reaction(&t), Some(Reaction::Rated(3)));
    }

    #[test]
    fn skipping_the_rating_or_an_empty_note_gets_no_reaction() {
        let mut t = TestApp::new();
        t.app.start_session("Writing", None, None);
        for code in [KeyCode::Char('n'), KeyCode::Enter] {
            t.key(code);
        }
        assert!(t.app.current_reaction().is_none());
        for code in [KeyCode::Char('e'), KeyCode::Enter, KeyCode::Enter] {
            t.key(code);
        }
        assert!(t.app.active_session.is_none());
        assert!(t.app.current_reaction().is_none());
    }

    #[test]
    fn reactions_end_after_the_reaction_time() {
        let mut t = TestApp::new();
        let long_ago = Instant::now()
            .checked_sub(std::time::Duration::from_millis(
                Reaction::Flinch.duration(),
            ))
            .unwrap();
        t.app.reaction = Some((Reaction::Flinch, long_ago));
        assert!(t.app.current_reaction().is_none());
    }

    // --- scrolling the active session's notes ------------------------------

    /// A running session with notes "note 1" … "note n", in an active
    /// section fixed at 13 rows: of its 11 inner rows, the first 5 are the
    /// pinned session info, leaving rows 7–12 of the text column (beside
    /// the cat) for the notes.
    fn with_notes(n: usize) -> TestApp {
        let mut t = TestApp::new();
        let mut session = running_session(chrono::Duration::minutes(1), 0);
        session.notes = Some((1..=n).map(|i| format!("note {i}\n")).collect());
        t.app.active_session = Some(session);
        t.app.layout.active_height = Some(13);
        t
    }

    #[test]
    fn notes_scroll_under_the_pinned_session_info_and_cat() {
        let mut t = with_notes(10);
        // "Session notes:" + 10 bullets = 11 rows in a 6-row viewport.
        assert_eq!(t.app.viewport(Section::Active), 6);
        assert_eq!(t.app.max_scroll(Section::Active), 5);

        t.key(KeyCode::Down);
        t.key(KeyCode::Char('j'));
        assert_eq!(t.app.notes_scroll, 2);
        let buf = render(&t.app);
        // Pinned: the timer and the cat's ear stay at the top.
        assert!(line(&buf, 2).starts_with("│00:01:00"));
        assert_eq!(buf[(65, 2)].symbol(), "▄");
        // The notes right below the info have moved: no gap above them.
        assert!(line(&buf, 6).starts_with("│Interruptions  0"));
        assert!(line_from(&buf, 7, 1).starts_with("  • note 2 "));
        // With a scrollbar on the border beside them.
        assert_ne!(buf[(79, 9)].symbol(), " ");

        t.key(KeyCode::End);
        assert!(line_from(&render(&t.app), 12, 1).starts_with("  • note 10 "));
        t.key(KeyCode::Home);
        assert!(line_from(&render(&t.app), 7, 1).starts_with("Session notes:"));
    }

    #[test]
    fn long_notes_wrap_with_an_indent_beside_the_cat() {
        let mut t = with_notes(0);
        let long = "a long note about the layout that keeps going well past the cat";
        t.app.active_session.as_mut().unwrap().notes = Some(format!("{long}\n"));
        let buf = render(&t.app);
        // The text column is 52 wide (78 inner minus the cat's 26): the note
        // wraps at a word boundary and continues under its text.
        assert!(
            line_from(&buf, 8, 1).starts_with("  • a long note about the layout that keeps going ")
        );
        assert!(line_from(&buf, 9, 1).starts_with("    well past the cat "));
        // Each row stays inside the text column (the cat's caption is to its
        // right on these rows).
        let text_column = |y| line_from(&buf, y, 1).chars().take(52).collect::<String>();
        assert_eq!(
            text_column(8).trim_end(),
            "  • a long note about the layout that keeps going"
        );
        assert_eq!(text_column(9).trim_end(), "    well past the cat");
        // Scrolling counts the wrapped rows: "Session notes:" + 2.
        assert_eq!(t.app.scroll_lines(Section::Active), 3);
    }

    #[test]
    fn adding_a_note_scrolls_it_into_view() {
        let mut t = TestApp::new();
        t.app.start_session("Writing", None, None);
        t.app.layout.active_height = Some(13);
        for i in 1..=6 {
            t.key(KeyCode::Char('n'));
            t.typed(&format!("note {i}"));
            t.key(KeyCode::Enter);
        }
        assert_eq!(t.app.notes_scroll, t.app.max_scroll(Section::Active));
        assert!(line_from(&render(&t.app), 12, 1).starts_with("  • note 6 "));
    }

    #[test]
    fn the_wheel_scrolls_the_section_under_the_pointer() {
        let mut t = with_notes(10);
        mouse(&mut t, MouseEventKind::ScrollDown, 10, 5); // over the active section
        assert_eq!(t.app.notes_scroll, 3);
        assert_eq!(t.app.today_scroll, 0);
    }

    #[test]
    fn with_the_summary_focused_the_keys_scroll_todays_sessions() {
        let mut t = with_sessions(with_notes(10), 10);
        t.key(KeyCode::Tab);
        t.key(KeyCode::Tab); // summary: it doesn't scroll
        t.key(KeyCode::Down);
        assert_eq!(t.app.today_scroll, 1);
        assert_eq!(t.app.notes_scroll, 0);
    }

    // --- main screen sections ----------------------------------------------

    /// `n` completed 30-minute sessions today, newest first.
    fn with_sessions(mut t: TestApp, n: usize) -> TestApp {
        let now = Utc::now();
        for i in 0..n {
            let end = now - chrono::Duration::minutes(40 * i as i64);
            let mut session = finished_session(
                (end - chrono::Duration::minutes(30)).into(),
                end.into(),
                Some(3),
            );
            session.id = i as i64;
            t.app.today_sessions.push(session);
        }
        t
    }

    #[test]
    fn tab_moves_focus_and_the_accent_border() {
        let mut t = TestApp::new();
        let border_colors = |t: &TestApp| {
            let buf = render(&t.app);
            let today = row_with(&buf, "TODAY");
            let summary = row_with(&buf, "SUMMARY");
            [buf[(0, 1)].fg, buf[(0, today)].fg, buf[(0, summary)].fg]
        };
        assert_eq!(border_colors(&t), [Color::Cyan, Color::Gray, Color::Gray]);
        t.key(KeyCode::Tab);
        assert_eq!(border_colors(&t), [Color::Gray, Color::Cyan, Color::Gray]);
        t.key(KeyCode::Tab);
        assert_eq!(border_colors(&t), [Color::Gray, Color::Gray, Color::Cyan]);
        t.key(KeyCode::BackTab);
        assert_eq!(t.app.focus, Section::Today);
    }

    #[test]
    fn space_collapses_the_focused_section_to_one_line() {
        let mut t = with_sessions(TestApp::new(), 2);
        t.key(KeyCode::Char(' ')); // the active section can't collapse
        assert_eq!(t.app.layout, LayoutPrefs::default());

        t.key(KeyCode::Tab);
        t.key(KeyCode::Char(' '));
        let buf = render(&t.app);
        assert_eq!(line(&buf, 11), "▸ TODAY · 2 sessions · 1h 0m");
        assert_eq!(buf[(0, 11)].fg, Color::Cyan); // still focused
        // The summary takes the freed rows.
        assert!(line(&buf, 12).starts_with("╭ ▾ SUMMARY "));
        assert!(line(&buf, RENDER_HEIGHT - 2).starts_with("╰"));

        t.key(KeyCode::Tab);
        t.key(KeyCode::Char(' '));
        let buf = render(&t.app);
        assert_eq!(
            line(&buf, 12),
            "▸ SUMMARY · tracked 1h 0m · Programming 1h 0m"
        );
        assert_eq!(line(&buf, 13), "");

        t.key(KeyCode::Char(' ')); // and back
        assert!(line(&render(&t.app), 12).starts_with("╭ ▾ SUMMARY "));
    }

    #[test]
    fn summary_shows_totals_for_the_day() {
        let t = with_sessions(TestApp::new(), 2);
        let buf = render(&t.app);
        let y = row_with(&buf, "Activity  ");
        assert!(line_from(&buf, y, 1).starts_with("Activity  Programming 1h 0m "));
        assert!(line_from(&buf, y + 1, 1).starts_with("Project   alpha 1h 0m "));
        assert!(
            line_from(&buf, y + 2, 1)
                .starts_with("Focus     avg 3.0 (2 rated) · 0 interruptions · no disruptions ")
        );
        // 30m + 10m gap + 30m.
        assert!(line_from(&buf, y + 3, 1).contains("· tracked 1h 0m · no breaks · gaps 10m"));
    }

    #[test]
    fn empty_day_shows_hints_in_every_section() {
        let t = TestApp::new();
        let buf = render(&t.app);
        row_with(&buf, "No active session.");
        row_with(&buf, "No sessions yet today.");
        row_with(&buf, "Nothing tracked yet today.");
    }

    #[test]
    fn scrolling_is_clamped_to_the_content() {
        let mut t = with_sessions(TestApp::new(), 1);
        t.key(KeyCode::Tab); // the keys scroll the focused section
        t.key(KeyCode::Down); // everything fits: nothing to scroll
        assert_eq!(t.app.today_scroll, 0);

        let mut t = with_sessions(TestApp::new(), 10);
        t.key(KeyCode::Tab);
        // 10 items × 2 rows + 9 separators = 29 rows; the viewport is 4.
        assert_eq!(t.app.viewport(Section::Today), 4);
        assert_eq!(t.app.max_scroll(Section::Today), 25);
        t.key(KeyCode::Down);
        t.key(KeyCode::Char('j'));
        assert_eq!(t.app.today_scroll, 2);
        t.key(KeyCode::PageDown);
        assert_eq!(t.app.today_scroll, 6);
        for _ in 0..5 {
            t.key(KeyCode::PageDown);
        }
        assert_eq!(t.app.today_scroll, 25);
        t.key(KeyCode::Home);
        assert_eq!(t.app.today_scroll, 0);
        t.key(KeyCode::Up);
        assert_eq!(t.app.today_scroll, 0);
        t.key(KeyCode::End);
        let buf = render(&t.app);
        // The oldest session is the last visible item.
        let last = row_with(&buf, "╭ ▾ SUMMARY ") - 3;
        let oldest = fmt_time(&t.app.today_sessions[9].started_at);
        assert!(line_from(&buf, last, 1).starts_with(&oldest));
    }

    #[test]
    fn plus_and_minus_resize_and_equals_resets() {
        let mut t = TestApp::new();
        t.key(KeyCode::Char('+'));
        t.key(KeyCode::Char('+'));
        assert_eq!(t.app.layout.active_height, Some(12));
        assert!(line(&render(&t.app), 12).starts_with("╰"));
        t.key(KeyCode::Char('-'));
        assert_eq!(t.app.layout.active_height, Some(11));

        t.key(KeyCode::Tab);
        t.key(KeyCode::Tab);
        t.key(KeyCode::Char('+'));
        assert_eq!(t.app.layout.summary_height, Some(4));

        t.key(KeyCode::Char('='));
        assert_eq!(t.app.layout, LayoutPrefs::default());
    }

    #[test]
    fn question_mark_opens_help_and_any_key_closes_it() {
        let mut t = TestApp::new();
        t.key(KeyCode::Char('?'));
        let buf = render(&t.app);
        assert!(line(&buf, row_with(&buf, "╭ Keys ")).contains("╭ Keys "));
        // The whole list fits a 24-row terminal, down to the last group.
        row_with(&buf, "wheel");
        assert!(!t.app.handle_key(press(KeyCode::Char('q')))); // closes, doesn't quit
        assert!(matches!(t.app.screen, Screen::Main));
    }

    #[test]
    fn layout_is_saved_on_change_and_restored_on_start() {
        let t = TestApp::new();
        let path = t.dir.join("layout.conf");
        let open = |t: &TestApp| {
            let db = Db::open_at(&t.dir.join("jiary.db")).unwrap();
            let mut app = App::new(db, Some(path.clone()));
            app.set_size((RENDER_WIDTH, RENDER_HEIGHT));
            app
        };

        let mut app = open(&t);
        app.handle_key(press(KeyCode::Char('+')));
        app.handle_key(press(KeyCode::Tab));
        app.handle_key(press(KeyCode::Char(' ')));
        assert!(app.error.is_none());
        drop(app);

        let app = open(&t);
        assert_eq!(
            app.layout,
            LayoutPrefs {
                active_height: Some(11),
                today_collapsed: true,
                ..Default::default()
            }
        );
    }

    // --- mouse -------------------------------------------------------------

    fn mouse(t: &mut TestApp, kind: MouseEventKind, column: u16, row: u16) {
        t.app.handle_mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    fn drag(t: &mut TestApp, from: u16, to: u16) {
        let left = MouseButton::Left;
        mouse(t, MouseEventKind::Down(left), 10, from);
        mouse(t, MouseEventKind::Drag(left), 10, to);
        mouse(t, MouseEventKind::Up(left), 10, to);
    }

    #[test]
    fn dragging_the_active_border_resizes_it() {
        let mut t = TestApp::new();
        // No session: the active section is rows 1–10 (tall enough for the
        // cat and its caption), its border on row 10.
        drag(&mut t, 10, 14);
        assert_eq!(t.app.layout.active_height, Some(14));
        assert!(line(&render(&t.app), 14).starts_with("╰"));
        assert!(t.app.drag.is_none());
        // Clamped so today's sessions keep their minimum.
        drag(&mut t, 14, 100);
        let buf = render(&t.app);
        assert_eq!(
            row_with(&buf, "╭ ▾ SUMMARY ") - row_with(&buf, "╭ ▾ TODAY "),
            3
        );
    }

    #[test]
    fn dragging_todays_bottom_border_resizes_the_summary() {
        let mut t = TestApp::new();
        let buf = render(&t.app);
        let border = row_with(&buf, "╭ ▾ SUMMARY ") - 1;
        drag(&mut t, border, border - 4);
        assert_eq!(t.app.layout.summary_height, Some(7));
        assert_eq!(t.app.focus, Section::Summary);
        assert_eq!(row_with(&render(&t.app), "╭ ▾ SUMMARY "), border - 3);
    }

    fn click(t: &mut TestApp, column: u16, row: u16) {
        mouse(t, MouseEventKind::Down(MouseButton::Left), column, row);
        mouse(t, MouseEventKind::Up(MouseButton::Left), column, row);
    }

    #[test]
    fn clicking_focuses_and_clicking_a_title_toggles_collapse() {
        let mut t = TestApp::new();
        let buf = render(&t.app);
        let today = row_with(&buf, "╭ ▾ TODAY ");

        click(&mut t, 10, today + 1); // inside the body: focus only
        assert_eq!(t.app.focus, Section::Today);
        assert!(!t.app.layout.today_collapsed);

        click(&mut t, 4, today); // on " ▾ TODAY · 0 sessions "
        assert!(t.app.layout.today_collapsed);
        click(&mut t, 4, today); // on the collapsed "▸ TODAY …" line
        assert!(!t.app.layout.today_collapsed);
    }

    #[test]
    fn pressing_a_title_only_toggles_on_release() {
        let mut t = TestApp::new();
        let today = row_with(&render(&t.app), "╭ ▾ TODAY ");
        mouse(&mut t, MouseEventKind::Down(MouseButton::Left), 4, today);
        assert!(!t.app.layout.today_collapsed);
        mouse(&mut t, MouseEventKind::Up(MouseButton::Left), 4, today);
        assert!(t.app.layout.today_collapsed);
    }

    #[test]
    fn clicking_a_border_away_from_the_title_never_collapses() {
        let mut t = TestApp::new();
        let buf = render(&t.app);
        let today = row_with(&buf, "╭ ▾ TODAY ");
        let summary = row_with(&buf, "╭ ▾ SUMMARY ");
        click(&mut t, 60, today);
        click(&mut t, 60, summary);
        assert_eq!(t.app.layout, LayoutPrefs::default());
    }

    #[test]
    fn either_row_of_the_line_between_sections_drags_it() {
        // No session: the active section's bottom border is row 10, today's
        // top border row 11. Grabbing row 11 moves the same line, keeping
        // the grab offset (no jump on the first move).
        let mut t = TestApp::new();
        drag(&mut t, 11, 15);
        assert_eq!(t.app.layout.active_height, Some(14));
        assert!(!t.app.layout.today_collapsed);
        assert_eq!(row_with(&render(&t.app), "╭ ▾ TODAY "), 15);

        // Same for the summary's top border, even when grabbed on the title.
        let mut t = TestApp::new();
        let summary = row_with(&render(&t.app), "╭ ▾ SUMMARY ");
        drag(&mut t, summary, summary - 4);
        assert_eq!(row_with(&render(&t.app), "╭ ▾ SUMMARY "), summary - 4);
        assert!(!t.app.layout.summary_collapsed);
        let mut t = TestApp::new();
        drag(&mut t, summary, summary - 2);
        assert_eq!(t.app.layout.summary_height, Some(5));
        let mut t = TestApp::new();
        let left = MouseButton::Left;
        mouse(&mut t, MouseEventKind::Down(left), 4, summary); // on "▾ SUMMARY"
        mouse(&mut t, MouseEventKind::Drag(left), 4, summary - 2);
        mouse(&mut t, MouseEventKind::Up(left), 4, summary - 2);
        assert!(!t.app.layout.summary_collapsed);
        assert_eq!(t.app.layout.summary_height, Some(5));
    }

    #[test]
    fn wheel_scrolls_todays_sessions() {
        let mut t = with_sessions(TestApp::new(), 10);
        mouse(&mut t, MouseEventKind::ScrollDown, 10, 13);
        assert_eq!(t.app.today_scroll, 3);
        mouse(&mut t, MouseEventKind::ScrollUp, 10, 13);
        mouse(&mut t, MouseEventKind::ScrollUp, 10, 13);
        assert_eq!(t.app.today_scroll, 0);
    }

    #[test]
    fn mouse_is_ignored_outside_the_main_screen() {
        let mut t = TestApp::new().on_project_step(vec![], "");
        drag(&mut t, 3, 7);
        assert_eq!(t.app.layout, LayoutPrefs::default());
    }

    #[test]
    fn every_screen_survives_tiny_terminals() {
        let screens = || {
            vec![
                Screen::Main,
                Screen::Help,
                TestApp::start_form_at(Step::Activity),
                Screen::AddNote(AddNoteForm {
                    input: "word ".repeat(50),
                }),
            ]
        };
        for (width, height) in [(0, 0), (1, 1), (5, 3), (12, 6), (30, 10)] {
            for screen in screens() {
                let mut t = with_sessions(TestApp::new(), 5).on_screen(screen);
                t.app.set_size((width, height));
                let backend = TestBackend::new(width, height);
                let mut terminal = Terminal::new(backend).unwrap();
                terminal.draw(|frame| t.app.draw(frame)).unwrap();
                t.app.cursor_position((width, height));
                if matches!(t.app.screen, Screen::Main) {
                    for code in [
                        KeyCode::Char('+'),
                        KeyCode::Tab,
                        KeyCode::Char('-'),
                        KeyCode::End,
                    ] {
                        t.key(code);
                    }
                    drag(&mut t, 2, 8);
                    mouse(&mut t, MouseEventKind::ScrollDown, 0, 0);
                    terminal.draw(|frame| t.app.draw(frame)).unwrap();
                }
            }
        }
    }

    // --- wrapped inputs ---------------------------------------------------

    #[test]
    fn wrap_text_breaks_after_the_last_space_that_fits() {
        assert_eq!(wrap_text("aaa bbb ccc", 8), ["aaa bbb ", "ccc"]);
        // A word longer than a row is split mid-word.
        assert_eq!(wrap_text("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        // A word that exactly fills a row stays whole.
        assert_eq!(wrap_text("abcd efgh", 5), ["abcd ", "efgh"]);
        assert_eq!(wrap_text("abcd efg", 4), ["abcd", " efg", ""]);
        // A full last row gets an empty row after it for the cursor.
        assert_eq!(wrap_text("abcd", 4), ["abcd", ""]);
        assert_eq!(wrap_text("", 4), [""]);
        // Counts chars, not bytes.
        assert_eq!(wrap_text("café olé", 5), ["café ", "olé"]);
    }

    #[test]
    fn wrap_text_keeps_every_char() {
        for text in [
            "a b c d e f g",
            "  leading",
            "trailing  ",
            "x".repeat(30).as_str(),
        ] {
            for width in 1..12 {
                let rows = wrap_text(text, width);
                assert_eq!(rows.concat(), text, "width {width}");
                assert!(rows.iter().all(|r| r.chars().count() <= width));
            }
        }
    }

    /// Index of the first rendered row containing `needle`.
    fn row_with(buf: &Buffer, needle: &str) -> u16 {
        (0..buf.area.height)
            .find(|&y| line(buf, y).contains(needle))
            .unwrap_or_else(|| panic!("no row contains {needle:?}"))
    }

    #[test]
    fn long_project_input_wraps_inside_the_modal() {
        let typed = "a very long project name that keeps going";
        let t = TestApp::new().on_project_step(vec![], typed);
        let buf = render(&t.app);

        // "› " leaves 34 columns per row; the value wraps at a word
        // boundary, continuing under itself.
        let y = row_with(&buf, "› a very long project name");
        assert!(line_from(&buf, y, 22).starts_with("› a very long project name that "));
        assert!(line_from(&buf, y + 1, 22).starts_with("  keeps going "));
        // Both rows stay inside the modal's right border (x = 59).
        assert_eq!(buf[(59, y)].symbol(), "│");
        assert_eq!(buf[(59, y + 1)].symbol(), "│");
        // The modal grew by one row instead of squeezing the list.
        let top = row_with(&buf, "╭ Start session ");
        assert_eq!(
            line_from(&buf, top + START_MODAL_HEIGHT, 20),
            "╰──────────────────────────────────────╯"
        );
        // The cursor follows the text onto the second row.
        assert_eq!(
            t.app.cursor_position((RENDER_WIDTH, RENDER_HEIGHT)),
            Some((22 + 2 + "keeps going".len() as u16, y + 1))
        );
    }

    #[test]
    fn long_notes_wrap_and_grow_the_note_modal() {
        let notes = "fixed the wrapping bug and wrote tests for every input";
        let t = TestApp::new().on_screen(Screen::AddNote(AddNoteForm {
            input: notes.into(),
        }));
        let buf = render(&t.app);

        let y = row_with(&buf, "› fixed");
        assert!(line_from(&buf, y, 22).starts_with("› fixed the wrapping bug and wrote "));
        assert!(line_from(&buf, y + 1, 22).starts_with("  tests for every input "));
        assert_eq!(buf[(59, y + 1)].symbol(), "│");
        // The modal grew by the extra row, keeping its two spare rows below
        // the input.
        assert_eq!(
            line(&buf, y + 2).trim(),
            "│                                      │"
        );
        assert_eq!(
            line(&buf, y + 3).trim(),
            "│                                      │"
        );
        assert!(line_from(&buf, y + 4, 20).starts_with("╰"));
        assert_eq!(
            t.app.cursor_position((RENDER_WIDTH, RENDER_HEIGHT)),
            Some((22 + 2 + "tests for every input".len() as u16, y + 1))
        );
    }

    #[test]
    fn input_taller_than_the_terminal_keeps_the_cursor_row_visible() {
        let note = "word ".repeat(200);
        let t = TestApp::new().on_screen(Screen::AddNote(AddNoteForm { input: note }));
        let (width, height) = (RENDER_WIDTH, 12);
        let input = t
            .app
            .wrapped_input(chrome_layout(ratatui::layout::Rect::new(0, 0, width, height))[1])
            .unwrap();
        // Content is 10 rows; the small modal's 5 other rows leave 5 for input.
        assert_eq!(input.height(), 5);
        let (_, y) = t.app.cursor_position((width, height)).unwrap();
        assert!(
            y < height - 1,
            "cursor row {y} must be above the status line"
        );
    }

    #[test]
    fn add_note_enter_saves_and_esc_discards() {
        let mut t = TestApp::new();
        t.app.start_session("Reading", None, None);
        for (keys, expected) in [("kept", Some("kept\n")), ("dropped", Some("kept\n"))] {
            t.app.handle_key(press(KeyCode::Char('n')));
            for c in keys.chars() {
                t.app.handle_key(press(KeyCode::Char(c)));
            }
            let confirm = if keys == "kept" {
                KeyCode::Enter
            } else {
                KeyCode::Esc
            };
            t.app.handle_key(press(confirm));
            assert!(matches!(t.app.screen, Screen::Main));
            let notes = t.app.active_session.as_ref().unwrap().notes.as_deref();
            assert_eq!(notes, expected);
        }
    }

    /// The session `e` just ended (today's list is newest first).
    fn last_ended(t: &TestApp) -> &Session {
        &t.app.today_sessions[0]
    }

    #[test]
    fn e_ends_at_once_and_a_digit_rates_it_within_the_window() {
        let mut t = TestApp::new();
        t.app.start_session("Reading", None, None);
        t.key(KeyCode::Char('e'));
        assert!(t.app.active_session.is_none());
        assert!(matches!(t.app.screen, Screen::Main));
        assert!(last_ended(&t).ended_at.is_some());
        assert_eq!(last_ended(&t).focus, None);

        t.key(KeyCode::Char('3'));
        assert_eq!(last_ended(&t).focus, Some(3));
        assert!(matches!(
            t.app.current_reaction(),
            Some((Reaction::Rated(3), _))
        ));
        // One rating per window.
        t.key(KeyCode::Char('1'));
        assert_eq!(last_ended(&t).focus, Some(3));
    }

    #[test]
    fn digits_do_nothing_once_the_window_closes() {
        let mut t = TestApp::new();
        t.key(KeyCode::Char('2')); // nothing has ended
        t.app.start_session("Reading", None, None);
        t.key(KeyCode::Char('e'));
        let (id, _) = t.app.rating.unwrap();
        t.app.rating = Some((id, Instant::now() - RATING_WINDOW));
        t.key(KeyCode::Char('2'));
        assert_eq!(last_ended(&t).focus, None);
        assert!(t.app.current_reaction().is_none());
    }

    #[test]
    fn breaks_and_disruptions_end_without_a_window_but_meetings_get_one() {
        for kind in [BREAK, DISRUPTION] {
            let mut t = TestApp::new();
            t.app.start_session(kind, None, None);
            t.key(KeyCode::Char('e'));
            assert!(t.app.active_session.is_none(), "{kind}");
            assert!(!t.app.rating_open(), "{kind}");
        }
        let mut t = TestApp::new();
        t.app.start_session(MEETING, None, None);
        t.key(KeyCode::Char('e'));
        assert!(t.app.rating_open());
    }

    #[test]
    fn switching_away_from_work_opens_the_window_too() {
        let mut t = TestApp::new();
        t.app.start_session("Reading", None, None);
        t.key(KeyCode::Char('m'));
        t.key(KeyCode::Char('2')); // rates the reading, not the meeting
        assert_eq!(activity(&t), Some(MEETING));
        assert_eq!(t.app.today_sessions[1].focus, Some(2));

        // Starting from nothing keeps the window from `e` open.
        let mut t = TestApp::new();
        t.app.start_session("Reading", None, None);
        t.key(KeyCode::Char('e'));
        t.key(KeyCode::Char('b'));
        t.key(KeyCode::Char('1'));
        assert_eq!(t.app.today_sessions[1].focus, Some(1));

        // Ending a break doesn't replace it with nothing to rate.
        t.key(KeyCode::Char('d'));
        assert!(!t.app.rating_open());
    }

    #[test]
    fn undo_reopens_what_e_ended_even_once_rated() {
        let mut t = TestApp::new();
        t.app.start_session("Reading", None, None);
        let id = t.app.active_session.as_ref().unwrap().id;
        t.key(KeyCode::Char('e'));
        t.key(KeyCode::Char('3'));
        t.key(KeyCode::Char('u'));
        let active = t.app.active_session.as_ref().unwrap();
        assert_eq!(active.id, id);
        assert_eq!((active.ended_at, active.focus), (None, None));
        assert!(!t.app.rating_open());
        // Only the last change: a second `u` does nothing.
        t.key(KeyCode::Char('u'));
        assert_eq!(t.app.active_session.as_ref().unwrap().id, id);
    }

    #[test]
    fn undo_takes_back_a_switch_or_a_start() {
        let mut t = TestApp::new();
        t.app
            .start_session("Reading", Some("alpha".into()), Some("beta".into()));
        t.key(KeyCode::Char('d'));
        t.key(KeyCode::Char('u'));
        assert_eq!(activity(&t), Some("Reading"));
        assert_eq!(t.app.today_sessions.len(), 1, "the disruption is gone");

        let mut t = TestApp::new();
        t.key(KeyCode::Char('b'));
        t.key(KeyCode::Char('u'));
        assert!(t.app.active_session.is_none());
        assert!(t.app.today_sessions.is_empty());
    }

    fn activity(t: &TestApp) -> Option<&str> {
        t.app.active_session.as_ref().map(|s| s.activity.as_str())
    }

    #[test]
    fn disrupt_then_resume_restores_previous_session() {
        let mut t = TestApp::new();
        t.app
            .start_session("Reading", Some("alpha".into()), Some("beta".into()));
        t.app.handle_key(press(KeyCode::Char('d')));
        assert_eq!(activity(&t), Some(DISRUPTION));
        t.app.handle_key(press(KeyCode::Char('r')));
        let active = t.app.active_session.as_ref().unwrap();
        assert_eq!(active.activity, "Reading");
        assert_eq!(active.project.as_deref(), Some("alpha"));
        assert_eq!(active.task.as_deref(), Some("beta"));
        assert_eq!(t.app.today_sessions.len(), 3);
    }

    #[test]
    fn disrupt_during_a_disruption_is_a_noop() {
        let mut t = TestApp::new();
        t.app.start_session("Reading", None, None);
        t.app.handle_key(press(KeyCode::Char('d')));
        let id = t.app.active_session.as_ref().unwrap().id;
        t.app.handle_key(press(KeyCode::Char('d')));
        assert_eq!(t.app.active_session.as_ref().unwrap().id, id);
        assert_eq!(t.app.today_sessions.len(), 2);
    }

    #[test]
    fn break_then_resume_restores_previous_session() {
        let mut t = TestApp::new();
        t.app
            .start_session("Reading", Some("alpha".into()), Some("beta".into()));
        t.key(KeyCode::Char('b'));
        assert_eq!(activity(&t), Some(BREAK));
        t.key(KeyCode::Char('b')); // no-op on top of a break
        assert_eq!(t.app.today_sessions.len(), 2);
        t.key(KeyCode::Char('r'));
        let active = t.app.active_session.as_ref().unwrap();
        assert_eq!(active.activity, "Reading");
        assert_eq!(active.project.as_deref(), Some("alpha"));
        assert_eq!(active.task.as_deref(), Some("beta"));
    }

    #[test]
    fn breaks_and_disruptions_switch_and_resume_skips_both() {
        let mut t = TestApp::new();
        t.app.start_session("Writing", None, None);
        t.key(KeyCode::Char('b'));
        t.key(KeyCode::Char('d')); // the break turned into a disruption
        assert_eq!(activity(&t), Some(DISRUPTION));
        t.key(KeyCode::Char('b')); // and back
        assert_eq!(activity(&t), Some(BREAK));
        t.key(KeyCode::Char('r')); // past the break and the disruption
        assert_eq!(activity(&t), Some("Writing"));
    }

    #[test]
    fn meeting_starts_from_work_or_from_nothing_and_resume_returns() {
        let mut t = TestApp::new();
        t.key(KeyCode::Char('m')); // nothing running: starts anyway
        assert_eq!(activity(&t), Some(MEETING));
        t.key(KeyCode::Char('m')); // no-op on top of a meeting
        assert_eq!(t.app.today_sessions.len(), 1);
        t.key(KeyCode::Char('r')); // nothing to go back to: keeps meeting
        assert_eq!(activity(&t), Some(MEETING));

        t.app.close_active_session();
        t.app.start_session("Writing", Some("p".into()), None);
        t.key(KeyCode::Char('m'));
        t.key(KeyCode::Char('b')); // a break straight after the meeting
        t.key(KeyCode::Char('r')); // back to work, past both detours
        let active = t.app.active_session.as_ref().unwrap();
        assert_eq!(active.activity, "Writing");
        assert_eq!(active.project.as_deref(), Some("p"));
    }

    #[test]
    fn break_can_start_from_nothing_but_disruption_cannot() {
        let mut t = TestApp::new();
        t.key(KeyCode::Char('d')); // nothing to interrupt
        assert!(t.app.active_session.is_none());
        t.key(KeyCode::Char('b')); // e.g. right after ending a session
        assert_eq!(activity(&t), Some(BREAK));
    }

    #[test]
    fn resume_with_nothing_to_resume_keeps_the_disruption() {
        let mut t = TestApp::new();
        t.app.start_session(DISRUPTION, None, None);
        t.app.handle_key(press(KeyCode::Char('r')));
        assert_eq!(activity(&t), Some(DISRUPTION));
    }

    #[test]
    fn note_modal_shows_context_and_input() {
        let mut t = TestApp::new();
        t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 0));
        t.app.screen = Screen::AddNote(AddNoteForm {
            input: "hello".into(),
        });
        let buf = render(&t.app);
        assert_eq!(buf[(20, 9)].symbol(), "╭");
        assert_eq!(buf[(20, 9)].fg, Color::Reset);
        assert!(line_from(&buf, 9, 20).starts_with("╭ Add note "));
        // Dimmed project·task·activity context line.
        assert!(line_from(&buf, 10, 22).starts_with("alpha · beta · Programming"));
        assert_eq!(buf[(22, 10)].fg, Color::Gray);
        // Input row: prompt + typed text.
        assert!(line_from(&buf, 11, 22).starts_with("› hello"));
        assert_eq!(buf[(22, 11)].fg, Color::Cyan);
        assert_eq!(buf[(24, 11)].fg, Color::Reset);
    }
}
