use std::path::PathBuf;

use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Tabs,
};

use crate::db::Db;
use crate::layout::{Areas, ContentHeights, LayoutPrefs, Section};
use crate::session::Session;
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
    "Meeting",
    "Administration",
    "Disruption",
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
    scroll: usize,
    /// Terminal size as of the last frame. Key and mouse handlers lay the
    /// main screen out from it, so they see exactly what was drawn.
    size: (u16, u16),
    /// While a border is dragged with the mouse: the section whose height
    /// it sets (see `border_at`).
    drag: Option<Section>,
}

enum Screen {
    Main,
    Help,
    // Boxed: three autocompletes make it far larger than the other variants.
    StartSession(Box<StartSessionForm>),
    EndSession(EndSessionForm),
    AddNote(AddNoteForm),
}

#[derive(Clone, Copy, PartialEq)]
enum EndStep {
    Notes,
    Focus,
}

struct EndSessionForm {
    notes: String,
    step: EndStep,
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
            scroll: 0,
            size: (0, 0),
            drag: None,
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

    /// Closes the currently active session, if any. Returns true on success
    /// (or when there was nothing to close) and clears the error line; on
    /// failure restores the session, records the error, and returns false so
    /// the caller can abort (e.g. not start a replacement session).
    fn close_active_session(&mut self) -> bool {
        match self.active_session.take() {
            None => true,
            Some(session) => match self.db.complete_session(session.id, None) {
                Ok(()) => {
                    self.clear_error();
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
                self.active_session = Some(session);
            }
            Err(e) => self.set_error(e),
        }
    }

    /// Ends the active session with the given focus rating (if any) and
    /// refreshes the main screen lists. On failure restores the session and
    /// records the error. No-op if there is no active session.
    fn complete_active(&mut self, focus: Option<i32>) {
        if let Some(session) = self.active_session.take() {
            if let Err(e) = self.db.complete_session(session.id, focus) {
                self.set_error(e);
                self.active_session = Some(session);
            } else {
                self.clear_error();
            }
            self.refresh_after_session_change();
        }
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
            Screen::EndSession(_) => self.handle_end_session(key),
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

            // Today's sessions is the only section that scrolls, so the
            // scroll keys act on it whichever section has focus.
            KeyCode::Up | KeyCode::Char('k') => self.scroll_today(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_today(1),
            KeyCode::PageUp => self.scroll_today(-(self.today_viewport() as isize)),
            KeyCode::PageDown => self.scroll_today(self.today_viewport() as isize),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.max_scroll(),

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
                        }
                    }
                }
            }
            KeyCode::Char('r') => {
                let should_proceed = match &self.active_session {
                    None => true,
                    Some(s) if s.activity == "Disruption" => true,
                    Some(_) => false,
                };

                if !should_proceed {
                    return false;
                }

                // Look up what to resume before closing the disruption, so
                // it keeps running when there is nothing to resume.
                let combo = match self.db.latest_non_disruption() {
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

            KeyCode::Char('d') => {
                // Only start a Disruption if there was a session to interrupt,
                // and not another one on top of a running Disruption.
                let interruptible = self
                    .active_session
                    .as_ref()
                    .is_some_and(|s| s.activity != "Disruption");
                if interruptible && self.close_active_session() {
                    self.start_session("Disruption", None, None);
                    self.refresh_after_session_change();
                }
            }
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
                self.screen = Screen::EndSession(EndSessionForm {
                    notes: String::new(),
                    step: EndStep::Notes,
                });
            }
            _ => {}
        };
        false
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

    /// Handles keys while ending a session (notes, then focus rating). Esc
    /// at either step cancels and leaves the session running.
    fn handle_end_session(&mut self, key: KeyCode) {
        if key == KeyCode::Esc {
            self.screen = Screen::Main;
            return;
        }

        // Read the step by copy so the borrow of self.screen drops before the
        // per-step handler takes &mut self.
        let step = match &self.screen {
            Screen::EndSession(form) => form.step,
            _ => return,
        };
        match step {
            EndStep::Notes => self.end_notes_key(key),
            EndStep::Focus => self.end_focus_key(key),
        }
    }

    fn end_notes_key(&mut self, key: KeyCode) {
        let Screen::EndSession(form) = &mut self.screen else {
            return;
        };
        match key {
            KeyCode::Char(c) => {
                form.notes.push(c);
            }
            KeyCode::Backspace => {
                form.notes.pop();
            }
            KeyCode::Enter => form.step = EndStep::Focus,
            _ => {}
        }
    }

    fn end_focus_key(&mut self, key: KeyCode) {
        let Screen::EndSession(form) = &mut self.screen else {
            return;
        };
        let focus = match key {
            KeyCode::Char(c) if ('1'..='3').contains(&c) => Some(c as i32 - '0' as i32),
            KeyCode::Enter => None,
            _ => return,
        };
        // Notes are only saved once the session actually ends, so cancelling
        // at either step leaves the session untouched. Copy them out of
        // `form` before calling into self.
        let notes = std::mem::take(&mut form.notes);
        if !notes.is_empty() {
            self.append_active_note(&notes);
        }
        self.complete_active(focus);
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
            Screen::EndSession(form) => self.draw_end_session(frame, content, form, input),
            Screen::AddNote(_) => self.draw_add_note(frame, content, input),
        }

        frame.render_widget(self.status_line(status.width), status);
    }

    /// Terminal cursor position for the current screen: `Some` on input lines
    /// (start form's project/task steps, end form's notes step, add-note),
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

    /// The current screen's text input, if it has one: (label, value).
    fn text_input(&self) -> Option<(&'static str, &str)> {
        match &self.screen {
            Screen::StartSession(form) => match form.step {
                Step::Project => Some(("Project", &form.project.input)),
                Step::Task => Some(("Task", &form.task.input)),
                Step::Activity => Some(("Activity", &form.activity.input)),
            },
            Screen::EndSession(form) if form.step == EndStep::Notes => Some(("Notes", &form.notes)),
            Screen::AddNote(form) => Some(("Note", &form.input)),
            _ => None,
        }
    }

    /// The current screen's text input wrapped to its modal's width, with at
    /// most as many rows as the modal can grow to on `content`. Shared by
    /// `draw` and `cursor_position` so the two always agree.
    fn wrapped_input(&self, content: ratatui::layout::Rect) -> Option<WrappedInput> {
        let (label, value) = self.text_input()?;
        let base_height = match self.screen {
            Screen::StartSession(_) => START_MODAL_HEIGHT,
            _ => SMALL_MODAL_HEIGHT,
        };
        let max_rows = content.height.saturating_sub(base_height - 1).max(1);
        Some(WrappedInput::new(
            label,
            value,
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
            // Most useful first for the focused section; `[?] help` is always
            // shown and lists everything.
            Screen::Main => match self.focus {
                Section::Active => &[
                    ("s", "start"),
                    ("e", "end"),
                    ("n", "note"),
                    ("i", "interrupt"),
                    ("d", "disrupt"),
                    ("r", "resume"),
                    ("Tab", "sections"),
                    ("q", "quit"),
                ],
                Section::Today => &[
                    ("↑↓", "scroll"),
                    ("Space", "collapse"),
                    ("+/-", "resize"),
                    ("Tab", "next"),
                    ("s", "start"),
                    ("e", "end"),
                    ("q", "quit"),
                ],
                Section::Summary => &[
                    ("Space", "collapse"),
                    ("+/-", "resize"),
                    ("Tab", "next"),
                    ("s", "start"),
                    ("e", "end"),
                    ("q", "quit"),
                ],
            },
            Screen::Help => &[("any key", "close")],
            Screen::StartSession(_) => &[("Tab", "accept"), ("Enter", "next"), ("Esc", "cancel")],
            Screen::EndSession(form) => match form.step {
                EndStep::Notes => &[("Enter", "continue"), ("Esc", "cancel")],
                EndStep::Focus => &[
                    ("1", "bad"),
                    ("2", "ok"),
                    ("3", "good"),
                    ("Enter", "skip"),
                    ("Esc", "cancel"),
                ],
            },
            Screen::AddNote(_) => &[("Enter", "save"), ("Esc", "cancel")],
        };
        let help = matches!(self.screen, Screen::Main).then_some(("?", "help"));

        let hint_width =
            |(key, desc): &(&str, &str)| key.chars().count() + desc.chars().count() + 3;
        // Saturating: terminals can be narrower than `[?] help` itself.
        let budget =
            (width as usize).saturating_sub(help.as_ref().map_or(0, |h| hint_width(h) + 2));
        let mut used = 0;
        let mut shown = Vec::new();
        for hint in hints {
            let needed = hint_width(hint) + if shown.is_empty() { 0 } else { 2 };
            if used + needed > budget {
                break;
            }
            used += needed;
            shown.push(*hint);
        }
        shown.extend(help);

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

    /// Handles a mouse event; only the main screen uses the mouse. Dragging
    /// a section border resizes, clicking focuses the section under the
    /// pointer (and clicking a collapsible section's title row collapses or
    /// expands it), and the wheel scrolls today's sessions.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.screen, Screen::Main) {
            return;
        }
        let [_, content, _] = chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        let areas = self.resolve_layout(content);
        let (column, row) = (mouse.column, mouse.row);

        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_today(-3),
            MouseEventKind::ScrollDown => self.scroll_today(3),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(section) = self.border_at(&areas, row) {
                    self.drag = Some(section);
                    self.focus = section;
                    return;
                }
                let position = ratatui::layout::Position::new(column, row);
                let Some(section) = [Section::Active, Section::Today, Section::Summary]
                    .into_iter()
                    .find(|&s| areas.get(s).contains(position))
                else {
                    return;
                };
                self.focus = section;
                if section != Section::Active && row == areas.get(section).y {
                    self.layout.toggle_collapsed(section);
                    self.save_layout();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let heights = self.content_heights(content.width);
                match self.drag {
                    Some(Section::Active) => {
                        let height = row as i32 - areas.active.y as i32 + 1;
                        self.layout.set_active_height(height, content, heights);
                    }
                    Some(Section::Summary) => {
                        let height = content.bottom() as i32 - row as i32 - 1;
                        self.layout.set_summary_height(height, content, heights);
                    }
                    _ => {}
                }
            }
            MouseEventKind::Up(MouseButton::Left) if self.drag.is_some() => {
                self.drag = None;
                self.save_layout();
            }
            _ => {}
        }
    }

    /// The draggable border on `row`, as the section whose height it sets:
    /// the active section's bottom border sets the active section's height,
    /// and today's bottom border sets the summary's (only while both are
    /// expanded; otherwise the summary just fills the space).
    fn border_at(&self, areas: &Areas, row: u16) -> Option<Section> {
        let bottom_border = |area: Rect| area.height > 0 && row + 1 == area.bottom();
        if bottom_border(areas.active) {
            Some(Section::Active)
        } else if !self.layout.today_collapsed
            && !self.layout.summary_collapsed
            && bottom_border(areas.today)
        {
            Some(Section::Summary)
        } else {
            None
        }
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
        ContentHeights {
            active: self.active_lines().len() as u16 + 2,
            summary: self.summary_lines(width.saturating_sub(2)).len() as u16 + 2,
        }
    }

    /// Rows of today's sessions visible inside the section's border.
    fn today_viewport(&self) -> usize {
        if self.layout.today_collapsed {
            return 0;
        }
        self.main_areas().today.height.saturating_sub(2) as usize
    }

    fn max_scroll(&self) -> usize {
        self.timeline_lines()
            .len()
            .saturating_sub(self.today_viewport())
    }

    fn scroll_today(&mut self, delta: isize) {
        self.scroll = self
            .scroll
            .saturating_add_signed(delta)
            .min(self.max_scroll());
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

        let block = self.section_block(Section::Active, " ACTIVE ".to_string());
        let inner = block.inner(areas.active);
        frame.render_widget(block, areas.active);
        frame.render_widget(Paragraph::new(self.active_lines()), inner);

        let sessions = plural(self.today_sessions.len(), "session");
        if self.layout.today_collapsed {
            let text = format!("TODAY · {sessions} · {}", fmt_hms(self.total_today()));
            self.draw_collapsed(frame, areas.today, Section::Today, text);
        } else {
            self.draw_today(frame, areas.today, &sessions);
        }

        if self.layout.summary_collapsed {
            let text = self.summary_one_liner();
            self.draw_collapsed(frame, areas.summary, Section::Summary, text);
        } else {
            let block = self.section_block(Section::Summary, " ▾ SUMMARY ".to_string());
            let inner = block.inner(areas.summary);
            frame.render_widget(block, areas.summary);
            frame.render_widget(Paragraph::new(self.summary_lines(inner.width)), inner);
        }
    }

    /// A section's rounded border: accent while focused, dimmed otherwise.
    fn section_block(&self, section: Section, title: String) -> Block<'static> {
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
    fn draw_collapsed(&self, frame: &mut Frame, area: Rect, section: Section, text: String) {
        let style = if self.focus == section {
            theme::TITLE
        } else {
            theme::DIMMED.add_modifier(Modifier::BOLD)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!("▸ {text}"), style)),
            area,
        );
    }

    /// Today's sessions, scrolled to `scroll` (clamped to the content), with
    /// a scrollbar on the right border when they don't all fit.
    fn draw_today(&self, frame: &mut Frame, area: Rect, sessions: &str) {
        let block = self.section_block(Section::Today, format!(" ▾ TODAY · {sessions} "));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines = self.timeline_lines();
        let total = lines.len();
        let visible = inner.height as usize;
        let offset = self.scroll.min(total.saturating_sub(visible));
        frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), inner);

        if total > visible {
            let mut state = ScrollbarState::new(total.saturating_sub(visible))
                .position(offset)
                .viewport_content_length(visible);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .track_symbol(Some("│"))
                    .thumb_style(Style::new().fg(theme::ACCENT)),
                area.inner(Margin::new(0, 1)),
                &mut state,
            );
        }
    }

    /// Body of the active section: the running session, or a hint.
    fn active_lines(&self) -> Vec<Line<'static>> {
        match &self.active_session {
            Some(session) => self.active_panel_lines(session),
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

    /// Lines inside the active-session panel: bold timer, dimmed label/value
    /// rows, interruptions colored when > 0, notes as dimmed bullets.
    fn active_panel_lines(&self, session: &Session) -> Vec<Line<'static>> {
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

        // Cached; refreshed whenever the active session changes.
        if !self.previous_notes.is_empty() {
            lines.push(Line::from(Span::styled("Previous notes:", theme::DIMMED)));
            // Each entry is one session's notes: newline-separated, one
            // bullet per line.
            for line in self.previous_notes.iter().flat_map(|notes| notes.lines()) {
                lines.push(Line::from(Span::styled(
                    format!("  • {line}"),
                    theme::DIMMED,
                )));
            }
        }

        // Current session's own notes.
        if let Some(notes) = &session.notes
            && !notes.is_empty()
        {
            lines.push(Line::from(Span::styled("Session notes:", theme::DIMMED)));
            for line in notes.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  • {line}"),
                    theme::DIMMED,
                )));
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

    /// End form: small centered modal with a Notes·Focus `Tabs` row; the notes
    /// step is an input line (cursor placed by main.rs), the focus step a
    /// colored `[1] Bad [2] OK [3] Good` line.
    fn draw_end_session(
        &self,
        frame: &mut Frame,
        area: ratatui::layout::Rect,
        form: &EndSessionForm,
        input: Option<&WrappedInput>,
    ) {
        let rows = input.map_or(1, WrappedInput::height);
        let (modal, tabs_area, body_area) = small_modal_layout(area, rows);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(Span::styled(" End session ", theme::TITLE)));
        frame.render_widget(block, modal);

        let step_index = match form.step {
            EndStep::Notes => 0,
            EndStep::Focus => 1,
        };
        frame.render_widget(
            Tabs::new(["Notes", "Focus"])
                .select(step_index)
                .highlight_style(Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
            tabs_area,
        );

        match (form.step, input) {
            (EndStep::Notes, Some(input)) => frame.render_widget(input.paragraph(), body_area),
            (EndStep::Notes, None) => {}
            (EndStep::Focus, _) => {
                let line = Line::from(vec![
                    Span::styled("[1] Bad  ", Style::new().fg(theme::BAD)),
                    Span::styled("[2] OK  ", Style::new().fg(theme::OK)),
                    Span::styled("[3] Good", Style::new().fg(theme::GOOD)),
                ]);
                frame.render_widget(Paragraph::new(line), body_area);
            }
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
const HELP_WIDTH: u16 = 50;

/// A picker `List` with the accent highlight symbol and style.
fn picker_list<'a>(items: Vec<ListItem<'a>>) -> List<'a> {
    List::new(items)
        .highlight_symbol(theme::picker_symbol())
        .highlight_style(theme::PICKER_HIGHLIGHT)
}

/// An autocomplete step's suggestion list, with the selected suggestion (if
/// any) highlighted. The input itself is drawn as a `WrappedInput`.
fn draw_suggestions(frame: &mut Frame, area: ratatui::layout::Rect, field: &Autocomplete) {
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

/// A text input wrapped to fit its modal: the dimmed label on the first row,
/// the value word-wrapped beside it with a hanging indent so every row starts
/// at the same column, and the cursor right after the last character.
struct WrappedInput {
    lines: Vec<Line<'static>>,
    /// Cursor (column, row), relative to the input area.
    cursor: (u16, u16),
}

impl WrappedInput {
    /// Wraps `value` beside `label` within `width` columns. When that takes
    /// more than `max_rows` rows, only the last ones are kept: the cursor is
    /// always on the last row, so it stays visible.
    fn new(label: &str, value: &str, width: u16, max_rows: u16) -> Self {
        let indent = label.chars().count() + 2; // as in `theme::kv`
        let rows = wrap_text(value, (width as usize).saturating_sub(indent));
        let last = rows.len() - 1;
        let cursor_col = indent + rows[last].chars().count();

        let mut lines = hanging_lines(label, rows);
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

/// The help screen: every key, grouped. A centered modal sized to its
/// content (shrunk on small terminals).
fn draw_help(frame: &mut Frame, content: Rect) {
    const SECTIONS: &[(&str, &[(&str, &str)])] = &[
        (
            "Sessions",
            &[
                ("s", "start a session"),
                ("e", "end the active session"),
                ("n", "add a note"),
                ("i", "count an interruption"),
                ("d", "start a disruption"),
                ("r", "resume after a disruption"),
                ("q / Ctrl+C", "quit"),
            ],
        ),
        (
            "Sections",
            &[
                ("Tab / Shift+Tab", "focus next / previous"),
                ("Space", "collapse or expand"),
                ("+ / -", "grow or shrink"),
                ("=", "reset sizes"),
                ("↑↓ j k PgUp PgDn", "scroll today's sessions"),
                ("Home / End", "jump to first / last"),
            ],
        ),
        (
            "Mouse",
            &[
                ("drag a border", "resize"),
                ("click a title", "collapse or expand"),
                ("wheel", "scroll today's sessions"),
            ],
        ),
    ];

    let key_width = 18;
    let mut lines = Vec::new();
    // No blank rows between groups: the bold headings separate them, and
    // the whole list fits a 24-row terminal.
    for (heading, keys) in SECTIONS {
        lines.push(Line::from(Span::styled(
            heading.to_string(),
            Style::new().fg(theme::MUTED).add_modifier(Modifier::BOLD),
        )));
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

    let (modal, inner) = modal_layout(content, HELP_WIDTH, lines.len() as u16 + 2);
    frame.render_widget(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(Span::styled(" Keys ", theme::TITLE))),
        modal,
    );
    frame.render_widget(Paragraph::new(lines), inner);
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
            "[s] start  [e] end  [n] note  [i] interrupt  [d] disrupt  [r] resume  [?] help"
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
        assert_eq!(text(120).matches('[').count(), 9); // everything, quit included
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
    fn status_line_shows_end_notes_hints() {
        let t = TestApp::new().on_screen(Screen::EndSession(EndSessionForm {
            notes: String::new(),
            step: EndStep::Notes,
        }));
        let buf = render(&t.app);
        assert_eq!(
            line(&buf, RENDER_HEIGHT - 1),
            "[Enter] continue  [Esc] cancel"
        );
    }

    #[test]
    fn status_line_shows_end_focus_hints() {
        let t = TestApp::new().on_screen(Screen::EndSession(EndSessionForm {
            notes: String::new(),
            step: EndStep::Focus,
        }));
        let buf = render(&t.app);
        assert_eq!(
            line(&buf, RENDER_HEIGHT - 1),
            "[1] bad  [2] ok  [3] good  [Enter] skip  [Esc] cancel"
        );
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
        assert_eq!(
            line(&buf, 2),
            format!(
                "│No active session. Press s to start one.{}│",
                " ".repeat(38)
            )
        );
        assert_eq!(buf[(1, 2)].fg, Color::Gray);
        // Content-fitting: border, hint, border.
        assert!(line(&buf, 3).starts_with("╰"));
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
        t.app.complete_active(None);
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
        // Below the 3-row active section; inner text starts at column 1.
        assert!(line(&buf, 4).starts_with("╭ ▾ TODAY · 1 session "));
        assert!(line_from(&buf, 5, 1).starts_with("10:00 – 11:30 (1h 30m) F- I:0 "));
        assert!(buf[(1, 5)].modifier.contains(Modifier::BOLD)); // time range
        let dur_col = col_of(&buf, 5, "1h 30m");
        assert!(buf[(dur_col, 5)].modifier.contains(Modifier::BOLD)); // duration
        assert!(line_from(&buf, 6, 1).starts_with("alpha · beta · Programming "));
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
        // Input row at y=8 shows the selection; suggestions from y=9. The
        // selected item gets a bold accent `>` and highlighted text (list
        // content always sits one column right of the symbol).
        assert!(line_from(&buf, 8, 22).starts_with("Activity  Writing"));
        assert!(line_from(&buf, 9, 21).starts_with("  Programming"));
        assert_eq!(buf[(23, 9)].fg, Color::Reset); // unselected item text
        assert!(line_from(&buf, 11, 21).starts_with(" >Writing"));
        assert_eq!(buf[(22, 11)].fg, Color::Cyan);
        assert!(buf[(22, 11)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(23, 11)].fg, Color::Cyan);
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
        t.app.complete_active(None);
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
        // Input row: dimmed label + the highlighted suggestion.
        assert!(line_from(&buf, 8, 22).starts_with("Project  alpine"));
        assert_eq!(buf[(22, 8)].fg, Color::Gray);
        assert_eq!(buf[(31, 8)].fg, Color::Reset);
        // Suggestions below; the selection is highlighted.
        assert!(line_from(&buf, 9, 23).starts_with("alpha"));
        assert!(line_from(&buf, 10, 22).starts_with(">alpine"));
        assert_eq!(buf[(22, 10)].fg, Color::Cyan);
    }

    #[test]
    fn end_modal_notes_step_shows_tab_and_input() {
        let t = TestApp::new().on_screen(Screen::EndSession(EndSessionForm {
            notes: "did stuff".into(),
            step: EndStep::Notes,
        }));
        let buf = render(&t.app);
        assert_eq!(buf[(20, 9)].symbol(), "╭");
        assert_eq!(buf[(20, 9)].fg, Color::Reset);
        assert!(line_from(&buf, 9, 20).starts_with("╭ End session "));
        // Notes tab selected, Focus plain.
        assert!(line_from(&buf, 10, 22).starts_with(" Notes │ Focus"));
        assert_eq!(buf[(23, 10)].fg, Color::Cyan);
        assert!(buf[(23, 10)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(31, 10)].fg, Color::Reset);
        // Input row: dimmed label + typed text.
        assert!(line_from(&buf, 11, 22).starts_with("Notes  did stuff"));
        assert_eq!(buf[(22, 11)].fg, Color::Gray);
        assert_eq!(buf[(29, 11)].fg, Color::Reset);
    }

    #[test]
    fn end_modal_focus_step_shows_colored_ratings() {
        let t = TestApp::new().on_screen(Screen::EndSession(EndSessionForm {
            notes: String::new(),
            step: EndStep::Focus,
        }));
        let buf = render(&t.app);
        // Focus tab selected, Notes plain.
        assert_eq!(buf[(31, 10)].fg, Color::Cyan);
        assert!(buf[(31, 10)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(23, 10)].fg, Color::Reset);
        // Colored rating line.
        assert!(line_from(&buf, 11, 22).starts_with("[1] Bad  [2] OK  [3] Good"));
        assert_eq!(buf[(col_of(&buf, 11, "Bad"), 11)].fg, Color::Red);
        assert_eq!(buf[(col_of(&buf, 11, "OK"), 11)].fg, Color::Yellow);
        assert_eq!(buf[(col_of(&buf, 11, "Good"), 11)].fg, Color::Green);
    }

    #[test]
    fn cursor_position_tracks_input_lines() {
        // 80×24 terminal: the start modal's input row is y=8 (values start at
        // column 31), the end/note modals' is y=11.
        let t = TestApp::new().on_project_step(vec![], "");
        assert_eq!(t.app.cursor_position((80, 24)), Some((31, 8)));

        let t = TestApp::new().on_project_step(vec![], "ab");
        assert_eq!(t.app.cursor_position((80, 24)), Some((33, 8)));

        let t = TestApp::new().on_screen(TestApp::start_form_at(Step::Task));
        assert_eq!(t.app.cursor_position((80, 24)), Some((28, 8))); // "Task  "

        let t = TestApp::new().on_screen(TestApp::start_form_at(Step::Activity));
        assert_eq!(t.app.cursor_position((80, 24)), Some((32, 8))); // "Activity  "

        let t = TestApp::new().on_screen(Screen::EndSession(EndSessionForm {
            notes: "hi".into(),
            step: EndStep::Notes,
        }));
        assert_eq!(t.app.cursor_position((80, 24)), Some((31, 11)));

        let t = TestApp::new().on_screen(Screen::EndSession(EndSessionForm {
            notes: String::new(),
            step: EndStep::Focus,
        }));
        assert_eq!(t.app.cursor_position((80, 24)), None); // rating line, no input

        let t = TestApp::new().on_screen(Screen::AddNote(AddNoteForm { input: "hi".into() }));
        assert_eq!(t.app.cursor_position((80, 24)), Some((30, 11))); // "Note  hi"

        let t = TestApp::new();
        assert_eq!(t.app.cursor_position((80, 24)), None); // main screen
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
        assert_eq!(line(&buf, 4), "▸ TODAY · 2 sessions · 1h 0m");
        assert_eq!(buf[(0, 4)].fg, Color::Cyan); // still focused
        // The summary takes the freed rows.
        assert!(line(&buf, 5).starts_with("╭ ▾ SUMMARY "));
        assert!(line(&buf, RENDER_HEIGHT - 2).starts_with("╰"));

        t.key(KeyCode::Tab);
        t.key(KeyCode::Char(' '));
        let buf = render(&t.app);
        assert_eq!(
            line(&buf, 5),
            "▸ SUMMARY · tracked 1h 0m · Programming 1h 0m"
        );
        assert_eq!(line(&buf, 6), "");

        t.key(KeyCode::Char(' ')); // and back
        assert!(line(&render(&t.app), 5).starts_with("╭ ▾ SUMMARY "));
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
        assert!(line_from(&buf, y + 3, 1).contains("· tracked 1h 0m · gaps 10m"));
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
        let mut t = with_sessions(TestApp::new(), 2);
        t.key(KeyCode::Down); // everything fits: nothing to scroll
        assert_eq!(t.app.scroll, 0);

        let mut t = with_sessions(TestApp::new(), 10);
        // 10 items × 2 rows + 9 separators = 29 rows; the viewport is 11.
        assert_eq!(t.app.today_viewport(), 11);
        assert_eq!(t.app.max_scroll(), 18);
        t.key(KeyCode::Down);
        t.key(KeyCode::Char('j'));
        assert_eq!(t.app.scroll, 2);
        t.key(KeyCode::PageDown);
        assert_eq!(t.app.scroll, 13);
        t.key(KeyCode::PageDown);
        assert_eq!(t.app.scroll, 18);
        t.key(KeyCode::Home);
        assert_eq!(t.app.scroll, 0);
        t.key(KeyCode::Up);
        assert_eq!(t.app.scroll, 0);
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
        assert_eq!(t.app.layout.active_height, Some(5));
        assert!(line(&render(&t.app), 5).starts_with("╰"));
        t.key(KeyCode::Char('-'));
        assert_eq!(t.app.layout.active_height, Some(4));

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
                active_height: Some(4),
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
        // No session: the active section is rows 1–3, its border on row 3.
        drag(&mut t, 3, 7);
        assert_eq!(t.app.layout.active_height, Some(7));
        assert!(line(&render(&t.app), 7).starts_with("╰"));
        assert!(t.app.drag.is_none());
        // Clamped so today's sessions keep their minimum.
        drag(&mut t, 7, 100);
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

    #[test]
    fn clicking_focuses_and_title_rows_toggle_collapse() {
        let mut t = TestApp::new();
        let click = MouseEventKind::Down(MouseButton::Left);
        let buf = render(&t.app);
        let today = row_with(&buf, "╭ ▾ TODAY ");

        mouse(&mut t, click, 10, today + 1); // inside the body: focus only
        assert_eq!(t.app.focus, Section::Today);
        assert!(!t.app.layout.today_collapsed);

        mouse(&mut t, click, 10, today); // the title row
        assert!(t.app.layout.today_collapsed);
        mouse(&mut t, click, 10, today); // now the collapsed line
        assert!(!t.app.layout.today_collapsed);
    }

    #[test]
    fn wheel_scrolls_todays_sessions() {
        let mut t = with_sessions(TestApp::new(), 10);
        mouse(&mut t, MouseEventKind::ScrollDown, 10, 10);
        assert_eq!(t.app.scroll, 3);
        mouse(&mut t, MouseEventKind::ScrollUp, 10, 10);
        mouse(&mut t, MouseEventKind::ScrollUp, 10, 10);
        assert_eq!(t.app.scroll, 0);
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
                Screen::EndSession(EndSessionForm {
                    notes: "x".repeat(200),
                    step: EndStep::Notes,
                }),
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

        let y = row_with(&buf, "Project  a very long project name");
        // The value wraps at a word boundary, continuing under itself.
        assert!(line_from(&buf, y, 22).starts_with("Project  a very long project name "));
        assert!(line_from(&buf, y + 1, 22).starts_with("         that keeps going "));
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
            Some((22 + 9 + "that keeps going".len() as u16, y + 1))
        );
    }

    #[test]
    fn long_notes_wrap_and_grow_the_end_modal() {
        let notes = "fixed the wrapping bug and wrote tests for every input";
        let t = TestApp::new().on_screen(Screen::EndSession(EndSessionForm {
            notes: notes.into(),
            step: EndStep::Notes,
        }));
        let buf = render(&t.app);

        // "Notes  " leaves 29 columns per row.
        let y = row_with(&buf, "Notes  fixed");
        assert!(line_from(&buf, y, 22).starts_with("Notes  fixed the wrapping bug and "));
        assert!(line_from(&buf, y + 1, 22).starts_with("       wrote tests for every input "));
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
            Some((22 + 7 + "wrote tests for every input".len() as u16, y + 1))
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

    #[test]
    fn end_session_saves_notes_and_focus() {
        let mut t = TestApp::new();
        t.app.start_session("Reading", None, None);
        for code in [
            KeyCode::Char('e'),
            KeyCode::Char('o'),
            KeyCode::Char('k'),
            KeyCode::Enter,
            KeyCode::Char('3'),
        ] {
            t.app.handle_key(press(code));
        }
        assert!(t.app.active_session.is_none());
        let ended = &t.app.today_sessions[0];
        assert_eq!(ended.notes.as_deref(), Some("ok\n"));
        assert_eq!(ended.focus, Some(3));
    }

    #[test]
    fn end_session_esc_cancels_at_either_step() {
        let mut t = TestApp::new();
        t.app.start_session("Reading", None, None);
        for steps in [&[][..], &[KeyCode::Enter][..]] {
            t.app.handle_key(press(KeyCode::Char('e')));
            t.app.handle_key(press(KeyCode::Char('x')));
            for &code in steps {
                t.app.handle_key(press(code));
            }
            t.app.handle_key(press(KeyCode::Esc));
            assert!(matches!(t.app.screen, Screen::Main));
            let active = t.app.active_session.as_ref().unwrap();
            assert!(active.ended_at.is_none());
            assert!(active.notes.is_none(), "cancelled notes must not be saved");
        }
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
        assert_eq!(activity(&t), Some("Disruption"));
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
    fn resume_with_nothing_to_resume_keeps_the_disruption() {
        let mut t = TestApp::new();
        t.app.start_session("Disruption", None, None);
        t.app.handle_key(press(KeyCode::Char('r')));
        assert_eq!(activity(&t), Some("Disruption"));
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
        // Input row: dimmed label + typed text.
        assert!(line_from(&buf, 11, 22).starts_with("Note  hello"));
        assert_eq!(buf[(22, 11)].fg, Color::Gray);
        assert_eq!(buf[(28, 11)].fg, Color::Reset);
    }
}
