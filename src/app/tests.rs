use super::graphs::{
    focus_grid, focus_legend, focus_row, half_cells, hour_labels, item_width, share_row,
    start_of_hour, timeline_row, year_focus_grid,
};
use super::text::{fmt_time, wrap_text};
use super::*;
use crate::db::Db;
use crate::idle;
use chrono::TimeZone;
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
        let mut app = App::new(db, None);
        app.set_size((RENDER_WIDTH, RENDER_HEIGHT));
        Self { app, dir }
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

// --- autocomplete -------------------------------------------------

fn ab() -> Vec<String> {
    vec!["alpha".into(), "beta".into()]
}

#[test]
fn tab_accepts_best_match_first() {
    // Regression (61d9181): the first Tab must accept the top suggestion,
    // not skip past it.
    let mut t = TestApp::new().on_project_step(ab(), "b");
    t.key(KeyCode::Tab);
    assert_eq!(t.form().project.input, "beta");
    assert_eq!(t.form().project.selection, Some(0));
}

#[test]
fn tab_cycles_and_wraps_around() {
    let mut t = TestApp::new().on_project_step(ab(), "");
    let mut seen = Vec::new();
    for _ in 0..3 {
        t.key(KeyCode::Tab);
        seen.push(t.form().project.input.clone());
    }
    assert_eq!(seen, ["alpha", "beta", "alpha"]);
}

#[test]
fn arrows_select_and_up_past_top_restores_typed_text() {
    let mut t = TestApp::new().on_project_step(ab(), "");
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    t.key(KeyCode::Down); // stops at the end
    assert_eq!(t.form().project.input, "beta");
    t.key(KeyCode::Up);
    assert_eq!(t.form().project.input, "alpha");
    t.key(KeyCode::Up);
    assert_eq!(t.form().project.input, "");
    assert_eq!(t.form().project.selection, None);
}

#[test]
fn enter_submits_the_highlighted_suggestion() {
    let mut t = TestApp::new().on_project_step(ab(), "");
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    t.key(KeyCode::Enter); // project → task step
    t.key(KeyCode::Enter); // start the session
    let active = t.app.active_session.as_ref().unwrap();
    assert_eq!(active.project.as_deref(), Some("beta"));
}

#[test]
fn typing_after_accepting_continues_from_the_suggestion() {
    let mut t = TestApp::new().on_project_step(ab(), "");
    t.key(KeyCode::Tab);
    t.typed("2");
    let f = &t.form().project;
    assert_eq!(f.input, "alpha2");
    assert_eq!(f.query, "alpha2"); // suggestions filter on what is shown
    assert_eq!(f.selection, None);
}

#[test]
fn backspace_clears_selection() {
    let mut t = TestApp::new().on_project_step(ab(), "al");
    t.key(KeyCode::Tab);
    t.key(KeyCode::Backspace);
    let f = &t.form().project;
    assert_eq!(f.input, "alph");
    assert_eq!(f.query, "alph");
    assert_eq!(f.selection, None);
}

#[test]
fn tab_is_noop_when_no_suggestions() {
    let mut t = TestApp::new().on_project_step(ab(), "zzz");
    t.key(KeyCode::Tab);
    assert_eq!(t.form().project.input, "zzz");
    assert_eq!(t.form().project.selection, None);
}

#[test]
fn blank_fields_are_stored_as_none() {
    let mut t = TestApp::new().on_project_step(ab(), "   ");
    t.key(KeyCode::Enter);
    t.key(KeyCode::Enter);
    let active = t.app.active_session.as_ref().unwrap();
    assert_eq!(active.project, None);
    assert_eq!(active.task, None);
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
        "[s] start  [e] end  [n] note  [i] interrupt  [d/b/m] away  [r] resume  [?] help"
    );
}

#[test]
fn status_line_drops_hints_that_dont_fit_but_keeps_help() {
    let t = TestApp::new();
    let text = |width| {
        t.app
            .status_line(width)
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect::<String>()
    };
    assert_eq!(text(40), "[s] start  [e] end  [n] note  [?] help");
    // Narrower than the help hint itself (a pty can report 0 columns).
    assert_eq!(text(5), "[?] help");
    assert_eq!(text(0), "[?] help");
    assert!(text(120).ends_with("[?] help  [Tab] sections  [q] quit")); // everything
}

#[test]
fn status_line_shows_start_form_hints() {
    let t = TestApp::new().on_project_step(vec![], "");
    let buf = render(&t.app);
    assert_eq!(
        line(&buf, RENDER_HEIGHT - 1),
        "[Tab] accept  [Enter] next  [Esc] cancel"
    );
}

#[test]
fn the_rating_prompt_is_a_draining_bar_and_the_cat_asks_too() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    t.key(KeyCode::Char('e'));
    let (id, _) = t.app.rating.unwrap();
    let buf = render(&t.app);
    // Its own row, between the active section and today's.
    let y = row_with(&buf, "How was your focus?");
    assert!(line(&buf, y - 1).starts_with("╰"));
    assert!(line(&buf, y + 1).starts_with("╭ ▾ TODAY"));
    let bar = line(&buf, y);
    assert!(
        bar.starts_with(" How was your focus?  [1] bad  [2] ok  [3] good   [u] undo"),
        "{bar}"
    );
    assert!(bar.ends_with("5s"), "{bar}");
    // Full width in the accent color...
    assert_eq!(buf[(0, y)].bg, Color::Cyan);
    assert_eq!(buf[(RENDER_WIDTH - 1, y)].bg, Color::Cyan);
    assert!(caption_rows(&buf).join(" ").contains("how was your focus?"));
    // ...with the usual hints still at the bottom.
    assert!(line(&buf, RENDER_HEIGHT - 1).starts_with("[s] start"));

    // It drains from the right as time runs out.
    t.app.rating = Some((id, Instant::now() - RATING_WINDOW / 2));
    let buf = render(&t.app);
    assert_eq!(buf[(0, y)].bg, Color::Cyan);
    assert_ne!(buf[(RENDER_WIDTH - 1, y)].bg, Color::Cyan);
    assert!(line(&buf, y).ends_with("3s"));

    // Once it closes, the row goes back to today's sessions.
    t.app.rating = Some((id, Instant::now() - RATING_WINDOW));
    let buf = render(&t.app);
    assert!(line(&buf, y).starts_with("╭ ▾ TODAY"));
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

// --- main screen ------------------------------------------------------

#[test]
fn main_screen_no_active_shows_a_dimmed_hint_in_the_active_section() {
    let t = TestApp::new();
    let buf = render(&t.app);
    assert!(line(&buf, 1).starts_with("╭ ACTIVE "));
    assert!(line(&buf, 2).starts_with("│No active session. Press s to start one.  "));
    assert_eq!(buf[(1, 2)].fg, Color::Gray);
    // As tall as the idling cat and its caption rows beside the hint:
    // border, 8 rows (cat, gap, caption), border.
    assert!(line(&buf, 10).starts_with("╰"));
}

#[test]
fn main_screen_active_panel_has_accent_border_and_title() {
    let mut t = TestApp::new();
    t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 0));
    let buf = render(&t.app);
    assert_eq!(buf[(0, 1)].symbol(), "╭");
    assert_eq!(buf[(0, 1)].fg, Color::Cyan);
    // The title sits on the top border, left-aligned inside it.
    assert!(line(&buf, 1).starts_with("╭ ACTIVE "));
    let cell = &buf[(2, 1)];
    assert_eq!(cell.fg, Color::Cyan);
    assert!(cell.modifier.contains(Modifier::BOLD));
}

#[test]
fn main_screen_active_panel_shows_timer_and_kv_rows() {
    let mut t = TestApp::new();
    let elapsed =
        chrono::Duration::hours(5) + chrono::Duration::minutes(23) + chrono::Duration::seconds(45);
    t.app.active_session = Some(running_session(elapsed, 0));
    let buf = render(&t.app);
    // The panel's inner area starts right at the border (no padding).
    assert!(line(&buf, 2).starts_with("│05:23:45"));
    assert!(buf[(1, 2)].modifier.contains(Modifier::BOLD));
    assert!(line(&buf, 3).starts_with("│Project  alpha"));
    assert_eq!(buf[(1, 3)].fg, Color::Gray); // dimmed label
    // The project in its color (the first project's: orange), bold.
    assert_eq!(buf[(10, 3)].fg, Color::Indexed(208));
    assert!(buf[(10, 3)].modifier.contains(Modifier::BOLD));
    assert!(line(&buf, 4).starts_with("│Task  beta"));
    assert!(line(&buf, 5).starts_with("│Activity  Programming"));
    assert!(line(&buf, 6).starts_with("│Interruptions  0"));
}

#[test]
fn main_screen_active_panel_colors_interruptions_when_positive() {
    let mut t = TestApp::new();
    t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 2));
    let buf = render(&t.app);
    assert!(line(&buf, 6).starts_with("│Interruptions  2"));
    let col = col_of(&buf, 6, "2");
    assert_eq!(buf[(col, 6)].fg, Color::Red);
}

#[test]
fn previous_notes_render_one_bullet_per_line() {
    let mut t = TestApp::new();
    let start = |t: &mut TestApp| {
        t.app
            .start_session("Reading", Some("alpha".into()), Some("beta".into()));
        t.app.refresh_after_session_change();
    };
    start(&mut t);
    t.app.append_active_note("first");
    t.app.append_active_note("second");
    t.app.close_active_session();
    start(&mut t);

    let buf = render(&t.app);
    let rows: Vec<String> = (0..RENDER_HEIGHT).map(|y| line(&buf, y)).collect();
    let at = rows
        .iter()
        .position(|r| r.starts_with("│Previous notes:"))
        .expect("previous notes shown");
    assert!(rows[at + 1].starts_with("│  • first"));
    assert!(rows[at + 2].starts_with("│  • second"));
}

#[test]
fn timeline_item_shows_range_duration_and_parts() {
    let mut t = TestApp::new();
    let start = chrono::Local
        .with_ymd_and_hms(2026, 9, 24, 10, 0, 0)
        .unwrap();
    let end = chrono::Local
        .with_ymd_and_hms(2026, 9, 24, 11, 30, 0)
        .unwrap();
    t.app
        .today_sessions
        .push(finished_session(start, end, None));
    let buf = render(&t.app);
    // Below the 10-row active section (the cat, a gap and its caption); inner text starts at column 1.
    assert!(line(&buf, 11).starts_with("╭ ▾ TODAY · 1 session "));
    assert!(line_from(&buf, 12, 1).starts_with("10:00 – 11:30 (1h 30m)  F-  I:0 "));
    assert!(buf[(1, 12)].modifier.contains(Modifier::BOLD)); // time range
    let dur_col = col_of(&buf, 12, "1h 30m");
    assert_eq!(buf[(dur_col, 12)].fg, Color::Cyan); // duration
    assert!(buf[(dur_col, 12)].modifier.contains(Modifier::BOLD));
    assert!(line_from(&buf, 13, 1).starts_with("alpha · beta · Programming "));
    // Project bold, separators dimmed, the activity in its color.
    assert!(buf[(1, 13)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(col_of(&buf, 13, "·"), 13)].fg, Color::Gray);
    assert_eq!(
        buf[(col_of(&buf, 13, "Programming"), 13)].fg,
        Color::LightBlue
    );
}

#[test]
fn breaks_and_disruptions_get_no_focus_badge() {
    let mut t = TestApp::new();
    let start = chrono::Local
        .with_ymd_and_hms(2026, 9, 24, 10, 0, 0)
        .unwrap();
    let end = chrono::Local
        .with_ymd_and_hms(2026, 9, 24, 10, 15, 0)
        .unwrap();
    let mut pause = finished_session(start, end, None);
    pause.activity = BREAK.into();
    t.app.today_sessions.push(pause);
    let buf = render(&t.app);
    assert!(line_from(&buf, 12, 1).starts_with("10:00 – 10:15 (15m) I:0"));
    assert_eq!(buf[(col_of(&buf, 13, "Break"), 13)].fg, Color::LightGreen);
}

#[test]
fn timeline_focus_badges_use_semantic_colors() {
    let mut t = TestApp::new();
    for focus in [Some(1), Some(2), Some(3), None] {
        let start = chrono::Local
            .with_ymd_and_hms(2026, 9, 24, 10, 0, 0)
            .unwrap();
        let end = chrono::Local
            .with_ymd_and_hms(2026, 9, 24, 10, 30, 0)
            .unwrap();
        t.app
            .today_sessions
            .push(finished_session(start, end, focus));
    }
    // With the summary collapsed and the active section shrunk to its
    // 3-row minimum, all four items fit.
    t.app.layout.summary_collapsed = true;
    t.app.layout.active_height = Some(3);
    let buf = render(&t.app);
    // Items stack three rows apart (range, parts, spacing), from row 5.
    // Rated: black on the outcome color, padded a cell each side.
    let expectations = [
        ("F1", Color::Red),
        ("F2", Color::Yellow),
        ("F3", Color::Green),
    ];
    for (i, (badge, color)) in expectations.iter().enumerate() {
        let y = 5 + 3 * i as u16;
        let col = col_of(&buf, y, badge);
        for x in col - 1..col + 3 {
            assert_eq!(buf[(x, y)].bg, *color, "{badge}");
        }
        assert_eq!(buf[(col, y)].fg, Color::Black);
    }
    // Unrated: just dimmed.
    let col = col_of(&buf, 14, "F-");
    assert_eq!(buf[(col, 14)].fg, Color::Gray);
    assert_eq!(buf[(col, 14)].bg, Color::Reset);
}

// --- forms ------------------------------------------------------------

/// The start modal is 40×12 centered in the content area: x=20, y=6;
/// tabs row at y=7, body from y=8. End/note modals are 40×6: x=20, y=9;
/// tabs/context row at y=10, input row at y=11. All inner text starts at
/// the padded column 22.

#[test]
fn start_modal_shows_title_and_selected_tab() {
    let t = TestApp::new().on_activity_step();
    let buf = render(&t.app);
    // The modal border is default color; only the title carries the accent.
    assert_eq!(buf[(20, 6)].symbol(), "╭");
    assert_eq!(buf[(20, 6)].fg, Color::Reset);
    assert!(line_from(&buf, 6, 20).starts_with("╭ Start session "));
    let title = &buf[(22, 6)];
    assert_eq!(title.fg, Color::Cyan);
    assert!(title.modifier.contains(Modifier::BOLD));
    // Tabs row: the selected step is highlighted, the others are plain.
    assert!(line_from(&buf, 7, 22).starts_with(" Activity │ Project │ Task"));
    assert_eq!(buf[(23, 7)].fg, Color::Cyan); // "Activity" selected
    assert!(buf[(23, 7)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(34, 7)].fg, Color::Reset); // "Project"
    assert_eq!(buf[(44, 7)].fg, Color::Reset); // "Task"
}

#[test]
fn start_modal_activity_step_shows_input_and_suggestions() {
    let mut t = TestApp::new().on_activity_step();
    for _ in 0..3 {
        t.key(KeyCode::Down);
    }
    let buf = render(&t.app);
    // Input row at y=8 shows the selection after the prompt;
    // suggestions from y=9, their text aligned with the typed text
    // (column 24) and the bold accent `>` under the prompt's space.
    assert!(line_from(&buf, 8, 22).starts_with("› Writing"));
    assert!(line_from(&buf, 9, 22).starts_with("  Programming"));
    assert_eq!(buf[(24, 9)].fg, Color::Reset); // unselected item text
    assert!(line_from(&buf, 11, 22).starts_with(" >Writing"));
    assert_eq!(buf[(23, 11)].fg, Color::Cyan);
    assert!(buf[(23, 11)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(24, 11)].fg, Color::Cyan);
}

// --- custom activities ------------------------------------------------

fn activity_suggestions(t: &TestApp) -> Vec<String> {
    match &t.app.screen {
        Screen::StartSession(form) => form.activity.suggestions(),
        _ => panic!("expected start-session screen"),
    }
}

#[test]
fn fresh_database_suggests_the_default_activities() {
    let t = TestApp::new().on_activity_step();
    assert_eq!(activity_suggestions(&t), DEFAULT_ACTIVITIES);
}

#[test]
fn typed_activity_is_stored_and_replaces_the_defaults() {
    let mut t = TestApp::new().on_activity_step();
    t.typed("Gardening");
    t.key(KeyCode::Enter); // → project
    t.key(KeyCode::Enter); // → task
    t.key(KeyCode::Enter); // start
    assert_eq!(activity(&t), Some("Gardening"));

    // With history, only activities actually used are suggested.
    t.key(KeyCode::Char('s'));
    assert_eq!(activity_suggestions(&t), ["Gardening"]);
}

#[test]
fn activities_are_suggested_most_recent_first() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    t.app.close_active_session();
    t.app.start_session("Writing", None, None);
    t.app.refresh_suggestions();
    // One Tab repeats the most recent activity.
    let mut t = t.on_activity_step();
    t.key(KeyCode::Tab);
    assert_eq!(t.form().activity.value().as_deref(), Some("Writing"));
    assert_eq!(activity_suggestions(&t), ["Writing", "Reading"]);
}

#[test]
fn enter_on_an_empty_activity_stays_on_the_step() {
    let mut t = TestApp::new().on_activity_step();
    t.key(KeyCode::Enter);
    assert!(matches!(t.form().step, Step::Activity));
    t.typed("   ");
    t.key(KeyCode::Enter);
    assert!(matches!(t.form().step, Step::Activity));
}

#[test]
fn start_modal_project_step_shows_input_and_suggestions() {
    let mut t = TestApp::new().on_project_step(vec!["alpha".into(), "alpine".into()], "a");
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    let buf = render(&t.app);
    // The Project tab is now selected.
    assert_eq!(buf[(34, 7)].fg, Color::Cyan);
    assert!(buf[(34, 7)].modifier.contains(Modifier::BOLD));
    // Input row: bold accent prompt + the highlighted suggestion.
    assert!(line_from(&buf, 8, 22).starts_with("› alpine"));
    assert_eq!(buf[(22, 8)].fg, Color::Cyan);
    assert!(buf[(22, 8)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(24, 8)].fg, Color::Reset);
    // Suggestions below, aligned with the value; the selection is
    // highlighted.
    assert!(line_from(&buf, 9, 24).starts_with("alpha"));
    assert!(line_from(&buf, 10, 23).starts_with(">alpine"));
    assert_eq!(buf[(23, 10)].fg, Color::Cyan);
}

#[test]
fn empty_input_shows_a_dimmed_placeholder_not_a_label() {
    let t = TestApp::new().on_activity_step();
    let buf = render(&t.app);
    // The tabs row names the field; the input row doesn't repeat it.
    assert_eq!(
        line_from(&buf, 8, 22),
        "› type or pick an activity           │"
    );
    assert_eq!(buf[(24, 8)].fg, Color::Gray);
    assert!(buf[(24, 8)].modifier.contains(Modifier::ITALIC));
    // The cursor sits at the start of the placeholder.
    assert_eq!(t.app.cursor_position((80, 24)), Some((24, 8)));
}

#[test]
fn cursor_position_tracks_input_lines() {
    // 80×24 terminal: the start modal's input row is y=8, the note
    // modal's is y=11; values start at column 24, after "› ".
    let t = TestApp::new().on_project_step(vec![], "");
    assert_eq!(t.app.cursor_position((80, 24)), Some((24, 8)));

    let t = TestApp::new().on_project_step(vec![], "ab");
    assert_eq!(t.app.cursor_position((80, 24)), Some((26, 8)));

    let t = TestApp::new().on_screen(TestApp::start_form_at(Step::Task));
    assert_eq!(t.app.cursor_position((80, 24)), Some((24, 8)));

    let t = TestApp::new().on_screen(TestApp::start_form_at(Step::Activity));
    assert_eq!(t.app.cursor_position((80, 24)), Some((24, 8)));

    let t = TestApp::new().on_screen(Screen::AddNote(AddNoteForm { input: "hi".into() }));
    assert_eq!(t.app.cursor_position((80, 24)), Some((26, 11))); // "› hi"

    let t = TestApp::new();
    assert_eq!(t.app.cursor_position((80, 24)), None); // main screen
}

// --- mascot -------------------------------------------------------------

#[test]
fn the_cat_sits_on_the_right_of_the_active_section() {
    let mut t = TestApp::new();
    t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 0));
    let buf = render(&t.app);
    // Inner area is columns 1–78; the 14-wide cat ends one column before
    // the border, so its left edge is column 64. The left ear (same in
    // every frame): a lower half, then fur over the pink inner ear.
    assert_eq!(buf[(65, 2)].symbol(), "▄");
    assert_eq!(buf[(65, 2)].fg, Color::Cyan);
    assert_eq!(buf[(66, 2)].symbol(), "▀");
    assert_eq!(buf[(66, 2)].bg, Color::LightRed);
    // The session text keeps the left side.
    assert!(line(&buf, 3).starts_with("│Project  alpha"));
}

#[test]
fn the_cat_hides_when_the_section_is_too_small() {
    // Shrunk below the cat's height.
    let mut t = TestApp::new();
    t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 0));
    t.app.layout.active_height = Some(5);
    let buf = render(&t.app);
    // (The timer on the left is cyan too; the cat would be on the right.)
    assert!((54..79).all(|x| (2..5).all(|y| buf[(x, y)].fg != Color::Cyan)));

    // Too narrow to keep 24 columns of text beside it.
    let mut t = TestApp::new();
    t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 0));
    t.app.set_size((40, 24));
    let backend = TestBackend::new(40, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| t.app.draw(frame)).unwrap();
    let buf = terminal.backend().buffer();
    // Where it would be: columns 14–37, right of the timer.
    assert!((14..39).all(|x| (2..7).all(|y| buf[(x, y)].fg != Color::Cyan)));
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

#[test]
fn fresh_seeds_pick_every_sentence_of_a_short_list() {
    // The clock-based seed this replaces landed on one sentence of 4
    // for a whole second.
    let picks = (0..64)
        .map(|_| fresh_seed() % 4)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(picks.len(), 4);
}

#[test]
fn an_interruption_says_oups_then_picks_another_sentence() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    t.app.caption_seed = 0;
    let before = t.app.caption().unwrap();
    t.key(KeyCode::Char('i'));
    assert_eq!(caption_rows(&render(&t.app)), ["oups!", ""]);

    // Once the flinch is over, a different sentence from the list.
    t.app.reaction = None;
    let after = t.app.caption().unwrap();
    assert_ne!(after, before);
    assert_eq!(caption_rows(&render(&t.app)).join(" ").trim(), after);
}

#[test]
fn a_caption_shows_under_the_cat_in_faded_italic() {
    let mut t = TestApp::new();
    t.app.active_session = Some(running_session(chrono::Duration::minutes(1), 0));
    t.app.caption_seed = 0; // "working on world domination..."
    let buf = render(&t.app);
    let rows = caption_rows(&buf);
    // Wrapped to the cat's width.
    assert_eq!(rows.join(" "), "working on world domination...");
    assert!(rows.iter().all(|r| !r.is_empty()), "{rows:?}");
    let x = 54
        + line_from(&buf, 8, 54)
            .chars()
            .position(|c| c != ' ')
            .unwrap() as u16;
    assert_eq!(buf[(x, 8)].fg, Color::Gray);
    assert!(buf[(x, 8)].modifier.contains(Modifier::ITALIC));
}

#[test]
fn the_caption_centres_under_the_cat_not_the_canvas() {
    // Working: the laptop is on the left of the canvas and the cat's
    // body on columns 10–20, so each row centres on column 54 + 15.
    let mut t = TestApp::new();
    t.app.active_session = Some(running_session(chrono::Duration::minutes(1), 0));
    t.app.caption_seed = 0; // "working on world domination..."
    let buf = render(&t.app);
    for y in 8..10 {
        let row: String = line_from(&buf, y, 54).chars().take(24).collect();
        let first = row.chars().position(|c| c != ' ').unwrap();
        let last = row.chars().count() - 1 - row.chars().rev().position(|c| c != ' ').unwrap();
        assert!((29..=31).contains(&(first + last)), "{row:?}");
    }
}

#[test]
fn caption_rows_too_wide_to_centre_stay_on_the_canvas() {
    let lines = caption_lines("don't be evil, don't be evil", Mood::Idle, 24);
    let widths: Vec<usize> = lines.iter().map(Line::width).collect();
    assert!(widths.iter().all(|&w| w <= 24), "{widths:?}");
    assert_eq!(widths[0], 24, "pushed back to the canvas's left edge");
}

#[test]
fn idling_shows_its_caption_on_one_row() {
    let mut t = TestApp::new();
    t.app.caption_seed = 0;
    assert_eq!(caption_rows(&render(&t.app)), ["what's next, boss?", ""]);
}

#[test]
fn the_cats_mood_follows_the_session() {
    let mut t = TestApp::new();
    assert_eq!(t.app.mood(), Mood::Idle);
    for (activity, mood) in [
        ("Reading", Mood::Working),
        (BREAK, Mood::Sleeping),
        (MEETING, Mood::Meeting),
        (DISRUPTION, Mood::Gaming),
    ] {
        let mut session = running_session(chrono::Duration::minutes(5), 0);
        session.activity = activity.into();
        t.app.active_session = Some(session);
        assert_eq!(t.app.mood(), mood, "{activity}");
    }
    // A long work session gets drowsy; a long meeting doesn't.
    let long = chrono::Duration::minutes(DROWSY_AFTER_MINUTES);
    t.app.active_session = Some(running_session(long, 0));
    assert_eq!(t.app.mood(), Mood::Drowsy);
    let mut meeting = running_session(long, 0);
    meeting.activity = MEETING.into();
    t.app.active_session = Some(meeting);
    assert_eq!(t.app.mood(), Mood::Meeting);
}

#[test]
fn actions_trigger_short_reactions() {
    let reaction = |t: &TestApp| t.app.current_reaction().map(|(r, _)| r);
    let mut t = TestApp::new();
    t.app.start_session("Writing", None, None);
    assert_eq!(reaction(&t), None);

    t.key(KeyCode::Char('i'));
    assert_eq!(reaction(&t), Some(Reaction::Flinch));

    t.key(KeyCode::Char('n'));
    t.typed("idea");
    t.key(KeyCode::Enter);
    assert_eq!(reaction(&t), Some(Reaction::Noted));

    for code in [KeyCode::Char('e'), KeyCode::Enter, KeyCode::Char('3')] {
        t.key(code);
    }
    assert_eq!(reaction(&t), Some(Reaction::Rated(3)));
}

#[test]
fn skipping_the_rating_or_an_empty_note_gets_no_reaction() {
    let mut t = TestApp::new();
    t.app.start_session("Writing", None, None);
    for code in [KeyCode::Char('n'), KeyCode::Enter] {
        t.key(code);
    }
    assert!(t.app.current_reaction().is_none());
    for code in [KeyCode::Char('e'), KeyCode::Enter, KeyCode::Enter] {
        t.key(code);
    }
    assert!(t.app.active_session.is_none());
    assert!(t.app.current_reaction().is_none());
}

#[test]
fn reactions_end_after_the_reaction_time() {
    let mut t = TestApp::new();
    let long_ago = Instant::now()
        .checked_sub(std::time::Duration::from_millis(
            Reaction::Flinch.duration(),
        ))
        .unwrap();
    t.app.reaction = Some((Reaction::Flinch, long_ago));
    assert!(t.app.current_reaction().is_none());
}

// --- scrolling the active session's notes ------------------------------

/// A running session with notes "note 1" … "note n", in an active
/// section fixed at 13 rows: of its 11 inner rows, the first 5 are the
/// pinned session info, leaving rows 7–12 of the text column (beside
/// the cat) for the notes.
fn with_notes(n: usize) -> TestApp {
    let mut t = TestApp::new();
    let mut session = running_session(chrono::Duration::minutes(1), 0);
    session.notes = Some((1..=n).map(|i| format!("note {i}\n")).collect());
    t.app.active_session = Some(session);
    t.app.layout.active_height = Some(13);
    t
}

#[test]
fn notes_scroll_under_the_pinned_session_info_and_cat() {
    let mut t = with_notes(10);
    // "Session notes:" + 10 bullets = 11 rows in a 6-row viewport.
    assert_eq!(t.app.viewport(Section::Active), 6);
    assert_eq!(t.app.max_scroll(Section::Active), 5);

    t.key(KeyCode::Down);
    t.key(KeyCode::Char('j'));
    assert_eq!(t.app.notes_scroll, 2);
    let buf = render(&t.app);
    // Pinned: the timer and the cat's ear stay at the top.
    assert!(line(&buf, 2).starts_with("│00:01:00"));
    assert_eq!(buf[(65, 2)].symbol(), "▄");
    // The notes right below the info have moved: no gap above them.
    assert!(line(&buf, 6).starts_with("│Interruptions  0"));
    assert!(line_from(&buf, 7, 1).starts_with("  • note 2 "));
    // With a scrollbar on the border beside them.
    assert_ne!(buf[(79, 9)].symbol(), " ");

    t.key(KeyCode::End);
    assert!(line_from(&render(&t.app), 12, 1).starts_with("  • note 10 "));
    t.key(KeyCode::Home);
    assert!(line_from(&render(&t.app), 7, 1).starts_with("Session notes:"));
}

#[test]
fn long_notes_wrap_with_an_indent_beside_the_cat() {
    let mut t = with_notes(0);
    let long = "a long note about the layout that keeps going well past the cat";
    t.app.active_session.as_mut().unwrap().notes = Some(format!("{long}\n"));
    let buf = render(&t.app);
    // The text column is 52 wide (78 inner minus the cat's 26): the note
    // wraps at a word boundary and continues under its text.
    assert!(
        line_from(&buf, 8, 1).starts_with("  • a long note about the layout that keeps going ")
    );
    assert!(line_from(&buf, 9, 1).starts_with("    well past the cat "));
    // Each row stays inside the text column (the cat's caption is to its
    // right on these rows).
    let text_column = |y| line_from(&buf, y, 1).chars().take(52).collect::<String>();
    assert_eq!(
        text_column(8).trim_end(),
        "  • a long note about the layout that keeps going"
    );
    assert_eq!(text_column(9).trim_end(), "    well past the cat");
    // Scrolling counts the wrapped rows: "Session notes:" + 2.
    assert_eq!(t.app.scroll_lines(Section::Active), 3);
}

#[test]
fn adding_a_note_scrolls_it_into_view() {
    let mut t = TestApp::new();
    t.app.start_session("Writing", None, None);
    t.app.layout.active_height = Some(13);
    for i in 1..=6 {
        t.key(KeyCode::Char('n'));
        t.typed(&format!("note {i}"));
        t.key(KeyCode::Enter);
    }
    assert_eq!(t.app.notes_scroll, t.app.max_scroll(Section::Active));
    assert!(line_from(&render(&t.app), 12, 1).starts_with("  • note 6 "));
}

#[test]
fn the_wheel_scrolls_the_section_under_the_pointer() {
    let mut t = with_notes(10);
    mouse(&mut t, MouseEventKind::ScrollDown, 10, 5); // over the active section
    assert_eq!(t.app.notes_scroll, 3);
    assert_eq!(t.app.today_scroll, 0);
}

#[test]
fn with_the_summary_focused_the_keys_scroll_todays_sessions() {
    let mut t = with_sessions(with_notes(10), 10);
    t.key(KeyCode::Tab);
    t.key(KeyCode::Tab); // summary: it doesn't scroll
    t.key(KeyCode::Down);
    assert_eq!(t.app.today_scroll, 1);
    assert_eq!(t.app.notes_scroll, 0);
}

// --- main screen sections ----------------------------------------------

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

#[test]
fn tab_moves_focus_and_the_accent_border() {
    let mut t = TestApp::new();
    let border_colors = |t: &TestApp| {
        let buf = render(&t.app);
        let today = row_with(&buf, "TODAY");
        let summary = row_with(&buf, "SUMMARY");
        [buf[(0, 1)].fg, buf[(0, today)].fg, buf[(0, summary)].fg]
    };
    assert_eq!(border_colors(&t), [Color::Cyan, Color::Gray, Color::Gray]);
    t.key(KeyCode::Tab);
    assert_eq!(border_colors(&t), [Color::Gray, Color::Cyan, Color::Gray]);
    t.key(KeyCode::Tab);
    assert_eq!(border_colors(&t), [Color::Gray, Color::Gray, Color::Cyan]);
    t.key(KeyCode::BackTab);
    assert_eq!(t.app.focus, Section::Today);
}

#[test]
fn space_collapses_the_focused_section_to_one_line() {
    let mut t = with_sessions(TestApp::new(), 2);
    t.key(KeyCode::Char(' ')); // the active section can't collapse
    assert_eq!(t.app.layout, LayoutPrefs::default());

    t.key(KeyCode::Tab);
    t.key(KeyCode::Char(' '));
    let buf = render(&t.app);
    assert_eq!(line(&buf, 11), "▸ TODAY · 2 sessions · 1h 0m");
    assert_eq!(buf[(0, 11)].fg, Color::Cyan); // still focused
    // The summary takes the freed rows.
    assert!(line(&buf, 12).starts_with("╭ ▾ SUMMARY "));
    assert!(line(&buf, RENDER_HEIGHT - 2).starts_with("╰"));

    t.key(KeyCode::Tab);
    t.key(KeyCode::Char(' '));
    let buf = render(&t.app);
    assert_eq!(
        line(&buf, 12),
        "▸ SUMMARY · tracked 1h 0m · Programming 1h 0m"
    );
    assert_eq!(line(&buf, 13), "");

    t.key(KeyCode::Char(' ')); // and back
    assert!(line(&render(&t.app), 12).starts_with("╭ ▾ SUMMARY "));
}

#[test]
fn summary_shows_totals_for_the_day() {
    // 80×24: no room for the graphs, so text only.
    let t = with_sessions(TestApp::new(), 2);
    let buf = render(&t.app);
    let y = row_with(&buf, "Day       ");
    // 30m + 10m gap + 30m.
    assert!(line_from(&buf, y, 1).contains("· tracked 1h 0m · work 1h 0m · gaps 10m"));
    assert!(line_from(&buf, y + 1, 1).starts_with("Activity  Programming 1h 0m "));
    assert!(line_from(&buf, y + 2, 1).starts_with("Project   alpha 1h 0m "));
    assert!(
        line_from(&buf, y + 3, 1)
            .starts_with("Focus     avg 3.0 (2 rated) · 0 interruptions · no disruptions ")
    );
}

#[test]
fn the_day_splits_tracked_time_into_work_breaks_and_disruptions() {
    let mut t = TestApp::new();
    let at = |h, m| {
        chrono::Local
            .with_ymd_and_hms(2026, 10, 31, h, m, 0)
            .unwrap()
    };
    for (from, to, activity) in [
        ((9, 0), (10, 0), "Programming"),
        ((10, 0), (10, 15), BREAK),
        ((10, 15), (10, 25), DISRUPTION),
    ] {
        let mut s = finished_session(at(from.0, from.1), at(to.0, to.1), None);
        s.activity = activity.into();
        t.app.today_sessions.insert(0, s);
    }
    let buf = render(&t.app);
    let y = row_with(&buf, "Day       ");
    // Wrapped onto a second row at 80 columns.
    assert!(line_from(&buf, y, 1).contains("tracked 1h 25m · work 1h 0m · breaks 15m"));
    assert!(line_from(&buf, y + 1, 1).contains("disrupted 10m · gaps 0m"));
    for (row, value, color) in [
        (y, "1h 0m", Color::LightBlue),
        (y, "15m", Color::LightGreen),
        (y + 1, "10m", Color::LightRed),
    ] {
        assert_eq!(buf[(col_of(&buf, row, value), row)].fg, color, "{value}");
    }
    // The disruption's time isn't repeated next to its count.
    let focus = row_with(&buf, "Focus     ");
    assert!(line(&buf, focus).contains("· 1 disruption"));
    assert!(!line(&buf, focus).contains("(10m)"));
}

#[test]
fn a_tall_terminal_adds_graphs_of_the_day_to_the_summary() {
    let mut t = with_sessions(TestApp::new(), 2);
    t.app.set_size((80, 40));
    let mut terminal = Terminal::new(TestBackend::new(80, 40)).unwrap();
    terminal.draw(|frame| t.app.draw(frame)).unwrap();
    let buf = terminal.backend().buffer();
    let y = row_with(buf, "Day       ");
    // Values start at column 11 and take 68 columns.
    let (first, last) = (11, 11 + 67);

    // The timeline (from the top of the hour): 30m of programming, a
    // 10m gap, 30m more, up to now at the right edge.
    assert_eq!(buf[(last, y)].symbol(), "█");
    assert_eq!(buf[(last, y)].fg, Color::LightBlue);
    let timeline: String = line_from(buf, y, first).chars().take(68).collect();
    assert!(timeline.trim_start().contains(' '), "the gap: {timeline:?}");
    // On top, the focus line: green over the two rated (3) sessions,
    // blank over the gap.
    let focus: String = line_from(buf, y - 1, first).chars().take(68).collect();
    assert_eq!(buf[(last, y - 1)].symbol(), "▀");
    assert_eq!(buf[(last, y - 1)].fg, Color::Green);
    assert!(focus.trim().contains(' '), "the gap: {focus:?}");
    assert_eq!(
        line_from(buf, y - 1, 1)
            .chars()
            .take(10)
            .collect::<String>()
            .trim(),
        ""
    );
    // Hours under it, the first at the left edge, then the day in
    // words, without the span again.
    let hours: String = line_from(buf, y + 1, first).chars().take(68).collect();
    assert!(
        hours.chars().take(2).all(|c| c.is_ascii_digit()),
        "{hours:?}"
    );
    assert!(
        hours.chars().all(|c| c.is_ascii_digit() || c == ' '),
        "{hours:?}"
    );
    assert!(line_from(buf, y + 2, first).starts_with("tracked 1h 0m · work 1h 0m · gaps 10m"));
    assert_eq!(
        line_from(buf, y + 3, 1).trim_end_matches(['│', ' ']),
        "",
        "spacing"
    );

    // The share bar: all programming.
    assert!(line_from(buf, y + 4, 1).starts_with("Activity  "));
    assert!((first..=last).all(|x| buf[(x, y + 4)].fg == Color::LightBlue));
    assert!(line_from(buf, y + 5, first).starts_with("Programming 1h 0m"));
    assert_eq!(line_from(buf, y + 6, 1).trim_end_matches(['│', ' ']), "");
    // The project bar: all alpha, in its color.
    assert!(line_from(buf, y + 7, 1).starts_with("Project   "));
    assert!((first..=last).all(|x| buf[(x, y + 7)].fg == Color::Indexed(208)));
    assert!(line_from(buf, y + 8, first).starts_with("alpha 1h 0m"));
    assert_eq!(buf[(first, y + 8)].fg, Color::Indexed(208));
    assert_eq!(line_from(buf, y + 9, 1).trim_end_matches(['│', ' ']), "");
    assert!(line_from(buf, y + 10, 1).starts_with("Focus     "));
}

#[test]
fn the_timeline_starts_on_the_hour_with_its_label() {
    let local = |h, m| {
        chrono::Local
            .with_ymd_and_hms(2026, 10, 31, h, m, 0)
            .unwrap()
            .with_timezone(&Utc)
    };
    // The day began at 09:17; the graph starts at 09:00.
    let from = start_of_hour(local(9, 17));
    assert_eq!(from, local(9, 0));
    let hours = hour_labels(from, local(11, 30), 30);
    // 2h 30m over 30 cells: an hour every 12.
    assert_eq!(
        hours[0].content,
        format!(
            "09{}10{}11{}",
            " ".repeat(10),
            " ".repeat(10),
            " ".repeat(4)
        )
    );

    // The minutes before the first start are blank.
    let mut t = TestApp::new();
    t.app.today_sessions.push(finished_session(
        local(9, 30).into(),
        local(11, 30).into(),
        None,
    ));
    let summary = t.app.day_summary();
    let row = timeline_row(&summary, from, local(11, 30), 10);
    let symbols = row.iter().map(|s| s.content.as_ref()).collect::<String>();
    assert_eq!(symbols, "  ████████", "09:00–09:30 of 2h 30m: 2 cells");
}

#[test]
fn the_focus_line_shows_ratings_and_skips_breaks() {
    let local = |h, m| {
        chrono::Local
            .with_ymd_and_hms(2026, 10, 31, h, m, 0)
            .unwrap()
    };
    let mut t = TestApp::new();
    for (from, to, activity, focus) in [
        ((9, 0), (9, 30), "Programming", Some(1)),
        ((9, 30), (10, 0), BREAK, None),
        ((10, 0), (10, 30), "Programming", None),
        ((10, 30), (11, 0), MEETING, Some(3)),
    ] {
        let mut s = finished_session(local(from.0, from.1), local(to.0, to.1), focus);
        s.activity = activity.into();
        t.app.today_sessions.insert(0, s);
    }
    let summary = t.app.day_summary();
    let row = focus_row(
        &summary,
        local(9, 0).with_timezone(&Utc),
        local(11, 0).with_timezone(&Utc),
        8,
    );
    let shown = row
        .iter()
        .map(|s| (s.content.as_ref(), s.style.fg))
        .collect::<Vec<_>>();
    let (red, gray, green) = (Some(Color::Red), Some(Color::DarkGray), Some(Color::Green));
    assert_eq!(
        shown,
        [
            ("▀", red),
            ("▀", red),
            (" ", None),
            (" ", None),
            ("▀", gray),
            ("▀", gray),
            ("▀", green),
            ("▀", green),
        ]
    );
}

#[test]
fn half_cells_pack_two_colors_per_cell() {
    let (b, g) = (Some(Color::LightBlue), Some(Color::LightGreen));
    let cells = half_cells(&[b, b, b, g, None, g, b, None, None, None]);
    let shown = cells
        .iter()
        .map(|s| (s.content.as_ref(), s.style.fg, s.style.bg))
        .collect::<Vec<_>>();
    assert_eq!(
        shown,
        [
            ("█", b, None),
            ("▌", b, g),
            ("▐", g, None),
            ("▌", b, None),
            (" ", None, None),
        ]
    );
}

#[test]
fn projects_keep_colors_in_the_order_they_were_first_used() {
    let mut t = TestApp::new();
    for project in ["zeta", "alpha", "zeta", "mid"] {
        t.app.close_active_session();
        t.app
            .start_session("Programming", Some(project.into()), None);
    }
    t.app.refresh_suggestions();
    let color = |t: &TestApp, p| t.app.project_color(Some(p));
    // Not alphabetical: zeta came first.
    assert_eq!(color(&t, "zeta"), theme::PROJECT_PALETTE[0]);
    assert_eq!(color(&t, "alpha"), theme::PROJECT_PALETTE[1]);
    assert_eq!(color(&t, "mid"), theme::PROJECT_PALETTE[2]);
    assert_eq!(t.app.project_color(None), theme::NO_PROJECT);
    // All distinct, and none an activity's.
    let palette = theme::PROJECT_PALETTE;
    let distinct = palette.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(distinct.len(), palette.len());
    for activity in ["Programming", MEETING, BREAK, DISRUPTION] {
        assert!(!palette.contains(&theme::activity_color(activity)));
    }
}

#[test]
fn projects_sharing_a_preferred_color_get_distinct_ones_on_the_same_day() {
    let mut t = TestApp::new();
    // Ranks 0..=19: "p2" and "p18" both prefer the palette's third color.
    t.app.project_order = (0..20).map(|i| format!("p{i}")).collect();
    let p = theme::PROJECT_PALETTE;
    // Alone on a day, each gets its usual color.
    assert_eq!(t.app.project_color(Some("p18")), p[2]);
    let start = chrono::Local::now() - chrono::Duration::hours(2);
    for name in ["p18", "p2"] {
        let mut s = finished_session(start, start + chrono::Duration::minutes(30), None);
        s.project = Some(name.into());
        t.app.today_sessions.push(s);
    }
    // Together, the older keeps it and the other moves on.
    assert_eq!(t.app.project_color(Some("p2")), p[2]);
    assert_eq!(t.app.project_color(Some("p18")), p[3]);
}

#[test]
fn the_share_bar_splits_by_time() {
    let minutes = |m| chrono::Duration::minutes(m);
    let row = share_row(
        &[
            (theme::activity_color("Programming"), minutes(30)),
            (theme::activity_color(BREAK), minutes(10)),
        ],
        4,
    );
    // 8 halves: 6 programming, 2 break.
    let symbols = row.iter().map(|s| s.content.as_ref()).collect::<String>();
    assert_eq!(symbols, "████");
    assert_eq!(row[2].style.fg, Some(Color::LightBlue));
    assert_eq!(row[3].style.fg, Some(Color::LightGreen));
}

#[test]
fn empty_day_shows_hints_in_every_section() {
    let t = TestApp::new();
    let buf = render(&t.app);
    row_with(&buf, "No active session.");
    row_with(&buf, "No sessions yet today.");
    row_with(&buf, "Nothing tracked yet today.");
}

#[test]
fn scrolling_is_clamped_to_the_content() {
    let mut t = with_sessions(TestApp::new(), 1);
    t.key(KeyCode::Tab); // the keys scroll the focused section
    t.key(KeyCode::Down); // everything fits: nothing to scroll
    assert_eq!(t.app.today_scroll, 0);

    let mut t = with_sessions(TestApp::new(), 10);
    t.key(KeyCode::Tab);
    // 10 items × 2 rows + 9 separators = 29 rows; the viewport is 4.
    assert_eq!(t.app.viewport(Section::Today), 4);
    assert_eq!(t.app.max_scroll(Section::Today), 25);
    t.key(KeyCode::Down);
    t.key(KeyCode::Char('j'));
    assert_eq!(t.app.today_scroll, 2);
    t.key(KeyCode::PageDown);
    assert_eq!(t.app.today_scroll, 6);
    for _ in 0..5 {
        t.key(KeyCode::PageDown);
    }
    assert_eq!(t.app.today_scroll, 25);
    t.key(KeyCode::Home);
    assert_eq!(t.app.today_scroll, 0);
    t.key(KeyCode::Up);
    assert_eq!(t.app.today_scroll, 0);
    t.key(KeyCode::End);
    let buf = render(&t.app);
    // The oldest session is the last visible item.
    let last = row_with(&buf, "╭ ▾ SUMMARY ") - 3;
    let oldest = fmt_time(&t.app.today_sessions[9].started_at);
    assert!(line_from(&buf, last, 1).starts_with(&oldest));
}

#[test]
fn plus_and_minus_resize_and_equals_resets() {
    let mut t = TestApp::new();
    t.key(KeyCode::Char('+'));
    t.key(KeyCode::Char('+'));
    assert_eq!(t.app.layout.active_height, Some(12));
    assert!(line(&render(&t.app), 12).starts_with("╰"));
    t.key(KeyCode::Char('-'));
    assert_eq!(t.app.layout.active_height, Some(11));

    t.key(KeyCode::Tab);
    t.key(KeyCode::Tab);
    t.key(KeyCode::Char('+'));
    assert_eq!(t.app.layout.summary_height, Some(4));

    t.key(KeyCode::Char('='));
    assert_eq!(t.app.layout, LayoutPrefs::default());
}

#[test]
fn question_mark_opens_help_and_any_key_closes_it() {
    let mut t = TestApp::new();
    t.key(KeyCode::Char('?'));
    let buf = render(&t.app);
    assert!(line(&buf, row_with(&buf, "╭ Keys ")).contains("╭ Keys "));
    // The whole list fits a 24-row terminal, down to the last group.
    row_with(&buf, "wheel");
    assert!(!t.app.handle_key(press(KeyCode::Char('q')))); // closes, doesn't quit
    assert!(matches!(t.app.screen, Screen::Main));
}

#[test]
fn layout_is_saved_on_change_and_restored_on_start() {
    let t = TestApp::new();
    let path = t.dir.join("layout.conf");
    let open = |t: &TestApp| {
        let db = Db::open_at(&t.dir.join("jiary.db")).unwrap();
        let mut app = App::new(db, Some(path.clone()));
        app.set_size((RENDER_WIDTH, RENDER_HEIGHT));
        app
    };

    let mut app = open(&t);
    app.handle_key(press(KeyCode::Char('+')));
    app.handle_key(press(KeyCode::Tab));
    app.handle_key(press(KeyCode::Char(' ')));
    assert!(app.error.is_none());
    drop(app);

    let app = open(&t);
    assert_eq!(
        app.layout,
        LayoutPrefs {
            active_height: Some(11),
            today_collapsed: true,
            ..Default::default()
        }
    );
}

// --- mouse -------------------------------------------------------------

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

#[test]
fn dragging_the_active_border_resizes_it() {
    let mut t = TestApp::new();
    // No session: the active section is rows 1–10 (tall enough for the
    // cat and its caption), its border on row 10.
    drag(&mut t, 10, 14);
    assert_eq!(t.app.layout.active_height, Some(14));
    assert!(line(&render(&t.app), 14).starts_with("╰"));
    assert!(t.app.drag.is_none());
    // Clamped so today's sessions keep their minimum.
    drag(&mut t, 14, 100);
    let buf = render(&t.app);
    assert_eq!(
        row_with(&buf, "╭ ▾ SUMMARY ") - row_with(&buf, "╭ ▾ TODAY "),
        3
    );
}

#[test]
fn dragging_todays_bottom_border_resizes_the_summary() {
    let mut t = TestApp::new();
    let buf = render(&t.app);
    let border = row_with(&buf, "╭ ▾ SUMMARY ") - 1;
    drag(&mut t, border, border - 4);
    assert_eq!(t.app.layout.summary_height, Some(7));
    assert_eq!(t.app.focus, Section::Summary);
    assert_eq!(row_with(&render(&t.app), "╭ ▾ SUMMARY "), border - 3);
}

fn click(t: &mut TestApp, column: u16, row: u16) {
    mouse(t, MouseEventKind::Down(MouseButton::Left), column, row);
    mouse(t, MouseEventKind::Up(MouseButton::Left), column, row);
}

#[test]
fn clicking_focuses_and_clicking_a_title_toggles_collapse() {
    let mut t = TestApp::new();
    let buf = render(&t.app);
    let today = row_with(&buf, "╭ ▾ TODAY ");

    click(&mut t, 10, today + 1); // inside the body: focus only
    assert_eq!(t.app.focus, Section::Today);
    assert!(!t.app.layout.today_collapsed);

    click(&mut t, 4, today); // on " ▾ TODAY · 0 sessions "
    assert!(t.app.layout.today_collapsed);
    click(&mut t, 4, today); // on the collapsed "▸ TODAY …" line
    assert!(!t.app.layout.today_collapsed);
}

#[test]
fn pressing_a_title_only_toggles_on_release() {
    let mut t = TestApp::new();
    let today = row_with(&render(&t.app), "╭ ▾ TODAY ");
    mouse(&mut t, MouseEventKind::Down(MouseButton::Left), 4, today);
    assert!(!t.app.layout.today_collapsed);
    mouse(&mut t, MouseEventKind::Up(MouseButton::Left), 4, today);
    assert!(t.app.layout.today_collapsed);
}

#[test]
fn clicking_a_border_away_from_the_title_never_collapses() {
    let mut t = TestApp::new();
    let buf = render(&t.app);
    let today = row_with(&buf, "╭ ▾ TODAY ");
    let summary = row_with(&buf, "╭ ▾ SUMMARY ");
    click(&mut t, 60, today);
    click(&mut t, 60, summary);
    assert_eq!(t.app.layout, LayoutPrefs::default());
}

#[test]
fn either_row_of_the_line_between_sections_drags_it() {
    // No session: the active section's bottom border is row 10, today's
    // top border row 11. Grabbing row 11 moves the same line, keeping
    // the grab offset (no jump on the first move).
    let mut t = TestApp::new();
    drag(&mut t, 11, 15);
    assert_eq!(t.app.layout.active_height, Some(14));
    assert!(!t.app.layout.today_collapsed);
    assert_eq!(row_with(&render(&t.app), "╭ ▾ TODAY "), 15);

    // Same for the summary's top border, even when grabbed on the title.
    let mut t = TestApp::new();
    let summary = row_with(&render(&t.app), "╭ ▾ SUMMARY ");
    drag(&mut t, summary, summary - 4);
    assert_eq!(row_with(&render(&t.app), "╭ ▾ SUMMARY "), summary - 4);
    assert!(!t.app.layout.summary_collapsed);
    let mut t = TestApp::new();
    drag(&mut t, summary, summary - 2);
    assert_eq!(t.app.layout.summary_height, Some(5));
    let mut t = TestApp::new();
    let left = MouseButton::Left;
    mouse(&mut t, MouseEventKind::Down(left), 4, summary); // on "▾ SUMMARY"
    mouse(&mut t, MouseEventKind::Drag(left), 4, summary - 2);
    mouse(&mut t, MouseEventKind::Up(left), 4, summary - 2);
    assert!(!t.app.layout.summary_collapsed);
    assert_eq!(t.app.layout.summary_height, Some(5));
}

#[test]
fn wheel_scrolls_todays_sessions() {
    let mut t = with_sessions(TestApp::new(), 10);
    mouse(&mut t, MouseEventKind::ScrollDown, 10, 13);
    assert_eq!(t.app.today_scroll, 3);
    mouse(&mut t, MouseEventKind::ScrollUp, 10, 13);
    mouse(&mut t, MouseEventKind::ScrollUp, 10, 13);
    assert_eq!(t.app.today_scroll, 0);
}

#[test]
fn mouse_is_ignored_outside_the_main_screen() {
    let mut t = TestApp::new().on_project_step(vec![], "");
    drag(&mut t, 3, 7);
    assert_eq!(t.app.layout, LayoutPrefs::default());
}

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
                scroll: 99,
            }),
            Screen::Period(PeriodView {
                period: Period::Month,
                offset: 0,
                first: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                last: chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
                sessions: Vec::new(),
                scroll: 99,
            }),
        ]
    };
    for (width, height) in [(0, 0), (1, 1), (5, 3), (12, 6), (30, 10)] {
        for screen in screens() {
            let mut t = with_sessions(TestApp::new(), 5).on_screen(screen);
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
    }
}

// --- wrapped inputs ---------------------------------------------------

#[test]
fn wrap_text_breaks_after_the_last_space_that_fits() {
    assert_eq!(wrap_text("aaa bbb ccc", 8), ["aaa bbb ", "ccc"]);
    // A word longer than a row is split mid-word.
    assert_eq!(wrap_text("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    // A word that exactly fills a row stays whole.
    assert_eq!(wrap_text("abcd efgh", 5), ["abcd ", "efgh"]);
    assert_eq!(wrap_text("abcd efg", 4), ["abcd", " efg", ""]);
    // A full last row gets an empty row after it for the cursor.
    assert_eq!(wrap_text("abcd", 4), ["abcd", ""]);
    assert_eq!(wrap_text("", 4), [""]);
    // Counts chars, not bytes.
    assert_eq!(wrap_text("café olé", 5), ["café ", "olé"]);
}

#[test]
fn wrap_text_keeps_every_char() {
    for text in [
        "a b c d e f g",
        "  leading",
        "trailing  ",
        "x".repeat(30).as_str(),
    ] {
        for width in 1..12 {
            let rows = wrap_text(text, width);
            assert_eq!(rows.concat(), text, "width {width}");
            assert!(rows.iter().all(|r| r.chars().count() <= width));
        }
    }
}

/// Index of the first rendered row containing `needle`.
fn row_with(buf: &Buffer, needle: &str) -> u16 {
    (0..buf.area.height)
        .find(|&y| line(buf, y).contains(needle))
        .unwrap_or_else(|| panic!("no row contains {needle:?}"))
}

#[test]
fn long_project_input_wraps_inside_the_modal() {
    let typed = "a very long project name that keeps going";
    let t = TestApp::new().on_project_step(vec![], typed);
    let buf = render(&t.app);

    // "› " leaves 34 columns per row; the value wraps at a word
    // boundary, continuing under itself.
    let y = row_with(&buf, "› a very long project name");
    assert!(line_from(&buf, y, 22).starts_with("› a very long project name that "));
    assert!(line_from(&buf, y + 1, 22).starts_with("  keeps going "));
    // Both rows stay inside the modal's right border (x = 59).
    assert_eq!(buf[(59, y)].symbol(), "│");
    assert_eq!(buf[(59, y + 1)].symbol(), "│");
    // The modal grew by one row instead of squeezing the list.
    let top = row_with(&buf, "╭ Start session ");
    assert_eq!(
        line_from(&buf, top + START_MODAL_HEIGHT, 20),
        "╰──────────────────────────────────────╯"
    );
    // The cursor follows the text onto the second row.
    assert_eq!(
        t.app.cursor_position((RENDER_WIDTH, RENDER_HEIGHT)),
        Some((22 + 2 + "keeps going".len() as u16, y + 1))
    );
}

#[test]
fn long_notes_wrap_and_grow_the_note_modal() {
    let notes = "fixed the wrapping bug and wrote tests for every input";
    let t = TestApp::new().on_screen(Screen::AddNote(AddNoteForm {
        input: notes.into(),
    }));
    let buf = render(&t.app);

    let y = row_with(&buf, "› fixed");
    assert!(line_from(&buf, y, 22).starts_with("› fixed the wrapping bug and wrote "));
    assert!(line_from(&buf, y + 1, 22).starts_with("  tests for every input "));
    assert_eq!(buf[(59, y + 1)].symbol(), "│");
    // The modal grew by the extra row, keeping its two spare rows below
    // the input.
    assert_eq!(
        line(&buf, y + 2).trim(),
        "│                                      │"
    );
    assert_eq!(
        line(&buf, y + 3).trim(),
        "│                                      │"
    );
    assert!(line_from(&buf, y + 4, 20).starts_with("╰"));
    assert_eq!(
        t.app.cursor_position((RENDER_WIDTH, RENDER_HEIGHT)),
        Some((22 + 2 + "tests for every input".len() as u16, y + 1))
    );
}

#[test]
fn input_taller_than_the_terminal_keeps_the_cursor_row_visible() {
    let note = "word ".repeat(200);
    let t = TestApp::new().on_screen(Screen::AddNote(AddNoteForm { input: note }));
    let (width, height) = (RENDER_WIDTH, 12);
    let input = t
        .app
        .wrapped_input(chrome_layout(ratatui::layout::Rect::new(0, 0, width, height))[1])
        .unwrap();
    // Content is 10 rows; the small modal's 5 other rows leave 5 for input.
    assert_eq!(input.height(), 5);
    let (_, y) = t.app.cursor_position((width, height)).unwrap();
    assert!(
        y < height - 1,
        "cursor row {y} must be above the status line"
    );
}

#[test]
fn add_note_enter_saves_and_esc_discards() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    for (keys, expected) in [("kept", Some("kept\n")), ("dropped", Some("kept\n"))] {
        t.app.handle_key(press(KeyCode::Char('n')));
        for c in keys.chars() {
            t.app.handle_key(press(KeyCode::Char(c)));
        }
        let confirm = if keys == "kept" {
            KeyCode::Enter
        } else {
            KeyCode::Esc
        };
        t.app.handle_key(press(confirm));
        assert!(matches!(t.app.screen, Screen::Main));
        let notes = t.app.active_session.as_ref().unwrap().notes.as_deref();
        assert_eq!(notes, expected);
    }
}

/// The session `e` just ended (today's list is newest first).
fn last_ended(t: &TestApp) -> &Session {
    &t.app.today_sessions[0]
}

#[test]
fn e_ends_at_once_and_a_digit_rates_it_within_the_window() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    t.key(KeyCode::Char('e'));
    assert!(t.app.active_session.is_none());
    assert!(matches!(t.app.screen, Screen::Main));
    assert!(last_ended(&t).ended_at.is_some());
    assert_eq!(last_ended(&t).focus, None);

    t.key(KeyCode::Char('3'));
    assert_eq!(last_ended(&t).focus, Some(3));
    assert!(matches!(
        t.app.current_reaction(),
        Some((Reaction::Rated(3), _))
    ));
    // One rating per window.
    t.key(KeyCode::Char('1'));
    assert_eq!(last_ended(&t).focus, Some(3));
}

#[test]
fn digits_do_nothing_once_the_window_closes() {
    let mut t = TestApp::new();
    t.key(KeyCode::Char('2')); // nothing has ended
    t.app.start_session("Reading", None, None);
    t.key(KeyCode::Char('e'));
    let (id, _) = t.app.rating.unwrap();
    t.app.rating = Some((id, Instant::now() - RATING_WINDOW));
    t.key(KeyCode::Char('2'));
    assert_eq!(last_ended(&t).focus, None);
    assert!(t.app.current_reaction().is_none());
}

#[test]
fn breaks_and_disruptions_end_without_a_window_but_meetings_get_one() {
    for kind in [BREAK, DISRUPTION] {
        let mut t = TestApp::new();
        t.app.start_session(kind, None, None);
        t.key(KeyCode::Char('e'));
        assert!(t.app.active_session.is_none(), "{kind}");
        assert!(!t.app.rating_open(), "{kind}");
    }
    let mut t = TestApp::new();
    t.app.start_session(MEETING, None, None);
    t.key(KeyCode::Char('e'));
    assert!(t.app.rating_open());
}

#[test]
fn switching_away_from_work_opens_the_window_too() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    t.key(KeyCode::Char('m'));
    t.key(KeyCode::Char('2')); // rates the reading, not the meeting
    assert_eq!(activity(&t), Some(MEETING));
    assert_eq!(t.app.today_sessions[1].focus, Some(2));

    // Starting from nothing keeps the window from `e` open.
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    t.key(KeyCode::Char('e'));
    t.key(KeyCode::Char('b'));
    t.key(KeyCode::Char('1'));
    assert_eq!(t.app.today_sessions[1].focus, Some(1));

    // Ending a break doesn't replace it with nothing to rate.
    t.key(KeyCode::Char('d'));
    assert!(!t.app.rating_open());
}

#[test]
fn undo_reopens_what_e_ended_even_once_rated() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    let id = t.app.active_session.as_ref().unwrap().id;
    t.key(KeyCode::Char('e'));
    t.key(KeyCode::Char('3'));
    t.key(KeyCode::Char('u'));
    let active = t.app.active_session.as_ref().unwrap();
    assert_eq!(active.id, id);
    assert_eq!((active.ended_at, active.focus), (None, None));
    assert!(!t.app.rating_open());
    // Only the last change: a second `u` does nothing.
    t.key(KeyCode::Char('u'));
    assert_eq!(t.app.active_session.as_ref().unwrap().id, id);
}

#[test]
fn undo_takes_back_a_switch_or_a_start() {
    let mut t = TestApp::new();
    t.app
        .start_session("Reading", Some("alpha".into()), Some("beta".into()));
    t.key(KeyCode::Char('d'));
    t.key(KeyCode::Char('u'));
    assert_eq!(activity(&t), Some("Reading"));
    assert_eq!(t.app.today_sessions.len(), 1, "the disruption is gone");

    let mut t = TestApp::new();
    t.key(KeyCode::Char('b'));
    t.key(KeyCode::Char('u'));
    assert!(t.app.active_session.is_none());
    assert!(t.app.today_sessions.is_empty());
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

fn started_at(t: &TestApp) -> chrono::DateTime<Utc> {
    t.app.active_session.as_ref().unwrap().started_at
}

#[test]
fn recent_picks_start_with_one_key() {
    let mut t = TestApp::new();
    t.app
        .start_session("Reading", Some("alpha".into()), Some("beta".into()));
    t.key(KeyCode::Char('e'));
    t.app.start_session("Writing", Some("gamma".into()), None);
    t.key(KeyCode::Char('b')); // detours aren't offered
    t.key(KeyCode::Char('s'));
    let buf = render(&t.app);
    assert!(line(&buf, row_with(&buf, "[1]")).contains("[1] Writing · gamma"));
    assert!(line(&buf, row_with(&buf, "[2]")).contains("[2] Reading · alpha · beta"));
    assert!(!line(&buf, row_with(&buf, "[2]") + 1).contains("[3]"));

    t.key(KeyCode::Char('2'));
    assert!(matches!(t.app.screen, Screen::Main));
    let active = t.app.active_session.as_ref().unwrap();
    assert_eq!(active.activity, "Reading");
    assert_eq!(active.project.as_deref(), Some("alpha"));
    assert_eq!(active.task.as_deref(), Some("beta"));

    // Once something is typed, digits are just text.
    t.key(KeyCode::Char('s'));
    t.typed("v2");
    assert_eq!(t.form().activity.input, "v2");
}

#[test]
fn brackets_move_the_start_but_never_into_the_future() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    let start = backdate(&mut t, 20);
    t.key(KeyCode::Char('['));
    assert_eq!(started_at(&t), start - START_STEP);
    assert_eq!(t.app.today_sessions[0].started_at, start - START_STEP);
    for _ in 0..10 {
        t.key(KeyCode::Char(']'));
    }
    assert!((Utc::now() - started_at(&t)).num_seconds() < 2);
}

#[test]
fn moving_the_start_after_a_switch_moves_the_switch() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    let reading_start = backdate(&mut t, 60);
    t.key(KeyCode::Char('m'));
    let meeting_start = started_at(&t);
    t.key(KeyCode::Char('['));
    t.key(KeyCode::Char('['));
    assert_eq!(started_at(&t), meeting_start - START_STEP * 2);
    assert_eq!(t.app.today_sessions[1].ended_at, Some(started_at(&t)));
    // The reading keeps at least a minute.
    for _ in 0..20 {
        t.key(KeyCode::Char('['));
    }
    assert_eq!(
        started_at(&t),
        reading_start + chrono::TimeDelta::minutes(1)
    );
}

#[test]
fn moving_the_start_stops_once_at_the_end_of_a_session_before_a_gap() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    backdate(&mut t, 60);
    t.key(KeyCode::Char('e'));
    let reading = t.app.today_sessions[0].id;
    let reading_end = Utc::now() - chrono::TimeDelta::minutes(12);
    t.app.db.set_ended_at(reading, reading_end).unwrap();
    t.app.start_session("Writing", None, None);
    for _ in 0..3 {
        t.key(KeyCode::Char('['));
    }
    let ended = |t: &TestApp| t.app.today_sessions[1].ended_at.unwrap().timestamp();
    assert_eq!(started_at(&t).timestamp(), reading_end.timestamp());
    assert_eq!(ended(&t), reading_end.timestamp());
    // Now they touch: pressing on moves both, as after a switch.
    t.key(KeyCode::Char('['));
    assert_eq!(
        started_at(&t).timestamp(),
        (reading_end - START_STEP).timestamp()
    );
    assert_eq!(ended(&t), started_at(&t).timestamp());
}

/// A work session running for an hour, then 20 minutes away and back:
/// returns when the absence began.
fn away_and_back(t: &mut TestApp) -> chrono::DateTime<Utc> {
    let now = Utc::now();
    t.app
        .observe_idle(Some(std::time::Duration::from_secs(20 * 60)), now);
    assert!(matches!(t.app.screen, Screen::Main), "still away");
    t.app
        .observe_idle(Some(std::time::Duration::from_secs(1)), now + idle::POLL);
    now - chrono::TimeDelta::minutes(20)
}

fn working_for_an_hour() -> TestApp {
    let mut t = TestApp::new();
    t.app
        .start_session("Reading", Some("alpha".into()), Some("beta".into()));
    backdate(&mut t, 60);
    t
}

#[test]
fn coming_back_asks_about_the_absence_and_k_keeps_it() {
    let mut t = working_for_an_hour();
    let since = away_and_back(&mut t);
    assert!(matches!(t.app.screen, Screen::Away(s) if s == since));
    let buf = render(&t.app);
    assert!(line(&buf, row_with(&buf, "Welcome back")).contains("Welcome back"));
    assert!(line(&buf, row_with(&buf, "Away since")).contains("(20m)"));
    assert!(line(&buf, row_with(&buf, "[k]")).contains("count it as Reading"));
    assert!(line(&buf, row_with(&buf, "[d]")).contains("it was a disruption"));
    assert!(line(&buf, row_with(&buf, "[e]")).contains("end it at"));

    let id = t.app.active_session.as_ref().unwrap().id;
    t.key(KeyCode::Char('k'));
    assert!(matches!(t.app.screen, Screen::Main));
    assert_eq!(t.app.active_session.as_ref().unwrap().id, id);
}

#[test]
fn e_ends_the_session_when_the_absence_began() {
    let mut t = working_for_an_hour();
    let since = away_and_back(&mut t);
    t.key(KeyCode::Char('e'));
    assert!(t.app.active_session.is_none());
    assert_eq!(
        t.app.today_sessions[0].ended_at.unwrap().timestamp(),
        since.timestamp()
    );
    assert!(t.app.rating_open());
    t.key(KeyCode::Char('u'));
    assert!(t.app.active_session.as_ref().unwrap().ended_at.is_none());
}

#[test]
fn b_or_d_turn_the_absence_into_a_detour_and_resume() {
    for (key, detour) in [('b', BREAK), ('d', DISRUPTION)] {
        let mut t = working_for_an_hour();
        let first = t.app.active_session.as_ref().unwrap().id;
        let since = away_and_back(&mut t);
        t.key(KeyCode::Char(key));
        assert!(matches!(t.app.screen, Screen::Main));
        let active = t.app.active_session.as_ref().unwrap();
        assert_ne!(active.id, first);
        assert_eq!(active.project.as_deref(), Some("alpha"));
        let [resumed, pause, before] = &t.app.today_sessions[..] else {
            panic!("{:?}", t.app.today_sessions);
        };
        assert_eq!(resumed.id, active.id);
        assert_eq!(pause.activity, detour);
        assert_eq!(pause.started_at.timestamp(), since.timestamp());
        assert_eq!(before.ended_at.unwrap().timestamp(), since.timestamp());

        // One undo takes it all back.
        t.key(KeyCode::Char('u'));
        assert_eq!(t.app.active_session.as_ref().unwrap().id, first);
        assert_eq!(t.app.today_sessions.len(), 1);
    }
}

#[test]
fn short_absences_detours_and_newer_sessions_dont_ask() {
    let back = |t: &mut TestApp, minutes: u64| {
        let now = Utc::now();
        t.app
            .observe_idle(Some(std::time::Duration::from_secs(minutes * 60)), now);
        t.app
            .observe_idle(Some(std::time::Duration::ZERO), now + idle::POLL);
        matches!(t.app.screen, Screen::Away(_))
    };
    let mut t = working_for_an_hour();
    assert!(!back(&mut t, 10), "under AWAY_AFTER");

    let mut t = TestApp::new();
    t.app.start_session(MEETING, None, None);
    backdate(&mut t, 60);
    assert!(!back(&mut t, 20), "a detour");

    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    assert!(!back(&mut t, 20), "started after the absence began");
}

#[test]
fn sleep_counts_as_away() {
    let mut t = working_for_an_hour();
    let now = Utc::now();
    let before = now - chrono::TimeDelta::minutes(30);
    // No idle time on this platform: only the gap between polls shows.
    t.app.observe_idle(None, before);
    t.app.observe_idle(None, now);
    assert!(matches!(t.app.screen, Screen::Away(s) if s == before));
}

#[test]
fn a_key_before_the_poll_notices_the_return_brings_the_question_up() {
    let mut t = working_for_an_hour();
    t.app
        .observe_idle(Some(std::time::Duration::from_secs(20 * 60)), Utc::now());
    t.key(KeyCode::Char('i'));
    assert!(matches!(t.app.screen, Screen::Away(_)));
    // The key only brought the question up.
    assert_eq!(t.app.active_session.as_ref().unwrap().interruptions, None);
}

// --- command palette --------------------------------------------------

fn palette_input(t: &TestApp) -> &str {
    match &t.app.screen {
        Screen::Palette(form) => &form.input,
        _ => panic!("the palette isn't open"),
    }
}

#[test]
fn slash_opens_the_palette_listing_every_command() {
    let mut t = TestApp::new();
    t.key(KeyCode::Char('/'));
    assert_eq!(palette_input(&t), "/");
    let buf = render(&t.app);
    row_with(&buf, "Commands");
    row_with(&buf, &format!("{} commands", COMMANDS.len()));
    // The views first (they have no key), then the actions with theirs.
    let first = row_with(&buf, "weekly");
    assert!(line(&buf, first).contains("this week, by day"));
    assert!(line(&buf, row_with(&buf, "start")).contains("start a session      s"));
    // Names line up under the typed text after "› /", the pick marked.
    assert_eq!(col_of(&buf, first, "weekly"), col_of(&buf, 8, "/") + 1);
    assert!(
        line(&buf, first).contains(">weekly"),
        "{}",
        line(&buf, first)
    );
    assert_eq!(
        line(&buf, RENDER_HEIGHT - 1),
        "[↑↓] select  [Tab] complete  [Enter] run  [Esc] close"
    );
    // The cursor follows the typed "/".
    assert_eq!(t.app.cursor_position((80, 24)), Some((25, 8)));
}

#[test]
fn typing_filters_the_commands_fuzzily() {
    let names = |q| {
        palette_matches(q)
            .iter()
            .map(|c| c.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(names("br")[0], "break");
    assert_eq!(names("und"), ["undo"]);
    assert_eq!(names("zzz"), Vec::<&str>::new());
    // Every command has a distinct name.
    let mut all = names("");
    all.dedup();
    assert_eq!(all.len(), COMMANDS.len());
}

#[test]
fn enter_runs_the_command_like_its_key() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    t.key(KeyCode::Char('e'));
    assert!(t.app.active_session.is_none());
    t.key(KeyCode::Char('/'));
    t.typed("und");
    t.key(KeyCode::Enter);
    assert!(matches!(t.app.screen, Screen::Main));
    assert!(t.app.active_session.is_some(), "undone");
}

#[test]
fn arrows_pick_and_tab_completes() {
    let mut t = TestApp::new();
    t.key(KeyCode::Char('/'));
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    t.key(KeyCode::Up);
    t.key(KeyCode::Tab); // the second command
    assert_eq!(palette_input(&t), "/monthly");
    t.key(KeyCode::Up); // stays at the top
    t.key(KeyCode::Enter);
    assert!(matches!(&t.app.screen, Screen::Period(v) if v.period == Period::Month));
}

#[test]
fn esc_or_backspace_past_the_slash_closes_the_palette() {
    let mut t = TestApp::new();
    t.key(KeyCode::Char('/'));
    t.typed("ab");
    t.key(KeyCode::Backspace);
    t.key(KeyCode::Backspace);
    assert_eq!(palette_input(&t), "/");
    t.key(KeyCode::Backspace);
    assert!(matches!(t.app.screen, Screen::Main));
    t.key(KeyCode::Char('/'));
    t.typed("star");
    t.key(KeyCode::Esc);
    assert!(matches!(t.app.screen, Screen::Main), "nothing run");
}

#[test]
fn a_command_can_open_another_screen_or_quit() {
    let mut t = TestApp::new();
    t.key(KeyCode::Char('/'));
    t.typed("start");
    t.key(KeyCode::Enter);
    assert!(matches!(t.app.screen, Screen::StartSession(_)));

    let mut t = TestApp::new();
    t.key(KeyCode::Char('/'));
    t.typed("quit");
    assert!(t.app.handle_key(press(KeyCode::Enter)));
}

// --- weeks and months -------------------------------------------------

fn period_view(t: &TestApp) -> &PeriodView {
    match &t.app.screen {
        Screen::Period(view) => view,
        _ => panic!("no period on screen"),
    }
}

fn open(t: &mut TestApp, command: &str) {
    t.key(KeyCode::Char('/'));
    t.typed(command);
    t.key(KeyCode::Enter);
}

#[test]
fn weekly_shows_this_week_day_by_day() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", Some("alpha".into()), None);
    backdate(&mut t, 30);
    t.key(KeyCode::Char('e'));
    open(&mut t, "weekly");

    let today = chrono::Local::now().date_naive();
    let (first, last) = Period::Week.days(0, today);
    let view = period_view(&t);
    assert_eq!((view.first, view.last, view.offset), (first, last, 0));
    let buf = render(&t.app);
    assert!(line(&buf, 1).contains(&Period::Week.title(first, last)));
    // Monday to Sunday, one row each, from the box's first row.
    for (i, day) in first.iter_days().take(7).enumerate() {
        let label = day.format("%a %-d").to_string();
        assert!(
            line_from(&buf, 2 + i as u16, 1).starts_with(&label),
            "{label}"
        );
    }
    // Today: the accent label, its timeline in the activity's color,
    // its time.
    // (By position: the header line names today too.)
    let y = 2 + (today - first).num_days() as u16;
    assert_eq!(buf[(1, y)].fg, Color::Cyan);
    assert!(line(&buf, y).contains("30m"));
    assert!((11..67).any(|x| buf[(x, y)].fg == Color::LightBlue));
    // Then the totals.
    assert!(line(&buf, row_with(&buf, "Total ")).contains("tracked 30m · work 30m · 1 day"));
    row_with(&buf, "Activity  ");
    row_with(&buf, "Project   ");
    assert_eq!(
        line(&buf, RENDER_HEIGHT - 1),
        "[←→] previous/next  [w/m/y] week/month/year  [↑↓] scroll  [Esc] back"
    );
}

#[test]
fn the_focus_grid_colors_each_day_by_its_focus() {
    let day = |d| chrono::NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
    let at = |d, h| {
        chrono::Local
            .with_ymd_and_hms(2026, 10, d, h, 0, 0)
            .unwrap()
    };
    let (first, last, today) = (
        day(26),
        chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        day(29),
    );
    let summary_of = |sessions: Vec<Session>| summarize(&sessions, None, Utc::now());
    let days = vec![
        (
            day(26),
            summary_of(vec![finished_session(at(26, 9), at(26, 11), Some(3))]),
        ),
        (
            day(27),
            summary_of(vec![finished_session(at(27, 9), at(27, 11), None)]),
        ),
        (day(28), summary_of(Vec::new())),
        (
            day(29),
            summary_of(vec![finished_session(at(29, 9), at(29, 10), Some(1))]),
        ),
    ];
    let grid = focus_grid(&days, first, last, today);
    // A header (the week's Monday), then Mon–Sun.
    assert_eq!(grid.len(), 8);
    assert_eq!(grid[0][1].content, "26");
    // Every row as wide as the others, so the block centers evenly.
    let widths = grid
        .iter()
        .map(item_width)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(widths.len(), 1, "{widths:?}");
    let cell = |weekday: usize| &grid[1 + weekday][1];
    let fg = |weekday: usize| cell(weekday).style.fg;
    assert_eq!(
        (cell(0).content.as_ref(), fg(0)),
        ("██", Some(theme::FOCUS_RAMP[8]))
    );
    assert_eq!(
        (cell(1).content.as_ref(), fg(1)),
        ("██", Some(theme::UNRATED))
    );
    assert_eq!(
        (cell(2).content.as_ref(), fg(2)),
        ("░░", Some(theme::UNRATED))
    );
    assert_eq!(
        (cell(3).content.as_ref(), fg(3)),
        ("██", Some(theme::FOCUS_RAMP[0]))
    );
    // Friday on: still to come.
    assert_eq!(cell(4).content, "  ");
    // Today's weekday in the accent.
    assert_eq!(grid[4][0].style, theme::TITLE);
    assert_eq!(grid[1][0].style, theme::DIMMED);
    // The legend: 1, the ramp, 3.
    let legend = focus_legend()
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();
    assert_eq!(legend, "1 █████████ 3");
}

#[test]
fn yearly_leads_with_the_focus_grid_then_a_row_per_month() {
    use chrono::Datelike;
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    backdate(&mut t, 60);
    t.key(KeyCode::Char('e'));
    t.key(KeyCode::Char('3'));
    open(&mut t, "yearly");
    let today = chrono::Local::now().date_naive();
    let view = period_view(&t);
    assert_eq!(view.period, Period::Year);
    let lines = t.app.period_lines(view, 78);
    let text = |l: &Line| {
        l.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    };
    // The month names over the grid, then four rows of weekday pairs.
    assert!(text(&lines[0]).contains("Jan"), "{}", text(&lines[0]));
    for (row, weekday) in ["Mon", "Wed", "Fri", "Sun"].iter().enumerate() {
        assert!(text(&lines[1 + row]).trim_start().starts_with(weekday));
    }
    // A row per month, this one with its hour and rating.
    let months = lines
        .iter()
        .map(text)
        .filter(|l| {
            [
                "Jan ", "Feb ", "Mar ", "Apr ", "May ", "Jun ", "Jul ", "Aug ", "Sep ", "Oct ",
                "Nov ", "Dec ",
            ]
            .iter()
            .any(|m| l.starts_with(m))
        })
        .collect::<Vec<_>>();
    assert_eq!(months.len(), 12, "{months:#?}");
    let this_month = &months[today.month0() as usize];
    assert!(this_month.contains("1h 0m 3.0"), "{this_month}");
    // Paging by year; y from another view.
    t.key(KeyCode::Left);
    assert_eq!(period_view(&t).first, Period::Year.days(-1, today).0);
    t.key(KeyCode::Char('w'));
    t.key(KeyCode::Char('y'));
    assert_eq!(period_view(&t).period, Period::Year);
}

#[test]
fn the_year_grid_stacks_two_days_per_cell() {
    let day = |m, d| chrono::NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let at = |d, h| chrono::Local.with_ymd_and_hms(2026, 1, d, h, 0, 0).unwrap();
    let summary_of = |sessions: Vec<Session>| summarize(&sessions, None, Utc::now());
    // 2026 starts on a Thursday: its first week's column holds Thu 1
    // (bottom of the Wed/Thu row) to Sun 4.
    let days = vec![
        (
            day(1, 1),
            summary_of(vec![finished_session(at(1, 9), at(1, 10), Some(3))]),
        ),
        (
            day(1, 2),
            summary_of(vec![finished_session(at(2, 9), at(2, 10), None)]),
        ),
        (day(1, 3), summary_of(Vec::new())),
        (
            day(1, 5),
            summary_of(vec![finished_session(at(5, 9), at(5, 10), Some(1))]),
        ),
    ];
    let grid = year_focus_grid(&days, day(1, 1), day(12, 31), day(1, 5));
    assert_eq!(grid.len(), 5, "month names, then four weekday-pair rows");
    assert!(grid[0][0].content.starts_with("    Jan"));
    let cell = |row: usize, week: usize| &grid[row][1 + week];
    let colors = |row, week| {
        (
            cell(row, week).content.as_ref(),
            cell(row, week).style.fg,
            cell(row, week).style.bg,
        )
    };
    // Week 0: Wed (before the year) blank over Thu 1 (rated 3).
    assert_eq!(colors(2, 0), ("▄", Some(theme::FOCUS_RAMP[8]), None));
    // Fri 2 (unrated) over Sat 3 (nothing tracked).
    assert_eq!(
        colors(3, 0),
        ("▀", Some(theme::UNRATED), Some(theme::EMPTY_DAY))
    );
    // Week 1: Mon 5 (rated 1) over Tue 6, still to come.
    assert_eq!(colors(1, 1), ("▀", Some(theme::FOCUS_RAMP[0]), None));
    // 53 weeks fit in 80 columns.
    assert!(item_width(&grid[1]) <= 78);
}

#[test]
fn arrows_page_through_weeks_but_not_into_the_future() {
    let mut t = TestApp::new();
    open(&mut t, "weekly");
    t.key(KeyCode::Right); // already the current week
    assert_eq!(period_view(&t).offset, 0);
    t.key(KeyCode::Left);
    t.key(KeyCode::Char('h'));
    let view = period_view(&t);
    assert_eq!(view.offset, -2);
    let today = chrono::Local::now().date_naive();
    assert_eq!(view.first, Period::Week.days(-2, today).0);
    let buf = render(&t.app);
    row_with(&buf, "Nothing tracked this week.");
    t.key(KeyCode::Char('l'));
    assert_eq!(period_view(&t).offset, -1);
    t.key(KeyCode::Esc);
    assert!(matches!(t.app.screen, Screen::Main));
}

#[test]
fn monthly_has_a_row_per_day_in_weeks_and_scrolls() {
    let mut t = TestApp::new();
    open(&mut t, "monthly");
    let today = chrono::Local::now().date_naive();
    let (first, last) = Period::Month.days(0, today);
    let view = period_view(&t);
    assert_eq!(view.period, Period::Month);
    let lines = t.app.period_lines(view, 78);
    let labels = lines
        .iter()
        .filter_map(|l| l.spans.first())
        .map(|span| span.content.trim().to_string())
        .collect::<Vec<_>>();
    for day in first.iter_days().take_while(|d| *d <= last) {
        let label = day.format("%a %-d").to_string();
        assert_eq!(labels.iter().filter(|l| **l == label).count(), 1, "{label}");
    }
    // More than fits: it scrolls, and not past its end.
    t.key(KeyCode::End);
    let end = period_view(&t).scroll;
    assert!(end > 0);
    t.key(KeyCode::Down);
    assert_eq!(period_view(&t).scroll, end);
    t.key(KeyCode::Home);
    assert_eq!(period_view(&t).scroll, 0);
    // w switches to this week; q goes back too.
    t.key(KeyCode::Char('w'));
    assert_eq!(period_view(&t).period, Period::Week);
    t.key(KeyCode::Char('q'));
    assert!(matches!(t.app.screen, Screen::Main));
}

fn activity(t: &TestApp) -> Option<&str> {
    t.app.active_session.as_ref().map(|s| s.activity.as_str())
}

#[test]
fn disrupt_then_resume_restores_previous_session() {
    let mut t = TestApp::new();
    t.app
        .start_session("Reading", Some("alpha".into()), Some("beta".into()));
    t.app.handle_key(press(KeyCode::Char('d')));
    assert_eq!(activity(&t), Some(DISRUPTION));
    t.app.handle_key(press(KeyCode::Char('r')));
    let active = t.app.active_session.as_ref().unwrap();
    assert_eq!(active.activity, "Reading");
    assert_eq!(active.project.as_deref(), Some("alpha"));
    assert_eq!(active.task.as_deref(), Some("beta"));
    assert_eq!(t.app.today_sessions.len(), 3);
}

#[test]
fn disrupt_during_a_disruption_is_a_noop() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    t.app.handle_key(press(KeyCode::Char('d')));
    let id = t.app.active_session.as_ref().unwrap().id;
    t.app.handle_key(press(KeyCode::Char('d')));
    assert_eq!(t.app.active_session.as_ref().unwrap().id, id);
    assert_eq!(t.app.today_sessions.len(), 2);
}

#[test]
fn break_then_resume_restores_previous_session() {
    let mut t = TestApp::new();
    t.app
        .start_session("Reading", Some("alpha".into()), Some("beta".into()));
    t.key(KeyCode::Char('b'));
    assert_eq!(activity(&t), Some(BREAK));
    t.key(KeyCode::Char('b')); // no-op on top of a break
    assert_eq!(t.app.today_sessions.len(), 2);
    t.key(KeyCode::Char('r'));
    let active = t.app.active_session.as_ref().unwrap();
    assert_eq!(active.activity, "Reading");
    assert_eq!(active.project.as_deref(), Some("alpha"));
    assert_eq!(active.task.as_deref(), Some("beta"));
}

#[test]
fn breaks_and_disruptions_switch_and_resume_skips_both() {
    let mut t = TestApp::new();
    t.app.start_session("Writing", None, None);
    t.key(KeyCode::Char('b'));
    t.key(KeyCode::Char('d')); // the break turned into a disruption
    assert_eq!(activity(&t), Some(DISRUPTION));
    t.key(KeyCode::Char('b')); // and back
    assert_eq!(activity(&t), Some(BREAK));
    t.key(KeyCode::Char('r')); // past the break and the disruption
    assert_eq!(activity(&t), Some("Writing"));
}

#[test]
fn meeting_starts_from_work_or_from_nothing_and_resume_returns() {
    let mut t = TestApp::new();
    t.key(KeyCode::Char('m')); // nothing running: starts anyway
    assert_eq!(activity(&t), Some(MEETING));
    t.key(KeyCode::Char('m')); // no-op on top of a meeting
    assert_eq!(t.app.today_sessions.len(), 1);
    t.key(KeyCode::Char('r')); // nothing to go back to: keeps meeting
    assert_eq!(activity(&t), Some(MEETING));

    t.app.close_active_session();
    t.app.start_session("Writing", Some("p".into()), None);
    t.key(KeyCode::Char('m'));
    t.key(KeyCode::Char('b')); // a break straight after the meeting
    t.key(KeyCode::Char('r')); // back to work, past both detours
    let active = t.app.active_session.as_ref().unwrap();
    assert_eq!(active.activity, "Writing");
    assert_eq!(active.project.as_deref(), Some("p"));
}

#[test]
fn break_can_start_from_nothing_but_disruption_cannot() {
    let mut t = TestApp::new();
    t.key(KeyCode::Char('d')); // nothing to interrupt
    assert!(t.app.active_session.is_none());
    t.key(KeyCode::Char('b')); // e.g. right after ending a session
    assert_eq!(activity(&t), Some(BREAK));
}

#[test]
fn resume_with_nothing_to_resume_keeps_the_disruption() {
    let mut t = TestApp::new();
    t.app.start_session(DISRUPTION, None, None);
    t.app.handle_key(press(KeyCode::Char('r')));
    assert_eq!(activity(&t), Some(DISRUPTION));
}

#[test]
fn note_modal_shows_context_and_input() {
    let mut t = TestApp::new();
    t.app.active_session = Some(running_session(chrono::Duration::seconds(60), 0));
    t.app.screen = Screen::AddNote(AddNoteForm {
        input: "hello".into(),
    });
    let buf = render(&t.app);
    assert_eq!(buf[(20, 9)].symbol(), "╭");
    assert_eq!(buf[(20, 9)].fg, Color::Reset);
    assert!(line_from(&buf, 9, 20).starts_with("╭ Add note "));
    // Dimmed project·task·activity context line.
    assert!(line_from(&buf, 10, 22).starts_with("alpha · beta · Programming"));
    assert_eq!(buf[(22, 10)].fg, Color::Gray);
    // Input row: prompt + typed text.
    assert!(line_from(&buf, 11, 22).starts_with("› hello"));
    assert_eq!(buf[(22, 11)].fg, Color::Cyan);
    assert_eq!(buf[(24, 11)].fg, Color::Reset);
}
