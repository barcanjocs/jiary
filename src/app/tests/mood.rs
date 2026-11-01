//! Sleep and mood: the prompt, `/mood`, `/mood-off` and `/mood-on`, and
//! what the longer views make of them.

use super::*;

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

fn mood_form(t: &TestApp) -> &MoodForm {
    match &t.app.screen {
        Screen::Mood(form) => form,
        _ => panic!("no sleep-and-mood prompt on screen"),
    }
}

fn stored(t: &TestApp) -> Option<(Option<i32>, Option<i32>)> {
    let day = t.app.db.day(today()).unwrap()?;
    Some((day.sleep, day.mood))
}

#[test]
fn the_days_first_start_asks_then_opens_the_start_form() {
    let mut t = TestApp::new().asking_mood();
    t.key(KeyCode::Char('s'));
    assert!(mood_form(&t).then_start);
    assert_eq!(mood_form(&t).step, MoodStep::Sleep);
    t.key(KeyCode::Char('3'));
    assert_eq!(mood_form(&t).step, MoodStep::Mood);
    t.key(KeyCode::Char('2'));
    assert!(matches!(t.app.screen, Screen::StartSession(_)));
    assert_eq!(stored(&t), Some((Some(3), Some(2))));

    // Asked once a day.
    t.key(KeyCode::Esc);
    t.key(KeyCode::Char('s'));
    assert!(matches!(t.app.screen, Screen::StartSession(_)));
}

#[test]
fn esc_skips_the_day_and_backspace_goes_back_to_sleep() {
    let mut t = TestApp::new().asking_mood();
    t.key(KeyCode::Char('s'));
    t.key(KeyCode::Char('1'));
    t.key(KeyCode::Backspace);
    assert_eq!(mood_form(&t).step, MoodStep::Sleep);
    assert_eq!(mood_form(&t).sleep, Some(1));
    t.key(KeyCode::Esc);
    assert!(matches!(t.app.screen, Screen::StartSession(_)));
    assert_eq!(stored(&t), Some((None, None)), "skipped, not answered");

    t.key(KeyCode::Esc);
    t.key(KeyCode::Char('s'));
    assert!(
        matches!(t.app.screen, Screen::StartSession(_)),
        "a skipped day isn't asked again"
    );
}

#[test]
fn slash_mood_changes_the_answers_and_esc_cancels() {
    let mut t = TestApp::new().asking_mood();
    open(&mut t, "mood");
    assert!(!mood_form(&t).then_start);
    t.key(KeyCode::Char('1'));
    t.key(KeyCode::Char('1'));
    assert!(matches!(t.app.screen, Screen::Main));
    assert_eq!(stored(&t), Some((Some(1), Some(1))));

    // Opens on what was stored; Enter keeps sleep, 3 changes mood.
    open(&mut t, "mood");
    assert_eq!(
        (mood_form(&t).sleep, mood_form(&t).mood),
        (Some(1), Some(1))
    );
    t.key(KeyCode::Enter);
    t.key(KeyCode::Char('3'));
    assert_eq!(stored(&t), Some((Some(1), Some(3))));

    open(&mut t, "mood");
    t.key(KeyCode::Char('2'));
    t.key(KeyCode::Esc);
    assert!(matches!(t.app.screen, Screen::Main));
    assert_eq!(stored(&t), Some((Some(1), Some(3))), "cancelled");
}

#[test]
fn mood_off_stops_asking_and_is_saved() {
    let t = TestApp::new();
    let path = t.dir.join("settings.conf");
    let db = || Db::open_at(&t.dir.join("jiary.db")).unwrap();
    let mut app = App::new(db(), settings_at(&path));
    assert!(app.settings.sleep_and_mood, "on by default");
    for c in "/mood-off".chars() {
        app.handle_key(press(KeyCode::Char(c)));
    }
    app.handle_key(press(KeyCode::Enter));
    assert!(!app.settings.sleep_and_mood);
    let buf = render(&app);
    assert!(line(&buf, RENDER_HEIGHT - 1).starts_with("Sleep and mood off"));
    app.handle_key(press(KeyCode::Char('s')));
    assert!(matches!(app.screen, Screen::StartSession(_)));
    drop(app);

    let mut app = App::new(db(), settings_at(&path));
    assert!(!app.settings.sleep_and_mood, "read back at start");
    for c in "/mood".chars() {
        app.handle_key(press(KeyCode::Char(c)));
    }
    app.handle_key(press(KeyCode::Enter));
    assert!(matches!(app.screen, Screen::Main), "/mood says it's off");
    let buf = render(&app);
    assert!(line(&buf, RENDER_HEIGHT - 1).contains("/mood-on turns them on"));

    for c in "/mood-on".chars() {
        app.handle_key(press(KeyCode::Char(c)));
    }
    app.handle_key(press(KeyCode::Enter));
    assert!(app.settings.sleep_and_mood);
    app.handle_key(press(KeyCode::Char('s')));
    assert!(matches!(app.screen, Screen::Mood(_)));
}

#[test]
fn the_prompt_shows_both_questions_and_the_answers_in_their_colors() {
    let mut t = TestApp::new().asking_mood();
    t.key(KeyCode::Char('s'));
    t.key(KeyCode::Char('3'));
    let buf = render(&t.app);
    let title = row_with(&buf, "Sleep and mood");
    assert!(line(&buf, title + 1).contains("Before the day's first session"));
    let sleep = row_with(&buf, "Sleep  ");
    assert!(line(&buf, sleep).contains("1 poor · 2 ok · 3 good"));
    let mood = row_with(&buf, "› Mood");
    assert!(line(&buf, mood).contains("1 low · 2 ok · 3 good"));
    let good = col_of(&buf, sleep, "3 good");
    assert_eq!(buf[(good, sleep)].fg, theme::GOOD);
    assert_eq!(buf[(good - 4, sleep)].fg, theme::MUTED, "2 ok not picked");
    assert!(line(&buf, RENDER_HEIGHT - 1).contains("[Esc] skip"));
}

/// Last month's first `n` days: an hour of work each, rated `focus`, and
/// each day's sleep and mood as `answers` gives them.
fn rated_days(t: &TestApp, answers: &[(i32, Option<i32>, Option<i32>)]) -> chrono::NaiveDate {
    use chrono::TimeZone;
    let (first, _) = Period::Month.days(-1, today());
    for (i, &(focus, sleep, mood)) in answers.iter().enumerate() {
        let date = first + chrono::Duration::days(i as i64);
        let at = |h| {
            chrono::Local
                .from_local_datetime(&date.and_hms_opt(h, 0, 0).unwrap())
                .unwrap()
                .with_timezone(&Utc)
        };
        let id = t
            .app
            .db
            .insert_finished_session("Programming", at(9), at(10))
            .unwrap();
        t.app.db.set_focus(id, focus).unwrap();
        t.app.db.set_day(Day { date, sleep, mood }).unwrap();
    }
    first
}

fn period_text(t: &TestApp) -> Vec<String> {
    let Screen::Period(view) = &t.app.screen else {
        panic!("no period on screen");
    };
    t.app
        .period_lines(view, RENDER_WIDTH - 2)
        .iter()
        .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

#[test]
fn a_month_sets_focus_against_sleep_and_mood() {
    let mut t = TestApp::new().asking_mood();
    let good = (3, Some(3), Some(2));
    rated_days(&t, &[good, good, good, good, good, (1, Some(1), Some(2))]);
    t.app.open_period(Period::Month, -1);
    let text = period_text(&t);
    let row = |label: &str| {
        text.iter()
            .find(|line| line.starts_with(label))
            .unwrap_or_else(|| panic!("no {label:?} row in {text:#?}"))
            .clone()
    };
    assert!(row("Sleep ").starts_with("Sleep     focus 3.0 after good sleep (5 days)"));
    assert!(
        text.iter()
            .any(|line| line.contains("after poor sleep: not enough data yet (1 day)"))
    );
    assert_eq!(row("Mood "), "Mood      focus 2.7 with ok mood (6 days)");
    // The three grids side by side, titled.
    let titles = text
        .iter()
        .find(|line| line.contains("Focus") && line.contains("Sleep") && line.contains("Mood"))
        .unwrap_or_else(|| panic!("no grid titles in {text:#?}"));
    assert!(titles.find("Focus") < titles.find("Sleep"));

    // Off: none of it.
    t.app.settings.sleep_and_mood = false;
    let text = period_text(&t);
    assert!(!text.iter().any(|line| line.starts_with("Sleep ")));
    assert!(!text.iter().any(|line| line.contains("Mood")));
}

#[test]
fn a_period_without_answers_shows_no_sleep_or_mood() {
    let mut t = TestApp::new().asking_mood();
    rated_days(&t, &[(3, None, None), (2, None, None)]);
    t.app.open_period(Period::Month, -1);
    let text = period_text(&t);
    assert!(!text.iter().any(|line| line.starts_with("Sleep ")));
}

#[test]
fn few_answers_are_not_enough_data_yet() {
    let mut t = TestApp::new().asking_mood();
    rated_days(&t, &[(3, Some(2), None), (2, Some(2), None)]);
    t.app.open_period(Period::Month, -1);
    let text = period_text(&t);
    assert!(text.contains(&"Sleep     after ok sleep: not enough data yet (2 days)".to_string()));
    assert!(text.contains(&"Mood      no rated days to compare".to_string()));
}

#[test]
fn a_year_stacks_the_grids_titled() {
    use chrono::Datelike;
    let mut t = TestApp::new().asking_mood();
    let first = rated_days(&t, &[(3, Some(1), Some(3))]);
    t.app
        .open_period(Period::Year, first.year() - today().year());
    let text = period_text(&t);
    let header = |title: &str| {
        text.iter()
            .position(|line| line.trim_start().starts_with(title) && line.contains("Jan"))
            .unwrap_or_else(|| panic!("no {title} grid in {text:#?}"))
    };
    let (focus, sleep, mood) = (header("Focus"), header("Sleep"), header("Mood"));
    assert!(focus < sleep && sleep < mood);
    assert_eq!(sleep - focus, 6, "five rows and a blank one apart");
}

#[test]
fn a_rating_grid_colors_the_answers_and_textures_the_rest() {
    let day = |d| chrono::NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
    let cell = |date| match date {
        d if d == day(26) => DayCell::Rated(3.0),
        d if d == day(27) => DayCell::Rated(1.0),
        _ => DayCell::Empty,
    };
    let grid = focus_grid(day(26), day(31), day(28), &cell, false);
    // No weekday labels: the squares start the row.
    let square = |weekday: usize| &grid[1 + weekday][0];
    assert_eq!(square(0).style.fg, Some(theme::FOCUS_RAMP[8]));
    assert_eq!(square(1).style.fg, Some(theme::FOCUS_RAMP[0]));
    assert_eq!(square(2).content, "░░");
    assert_eq!(square(3).content, "  ", "still to come");
}
