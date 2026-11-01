//! Ending, rating, undoing, picking and moving sessions, and detours.

use super::*;

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
