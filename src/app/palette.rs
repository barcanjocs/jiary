//! The command palette (`/`): the commands it offers, its fuzzy matching,
//! its keys and its modal.

use crossterm::event::KeyCode;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, ListItem, ListState, Paragraph};

use crate::period::Period;
use crate::theme;

use super::forms::matcher;
use super::modal::WrappedInput;
use super::modal::{picker_list, start_modal_layout};
use super::text::plural;
use super::{App, Screen};

/// The command palette's state: what was typed (always starting with the
/// `/` that opened it) and the highlighted match.
pub(super) struct PaletteForm {
    pub(super) input: String,
    pub(super) selection: usize,
}

/// A command the palette offers: its name (typed after `/`), what it does,
/// and what it runs.
pub(super) struct Command {
    pub(super) name: &'static str,
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
    /// The rename form, which has no key either.
    Rename,
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
    screen(name, description, Action::View(period))
}

/// A command that opens a screen with no key of its own.
const fn screen(name: &'static str, description: &'static str, action: Action) -> Command {
    Command {
        name,
        description,
        action,
        key_label: "",
    }
}

/// Everything the palette offers, in the order it lists them.
pub(super) const COMMANDS: &[Command] = &[
    view("weekly", "this week, by day", Period::Week),
    view("monthly", "this month, by day", Period::Month),
    view("yearly", "this year's focus", Period::Year),
    screen("rename", "rename or merge a label", Action::Rename),
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
pub(super) fn palette_matches(query: &str) -> Vec<&'static Command> {
    if query.is_empty() {
        return COMMANDS.iter().collect();
    }
    let mut scored = COMMANDS
        .iter()
        .filter_map(|c| matcher().fuzzy_match(c.name, query).map(|score| (score, c)))
        .collect::<Vec<_>>();
    // Stable: ties keep the list's order.
    scored.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
    scored.into_iter().map(|(_, c)| c).collect()
}

/// The command palette: a modal like the start form's, with the matches
/// under the input as `name  description  key`, the pick highlighted (the
/// list scrolls to keep it in view), and above them how many match.
pub(super) fn draw_palette(
    frame: &mut Frame,
    area: Rect,
    form: &PaletteForm,
    input: Option<&WrappedInput>,
) {
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

impl App {
    /// Handles keys in the command palette: typing filters, ↑↓ pick, Tab
    /// completes the pick's name, Enter runs it, Esc (or Backspace on the
    /// bare `/`) closes. Returns true if the command quits the app.
    pub(super) fn handle_palette(&mut self, key: KeyCode) -> bool {
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
                        Action::Rename => self.open_rename(),
                    }
                }
            }
            _ => {}
        }
        false
    }
}
