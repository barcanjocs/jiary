use chrono::Utc;
use ratatui::Frame;
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, Paragraph};

use crossterm::event::KeyCode;

use crate::db::Db;
use crate::session::Session;

const ACTIVITIES: &[&str] = &[
    "Programming",
    "Reading",
    "Writing",
    "Meeting",
    "Administration",
    "Other",
];

pub struct App {
    db: Db,
    active_session: Option<Session>,
    screen: Screen,
}
enum Screen {
    Main,
    NewSession(NewSessionForm),
    StoppingSession(StoppingForm),
}

struct StoppingForm {
    notes: String,
}
struct NewSessionForm {
    step: Step,
    activity_index: usize,
    project_input: String,
    task_input: String,
}

enum Step {
    Activity,
    Project,
    Task,
}

impl App {
    pub fn new(db: Db) -> Self {
        let active_session = db.get_active_session().unwrap_or(None);
        Self {
            db,
            active_session,
            screen: Screen::Main,
        }
    }

    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        // Returns true if the app should quit
        match &mut self.screen {
            Screen::Main => match key {
                KeyCode::Char('q') => return true,
                KeyCode::Char('n') => {
                    self.screen = Screen::NewSession(NewSessionForm {
                        step: Step::Activity,
                        activity_index: 0,
                        project_input: String::new(),
                        task_input: String::new(),
                    });
                }
                KeyCode::Char('s') => {
                    if self.active_session.is_some() {
                        self.screen = Screen::StoppingSession(StoppingForm {
                            notes: String::new(),
                        });
                    }
                }
                _ => {}
            },
            Screen::NewSession(form) => {
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
                        KeyCode::Char(c) if c >= '1' && c <= '6' => {
                            form.activity_index = (c as u8 - b'1') as usize;
                            form.step = Step::Project;
                        }

                        KeyCode::Enter => form.step = Step::Project,
                        _ => {}
                    },
                    Step::Project => match key {
                        KeyCode::Char(c) => form.project_input.push(c),
                        KeyCode::Backspace => {
                            form.project_input.pop();
                        }
                        KeyCode::Enter => form.step = Step::Task,
                        _ => {}
                    },
                    Step::Task => match key {
                        KeyCode::Char(c) => form.task_input.push(c),
                        KeyCode::Backspace => {
                            form.task_input.pop();
                        }
                        KeyCode::Enter => {
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
                                        description: None,
                                        outcome: None,
                                        focus: None,
                                        interruptions: None,
                                    });
                                }
                                Err(e) => eprintln!("{e}"),
                            }
                            self.screen = Screen::Main;
                        }
                        _ => {}
                    },
                }
            }
            Screen::StoppingSession(form) => match key {
                KeyCode::Char(c) => form.notes.push(c),
                KeyCode::Backspace => {
                    form.notes.pop();
                }
                KeyCode::Enter | KeyCode::Esc => {
                    let notes = if form.notes.is_empty() {
                        None
                    } else {
                        Some(form.notes.clone())
                    };

                    if let Some(session) = self.active_session.take() {
                        let desc = notes.as_deref();
                        if let Err(e) = self.db.complete_session(session.id, desc, None, None, None)
                        {
                            eprintln!("{e}");
                            self.active_session = Some(session);
                        }
                    }
                    self.screen = Screen::Main;
                }
                _ => {}
            },
        }
        false
    }

    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();

        match &self.screen {
            Screen::Main => self.draw_main(frame, area),
            Screen::NewSession(form) => self.draw_new_session(frame, area, form),
            Screen::StoppingSession(form) => {
                let lines = vec![
                    Line::from(format!("Notes: {}", form.notes)),
                    Line::from(""),
                    Line::from("(Enter or Esc to confirm stop)"),
                ];
                let paragraph =
                    Paragraph::new(lines).block(Block::default().title(" Stop session "));
                frame.render_widget(paragraph, area);
            }
        }
    }

    fn draw_main(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let lines: Vec<Line> = if let Some(session) = &self.active_session {
            let elapsed = Utc::now() - session.started_at;
            let h = elapsed.num_hours();
            let m = elapsed.num_minutes() % 60;
            let s = elapsed.num_seconds() % 60;

            let mut lines = vec![
                Line::from("JIARY — TODAY"),
                Line::from(""),
                Line::from(format!(
                    "Project:  {}",
                    session.project.as_deref().unwrap_or("(none)")
                )),
                Line::from(format!(
                    "Task:     {}",
                    session.task.as_deref().unwrap_or("(none)")
                )),
                Line::from(format!("Activity: {}", session.activity)),
            ];

            // Show notes from previous sessions with same project+task
            if let (Some(project), Some(task)) = (&session.project, &session.task) {
                if let Ok(notes) = self.db.recent_notes(project, task) {
                    if !notes.is_empty() {
                        lines.push(Line::from("Notes:"));
                        for note in notes {
                            lines.push(Line::from(format!("  • {note}")));
                        }
                    }
                }
            }

            lines.push(Line::from(""));
            lines.push(Line::from(format!("{h:02}:{m:02}:{s:02}")));
            lines.push(Line::from(""));
            lines.push(Line::from("[n] new session  [s] stop  [q] quit"));

            lines
        } else {
            vec![
                Line::from("JIARY — TODAY"),
                Line::from(""),
                Line::from("No active session."),
                Line::from(""),
                Line::from("[n] new session  [q] quit"),
            ]
        };

        let paragraph = Paragraph::new(lines).block(Block::default().title(" Jiary "));
        frame.render_widget(paragraph, area);
    }
    fn draw_new_session(
        &self,
        frame: &mut Frame,
        area: ratatui::layout::Rect,
        form: &NewSessionForm,
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
                let lines = vec![
                    Line::from(format!("Project: {}", form.project_input)),
                    Line::from(""),
                    Line::from("(Enter to confirm, Esc to cancel)"),
                ];
                let paragraph =
                    Paragraph::new(lines).block(Block::default().title(" New session "));
                frame.render_widget(paragraph, area);
            }
            Step::Task => {
                let lines = vec![
                    Line::from(format!("Activity: {}", ACTIVITIES[form.activity_index])),
                    Line::from(format!(
                        "Project:  {}",
                        if form.project_input.is_empty() {
                            "(none)"
                        } else {
                            &form.project_input
                        }
                    )),
                    Line::from(format!("Task:     {}", form.task_input)),
                    Line::from(""),
                    Line::from("(Enter to start, Esc to cancel)"),
                ];
                let paragraph =
                    Paragraph::new(lines).block(Block::default().title(" New session "));
                frame.render_widget(paragraph, area);
            }
        }
    }
}
