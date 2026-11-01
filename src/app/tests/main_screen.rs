//! The main screen: its sections, scrolling, resizing and the mouse.

use super::*;

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
