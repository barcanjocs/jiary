use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, ListState, Paragraph, Tabs};

use crate::db::Db;
use crate::session::Session;
use crate::theme;
use crossterm::event::KeyCode;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

const ACTIVITIES: &[&str] = &[
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

    projects: Vec<String>,
    tasks: Vec<String>,
    screen: Screen,
    /// "Previous notes" for the active session's project/task. Cached because
    /// draw() runs ~10x/second; refreshed when the active session changes.
    previous_notes: Vec<String>,
    /// Last error that occurred, kept as plain data so the UI renders it in
    /// exactly one place (and a future auto-clear-on-success can too).
    error: Option<String>,
}

enum Screen {
    Main,
    StartSession(StartSessionForm),
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
    focus: Option<i32>,
}
struct AddNoteForm {
    input: String,
}

struct StartSessionForm {
    step: Step,
    activity_index: usize,
    project_input: String,
    task_input: String,
    project_query: String,
    task_query: String,
    project_selection: usize,
    task_selection: usize,
    task_candidates: Vec<String>,
}

#[derive(Clone, Copy)]
enum Step {
    Activity,
    Project,
    Task,
}

impl App {
    pub fn new(db: Db) -> Self {
        let mut app = Self {
            db,
            active_session: None,
            today_sessions: Vec::new(),
            projects: Vec::new(),
            tasks: Vec::new(),
            screen: Screen::Main,
            previous_notes: Vec::new(),
            error: None,
        };

        // Attempt every load so partial data still shows; failures are recorded
        // for display instead of being swallowed.
        match app.db.get_active_session() {
            Ok(session) => app.active_session = session,
            Err(e) => app.set_error(e),
        }
        app.refresh_after_session_change();
        match app.db.distinct_projects() {
            Ok(projects) => app.projects = projects,
            Err(e) => app.set_error(e),
        }
        match app.db.distinct_tasks() {
            Ok(tasks) => app.tasks = tasks,
            Err(e) => app.set_error(e),
        }

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

    /// Creates a new session in the db and mirrors it in memory as the active
    /// session. On failure records the error and leaves the active session
    /// unchanged.
    fn start_session(&mut self, activity: &str, project: Option<String>, task: Option<String>) {
        match self
            .db
            .create_session(activity, project.as_deref(), task.as_deref())
        {
            Ok(id) => {
                self.clear_error();
                self.active_session = Some(Session {
                    id,
                    started_at: Utc::now(),
                    ended_at: None,
                    project,
                    task,
                    activity: activity.to_string(),
                    notes: None,
                    focus: None,
                    interruptions: None,
                });
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

    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        // Returns true if the app should quit
        match &self.screen {
            Screen::Main => return self.handle_main(key),
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

                if !self.close_active_session() {
                    return false;
                }

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

                let (activity, project, task) = combo;
                self.start_session(&activity, project, task);

                self.refresh_after_session_change();
            }

            KeyCode::Char('d') => {
                // Only start a Disruption if there was a session to interrupt.
                let had_active = self.active_session.is_some();
                if had_active && self.close_active_session() {
                    self.start_session("Disruption", None, None);
                }
                self.refresh_after_session_change();
            }
            KeyCode::Char('s') => {
                self.screen = Screen::StartSession(StartSessionForm {
                    step: Step::Activity,
                    activity_index: 0,
                    project_input: String::new(),
                    task_input: String::new(),
                    project_query: String::new(),
                    task_query: String::new(),
                    project_selection: 0,
                    task_selection: 0,
                    task_candidates: Vec::new(),
                });
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
                    focus: None,
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
            KeyCode::Up => {
                if form.activity_index > 0 {
                    form.activity_index -= 1;
                }
            }
            KeyCode::Down => {
                if form.activity_index < ACTIVITIES.len() - 1 {
                    form.activity_index += 1;
                }
            }
            KeyCode::Char(c) if c >= '1' && (c as usize - '1' as usize) < ACTIVITIES.len() => {
                form.activity_index = c as usize - '1' as usize;
                form.step = Step::Project;
            }
            KeyCode::Enter => form.step = Step::Project,
            _ => {}
        }
    }

    fn start_project_key(&mut self, key: KeyCode) {
        let Screen::StartSession(form) = &mut self.screen else {
            return;
        };
        let suggestions = fuzzy_filter(&form.project_query, &self.projects);
        match key {
            KeyCode::Char(c) => {
                form.project_input.push(c);
                form.project_query.push(c);
                form.project_selection = 0;
            }
            KeyCode::Backspace => {
                form.project_input.pop();
                form.project_query.pop();
                form.project_selection = 0;
            }
            KeyCode::Up => {
                if form.project_selection > 0 {
                    form.project_selection -= 1;
                }
            }
            KeyCode::Down => {
                if form.project_selection < suggestions.len().saturating_sub(1) {
                    form.project_selection += 1;
                }
            }
            KeyCode::Tab => {
                if !suggestions.is_empty() {
                    // Accept the highlighted suggestion; advance to the
                    // next one only if it was already accepted.
                    if form.project_input == *suggestions[form.project_selection] {
                        form.project_selection = (form.project_selection + 1) % suggestions.len();
                    }
                    form.project_input = suggestions[form.project_selection].clone();
                }
            }
            KeyCode::Enter => {
                let candidates = if form.project_input.is_empty() {
                    self.tasks.clone()
                } else {
                    self.db
                        .tasks_for_project(&form.project_input)
                        .unwrap_or_default()
                };
                form.task_candidates = candidates;
                form.step = Step::Task;
            }
            _ => {}
        }
    }

    fn start_task_key(&mut self, key: KeyCode) {
        let Screen::StartSession(form) = &mut self.screen else {
            return;
        };
        let suggestions = fuzzy_filter(&form.task_query, &form.task_candidates);
        match key {
            KeyCode::Char(c) => {
                form.task_input.push(c);
                form.task_query.push(c);
                form.task_selection = 0;
            }
            KeyCode::Backspace => {
                form.task_input.pop();
                form.task_query.pop();
                form.task_selection = 0;
            }
            KeyCode::Up => {
                if form.task_selection > 0 {
                    form.task_selection -= 1;
                }
            }
            KeyCode::Down => {
                if form.task_selection < suggestions.len().saturating_sub(1) {
                    form.task_selection += 1;
                }
            }
            KeyCode::Tab => {
                if !suggestions.is_empty() {
                    // Accept the highlighted suggestion; advance to the
                    // next one only if it was already accepted.
                    if form.task_input == *suggestions[form.task_selection] {
                        form.task_selection = (form.task_selection + 1) % suggestions.len();
                    }
                    form.task_input = suggestions[form.task_selection].clone();
                }
            }
            KeyCode::Enter => {
                // Copy everything out of `form` before any call that
                // needs &mut self, so the screen borrow is dead. Empty fields
                // become None so the db stores NULL, not "".
                let activity = ACTIVITIES[form.activity_index];
                let project = if form.project_input.is_empty() {
                    None
                } else {
                    Some(form.project_input.clone())
                };
                let task = if form.task_input.is_empty() {
                    None
                } else {
                    Some(form.task_input.clone())
                };

                // Auto-stop any currently active session. If that fails,
                // abort: creating anyway would leave two active sessions
                // (the db trigger rejects it, but failing early surfaces
                // the real error instead of the generic one).
                if !self.close_active_session() {
                    return;
                }

                self.start_session(activity, project, task);

                self.refresh_after_session_change();
                self.projects = self.db.distinct_projects().unwrap_or_default();
                self.tasks = self.db.distinct_tasks().unwrap_or_default();
                self.screen = Screen::Main;
            }
            _ => {}
        }
    }

    /// Handles keys while ending a session (notes, then focus rating).
    fn handle_end_session(&mut self, key: KeyCode) {
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
            KeyCode::Enter | KeyCode::Esc => {
                // Clone notes out of `form` before calling into self.
                let notes = form.notes.clone();
                if key != KeyCode::Esc {
                    form.step = EndStep::Focus;
                }

                if !notes.is_empty() {
                    self.append_active_note(&notes);
                }

                if key == KeyCode::Esc {
                    self.complete_active(None);
                    self.screen = Screen::Main;
                }
            }
            _ => {}
        }
    }

    fn end_focus_key(&mut self, key: KeyCode) {
        let Screen::EndSession(form) = &mut self.screen else {
            return;
        };
        match key {
            KeyCode::Char(c) if ('1'..='3').contains(&c) => {
                form.focus = Some(c as i32 - '0' as i32);
                // Copy out of `form` before calling into self.
                let focus = form.focus;
                self.complete_active(focus);
                self.screen = Screen::Main;
            }
            KeyCode::Enter | KeyCode::Esc => {
                // Copy out of `form` before calling into self.
                let focus = form.focus;
                self.complete_active(focus);
                self.screen = Screen::Main;
            }
            _ => {}
        }
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
            KeyCode::Enter | KeyCode::Esc => {
                if !form.input.is_empty() {
                    let note = form.input.clone();
                    self.append_active_note(&note);
                }
                self.screen = Screen::Main;
            }
            _ => {}
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();

        // Global chrome: header on top, status line at the bottom, screen
        // content in between. The rows are fixed (no conditional layout), so
        // the UI never jumps when an error appears or disappears.
        let [header, content, status] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(area);

        frame.render_widget(self.header_line(), header);

        match &self.screen {
            Screen::Main => self.draw_main(frame, content),
            Screen::StartSession(form) => self.draw_start_session(frame, content, form),
            Screen::EndSession(form) => self.draw_end_session(frame, content, form),
            Screen::AddNote(form) => self.draw_add_note(frame, content, form),
        }

        frame.render_widget(self.status_line(), status);
    }

    /// Terminal cursor position for the current screen: `Some` on input lines
    /// (start form's project/task steps, end form's notes step, add-note),
    /// right after the typed text. Pure function of the terminal size and form
    /// state — `Terminal::draw` discards closure return values, so main.rs
    /// calls this after drawing to show or hide the cursor.
    pub fn cursor_position(&self, size: (u16, u16)) -> Option<(u16, u16)> {
        let area = ratatui::layout::Rect::new(0, 0, size.0, size.1);
        match &self.screen {
            Screen::StartSession(form) => {
                let (_, _, body_area) = start_modal_layout(area);
                let [input_area, _] =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body_area);
                match form.step {
                    Step::Project => Some(input_cursor(
                        input_area,
                        "Project".len() + 2,
                        form.project_input.len(),
                    )),
                    Step::Task => Some(input_cursor(
                        input_area,
                        "Task".len() + 2,
                        form.task_input.len(),
                    )),
                    Step::Activity => None,
                }
            }
            Screen::EndSession(form) if form.step == EndStep::Notes => {
                let (_, inner) = modal_layout(area, MODAL_WIDTH, SMALL_MODAL_HEIGHT);
                let [_, body_area] =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
                Some(input_cursor(body_area, "Notes".len() + 2, form.notes.len()))
            }
            Screen::AddNote(form) => {
                let (_, inner) = modal_layout(area, MODAL_WIDTH, SMALL_MODAL_HEIGHT);
                let [_, input_area] =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
                Some(input_cursor(input_area, "Note".len() + 2, form.input.len()))
            }
            _ => None,
        }
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
    /// key hints for the current screen. The only place errors and key hints
    /// are rendered.
    fn status_line(&self) -> Line<'static> {
        if let Some(err) = &self.error {
            return Line::from(Span::styled(format!("✗ {err}"), theme::ERROR));
        }
        let hints: &[(&str, &str)] = match &self.screen {
            Screen::Main => &[
                ("s", "start"),
                ("e", "end"),
                ("n", "note"),
                ("i", "interrupt"),
                ("d", "disrupt"),
                ("r", "resume"),
                ("q", "quit"),
            ],
            Screen::StartSession(_) => &[("Tab", "accept"), ("Enter", "next"), ("Esc", "cancel")],
            Screen::EndSession(form) => match form.step {
                EndStep::Notes => &[("Enter", "continue"), ("Esc", "skip")],
                EndStep::Focus => &[("1", "bad"), ("2", "ok"), ("3", "good"), ("Enter", "skip")],
            },
            Screen::AddNote(_) => &[("Enter", "save"), ("Esc", "cancel")],
        };
        let mut spans = Vec::new();
        for (key, desc) in hints {
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

    fn draw_main(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        // The active session is a bordered panel; when there is none, a single
        // centered dimmed line takes its place. Today's timeline fills the rest.
        let (top, height) = match &self.active_session {
            Some(session) => {
                let lines = self.active_panel_lines(session);
                let height = lines.len() as u16 + 2; // + top/bottom border
                (
                    Paragraph::new(lines).block(
                        Block::bordered()
                            .border_type(BorderType::Rounded)
                            .border_style(Style::new().fg(theme::ACCENT))
                            .title(Line::from(Span::styled(" ACTIVE ", theme::TITLE))),
                    ),
                    Constraint::Length(height),
                )
            }
            None => (
                Paragraph::new("No active session.")
                    .style(theme::DIMMED)
                    .alignment(Alignment::Center),
                Constraint::Length(1),
            ),
        };

        let [top_area, timeline_area] = Layout::vertical([height, Constraint::Min(0)]).areas(area);
        frame.render_widget(top, top_area);
        self.draw_timeline(frame, timeline_area);
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
            for note in &self.previous_notes {
                lines.push(Line::from(Span::styled(
                    format!("  • {note}"),
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

    /// Today's sessions (completed plus the running one) as a multi-line list:
    /// bold time range + duration, colored focus badge, bright project·task·
    /// activity, dimmed notes.
    fn draw_timeline(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        if self.today_sessions.is_empty() {
            return;
        }

        // A dimmed section label anchors the list below the active panel.
        let [label_area, list_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "TODAY",
                Style::new().fg(theme::MUTED).add_modifier(Modifier::BOLD),
            ))),
            label_area,
        );

        let items = self
            .today_sessions
            .iter()
            .map(|session| ListItem::new(self.timeline_item_lines(session)))
            .collect::<Vec<_>>();
        frame.render_widget(List::new(items), list_area);
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

        lines.push(Line::default()); // spacing between sessions
        lines
    }

    /// The start form is a centered fixed-size modal: a `Tabs` row for
    /// Activity·Project·Task with the current step selected, then the step's
    /// content (a picker `List`, or an input line plus suggestion `List`).
    fn draw_start_session(
        &self,
        frame: &mut Frame,
        area: ratatui::layout::Rect,
        form: &StartSessionForm,
    ) {
        let (modal, tabs_area, body_area) = start_modal_layout(area);
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
            Step::Activity => {
                let items = ACTIVITIES
                    .iter()
                    .map(|a| ListItem::new(*a))
                    .collect::<Vec<_>>();
                frame.render_stateful_widget(
                    picker_list(items),
                    body_area,
                    &mut ListState::default().with_selected(Some(form.activity_index)),
                );
            }
            Step::Project => {
                let [input_area, list_area] =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body_area);

                frame.render_widget(
                    Paragraph::new(theme::kv("Project", form.project_input.clone())),
                    input_area,
                );

                let suggestions = fuzzy_filter(&form.project_query, &self.projects);
                if !suggestions.is_empty() {
                    let items = suggestions
                        .iter()
                        .map(|s| ListItem::new(s.clone()))
                        .collect::<Vec<_>>();
                    frame.render_stateful_widget(
                        picker_list(items),
                        list_area,
                        &mut ListState::default().with_selected(Some(form.project_selection)),
                    );
                }
            }
            Step::Task => {
                let [input_area, list_area] =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body_area);

                frame.render_widget(
                    Paragraph::new(theme::kv("Task", form.task_input.clone())),
                    input_area,
                );

                let suggestions = fuzzy_filter(&form.task_query, &form.task_candidates);
                if !suggestions.is_empty() {
                    let items = suggestions
                        .iter()
                        .map(|s| ListItem::new(s.clone()))
                        .collect::<Vec<_>>();
                    frame.render_stateful_widget(
                        picker_list(items),
                        list_area,
                        &mut ListState::default().with_selected(Some(form.task_selection)),
                    );
                }
            }
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
    ) {
        let (modal, inner) = modal_layout(area, MODAL_WIDTH, SMALL_MODAL_HEIGHT);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(Span::styled(" End session ", theme::TITLE)));
        frame.render_widget(block, modal);

        let [tabs_area, body_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
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

        match form.step {
            EndStep::Notes => {
                frame.render_widget(
                    Paragraph::new(theme::kv("Notes", form.notes.clone())),
                    body_area,
                );
            }
            EndStep::Focus => {
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
    /// note input line (cursor placed by main.rs).
    fn draw_add_note(&self, frame: &mut Frame, area: ratatui::layout::Rect, form: &AddNoteForm) {
        let (modal, inner) = modal_layout(area, MODAL_WIDTH, SMALL_MODAL_HEIGHT);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(Span::styled(" Add note ", theme::TITLE)));
        frame.render_widget(block, modal);

        let [context_area, input_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
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
        frame.render_widget(
            Paragraph::new(theme::kv("Note", form.input.clone())),
            input_area,
        );
    }
}

/// Fixed size of the modals; centered in the content area and shrunk to fit
/// on tiny terminals (see `centered_rect`). The start form needs room for the
/// activity list; end/note forms only hold a tabs/context line plus one input.
const MODAL_WIDTH: u16 = 40;
const START_MODAL_HEIGHT: u16 = 12;
const SMALL_MODAL_HEIGHT: u16 = 6;

/// A picker `List` with the accent highlight symbol and style.
fn picker_list<'a>(items: Vec<ListItem<'a>>) -> List<'a> {
    List::new(items)
        .highlight_symbol(theme::picker_symbol())
        .highlight_style(theme::PICKER_HIGHLIGHT)
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

/// Layout of the start-session modal: (modal rect, tabs row, body). Shared by
/// the renderer and `cursor_position` so the two can never disagree.
fn start_modal_layout(
    content: ratatui::layout::Rect,
) -> (
    ratatui::layout::Rect,
    ratatui::layout::Rect,
    ratatui::layout::Rect,
) {
    let (modal, inner) = modal_layout(content, MODAL_WIDTH, START_MODAL_HEIGHT);
    let [tabs_area, body_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    (modal, tabs_area, body_area)
}

/// Cursor position for an input line rendered as `prefix + text`: right after
/// the visible text, clamped to the last column so it stays on the line when
/// the text overflows (Paragraph truncates rather than wraps).
fn input_cursor(area: ratatui::layout::Rect, prefix_len: usize, text_len: usize) -> (u16, u16) {
    let col = (prefix_len + text_len).min(area.width.saturating_sub(1) as usize);
    (area.x + col as u16, area.y)
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
            Self {
                app: App::new(db),
                dir,
            }
        }

        /// Puts the app on the Project step of the start-session form.
        fn on_project_step(
            mut self,
            projects: Vec<String>,
            input: &str,
            query: &str,
            selection: usize,
        ) -> Self {
            self.app.projects = projects;
            self.app.screen = Screen::StartSession(StartSessionForm {
                step: Step::Project,
                activity_index: 0,
                project_input: input.to_string(),
                task_input: String::new(),
                project_query: query.to_string(),
                task_query: String::new(),
                project_selection: selection,
                task_selection: 0,
                task_candidates: Vec::new(),
            });
            self
        }

        /// Puts the app on an arbitrary screen (render tests of the chrome).
        fn on_screen(mut self, screen: Screen) -> Self {
            self.app.screen = screen;
            self
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

    // --- Tab completion -----------------------------------------------

    #[test]
    fn tab_accepts_first_suggestion() {
        let mut t = TestApp::new().on_project_step(vec!["alpha".into(), "beta".into()], "", "", 0);
        t.app.handle_key(KeyCode::Tab);
        let f = t.form();
        assert_eq!(f.project_input, "alpha");
        assert_eq!(f.project_selection, 0);
    }

    #[test]
    fn tab_advances_when_input_matches_highlighted() {
        // Input already equals the highlighted suggestion, so Tab moves on.
        let mut t =
            TestApp::new().on_project_step(vec!["alpha".into(), "beta".into()], "alpha", "", 0);
        t.app.handle_key(KeyCode::Tab);
        let f = t.form();
        assert_eq!(f.project_input, "beta");
        assert_eq!(f.project_selection, 1);
    }

    #[test]
    fn tab_wraps_around_to_first() {
        let mut t =
            TestApp::new().on_project_step(vec!["alpha".into(), "beta".into()], "beta", "", 1);
        t.app.handle_key(KeyCode::Tab);
        let f = t.form();
        assert_eq!(f.project_input, "alpha");
        assert_eq!(f.project_selection, 0);
    }

    #[test]
    fn tab_after_typing_accepts_highlighted_not_next() {
        // Regression (61d9181): after typing, selection is 0 and a single Tab
        // must accept the highlighted suggestion rather than skip past it.
        let mut t =
            TestApp::new().on_project_step(vec!["alpha".into(), "alpine".into()], "a", "a", 0);
        t.app.handle_key(KeyCode::Tab);
        assert_eq!(t.form().project_input, "alpha");
    }

    #[test]
    fn backspace_resets_selection() {
        let mut t =
            TestApp::new().on_project_step(vec!["alpha".into(), "alpine".into()], "al", "al", 1);
        t.app.handle_key(KeyCode::Backspace);
        let f = t.form();
        assert_eq!(f.project_input, "a");
        assert_eq!(f.project_query, "a");
        assert_eq!(f.project_selection, 0);
    }

    #[test]
    fn tab_is_noop_when_no_suggestions() {
        let mut t = TestApp::new().on_project_step(vec!["alpha".into()], "zzz", "zzz", 0);
        t.app.handle_key(KeyCode::Tab);
        assert_eq!(t.form().project_input, "zzz");
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
            "[s] start  [e] end  [n] note  [i] interrupt  [d] disrupt  [r] resume  [q] quit"
        );
    }

    #[test]
    fn status_line_shows_start_form_hints() {
        let t = TestApp::new().on_project_step(vec![], "", "", 0);
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
            focus: None,
        }));
        let buf = render(&t.app);
        assert_eq!(
            line(&buf, RENDER_HEIGHT - 1),
            "[Enter] continue  [Esc] skip"
        );
    }

    #[test]
    fn status_line_shows_end_focus_hints() {
        let t = TestApp::new().on_screen(Screen::EndSession(EndSessionForm {
            notes: String::new(),
            step: EndStep::Focus,
            focus: None,
        }));
        let buf = render(&t.app);
        assert_eq!(
            line(&buf, RENDER_HEIGHT - 1),
            "[1] bad  [2] ok  [3] good  [Enter] skip"
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
}
