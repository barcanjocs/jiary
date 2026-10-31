//! The start form (activity, project and task, each an `Autocomplete`) and
//! the note form.

use crossterm::event::KeyCode;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, ListItem, ListState, Paragraph, Tabs};

use crate::db::SessionCombo;
use crate::layout::Section;
use crate::mascot::Reaction;
use crate::theme;

use super::modal::{WrappedInput, picker_list, small_modal_layout, start_modal_layout};
use super::{App, Screen};

/// How many recent activity/project/task picks the start form offers.
pub(super) const RECENT_PICKS: usize = 5;

pub(super) struct AddNoteForm {
    pub(super) input: String,
}

pub(super) struct StartSessionForm {
    pub(super) step: Step,
    /// Recent combinations, started with `1`–`5` while the activity input
    /// is empty.
    pub(super) recent: Vec<SessionCombo>,
    pub(super) activity: Autocomplete,
    pub(super) project: Autocomplete,
    pub(super) task: Autocomplete,
}

/// A free-text input with fuzzy suggestions: the start form's project and
/// task steps. What is shown in the input line is always what Enter submits.
pub(super) struct Autocomplete {
    /// The field's value: what was typed, or the highlighted suggestion.
    pub(super) input: String,
    /// What was typed; suggestions are filtered by this, not by `input`.
    pub(super) query: String,
    /// The highlighted suggestion, if any. `None` means the typed text is
    /// the value and nothing in the list is highlighted.
    pub(super) selection: Option<usize>,
    candidates: Vec<String>,
}

impl Autocomplete {
    pub(super) fn new(candidates: Vec<String>) -> Self {
        Self {
            input: String::new(),
            query: String::new(),
            selection: None,
            candidates,
        }
    }

    pub(super) fn suggestions(&self) -> Vec<String> {
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
    pub(super) fn value(&self) -> Option<String> {
        let value = self.input.trim();
        (!value.is_empty()).then(|| value.to_string())
    }
}

#[derive(Clone, Copy)]
pub(super) enum Step {
    Activity,
    Project,
    Task,
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

pub(super) fn fuzzy_filter(query: &str, candidates: &[String]) -> Vec<String> {
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

impl App {
    /// Handles keys while starting a session (activity → project → task).
    pub(super) fn handle_start_session(&mut self, key: KeyCode) {
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

    /// Handles keys while adding a note to the active session.
    pub(super) fn handle_add_note(&mut self, key: KeyCode) {
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

    /// The start form is a centered fixed-size modal: a `Tabs` row for
    /// Activity·Project·Task with the current step selected, then the step's
    /// content: a wrapped input plus its suggestion `List`.
    pub(super) fn draw_start_session(
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

    /// Add-note form: small centered modal with a dimmed project·task·activity
    /// context line (the full note history stays on the main screen) and a
    /// note input (cursor placed by main.rs).
    pub(super) fn draw_add_note(
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
