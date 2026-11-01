//! The cat: its place, captions, moods and reactions.

use super::*;

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
