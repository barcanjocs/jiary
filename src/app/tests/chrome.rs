//! The header and the status line.

use super::*;

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
