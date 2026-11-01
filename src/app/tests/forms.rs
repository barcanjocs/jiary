//! The start and note forms: fuzzy matching, autocomplete, activities and
//! wrapped inputs.

use super::*;

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
