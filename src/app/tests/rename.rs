//! `/rename`.

use super::*;

// --- renaming labels ----------------------------------------------

fn rename_form(t: &TestApp) -> &RenameForm {
    match &t.app.screen {
        Screen::Rename(form) => form,
        _ => panic!("expected the rename form"),
    }
}

/// A finished session with these labels, then the running one's mirror
/// and suggestions reloaded, as after real use.
fn worked_on(t: &mut TestApp, activity: &str, project: Option<&str>, task: Option<&str>) {
    t.app
        .start_session(activity, project.map(String::from), task.map(String::from));
    t.app.close_active_session();
    t.app.refresh_after_session_change();
    t.app.refresh_suggestions();
}

/// Types `text` into the rename form's current step and presses Enter.
fn answer(t: &mut TestApp, text: &str) {
    t.typed(text);
    t.key(KeyCode::Enter);
}

#[test]
fn rename_merges_a_typo_into_the_name_in_use() {
    let mut t = TestApp::new();
    worked_on(&mut t, "Progamming", Some("jiary"), None);
    worked_on(&mut t, "Progamming", Some("jiary"), None);
    worked_on(&mut t, "Programming", Some("jiary"), None);
    t.app.start_session("Programming", None, None);

    open(&mut t, "rename");
    assert_eq!(rename_form(&t).step, RenameStep::What);
    t.key(KeyCode::Enter); // Activity
    answer(&mut t, "Progamming");
    answer(&mut t, "Programming");
    let form = rename_form(&t);
    assert_eq!(form.step, RenameStep::Confirm);
    assert_eq!(form.counts, (2, 2), "the running session counts too");
    assert_eq!(
        form.question(),
        "Merge \"Progamming\" into \"Programming\" (2 sessions; it has 2)?"
    );
    // Nothing is written before Enter.
    assert!(t.app.activities.contains(&"Progamming".to_string()));

    t.key(KeyCode::Enter);
    assert!(matches!(t.app.screen, Screen::Main));
    assert!(!t.app.activities.contains(&"Progamming".to_string()));
    assert!(
        t.app
            .today_sessions
            .iter()
            .all(|s| s.activity == "Programming"),
        "today's list shows the new name at once"
    );
}

#[test]
fn renaming_a_project_renames_the_running_sessions_too() {
    let mut t = TestApp::new();
    worked_on(&mut t, "Writing", Some("jiry"), None);
    t.app.start_session("Writing", Some("jiry".into()), None);

    open(&mut t, "rename");
    t.key(KeyCode::Down);
    t.key(KeyCode::Enter); // Project
    answer(&mut t, "jiry");
    answer(&mut t, "jiary");
    assert_eq!(
        rename_form(&t).question(),
        "Rename \"jiry\" to \"jiary\" in 2 sessions?"
    );
    t.key(KeyCode::Enter);
    let active = t.app.active_session.as_ref().unwrap();
    assert_eq!(active.project.as_deref(), Some("jiary"));
    assert_eq!(t.app.projects, ["jiary"]);
    assert_eq!(t.app.project_order, ["jiary"]);
}

#[test]
fn tasks_are_renamed_within_the_project_picked_first() {
    let mut t = TestApp::new();
    worked_on(&mut t, "Writing", Some("api"), Some("docs"));
    worked_on(&mut t, "Writing", Some("website"), Some("docs"));

    open(&mut t, "rename");
    t.key(KeyCode::Tab);
    t.key(KeyCode::Tab);
    t.key(KeyCode::Enter); // Task
    assert_eq!(rename_form(&t).step, RenameStep::Project);
    answer(&mut t, "api");
    answer(&mut t, "docs");
    answer(&mut t, "api docs");
    assert_eq!(
        rename_form(&t).question(),
        "Rename \"docs\" to \"api docs\" in 1 session? Only in api."
    );
    t.key(KeyCode::Enter);
    assert_eq!(t.app.db.tasks_in(Some("api")).unwrap(), ["api docs"]);
    assert_eq!(t.app.db.tasks_in(Some("website")).unwrap(), ["docs"]);
}

#[test]
fn detours_are_never_renamed_nor_become_the_new_name() {
    let mut t = TestApp::new();
    worked_on(&mut t, "Reading", None, None);
    worked_on(&mut t, BREAK, None, None);

    open(&mut t, "rename");
    t.key(KeyCode::Enter); // Activity
    assert!(
        !rename_form(&t)
            .from
            .suggestions()
            .contains(&BREAK.to_string()),
        "detours aren't offered"
    );
    answer(&mut t, BREAK);
    let form = rename_form(&t);
    assert_eq!(form.step, RenameStep::From);
    assert!(form.notice.as_ref().unwrap().contains("can't be renamed"));

    for _ in BREAK.chars() {
        t.key(KeyCode::Backspace);
    }
    answer(&mut t, "Reading");
    answer(&mut t, "break");
    let form = rename_form(&t);
    assert_eq!(form.step, RenameStep::To, "any case");
    assert!(
        form.notice
            .as_ref()
            .unwrap()
            .contains("Nothing can be renamed to")
    );
}

#[test]
fn unknown_blank_or_unchanged_names_are_refused_and_esc_writes_nothing() {
    let mut t = TestApp::new();
    worked_on(&mut t, "Reading", None, None);

    open(&mut t, "rename");
    t.key(KeyCode::Enter);
    answer(&mut t, "Readin");
    assert_eq!(rename_form(&t).step, RenameStep::From, "not a name in use");
    t.key(KeyCode::Char('g'));
    assert_eq!(rename_form(&t).notice, None, "the next key clears it");
    t.key(KeyCode::Enter);
    assert_eq!(rename_form(&t).step, RenameStep::To);

    t.key(KeyCode::Enter);
    assert_eq!(rename_form(&t).step, RenameStep::To, "blank");
    answer(&mut t, "Reading");
    assert_eq!(rename_form(&t).step, RenameStep::To, "unchanged");
    for _ in 0.."Reading".len() {
        t.key(KeyCode::Backspace);
    }
    // A case-only change is a real rename.
    answer(&mut t, "reading");
    assert_eq!(rename_form(&t).step, RenameStep::Confirm);

    t.key(KeyCode::Esc);
    assert!(matches!(t.app.screen, Screen::Main));
    assert_eq!(t.app.activities, ["Reading"]);
}

#[test]
fn the_rename_form_shows_its_steps_as_tabs_and_the_question() {
    let mut t = TestApp::new();
    worked_on(&mut t, "Writing", Some("api"), Some("docs"));
    open(&mut t, "rename");
    t.key(KeyCode::Up);
    let buf = render(&t.app);
    let title = row_with(&buf, " Rename ");
    assert!(line(&buf, title + 1).contains("What │ From │ To"));
    assert!(line(&buf, title + 3).contains(">Activity"));

    t.key(KeyCode::Tab);
    t.key(KeyCode::Tab);
    t.key(KeyCode::Enter); // Task
    let buf = render(&t.app);
    assert!(line(&buf, title + 1).contains("What │ Project │ From │ To"));
    assert!(
        t.app
            .cursor_position((RENDER_WIDTH, RENDER_HEIGHT))
            .is_some()
    );

    answer(&mut t, "api");
    answer(&mut t, "docs");
    answer(&mut t, "reference");
    let buf = render(&t.app);
    let text = (title..title + 10)
        .map(|y| line(&buf, y))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("Rename \"docs\""), "{text}");
    assert!(
        t.app
            .cursor_position((RENDER_WIDTH, RENDER_HEIGHT))
            .is_none()
    );
    assert!(line(&buf, RENDER_HEIGHT - 1).contains("[Enter] apply"));
}
