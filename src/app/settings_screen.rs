//! `/settings`: a small modal with a row per setting. ↑↓ pick a row,
//! Space or Enter change it (the next value), ←→ step it either way (the
//! Pomodoro target by `TARGET_STEP` minutes), Esc closes. Every change is
//! saved at once.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, ListItem, ListState, Paragraph};

use crate::settings::{TARGET_MAX, TARGET_MIN, TARGET_STEP};
use crate::theme;

use super::modal::{picker_list, small_modal_layout};
use super::{App, Screen};

/// A row of the settings screen, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Setting {
    Pomodoro,
    Target,
    SleepAndMood,
}

pub(super) const SETTINGS: &[Setting] =
    &[Setting::Pomodoro, Setting::Target, Setting::SleepAndMood];

/// The settings screen's state: the highlighted row.
pub(super) struct SettingsForm {
    pub(super) selection: usize,
}

impl Setting {
    fn label(self) -> &'static str {
        match self {
            Setting::Pomodoro => "Pomodoro",
            Setting::Target => "Target",
            Setting::SleepAndMood => "Sleep and mood",
        }
    }
}

impl App {
    pub(super) fn open_settings(&mut self) {
        self.screen = Screen::Settings(SettingsForm { selection: 0 });
    }

    /// A setting's value as shown.
    fn setting_value(&self, setting: Setting) -> String {
        match setting {
            Setting::Pomodoro => self.settings.pomodoro.name().to_string(),
            Setting::Target => format!("{}m", self.settings.target_minutes),
            Setting::SleepAndMood => match self.settings.sleep_and_mood {
                true => "on".to_string(),
                false => "off".to_string(),
            },
        }
    }

    /// Steps `setting` forward (or back), then saves.
    fn step_setting(&mut self, setting: Setting, forward: bool) {
        let settings = &mut self.settings;
        match setting {
            Setting::Pomodoro => {
                settings.pomodoro = match forward {
                    true => settings.pomodoro.next(),
                    false => settings.pomodoro.prev(),
                };
            }
            Setting::Target => {
                let minutes = match forward {
                    true => settings.target_minutes + TARGET_STEP,
                    false => settings.target_minutes.saturating_sub(TARGET_STEP),
                };
                settings.target_minutes = minutes.clamp(TARGET_MIN, TARGET_MAX);
            }
            Setting::SleepAndMood => settings.sleep_and_mood = !settings.sleep_and_mood,
        }
        self.save_settings();
    }

    pub(super) fn handle_settings(&mut self, key: KeyCode) {
        let Screen::Settings(form) = &mut self.screen else {
            return;
        };
        let setting = SETTINGS[form.selection.min(SETTINGS.len() - 1)];
        match key {
            KeyCode::Esc | KeyCode::Char('q') => self.screen = Screen::Main,
            KeyCode::Up | KeyCode::Char('k') => form.selection = form.selection.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                form.selection = (form.selection + 1).min(SETTINGS.len() - 1);
            }
            KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                self.step_setting(setting, true);
            }
            KeyCode::Left | KeyCode::Char('h') => self.step_setting(setting, false),
            _ => {}
        }
    }

    /// The settings modal: a dimmed line saying where they are kept, then
    /// a row per setting, `label  value`, the picked one highlighted.
    pub(super) fn draw_settings(&self, frame: &mut Frame, area: Rect, form: &SettingsForm) {
        let (modal, top_area, body_area) = small_modal_layout(area, SETTINGS.len() as u16);
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(Line::from(Span::styled(" Settings ", theme::TITLE))),
            modal,
        );
        frame.render_widget(
            Paragraph::new(Span::styled("Saved as you change them", theme::DIMMED)),
            top_area,
        );
        let label_width = SETTINGS.iter().map(|s| s.label().len()).max().unwrap_or(0) + 2;
        let items = SETTINGS
            .iter()
            .map(|&setting| {
                ListItem::new(Line::from(vec![
                    Span::raw(format!("{:label_width$}", setting.label())),
                    Span::styled(self.setting_value(setting), theme::VALUE),
                ]))
            })
            .collect::<Vec<_>>();
        frame.render_stateful_widget(
            picker_list(items),
            body_area,
            &mut ListState::default().with_selected(Some(form.selection)),
        );
    }
}
