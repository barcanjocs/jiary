//! The command palette.

use super::*;

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
