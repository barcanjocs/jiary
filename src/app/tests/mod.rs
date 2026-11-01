//! The UI's tests: key handling against a throwaway db, and rendering into
//! a `TestBackend` buffer.

use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::TimeZone;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::text::Line;

use crate::db::{Day, Db};
use crate::idle;
use crate::mascot::Mood;
use crate::period::Period;
use crate::summary::summarize;
use crate::theme;

use super::cat::{DROWSY_AFTER_MINUTES, caption_lines, fresh_seed};
use super::chrome::chrome_layout;
use super::forms::{AddNoteForm, Autocomplete, StartSessionForm, Step, fuzzy_filter};
use super::graphs::{
    DayCell, focus_cell, focus_grid, focus_legend, focus_row, half_cells, hour_labels, item_width,
    share_row, start_of_hour, timeline_row, year_focus_grid,
};
use super::items::{DAY_TOTAL_WIDTH, day_total};
use super::main_screen::START_STEP;
use super::modal::START_MODAL_HEIGHT;
use super::mood::{MoodForm, MoodStep};
use super::palette::{COMMANDS, PaletteForm, palette_matches};
use super::period_view::PeriodView;
use super::rename::RenameStep;
use super::settings_screen::SettingsForm;
use super::text::{fmt_time, wrap_text};
use super::*;

mod away;
mod cat;
mod chrome;
mod forms;
mod graphs;
mod main_screen;
mod mood;
mod palette;
mod period_view;
mod pomodoro;
mod rename;
mod rewind;
mod sessions;

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
        let mut app = App::new(db, None, None);
        app.set_size((RENDER_WIDTH, RENDER_HEIGHT));
        // Off, so `s` opens the start form at once; the sleep-and-mood
        // tests turn it on (`asking_mood`).
        app.settings.sleep_and_mood = false;
        Self { app, dir }
    }

    /// With sleep and mood on, as a fresh install has them.
    fn asking_mood(mut self) -> Self {
        self.app.settings.sleep_and_mood = true;
        self
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
            recent: Vec::new(),
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

// --- shared helpers -------------------------------------------------

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

/// Index of the first rendered row containing `needle`.
fn row_with(buf: &Buffer, needle: &str) -> u16 {
    (0..buf.area.height)
        .find(|&y| line(buf, y).contains(needle))
        .unwrap_or_else(|| panic!("no row contains {needle:?}"))
}

/// Moves the active session's start `minutes` into the past, in the db
/// and in memory; returns the new start.
fn backdate(t: &mut TestApp, minutes: i64) -> chrono::DateTime<Utc> {
    let session = t.app.active_session.as_mut().unwrap();
    session.started_at -= chrono::TimeDelta::minutes(minutes);
    t.app
        .db
        .set_started_at(session.id, session.started_at)
        .unwrap();
    session.started_at
}

fn open(t: &mut TestApp, command: &str) {
    t.key(KeyCode::Char('/'));
    t.typed(command);
    t.key(KeyCode::Enter);
}

/// The caption rows under the cat (rows 8–9, after a gap row; the cat's
/// columns 54–77),
/// each trimmed.
fn caption_rows(buf: &Buffer) -> Vec<String> {
    (8..10)
        .map(|y| {
            let row: String = line_from(buf, y, 54).chars().take(24).collect();
            row.trim().to_string()
        })
        .collect()
}

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

fn activity(t: &TestApp) -> Option<&str> {
    t.app.active_session.as_ref().map(|s| s.activity.as_str())
}

// --- every screen ---------------------------------------------------

#[test]
fn every_screen_survives_tiny_terminals() {
    let screens = || {
        vec![
            Screen::Main,
            Screen::Help,
            TestApp::start_form_at(Step::Activity),
            Screen::AddNote(AddNoteForm {
                input: "word ".repeat(50),
            }),
            Screen::Palette(PaletteForm {
                input: format!("/{}", "x".repeat(100)),
                selection: 0,
            }),
            Screen::Period(PeriodView {
                period: Period::Year,
                offset: 0,
                first: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                last: chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
                sessions: Vec::new(),
                ratings: vec![Day {
                    date: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                    sleep: Some(3),
                    mood: None,
                }],
                scroll: 99,
            }),
            Screen::Rename(Box::new(RenameForm {
                step: RenameStep::To,
                kind: 2,
                project: Autocomplete::new(Vec::new()),
                from: Autocomplete::new(vec!["docs".into()]),
                to: Autocomplete::new(vec!["word ".repeat(30)]),
                counts: (0, 0),
                notice: None,
            })),
            Screen::Rename(Box::new(RenameForm {
                step: RenameStep::Confirm,
                kind: 0,
                project: Autocomplete::new(Vec::new()),
                from: Autocomplete::new(Vec::new()),
                to: Autocomplete::new(Vec::new()),
                counts: (3, 4),
                notice: Some("word ".repeat(30)),
            })),
            Screen::Settings(SettingsForm { selection: 2 }),
            Screen::Mood(MoodForm {
                date: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                step: MoodStep::Mood,
                sleep: Some(1),
                mood: None,
                then_start: true,
            }),
            Screen::Period(PeriodView {
                period: Period::Month,
                offset: 0,
                first: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                last: chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
                sessions: Vec::new(),
                ratings: vec![Day {
                    date: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                    sleep: Some(3),
                    mood: None,
                }],
                scroll: 99,
            }),
        ]
    };
    for (width, height) in [(0, 0), (1, 1), (5, 3), (12, 6), (30, 10)] {
        for screen in screens() {
            let mut t = with_sessions(TestApp::new().asking_mood(), 5).on_screen(screen);
            // With the rating bar's row too.
            t.app.rating = Some((0, Instant::now()));
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
        // `/rewind` and a day from it, searched, scrolled and paged.
        let mut t = with_sessions(TestApp::new(), 5);
        t.app.set_size((width, height));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        t.app.open_rewind();
        t.typed("p:");
        for code in [KeyCode::Down, KeyCode::End, KeyCode::PageDown] {
            t.key(code);
        }
        terminal.draw(|frame| t.app.draw(frame)).unwrap();
        t.app.cursor_position((width, height));
        t.key(KeyCode::Enter);
        for code in [KeyCode::End, KeyCode::Left, KeyCode::PageDown] {
            t.key(code);
        }
        mouse(&mut t, MouseEventKind::ScrollDown, 0, 0);
        terminal.draw(|frame| t.app.draw(frame)).unwrap();
    }
}
