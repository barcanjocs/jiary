use chrono::Utc;
use ratatui::Frame;
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, Paragraph};

use crate::db::Db;
use crate::session::Session;
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
}

enum Screen {
    Main,
    StartSession(StartSessionForm),
    EndSession(EndSessionForm),
    AddNote(AddNoteForm),
}

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

enum Step {
    Activity,
    Project,
    Task,
}

impl App {
    pub fn new(db: Db) -> Self {
        let active_session = db.get_active_session().unwrap_or(None);
        let today_sessions = db.sessions_for_today().unwrap_or_default();
        let projects = db.distinct_projects().unwrap_or_default();
        let tasks = db.distinct_tasks().unwrap_or_default();
        Self {
            db,
            active_session,
            today_sessions,
            projects,
            tasks,
            screen: Screen::Main,
        }
    }

    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        // Returns true if the app should quit
        match &mut self.screen {
            Screen::Main => match key {
                KeyCode::Char('q') => return true,
                KeyCode::Char('i') => {
                    if let Some(session) = &mut self.active_session {
                        if let Err(e) = self.db.increment_interruptions(session.id) {
                            eprintln!("{e}");
                        } else {
                            session.interruptions = Some(session.interruptions.unwrap_or(0) + 1);
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

                    if let Some(session) = self.active_session.take() {
                        if let Err(e) = self.db.complete_session(session.id, None, None) {
                            eprintln!("{e}");
                            self.active_session = Some(session);
                            return false;
                        }
                    }

                    let combo = match self.db.latest_non_disruption() {
                        Ok(Some(combo)) => combo,
                        Ok(None) => return false,
                        Err(e) => {
                            eprintln!("{e}");
                            return false;
                        }
                    };

                    let (activity, project, task) = combo;
                    match self
                        .db
                        .create_session(&activity, project.as_deref(), task.as_deref())
                    {
                        Ok(id) => {
                            self.active_session = Some(Session {
                                id,
                                started_at: Utc::now(),
                                ended_at: None,
                                project,
                                task,
                                activity,
                                notes: None,
                                outcome: None,
                                focus: None,
                                interruptions: None,
                            });
                        }
                        Err(e) => eprintln!("{e}"),
                    }

                    self.today_sessions = self.db.sessions_for_today().unwrap_or_default();
                }

                KeyCode::Char('d') => {
                    if let Some(old) = self.active_session.take() {
                        match self.db.complete_session(old.id, None, None) {
                            Err(e) => {
                                eprintln!("{e}");
                                self.active_session = Some(old);
                            }
                            Ok(()) => match self.db.create_session("Disruption", None, None) {
                                Ok(id) => {
                                    self.active_session = Some(Session {
                                        id,
                                        started_at: Utc::now(),
                                        ended_at: None,
                                        project: None,
                                        task: None,
                                        activity: "Disruption".to_string(),
                                        notes: None,
                                        outcome: None,
                                        focus: None,
                                        interruptions: None,
                                    });
                                }
                                Err(e) => eprintln!("{e}"),
                            },
                        }
                    }
                    self.today_sessions = self.db.sessions_for_today().unwrap_or_default();
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
                KeyCode::Char('e') => {
                    if self.active_session.is_some() {
                        self.screen = Screen::EndSession(EndSessionForm {
                            notes: String::new(),
                            step: EndStep::Notes,
                            focus: None,
                        });
                    }
                }
                _ => {}
            },
            Screen::AddNote(form) => match key {
                KeyCode::Char(c) => form.input.push(c),
                KeyCode::Backspace => {
                    form.input.pop();
                }
                KeyCode::Enter | KeyCode::Esc => {
                    if !form.input.is_empty() {
                        let note = form.input.clone();
                        if let Some(session) = &mut self.active_session {
                            if let Err(e) = self.db.append_note(session.id, &note) {
                                eprintln!("{e}");
                            } else {
                                session.notes = Some(match &session.notes {
                                    None => note,
                                    Some(existing) => format!("{}\n{}", existing, note),
                                });
                            }
                        }
                    }
                    self.screen = Screen::Main;
                }
                _ => {}
            },
            Screen::StartSession(form) => {
                if key == KeyCode::Esc {
                    self.screen = Screen::Main;
                    return false;
                }

                match &mut form.step {
                    Step::Activity => match key {
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
                        KeyCode::Char(c) if c >= '1' && c <= '7' => {
                            form.activity_index = (c as u8 - b'1') as usize;
                            form.step = Step::Project;
                        }
                        KeyCode::Enter => form.step = Step::Project,
                        _ => {}
                    },
                    Step::Project => {
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
                                    if form.project_input.is_empty() {
                                        form.project_selection = 0;
                                    } else {
                                        form.project_selection =
                                            (form.project_selection + 1) % suggestions.len();
                                    }
                                    form.project_input =
                                        suggestions[form.project_selection].clone();
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
                    Step::Task => {
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
                                    if form.task_input.is_empty() {
                                        form.task_selection = 0;
                                    } else {
                                        form.task_selection =
                                            (form.task_selection + 1) % suggestions.len();
                                    }
                                    form.task_input = suggestions[form.task_selection].clone();
                                }
                            }
                            KeyCode::Enter => {
                                // Auto-stop any currently active session
                                if let Some(old) = self.active_session.take() {
                                    if let Err(e) = self.db.complete_session(old.id, None, None) {
                                        eprintln!("{e}");
                                        self.active_session = Some(old);
                                    }
                                }

                                let activity = ACTIVITIES[form.activity_index];
                                let project = form.project_input.clone();
                                let task = form.task_input.clone();

                                match self.db.create_session(
                                    activity,
                                    if project.is_empty() {
                                        None
                                    } else {
                                        Some(&project)
                                    },
                                    if task.is_empty() { None } else { Some(&task) },
                                ) {
                                    Ok(id) => {
                                        self.active_session = Some(Session {
                                            id,
                                            started_at: Utc::now(),
                                            ended_at: None,
                                            project: if project.is_empty() {
                                                None
                                            } else {
                                                Some(project)
                                            },
                                            task: if task.is_empty() { None } else { Some(task) },
                                            activity: activity.to_string(),
                                            notes: None,
                                            outcome: None,
                                            focus: None,
                                            interruptions: None,
                                        });
                                    }
                                    Err(e) => eprintln!("{e}"),
                                }

                                self.today_sessions =
                                    self.db.sessions_for_today().unwrap_or_default();
                                self.projects = self.db.distinct_projects().unwrap_or_default();
                                self.tasks = self.db.distinct_tasks().unwrap_or_default();
                                self.screen = Screen::Main;
                            }
                            _ => {}
                        }
                    }
                }
            }
            Screen::EndSession(form) => match &form.step {
                EndStep::Notes => match key {
                    KeyCode::Char(c) => {
                        form.notes.push(c);
                    }
                    KeyCode::Backspace => {
                        form.notes.pop();
                    }
                    KeyCode::Enter | KeyCode::Esc => {
                        if !form.notes.is_empty() {
                            if let Some(session) = &mut self.active_session {
                                if let Err(e) = self.db.append_note(session.id, &form.notes) {
                                    eprintln!("{e}");
                                } else {
                                    session.notes = Some(match &session.notes {
                                        None => form.notes.clone(),
                                        Some(existing) => format!("{}\n{}", existing, form.notes),
                                    });
                                }
                            }
                        }

                        if key == KeyCode::Esc {
                            if let Some(session) = self.active_session.take() {
                                if let Err(e) = self.db.complete_session(session.id, None, None) {
                                    eprintln!("{e}");
                                    self.active_session = Some(session);
                                }
                                self.today_sessions =
                                    self.db.sessions_for_today().unwrap_or_default();
                            }
                            self.screen = Screen::Main;
                        } else {
                            form.step = EndStep::Focus;
                        }
                    }
                    _ => {}
                },
                EndStep::Focus => match key {
                    KeyCode::Char(c) if c >= '1' && c <= '3' => {
                        form.focus = Some(c as i32 - '0' as i32);
                        if let Some(session) = self.active_session.take() {
                            if let Err(e) = self.db.complete_session(session.id, None, form.focus) {
                                eprintln!("{e}");
                                self.active_session = Some(session);
                            }
                            self.today_sessions = self.db.sessions_for_today().unwrap_or_default();
                        }
                        self.screen = Screen::Main;
                    }
                    KeyCode::Enter | KeyCode::Esc => {
                        if let Some(session) = self.active_session.take() {
                            if let Err(e) = self.db.complete_session(session.id, None, form.focus) {
                                eprintln!("{e}");
                                self.active_session = Some(session);
                            }
                            self.today_sessions = self.db.sessions_for_today().unwrap_or_default();
                        }
                        self.screen = Screen::Main;
                    }
                    _ => {}
                },
            },
        }
        false
    }

    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();

        match &self.screen {
            Screen::Main => self.draw_main(frame, area),
            Screen::StartSession(form) => self.draw_start_session(frame, area, form),
            Screen::EndSession(form) => {
                let lines = match &form.step {
                    EndStep::Notes => vec![
                        Line::from(format!("Notes: {}", form.notes)),
                        Line::from(""),
                        Line::from("(Enter continue · Esc skip)"),
                    ],
                    EndStep::Focus => vec![
                        Line::from("Focus:"),
                        Line::from(""),
                        Line::from("(1 bad · 2 ok · 3 good · Enter skip)"),
                    ],
                };
                let paragraph =
                    Paragraph::new(lines).block(Block::default().title(" End session "));
                frame.render_widget(paragraph, area);
            }
            Screen::AddNote(form) => {
                let mut lines: Vec<Line> = vec![];

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

                    lines.push(Line::from(parts));
                    if let Some(notes) = &session.notes {
                        if !notes.is_empty() {
                            lines.push(Line::from(""));
                            for line in notes.lines() {
                                lines.push(Line::from(format!("  • {line}")));
                            }
                        }
                    }
                }

                lines.push(Line::from(""));
                lines.push(Line::from(format!("> {}", form.input)));
                lines.push(Line::from(""));
                lines.push(Line::from("(Enter to save, Esc to cancel)"));

                let paragraph = Paragraph::new(lines).block(Block::default().title(" Add note "));
                frame.render_widget(paragraph, area);
            }
        }
    }

    fn draw_main(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let mut lines: Vec<Line> = vec![Line::from("JIARY — TODAY")];

        // Active session
        if let Some(session) = &self.active_session {
            let elapsed = Utc::now() - session.started_at;
            let h = elapsed.num_hours();
            let m = elapsed.num_minutes() % 60;
            let s = elapsed.num_seconds() % 60;

            lines.push(Line::from(""));
            lines.push(Line::from("ACTIVE"));
            lines.push(Line::from(format!(
                "Project:  {}",
                session.project.as_deref().unwrap_or("(none)")
            )));
            lines.push(Line::from(format!(
                "Task:     {}",
                session.task.as_deref().unwrap_or("(none)")
            )));
            lines.push(Line::from(format!("Activity: {}", session.activity)));

            if let (Some(project), Some(task)) = (&session.project, &session.task) {
                if let Ok(notes) = self.db.recent_notes(project, task) {
                    if !notes.is_empty() {
                        lines.push(Line::from("Previous notes:"));
                        for note in notes {
                            lines.push(Line::from(format!("  • {note}")));
                        }
                    }
                }
            }

            // Current session's own notes
            if let Some(notes) = &session.notes {
                if !notes.is_empty() {
                    lines.push(Line::from("Session notes:"));
                    for line in notes.lines() {
                        lines.push(Line::from(format!("  • {line}")));
                    }
                }
            }

            lines.push(Line::from(""));
            lines.push(Line::from(format!("{h:02}:{m:02}:{s:02}")));
            lines.push(Line::from(format!(
                "Interruptions: {}",
                session.interruptions.unwrap_or(0)
            )));
        } else {
            lines.push(Line::from(""));
            lines.push(Line::from("No active session."));
        }

        // Completed sessions timeline
        if !self.today_sessions.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from("─────────────────────────────"));

            for session in &self.today_sessions {
                let start = fmt_time(&session.started_at);
                let end = session
                    .ended_at
                    .as_ref()
                    .map(fmt_time)
                    .unwrap_or_else(|| "??:??".to_string());
                let mut detail = session
                    .ended_at
                    .as_ref()
                    .map(|end| format!("  ({})", fmt_duration(&session.started_at, end)))
                    .unwrap_or_default();

                detail.push_str(&match session.focus {
                    Some(f) => format!(" (F:{f})"),
                    None => " (F:-)".to_string(),
                });
                detail.push_str(&format!(" (I:{})", session.interruptions.unwrap_or(0)));

                lines.push(Line::from(format!("{} ────── {}{}", start, end, detail)));
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

                lines.push(Line::from(format!("     {parts}")));

                if let Some(nts) = &session.notes {
                    for line in nts.lines() {
                        lines.push(Line::from(format!("     • {line}")));
                    }
                }
                lines.push(Line::from(""));
            }
        }

        // Footer
        lines.push(Line::from(
            "[s] start  [e] end  [n] note  [i] int  [d] disrupt  [r] resume  [q] quit",
        ));
        let paragraph = Paragraph::new(lines).block(Block::default().title(" Jiary "));
        frame.render_widget(paragraph, area);
    }

    fn draw_start_session(
        &self,
        frame: &mut Frame,
        area: ratatui::layout::Rect,
        form: &StartSessionForm,
    ) {
        match &form.step {
            Step::Activity => {
                let items: Vec<ListItem> = ACTIVITIES
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let marker = if i == form.activity_index { ">" } else { " " };
                        ListItem::new(format!("{} {} {}", i + 1, marker, a))
                    })
                    .collect();

                let list = List::new(items)
                    .block(Block::default().title(" Activity (↑↓ or 1-6, Enter, Esc) "));
                frame.render_widget(list, area);
            }
            Step::Project => {
                let suggestions = fuzzy_filter(&form.project_query, &self.projects);
                let mut lines = vec![
                    Line::from(format!("Project: {}", form.project_input)),
                    Line::from(""),
                ];

                for (i, s) in suggestions.iter().enumerate() {
                    let marker = if i == form.project_selection {
                        ">"
                    } else {
                        " "
                    };
                    lines.push(Line::from(format!("  {marker} {s}")));
                }

                lines.push(Line::from(""));
                lines.push(Line::from("(Tab accept · Enter confirm · Esc cancel)"));

                let paragraph =
                    Paragraph::new(lines).block(Block::default().title(" Start session "));
                frame.render_widget(paragraph, area);
            }
            Step::Task => {
                let suggestions = fuzzy_filter(&form.task_query, &form.task_candidates);
                let mut lines = vec![
                    Line::from(format!("Task: {}", form.task_input)),
                    Line::from(""),
                ];

                for (i, s) in suggestions.iter().enumerate() {
                    let marker = if i == form.task_selection { ">" } else { " " };
                    lines.push(Line::from(format!("  {marker} {s}")));
                }

                lines.push(Line::from(""));
                lines.push(Line::from("(Tab accept · Enter confirm · Esc cancel)"));

                let paragraph =
                    Paragraph::new(lines).block(Block::default().title(" Start session "));
                frame.render_widget(paragraph, area);
            }
        }
    }
}

fn fmt_time(dt: &chrono::DateTime<chrono::Utc>) -> String {
    dt.with_timezone(&chrono::Local).format("%H:%M").to_string()
}
fn fmt_duration(
    start: &chrono::DateTime<chrono::Utc>,
    end: &chrono::DateTime<chrono::Utc>,
) -> String {
    let dur = *end - *start;
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
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(5).map(|(_, c)| c.clone()).collect()
}
