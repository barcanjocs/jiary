//! `/rewind`: the list of past days, searching it, and opening a day.

use super::*;

use super::super::rewind::Rewind;

/// A finished hour-long session `days_ago` days back at `hour`, with what
/// it was and a note.
fn past(
    t: &TestApp,
    days_ago: i64,
    hour: u32,
    activity: &str,
    project: Option<&str>,
    task: Option<&str>,
    note: Option<&str>,
) {
    let date = chrono::Local::now().date_naive() - chrono::Duration::days(days_ago);
    let at = |h| {
        chrono::Local
            .from_local_datetime(&date.and_hms_opt(h, 0, 0).unwrap())
            .unwrap()
            .with_timezone(&Utc)
    };
    let db = &t.app.db;
    let session = db.create_session(activity, project, task).unwrap();
    if let Some(note) = note {
        db.append_note(session.id, note).unwrap();
    }
    db.complete_session(session.id, Some(3)).unwrap();
    db.set_started_at(session.id, at(hour)).unwrap();
    db.set_ended_at(session.id, at(hour + 1)).unwrap();
}

/// Three days: yesterday on the api, three days ago on the blog (with a
/// note about the api), a week ago reading.
fn history() -> TestApp {
    let t = TestApp::new();
    past(&t, 1, 9, "Programming", Some("api"), Some("auth"), None);
    past(
        &t,
        1,
        11,
        "Writing",
        Some("api"),
        Some("docs"),
        Some("error codes table"),
    );
    past(
        &t,
        3,
        10,
        "Writing",
        Some("blog"),
        None,
        Some("compare the api limits"),
    );
    past(&t, 7, 14, "Reading", None, None, None);
    t
}

fn rewind(t: &TestApp) -> &Rewind {
    match &t.app.screen {
        Screen::Rewind(view) => view,
        Screen::PastDay(day) => &day.rewind,
        _ => panic!("not in /rewind"),
    }
}

fn listed(t: &TestApp) -> Vec<chrono::NaiveDate> {
    let view = rewind(t);
    view.found.iter().map(|f| view.days[f.day].date).collect()
}

fn ago(days: i64) -> chrono::NaiveDate {
    chrono::Local::now().date_naive() - chrono::Duration::days(days)
}

fn list_text(t: &TestApp) -> Vec<String> {
    let view = rewind(t);
    t.app
        .rewind_lines(view, RENDER_WIDTH - 2)
        .0
        .iter()
        .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

#[test]
fn rewind_lists_every_day_with_sessions_newest_first() {
    let mut t = history();
    open(&mut t, "rewind");
    assert_eq!(listed(&t), [ago(1), ago(3), ago(7)]);
    let buf = render(&t.app);
    assert!(line(&buf, 1).contains("REWIND · 3 days"));
    let text = list_text(&t);
    assert!(text[0].starts_with("› "), "the newest picked: {text:?}");
    assert_eq!(text.len(), 3, "a row per day, no sessions without a query");
    // The input at the bottom, with the cursor on it.
    let y = row_with(&buf, "search: words");
    assert_eq!(
        t.app.cursor_position((RENDER_WIDTH, RENDER_HEIGHT)),
        Some((3, y))
    );
}

#[test]
fn typing_searches_and_shows_the_matching_sessions() {
    let mut t = history();
    open(&mut t, "rewind");
    t.typed("api");
    // Yesterday's api sessions, and the blog note about the api.
    assert_eq!(listed(&t), [ago(1), ago(3)]);
    let text = list_text(&t);
    assert!(text.iter().any(|l| l.contains("api · auth · Programming")));
    assert!(text.iter().any(|l| l.contains("• compare the api limits")));
    assert!(line(&render(&t.app), 1).contains("REWIND · 2 of 3 days"));

    // Prefixes: only the project.
    t.key(KeyCode::Esc);
    t.typed("p:api n:codes");
    assert_eq!(listed(&t), [ago(1)]);
    assert_eq!(rewind(&t).found[0].sessions.len(), 1);

    // A date.
    t.key(KeyCode::Esc);
    t.typed(&ago(7).format("%Y-%m-%d").to_string());
    assert_eq!(listed(&t), [ago(7)]);

    t.key(KeyCode::Esc);
    t.typed("zebra");
    assert!(listed(&t).is_empty());
    row_with(&render(&t.app), "No day matches.");
}

#[test]
fn esc_clears_the_query_then_closes() {
    let mut t = history();
    open(&mut t, "rewind");
    t.typed("api");
    t.key(KeyCode::Esc);
    assert_eq!(rewind(&t).input, "");
    assert_eq!(listed(&t).len(), 3);
    t.key(KeyCode::Esc);
    assert!(matches!(t.app.screen, Screen::Main));
}

#[test]
fn the_grid_lights_only_the_matching_days() {
    let mut t = history();
    open(&mut t, "rewind");
    t.typed("p:blog");
    let view = rewind(&t);
    assert!(view.matched.contains(&ago(3)));
    assert!(!view.matched.contains(&ago(1)));
}

#[test]
fn enter_opens_the_day_and_arrows_step_through_the_list() {
    let mut t = history();
    open(&mut t, "rewind");
    t.key(KeyCode::Down);
    assert_eq!(rewind(&t).selection, 1);
    t.key(KeyCode::Enter);
    assert!(matches!(t.app.screen, Screen::PastDay(_)));
    let buf = render(&t.app);
    let title = ago(3).format("%A %-d %B %Y").to_string().to_uppercase();
    assert!(line(&buf, 1).contains(&title), "{}", line(&buf, 1));
    row_with(&buf, "1 session");
    row_with(&buf, "• compare the api limits");
    assert_eq!(
        line(&buf, RENDER_HEIGHT - 1),
        "[←→] older/newer  [↑↓] scroll  [Esc] back"
    );

    // ← older, → newer, within the list.
    t.key(KeyCode::Left);
    assert_eq!(rewind(&t).selection, 2);
    t.key(KeyCode::Left);
    assert_eq!(rewind(&t).selection, 2, "the oldest stays");
    t.key(KeyCode::Right);
    t.key(KeyCode::Right);
    assert_eq!(rewind(&t).selection, 0);
    row_with(&render(&t.app), "2 sessions");

    // Back to the list, the query and pick kept.
    t.key(KeyCode::Esc);
    assert!(matches!(t.app.screen, Screen::Rewind(_)));
    assert_eq!(rewind(&t).selection, 0);
}

#[test]
fn the_wheel_moves_the_pick_and_scrolls_a_day() {
    let mut t = history();
    open(&mut t, "rewind");
    mouse(&mut t, MouseEventKind::ScrollDown, 10, 10);
    assert_eq!(rewind(&t).selection, 1);
    mouse(&mut t, MouseEventKind::ScrollUp, 10, 10);
    mouse(&mut t, MouseEventKind::ScrollUp, 10, 10);
    assert_eq!(rewind(&t).selection, 0);
    t.key(KeyCode::Enter);
    mouse(&mut t, MouseEventKind::ScrollDown, 10, 10);
    let Screen::PastDay(day) = &t.app.screen else {
        panic!("no day open");
    };
    assert!(day.scroll > 0);
}

#[test]
fn the_pick_stays_in_view() {
    let t = TestApp::new();
    for days_ago in 1..40 {
        past(&t, days_ago, 9, "Reading", None, None, None);
    }
    let mut t = t;
    open(&mut t, "rewind");
    for _ in 0..30 {
        t.key(KeyCode::Down);
    }
    let buf = render(&t.app);
    let picked = (0..RENDER_HEIGHT)
        .find(|&y| line(&buf, y).contains("› ") && !line(&buf, y).contains("search"))
        .expect("the picked day on screen");
    assert!(line(&buf, picked).contains(&ago(31).format("%-d %b").to_string()));
    t.key(KeyCode::Home);
    assert_eq!(rewind(&t).selection, 0);
    assert_eq!(rewind(&t).scroll, 0);
}
