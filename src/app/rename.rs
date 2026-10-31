//! Renaming and merging labels (`/rename`): pick what to rename (an
//! activity, a project, or a task within its project), the label, and its
//! new name, then confirm. A new name already in use merges the two.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, ListItem, ListState, Paragraph, Tabs, Wrap};

use crate::db::Label;
use crate::session;
use crate::theme;

use super::forms::{Autocomplete, draw_suggestions};
use super::modal::{WrappedInput, picker_list, start_modal_layout};
use super::text::plural;
use super::{App, Screen};

/// What can be renamed, in the order the first step lists it.
const KINDS: [(Label, &str); 3] = [
    (Label::Activity, "Activity"),
    (Label::Project, "Project"),
    (Label::Task, "Task"),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RenameStep {
    /// Activity, project or task.
    What,
    /// A task's project, before the task (blank: no project).
    Project,
    From,
    To,
    /// The question, with the counts, before anything is written.
    Confirm,
}

pub(super) struct RenameForm {
    pub(super) step: RenameStep,
    /// The highlighted entry of `KINDS`.
    pub(super) kind: usize,
    pub(super) project: Autocomplete,
    pub(super) from: Autocomplete,
    pub(super) to: Autocomplete,
    /// Sessions using the old and the new name, counted on the way to
    /// `Confirm`.
    pub(super) counts: (usize, usize),
    /// Why the last Enter was refused; shown until the next key.
    pub(super) notice: Option<String>,
}

impl RenameForm {
    fn label(&self) -> Label {
        KINDS[self.kind].0
    }

    /// The project a task is renamed within; `None` for no project, and
    /// for activities and projects, which aren't scoped.
    fn scope(&self) -> Option<String> {
        match self.label() {
            Label::Task => self.project.value(),
            _ => None,
        }
    }

    /// The tabs row: What, then Project for tasks, From and To.
    fn tabs(&self) -> (Vec<&'static str>, usize) {
        let mut tabs = vec!["What"];
        if self.label() == Label::Task {
            tabs.push("Project");
        }
        tabs.extend(["From", "To"]);
        let at = |name| tabs.iter().position(|t| *t == name).unwrap_or(0);
        let selected = match self.step {
            RenameStep::What => 0,
            RenameStep::Project => at("Project"),
            RenameStep::From => at("From"),
            RenameStep::To | RenameStep::Confirm => at("To"),
        };
        (tabs, selected)
    }

    /// The field the current step types into, if any.
    pub(super) fn field(&self) -> Option<(&Autocomplete, &'static str)> {
        match self.step {
            RenameStep::Project => Some((&self.project, "pick a project (blank: none)")),
            RenameStep::From => Some((&self.from, "pick the name to change")),
            RenameStep::To => Some((&self.to, "type or pick the new name")),
            RenameStep::What | RenameStep::Confirm => None,
        }
    }

    /// The confirmation question.
    pub(super) fn question(&self) -> String {
        let from = self.from.value().unwrap_or_default();
        let to = self.to.value().unwrap_or_default();
        let (sessions, existing) = self.counts;
        let mut question = match existing {
            0 => format!(
                "Rename \"{from}\" to \"{to}\" in {}?",
                plural(sessions, "session")
            ),
            _ => format!(
                "Merge \"{from}\" into \"{to}\" ({}; it has {existing})?",
                plural(sessions, "session")
            ),
        };
        if self.label() == Label::Task {
            match self.project.value() {
                Some(project) => question.push_str(&format!(" Only in {project}.")),
                None => question.push_str(" Only tasks without a project."),
            }
        }
        question
    }
}

/// Detours are recognized by their activity's name, so renaming one, or
/// renaming something to one, would change what counts as a break,
/// disruption or meeting.
fn is_detour_name(label: Label, name: &str) -> bool {
    label == Label::Activity
        && session::DETOURS
            .iter()
            .any(|d| d.eq_ignore_ascii_case(name))
}

impl App {
    pub(super) fn open_rename(&mut self) {
        self.screen = Screen::Rename(Box::new(RenameForm {
            step: RenameStep::What,
            kind: 0,
            project: Autocomplete::new(Vec::new()),
            from: Autocomplete::new(Vec::new()),
            to: Autocomplete::new(Vec::new()),
            counts: (0, 0),
            notice: None,
        }));
    }

    /// The labels of `label`'s kind in use (a task's within `scope`),
    /// detour activities left out.
    fn labels_in_use(&mut self, label: Label, scope: Option<&str>) -> Vec<String> {
        let labels = match label {
            Label::Activity => self.db.recent_activities(),
            Label::Project => self.db.distinct_projects(),
            Label::Task => self.db.tasks_in(scope),
        };
        match labels {
            Ok(labels) => labels
                .into_iter()
                .filter(|name| !is_detour_name(label, name))
                .collect(),
            Err(e) => {
                self.set_error(e);
                Vec::new()
            }
        }
    }

    /// Keys in the rename form: Esc cancels at any step; Enter moves on
    /// once the step's value is acceptable, saying why when it isn't.
    pub(super) fn handle_rename(&mut self, key: KeyCode) {
        let Screen::Rename(form) = &mut self.screen else {
            return;
        };
        if key == KeyCode::Esc {
            self.screen = Screen::Main;
            return;
        }
        form.notice = None;
        let step = form.step;
        match (step, key) {
            (RenameStep::What, KeyCode::Up) => form.kind = form.kind.saturating_sub(1),
            (RenameStep::What, KeyCode::Down) => form.kind = (form.kind + 1).min(KINDS.len() - 1),
            (RenameStep::What, KeyCode::Tab) => form.kind = (form.kind + 1) % KINDS.len(),
            (RenameStep::What, KeyCode::Enter) => self.rename_kind_chosen(),
            (RenameStep::What, _) => {}
            (RenameStep::Project, KeyCode::Enter) => self.rename_project_chosen(),
            (RenameStep::Project, _) => form.project.handle_key(key),
            (RenameStep::From, KeyCode::Enter) => self.rename_from_chosen(),
            (RenameStep::From, _) => form.from.handle_key(key),
            (RenameStep::To, KeyCode::Enter) => self.rename_to_chosen(),
            (RenameStep::To, _) => form.to.handle_key(key),
            (RenameStep::Confirm, KeyCode::Enter) => self.apply_rename(),
            (RenameStep::Confirm, _) => {}
        }
    }

    fn rename_kind_chosen(&mut self) {
        let Screen::Rename(form) = &self.screen else {
            return;
        };
        let label = form.label();
        // A task is picked within its project, so that comes first.
        if label == Label::Task {
            let projects = self.labels_in_use(Label::Project, None);
            if let Screen::Rename(form) = &mut self.screen {
                form.project = Autocomplete::new(projects);
                form.step = RenameStep::Project;
            }
            return;
        }
        let labels = self.labels_in_use(label, None);
        if let Screen::Rename(form) = &mut self.screen {
            if labels.is_empty() {
                form.notice = Some("Nothing to rename yet.".to_string());
                return;
            }
            form.from = Autocomplete::new(labels);
            form.step = RenameStep::From;
        }
    }

    fn rename_project_chosen(&mut self) {
        let Screen::Rename(form) = &mut self.screen else {
            return;
        };
        if let Some(project) = form.project.value()
            && !form.project.offers(&project)
        {
            form.notice = Some(format!("No project is called \"{project}\"."));
            return;
        }
        let scope = form.scope();
        let tasks = self.labels_in_use(Label::Task, scope.as_deref());
        if let Screen::Rename(form) = &mut self.screen {
            if tasks.is_empty() {
                form.notice = Some("No tasks there to rename.".to_string());
                return;
            }
            form.from = Autocomplete::new(tasks);
            form.step = RenameStep::From;
        }
    }

    fn rename_from_chosen(&mut self) {
        let Screen::Rename(form) = &mut self.screen else {
            return;
        };
        let label = form.label();
        let Some(from) = form.from.value() else {
            return;
        };
        if is_detour_name(label, &from) {
            form.notice = Some(format!(
                "{from} can't be renamed: Jiary knows detours by name."
            ));
            return;
        }
        if !form.from.offers(&from) {
            form.notice = Some(format!("Nothing is called \"{from}\"; pick a name in use."));
            return;
        }
        let scope = form.scope();
        let others = self
            .labels_in_use(label, scope.as_deref())
            .into_iter()
            .filter(|name| *name != from)
            .collect();
        if let Screen::Rename(form) = &mut self.screen {
            form.to = Autocomplete::new(others);
            form.step = RenameStep::To;
        }
    }

    fn rename_to_chosen(&mut self) {
        let Screen::Rename(form) = &mut self.screen else {
            return;
        };
        let label = form.label();
        let from = form.from.value().unwrap_or_default();
        let Some(to) = form.to.value() else {
            form.notice = Some("Type the new name.".to_string());
            return;
        };
        if to == from {
            form.notice = Some("That's the name it already has.".to_string());
            return;
        }
        if is_detour_name(label, &to) {
            form.notice = Some(format!(
                "Nothing can be renamed to {to}: Jiary knows detours by name."
            ));
            return;
        }
        let scope = form.scope();
        let counts = self
            .db
            .count_label(label, &from, scope.as_deref())
            .and_then(|sessions| {
                let existing = self.db.count_label(label, &to, scope.as_deref())?;
                Ok((sessions, existing))
            });
        match counts {
            Ok(counts) => {
                self.clear_error();
                if let Screen::Rename(form) = &mut self.screen {
                    form.counts = counts;
                    form.step = RenameStep::Confirm;
                }
            }
            Err(e) => self.set_error(e),
        }
    }

    /// Writes the rename, then reloads everything `draw` reads, since it
    /// can touch any day and the running session.
    fn apply_rename(&mut self) {
        let Screen::Rename(form) = &self.screen else {
            return;
        };
        let label = form.label();
        let from = form.from.value().unwrap_or_default();
        let to = form.to.value().unwrap_or_default();
        let scope = form.scope();
        if let Err(e) = self.db.rename_label(label, &from, &to, scope.as_deref()) {
            self.set_error(e);
            return;
        }
        self.clear_error();
        match self.db.get_active_session() {
            Ok(session) => self.active_session = session,
            Err(e) => self.set_error(e),
        }
        self.refresh_after_session_change();
        self.refresh_suggestions();
        self.screen = Screen::Main;
    }

    /// The rename form: the start form's modal, with tabs for the steps,
    /// then the step's list, input and suggestions, or the question.
    pub(super) fn draw_rename(
        &self,
        frame: &mut Frame,
        area: Rect,
        form: &RenameForm,
        input: Option<&WrappedInput>,
    ) {
        // Steps without an input keep its row (the first step's prompt goes
        // there), so every step has the same shape.
        let rows = input.map_or(1, WrappedInput::height);
        let (modal, tabs_area, input_area, list_area) = start_modal_layout(area, rows);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(Span::styled(" Rename ", theme::TITLE)));
        frame.render_widget(block, modal);

        let (tabs, selected) = form.tabs();
        frame.render_widget(
            Tabs::new(tabs)
                .select(selected)
                .highlight_style(Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
            tabs_area,
        );
        match input {
            Some(input) => frame.render_widget(input.paragraph(), input_area),
            None if form.step == RenameStep::What => frame.render_widget(
                Span::styled("What do you want to rename?", theme::DIMMED),
                input_area,
            ),
            None => {}
        }

        if let Some(notice) = &form.notice {
            frame.render_widget(
                Paragraph::new(Span::styled(notice.clone(), theme::ERROR))
                    .wrap(Wrap { trim: true }),
                list_area,
            );
            return;
        }
        match form.step {
            RenameStep::What => {
                let area = Rect {
                    x: list_area.x + 1,
                    width: list_area.width.saturating_sub(1),
                    ..list_area
                };
                let items = KINDS.iter().map(|(_, name)| ListItem::new(*name));
                frame.render_stateful_widget(
                    picker_list(items.collect()),
                    area,
                    &mut ListState::default().with_selected(Some(form.kind)),
                );
            }
            RenameStep::Project => draw_suggestions(frame, list_area, &form.project),
            RenameStep::From => draw_suggestions(frame, list_area, &form.from),
            RenameStep::To => draw_suggestions(frame, list_area, &form.to),
            RenameStep::Confirm => {
                frame.render_widget(
                    Paragraph::new(form.question()).wrap(Wrap { trim: true }),
                    list_area,
                );
            }
        }
    }
}
