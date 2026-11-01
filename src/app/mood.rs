//! Sleep and mood: two ratings a day, on focus's 1–3 scale, to set the
//! day's work against. Asked once, before the day's first session (Esc
//! skips, and it doesn't ask again that day); `/mood` sets or changes
//! them later, and `/mood-off` and `/mood-on` turn the whole thing off and
//! back on.
//!
//! Deliberately just these two, fixed: no custom fields, no reminders (see
//! the backlog's J in DEVELOPMENT.md).

use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::db::Day;
use crate::theme;

use super::modal::small_modal_layout;
use super::{App, Screen};

/// What 1, 2 and 3 mean for each question.
pub(super) const SLEEP_WORDS: [&str; 3] = ["poor", "ok", "good"];
pub(super) const MOOD_WORDS: [&str; 3] = ["low", "ok", "good"];

/// The question being asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MoodStep {
    Sleep,
    Mood,
}

/// The sleep-and-mood prompt: the day it is for, the answers so far (the
/// stored ones when changing them), and whether the start form follows
/// (asked by the day's first `s`) or it was opened with `/mood`.
pub(super) struct MoodForm {
    pub(super) date: chrono::NaiveDate,
    pub(super) step: MoodStep,
    pub(super) sleep: Option<i32>,
    pub(super) mood: Option<i32>,
    pub(super) then_start: bool,
}

/// The color of a 1–3 answer: the focus rating's.
pub(super) fn answer_color(value: i32) -> Color {
    match value {
        1 => theme::BAD,
        2 => theme::OK,
        _ => theme::GOOD,
    }
}

impl App {
    /// `s`: the start form, after asking about sleep and mood when this is
    /// the day's first start and they are on.
    pub(super) fn start_or_ask(&mut self) {
        let today = chrono::Local::now().date_naive();
        if self.settings.sleep_and_mood {
            match self.db.day(today) {
                Ok(None) => return self.open_mood(today, None, true),
                Ok(Some(_)) => {}
                Err(e) => self.set_error(e),
            }
        }
        self.open_start_form();
    }

    /// `/mood`: sets or changes today's sleep and mood.
    pub(super) fn ask_mood(&mut self) {
        if !self.settings.sleep_and_mood {
            self.notify("Sleep and mood are off; /mood-on turns them on");
            return;
        }
        let today = chrono::Local::now().date_naive();
        match self.db.day(today) {
            Ok(day) => self.open_mood(today, day, false),
            Err(e) => self.set_error(e),
        }
    }

    fn open_mood(&mut self, date: chrono::NaiveDate, stored: Option<Day>, then_start: bool) {
        self.screen = Screen::Mood(MoodForm {
            date,
            step: MoodStep::Sleep,
            sleep: stored.and_then(|day| day.sleep),
            mood: stored.and_then(|day| day.mood),
            then_start,
        });
    }

    /// `/mood-on`, `/mood-off`: saved at once; what was recorded stays.
    pub(super) fn set_sleep_and_mood(&mut self, on: bool) {
        self.settings.sleep_and_mood = on;
        if let Some(path) = &self.settings_path
            && let Err(e) = self.settings.save(path)
        {
            self.set_error(format!("Could not save settings: {e}"));
            return;
        }
        self.notify(if on {
            "Sleep and mood on: asked at the day's first start"
        } else {
            "Sleep and mood off; /mood-on turns them back on"
        });
    }

    /// Shows `text` in the status line for a moment.
    fn notify(&mut self, text: &str) {
        self.notice = Some((text.to_string(), Instant::now()));
    }

    /// Keys in the prompt: 1–3 answer the question and move on (the second
    /// answer saves), Enter keeps the answer as it is, Backspace goes back
    /// to sleep, Esc skips (asked by `s`: it doesn't ask again today) or
    /// cancels (`/mood`).
    pub(super) fn handle_mood(&mut self, key: KeyCode) {
        let Screen::Mood(form) = &mut self.screen else {
            return;
        };
        match (key, form.step) {
            (KeyCode::Esc, _) => {
                let (date, then_start) = (form.date, form.then_start);
                if then_start {
                    self.save_day(Day {
                        date,
                        sleep: None,
                        mood: None,
                    });
                    self.open_start_form();
                } else {
                    self.screen = Screen::Main;
                }
            }
            (KeyCode::Backspace, MoodStep::Mood) => form.step = MoodStep::Sleep,
            (KeyCode::Char(c @ '1'..='3'), step) => {
                let value = Some(c as i32 - '0' as i32);
                match step {
                    MoodStep::Sleep => form.sleep = value,
                    MoodStep::Mood => form.mood = value,
                }
                self.next_mood_step();
            }
            (KeyCode::Enter, _) => self.next_mood_step(),
            _ => {}
        }
    }

    /// From sleep on to mood; after mood, saves and goes on to the start
    /// form or back to the main screen.
    fn next_mood_step(&mut self) {
        let Screen::Mood(form) = &mut self.screen else {
            return;
        };
        if form.step == MoodStep::Sleep {
            form.step = MoodStep::Mood;
            return;
        }
        let day = Day {
            date: form.date,
            sleep: form.sleep,
            mood: form.mood,
        };
        let then_start = form.then_start;
        if !self.save_day(day) {
            self.screen = Screen::Main;
        } else if then_start {
            self.open_start_form();
        } else {
            self.screen = Screen::Main;
        }
    }

    fn save_day(&mut self, day: Day) -> bool {
        match self.db.set_day(day) {
            Ok(()) => {
                self.clear_error();
                true
            }
            Err(e) => {
                self.set_error(e);
                false
            }
        }
    }

    /// The prompt: a small modal with a row per question, the current one
    /// highlighted and each answer in its rating color.
    pub(super) fn draw_mood(&self, frame: &mut Frame, area: Rect, form: &MoodForm) {
        let (modal, top_area, body_area) = small_modal_layout(area, 2);
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(Line::from(Span::styled(" Sleep and mood ", theme::TITLE))),
            modal,
        );
        let top = match form.then_start {
            true => "Before the day's first session".to_string(),
            false => form.date.format("%A %-d %B").to_string(),
        };
        frame.render_widget(Paragraph::new(Span::styled(top, theme::DIMMED)), top_area);
        let row = |label: &str, step: MoodStep, answer: Option<i32>, words: [&str; 3]| {
            let current = form.step == step;
            let mut spans = vec![Span::styled(
                format!("{}{label:7}", if current { "› " } else { "  " }),
                if current {
                    theme::PROMPT
                } else {
                    theme::DIMMED
                },
            )];
            for (i, word) in words.iter().enumerate() {
                let value = i as i32 + 1;
                if i > 0 {
                    spans.push(Span::styled(" · ", theme::DIMMED));
                }
                let style = match answer == Some(value) {
                    true => theme::VALUE.fg(answer_color(value)),
                    false => theme::DIMMED,
                };
                spans.push(Span::styled(
                    format!("{value} "),
                    style.add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(word.to_string(), style));
            }
            Line::from(spans)
        };
        frame.render_widget(
            Paragraph::new(vec![
                row("Sleep", MoodStep::Sleep, form.sleep, SLEEP_WORDS),
                row("Mood", MoodStep::Mood, form.mood, MOOD_WORDS),
            ]),
            body_area,
        );
    }
}
