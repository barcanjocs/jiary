//! Coming back after an absence.

use super::*;

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
    assert!(matches!(t.app.screen, Screen::Away(a) if a.since == since && !a.closed));
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
    assert!(matches!(t.app.screen, Screen::Away(a) if a.since == before));
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

// --- a session left running while Jiary was closed ----------------------

/// Opens Jiary again on `t`'s db, as if it last ran `minutes_ago`.
fn reopened(t: &TestApp, minutes_ago: Option<i64>) -> App {
    let seen = t.dir.join("last_seen");
    let _ = std::fs::remove_file(&seen);
    if let Some(minutes) = minutes_ago {
        let at = Utc::now() - chrono::TimeDelta::minutes(minutes);
        std::fs::write(&seen, format!("{}\n", at.to_rfc3339())).unwrap();
    }
    let db = Db::open_at(&t.dir.join("jiary.db")).unwrap();
    let mut app = App::new(
        db,
        Files {
            seen: Some(seen),
            ..Default::default()
        },
    );
    app.set_size((RENDER_WIDTH, RENDER_HEIGHT));
    app
}

#[test]
fn opening_on_a_session_left_running_asks_from_when_jiary_closed() {
    let t = working_for_an_hour();
    let mut app = reopened(&t, Some(40));
    let Screen::Away(away) = app.screen else {
        panic!("no question");
    };
    assert!(away.closed);
    assert!((Utc::now() - away.since).num_minutes() >= 39);
    let buf = render(&app);
    assert!(line(&buf, row_with(&buf, "Closed since")).contains("(40m)"));
    row_with(&buf, "[b] it was a break");

    // e: the session ends when Jiary last ran, not now.
    app.handle_key(press(KeyCode::Char('e')));
    assert!(app.active_session.is_none());
    assert_eq!(
        app.today_sessions[0].ended_at.unwrap().timestamp(),
        away.since.timestamp()
    );
}

#[test]
fn a_short_close_or_no_record_asks_nothing() {
    let t = working_for_an_hour();
    assert!(matches!(reopened(&t, Some(5)).screen, Screen::Main));
    assert!(matches!(reopened(&t, None).screen, Screen::Main));
    // Nothing running: nothing to ask.
    let t = TestApp::new();
    assert!(matches!(reopened(&t, Some(600)).screen, Screen::Main));
}

#[test]
fn a_detour_left_running_can_only_be_kept_or_ended() {
    let mut t = TestApp::new();
    t.app.start_session(BREAK, None, None);
    backdate(&mut t, 600);
    let mut app = reopened(&t, Some(500));
    assert!(matches!(app.screen, Screen::Away(_)));
    let buf = render(&app);
    assert!(!(0..RENDER_HEIGHT).any(|y| line(&buf, y).contains("[b]")));
    assert_eq!(line(&buf, RENDER_HEIGHT - 1), "[k] keep  [e] end");
    app.handle_key(press(KeyCode::Char('b')));
    assert!(matches!(app.screen, Screen::Away(_)), "b does nothing");
    app.handle_key(press(KeyCode::Char('k')));
    assert!(matches!(app.screen, Screen::Main));
    assert_eq!(app.active_session.as_ref().unwrap().activity, BREAK);
}

#[test]
fn the_heartbeat_notes_when_jiary_runs_at_most_every_minute() {
    let t = TestApp::new();
    let mut app = reopened(&t, None);
    let seen = t.dir.join("last_seen");
    let read = || std::fs::read_to_string(&seen).unwrap();
    let now = Utc::now();
    app.heartbeat(now, false);
    assert_eq!(read().trim(), now.to_rfc3339());
    let later = now + chrono::TimeDelta::seconds(5);
    app.heartbeat(later, false);
    assert_eq!(read().trim(), now.to_rfc3339(), "not yet");
    app.heartbeat(later, true);
    assert_eq!(read().trim(), later.to_rfc3339(), "forced, on quitting");
}

#[test]
fn a_session_from_an_earlier_day_says_so() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    let started = backdate(&mut t, 24 * 60 + 5);
    let buf = render(&t.app);
    let expected = format!(
        "since yesterday {}",
        started.with_timezone(&chrono::Local).format("%H:%M")
    );
    row_with(&buf, &expected);
}
