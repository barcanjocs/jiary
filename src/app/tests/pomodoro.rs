//! The Pomodoro: `p`, the cue under the timer, the cat, and `/settings`.

use super::*;

use crate::settings::PomodoroMode;

/// A work session running for `minutes`, with the Pomodoro `mode` and a
/// 25-minute target.
fn working_for(minutes: i64, mode: PomodoroMode) -> TestApp {
    let mut t = TestApp::new();
    t.app.settings.pomodoro = mode;
    t.app.active_session = Some(running_session(chrono::Duration::minutes(minutes), 0));
    t
}

/// The active section's info rows as text.
fn info(t: &TestApp) -> Vec<String> {
    t.app
        .active_lines()
        .iter()
        .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

#[test]
fn the_bar_under_the_timer_fills_toward_the_target() {
    let t = working_for(10, PomodoroMode::Soft);
    // 10 of 25 minutes: 7 of the bar's 18 cells.
    assert_eq!(
        info(&t)[1],
        format!("{}{} 25m", "━".repeat(7), "─".repeat(11))
    );
    assert_eq!(t.app.timer_style(), theme::TITLE);
    assert_eq!(t.app.mood(), Mood::Working);

    // Off: no bar, and the rows are as before.
    let t = working_for(10, PomodoroMode::Off);
    assert!(info(&t)[1].starts_with("Project"));
}

#[test]
fn at_the_target_the_timer_turns_green_and_the_cat_has_a_coffee() {
    let t = working_for(40, PomodoroMode::Soft);
    let line = &t.app.active_lines()[1];
    assert_eq!(
        line.spans[0].content,
        "━".repeat(18),
        "full, and stays full"
    );
    assert_eq!(line.spans[0].style.fg, Some(theme::GOOD));
    assert_eq!(t.app.timer_style().fg, Some(theme::GOOD));
    assert_eq!(t.app.mood(), Mood::BreakEarned);
    assert!(t.app.caption().is_some());

    // On the screen: the timer's digits in green.
    let buf = render(&t.app);
    let y = row_with(&buf, "since");
    let x = col_of(&buf, y, "00:40");
    assert_eq!(buf[(x, y)].fg, theme::GOOD);
}

#[test]
fn off_the_cat_dozes_as_before_and_detours_get_no_cue() {
    let t = working_for(40, PomodoroMode::Off);
    assert_eq!(t.app.mood(), Mood::Drowsy);
    assert_eq!(t.app.timer_style(), theme::TITLE);

    let mut t = working_for(40, PomodoroMode::Soft);
    for detour in [BREAK, MEETING, DISRUPTION] {
        t.app.active_session.as_mut().unwrap().activity = detour.into();
        assert_eq!(t.app.pomodoro(), None, "{detour}");
        assert!(info(&t)[1].starts_with("Project"), "{detour}");
    }
}

#[test]
fn the_target_comes_from_the_settings() {
    let mut t = working_for(40, PomodoroMode::Soft);
    t.app.settings.target_minutes = 50;
    assert!(!t.app.pomodoro().unwrap().reached());
    assert!(info(&t)[1].ends_with(" 50m"));
    assert_eq!(t.app.mood(), Mood::Working);
}

#[test]
fn p_turns_it_on_and_off_and_is_saved() {
    let t = TestApp::new();
    let path = t.dir.join("settings.conf");
    let db = || Db::open_at(&t.dir.join("jiary.db")).unwrap();
    let mut app = App::new(db(), None, Some(path.clone()));
    assert_eq!(app.settings.pomodoro, PomodoroMode::Off, "off by default");
    app.handle_key(press(KeyCode::Char('p')));
    assert_eq!(app.settings.pomodoro, PomodoroMode::Soft);
    let buf = render(&app);
    assert!(line(&buf, RENDER_HEIGHT - 1).starts_with("Pomodoro on (soft): 25m target"));
    drop(app);

    let mut app = App::new(db(), None, Some(path));
    assert_eq!(app.settings.pomodoro, PomodoroMode::Soft, "read back");
    app.handle_key(press(KeyCode::Char('p')));
    assert_eq!(app.settings.pomodoro, PomodoroMode::Off);
}

#[test]
fn settings_change_with_the_arrows_and_space() {
    let mut t = TestApp::new();
    open(&mut t, "settings");
    assert!(matches!(t.app.screen, Screen::Settings(_)));
    let buf = render(&t.app);
    row_with(&buf, " Settings ");
    assert!(line(&buf, row_with(&buf, "Pomodoro ")).contains("off"));
    assert!(line(&buf, row_with(&buf, "Target ")).contains("25m"));
    assert!(line(&buf, row_with(&buf, "Sleep and mood ")).contains("off"));

    // Pomodoro: Space cycles the modes.
    t.key(KeyCode::Char(' '));
    assert_eq!(t.app.settings.pomodoro, PomodoroMode::Soft);
    // Target: 5-minute steps, within its limits.
    t.key(KeyCode::Down);
    t.key(KeyCode::Right);
    t.key(KeyCode::Right);
    assert_eq!(t.app.settings.target_minutes, 35);
    for _ in 0..30 {
        t.key(KeyCode::Left);
    }
    assert_eq!(t.app.settings.target_minutes, 5);
    // Sleep and mood: a switch.
    t.key(KeyCode::Down);
    t.key(KeyCode::Enter);
    assert!(t.app.settings.sleep_and_mood);
    t.key(KeyCode::Down);
    t.key(KeyCode::Char(' '));
    assert!(!t.app.settings.sleep_and_mood, "the last row stays picked");

    t.key(KeyCode::Esc);
    assert!(matches!(t.app.screen, Screen::Main));
}
