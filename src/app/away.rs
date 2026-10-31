use super::modal::small_modal_layout;
use super::text::{fmt_duration, fmt_time};
use super::{App, Screen};
use crate::session::{self, BREAK, DISRUPTION};
use crate::theme;
use chrono::Utc;
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

/// How long nobody has to touch the computer during a work session before
/// coming back asks what that time was.
const AWAY_AFTER: chrono::TimeDelta = chrono::TimeDelta::minutes(15);

impl App {
    /// Takes in the system idle time (`None` when the platform can't tell),
    /// polled every `idle::POLL` by `main`. Once nobody has touched the
    /// computer for `AWAY_AFTER`, the absence is remembered from when it
    /// started; on return (input again, or a key in the app) a work
    /// session still running from before then asks what that time was. A
    /// gap of `AWAY_AFTER` between polls means the computer slept, which
    /// counts as away too, from the last poll before it.
    pub fn observe_idle(&mut self, idle: Option<std::time::Duration>, now: chrono::DateTime<Utc>) {
        if let Some(last) = self.last_observed.replace(now)
            && now - last >= AWAY_AFTER
        {
            self.away_since = Some(self.away_since.map_or(last, |since| since.min(last)));
        }
        let idle = idle.and_then(|idle| chrono::TimeDelta::from_std(idle).ok());
        match idle {
            Some(idle) if idle >= AWAY_AFTER => {
                let since = now - idle;
                self.away_since = Some(self.away_since.map_or(since, |s| s.min(since)));
            }
            // Input again (or no way to tell, after a sleep): back.
            _ if self.away_since.is_some() => {
                self.came_back();
            }
            _ => {}
        }
    }

    /// Ends an absence: asks about it when a work session ran through it
    /// and the main screen is up (not to throw away a half-filled form).
    /// Returns whether it asked.
    pub(super) fn came_back(&mut self) -> bool {
        let Some(since) = self.away_since.take() else {
            return false;
        };
        let asks = matches!(self.screen, Screen::Main)
            && self
                .active_session
                .as_ref()
                .is_some_and(|s| !session::is_detour(&s.activity) && s.started_at < since);
        if asks {
            self.screen = Screen::Away(since);
        }
        asks
    }

    /// The away question: `k` (or Esc) keeps the time as it was, `b` / `d`
    /// turn it into a break / disruption and pick the work back up, `e`
    /// ends the session when the absence began.
    pub(super) fn handle_away(&mut self, key: KeyCode, since: chrono::DateTime<Utc>) {
        match key {
            KeyCode::Char('k') | KeyCode::Esc => {}
            KeyCode::Char('e') => {
                self.close_active_session_at(since);
            }
            KeyCode::Char(c @ ('b' | 'd')) => {
                let detour = if c == 'b' { BREAK } else { DISRUPTION };
                let Some(session) = self.active_session.as_ref() else {
                    return;
                };
                let (activity, project, task) = (
                    session.activity.clone(),
                    session.project.clone(),
                    session.task.clone(),
                );
                if !self.close_active_session_at(since) {
                    return;
                }
                match self.db.insert_finished_session(detour, since, Utc::now()) {
                    Ok(id) => self.undo.get_or_insert_default().created.push(id),
                    Err(e) => {
                        self.set_error(e);
                        return;
                    }
                }
                self.start_session(&activity, project, task);
                self.refresh_after_session_change();
            }
            _ => return,
        }
        self.screen = Screen::Main;
    }

    /// Like `close_active_session`, but the session ends at `at` (in the
    /// past), then the main screen refreshes.
    fn close_active_session_at(&mut self, at: chrono::DateTime<Utc>) -> bool {
        let Some(id) = self.active_session.as_ref().map(|s| s.id) else {
            return false;
        };
        if let Err(e) = self.db.complete_session_at(id, at) {
            self.set_error(e);
            return false;
        }
        // Already ended in the db, so this records the change (undo, the
        // rating window) without ending it again.
        let closed = self.close_active_session();
        self.refresh_after_session_change();
        closed
    }

    /// The away question: a small modal saying since when, with the
    /// answers.
    pub(super) fn draw_away(&self, frame: &mut Frame, area: Rect, since: chrono::DateTime<Utc>) {
        let (modal, top_area, body_area) = small_modal_layout(area, 4);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(Span::styled(" Welcome back ", theme::TITLE)));
        frame.render_widget(block, modal);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(
                    "Away since {} ({})",
                    fmt_time(&since),
                    fmt_duration(&since, &Utc::now())
                ),
                theme::DIMMED,
            ))),
            top_area,
        );
        let activity = self
            .active_session
            .as_ref()
            .map_or("the session", |s| s.activity.as_str());
        let key =
            |k: &str| Span::styled(format!("[{k}] "), Style::new().add_modifier(Modifier::BOLD));
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![key("k"), Span::raw(format!("count it as {activity}"))]),
                Line::from(vec![key("b"), Span::raw("it was a break")]),
                Line::from(vec![key("d"), Span::raw("it was a disruption")]),
                Line::from(vec![
                    key("e"),
                    Span::raw(format!("end it at {}", fmt_time(&since))),
                ]),
            ]),
            body_area,
        );
    }
}
