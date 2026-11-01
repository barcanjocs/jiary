//! The day's summary and graphs.

use super::*;

#[test]
fn summary_shows_totals_for_the_day() {
    // 80×24: no room for the graphs, so text only.
    let t = with_sessions(TestApp::new(), 2);
    let buf = render(&t.app);
    let y = row_with(&buf, "Day       ");
    // 30m + 10m gap + 30m.
    assert!(line_from(&buf, y, 1).contains("· tracked 1h 0m · work 1h 0m · gaps 10m"));
    assert!(line_from(&buf, y + 1, 1).starts_with("Activity  Programming 1h 0m "));
    assert!(line_from(&buf, y + 2, 1).starts_with("Project   alpha 1h 0m "));
    assert!(
        line_from(&buf, y + 3, 1)
            .starts_with("Focus     avg 3.0 (2 rated) · 0 interruptions · no disruptions ")
    );
}

#[test]
fn the_day_splits_tracked_time_into_work_breaks_and_disruptions() {
    let mut t = TestApp::new();
    let at = |h, m| {
        chrono::Local
            .with_ymd_and_hms(2026, 10, 31, h, m, 0)
            .unwrap()
    };
    for (from, to, activity) in [
        ((9, 0), (10, 0), "Programming"),
        ((10, 0), (10, 15), BREAK),
        ((10, 15), (10, 25), DISRUPTION),
    ] {
        let mut s = finished_session(at(from.0, from.1), at(to.0, to.1), None);
        s.activity = activity.into();
        t.app.today_sessions.insert(0, s);
    }
    let buf = render(&t.app);
    let y = row_with(&buf, "Day       ");
    // Wrapped onto a second row at 80 columns.
    assert!(line_from(&buf, y, 1).contains("tracked 1h 25m · work 1h 0m · breaks 15m"));
    assert!(line_from(&buf, y + 1, 1).contains("disrupted 10m · gaps 0m"));
    for (row, value, color) in [
        (y, "1h 0m", Color::LightBlue),
        (y, "15m", Color::LightGreen),
        (y + 1, "10m", Color::LightRed),
    ] {
        assert_eq!(buf[(col_of(&buf, row, value), row)].fg, color, "{value}");
    }
    // The disruption's time isn't repeated next to its count.
    let focus = row_with(&buf, "Focus     ");
    assert!(line(&buf, focus).contains("· 1 disruption"));
    assert!(!line(&buf, focus).contains("(10m)"));
}

#[test]
fn a_tall_terminal_adds_graphs_of_the_day_to_the_summary() {
    let mut t = with_sessions(TestApp::new(), 2);
    t.app.set_size((80, 40));
    let mut terminal = Terminal::new(TestBackend::new(80, 40)).unwrap();
    terminal.draw(|frame| t.app.draw(frame)).unwrap();
    let buf = terminal.backend().buffer();
    let y = row_with(buf, "Day       ");
    // Values start at column 11 and take 68 columns.
    let (first, last) = (11, 11 + 67);

    // The timeline (from the top of the hour): 30m of programming, a
    // 10m gap, 30m more, up to now at the right edge.
    assert_eq!(buf[(last, y)].symbol(), "█");
    assert_eq!(buf[(last, y)].fg, Color::LightBlue);
    let timeline: String = line_from(buf, y, first).chars().take(68).collect();
    assert!(timeline.trim_start().contains(' '), "the gap: {timeline:?}");
    // On top, the focus line: green over the two rated (3) sessions,
    // blank over the gap.
    let focus: String = line_from(buf, y - 1, first).chars().take(68).collect();
    assert_eq!(buf[(last, y - 1)].symbol(), "▀");
    assert_eq!(buf[(last, y - 1)].fg, Color::Green);
    assert!(focus.trim().contains(' '), "the gap: {focus:?}");
    assert_eq!(
        line_from(buf, y - 1, 1)
            .chars()
            .take(10)
            .collect::<String>()
            .trim(),
        ""
    );
    // Hours under it, the first at the left edge, then the day in
    // words, without the span again.
    let hours: String = line_from(buf, y + 1, first).chars().take(68).collect();
    assert!(
        hours.chars().take(2).all(|c| c.is_ascii_digit()),
        "{hours:?}"
    );
    assert!(
        hours.chars().all(|c| c.is_ascii_digit() || c == ' '),
        "{hours:?}"
    );
    assert!(line_from(buf, y + 2, first).starts_with("tracked 1h 0m · work 1h 0m · gaps 10m"));
    assert_eq!(
        line_from(buf, y + 3, 1).trim_end_matches(['│', ' ']),
        "",
        "spacing"
    );

    // The share bar: all programming.
    assert!(line_from(buf, y + 4, 1).starts_with("Activity  "));
    assert!((first..=last).all(|x| buf[(x, y + 4)].fg == Color::LightBlue));
    assert!(line_from(buf, y + 5, first).starts_with("Programming 1h 0m"));
    assert_eq!(line_from(buf, y + 6, 1).trim_end_matches(['│', ' ']), "");
    // The project bar: all alpha, in its color.
    assert!(line_from(buf, y + 7, 1).starts_with("Project   "));
    assert!((first..=last).all(|x| buf[(x, y + 7)].fg == Color::Indexed(208)));
    assert!(line_from(buf, y + 8, first).starts_with("alpha 1h 0m"));
    assert_eq!(buf[(first, y + 8)].fg, Color::Indexed(208));
    assert_eq!(line_from(buf, y + 9, 1).trim_end_matches(['│', ' ']), "");
    assert!(line_from(buf, y + 10, 1).starts_with("Focus     "));
}

#[test]
fn the_timeline_starts_on_the_hour_with_its_label() {
    let local = |h, m| {
        chrono::Local
            .with_ymd_and_hms(2026, 10, 31, h, m, 0)
            .unwrap()
            .with_timezone(&Utc)
    };
    // The day began at 09:17; the graph starts at 09:00.
    let from = start_of_hour(local(9, 17));
    assert_eq!(from, local(9, 0));
    let hours = hour_labels(from, local(11, 30), 30);
    // 2h 30m over 30 cells: an hour every 12.
    assert_eq!(
        hours[0].content,
        format!(
            "09{}10{}11{}",
            " ".repeat(10),
            " ".repeat(10),
            " ".repeat(4)
        )
    );

    // The minutes before the first start are blank.
    let mut t = TestApp::new();
    t.app.today_sessions.push(finished_session(
        local(9, 30).into(),
        local(11, 30).into(),
        None,
    ));
    let summary = t.app.day_summary();
    let row = timeline_row(&summary, from, local(11, 30), 10);
    let symbols = row.iter().map(|s| s.content.as_ref()).collect::<String>();
    assert_eq!(symbols, "  ████████", "09:00–09:30 of 2h 30m: 2 cells");
}

#[test]
fn the_focus_line_shows_ratings_and_skips_breaks() {
    let local = |h, m| {
        chrono::Local
            .with_ymd_and_hms(2026, 10, 31, h, m, 0)
            .unwrap()
    };
    let mut t = TestApp::new();
    for (from, to, activity, focus) in [
        ((9, 0), (9, 30), "Programming", Some(1)),
        ((9, 30), (10, 0), BREAK, None),
        ((10, 0), (10, 30), "Programming", None),
        ((10, 30), (11, 0), MEETING, Some(3)),
    ] {
        let mut s = finished_session(local(from.0, from.1), local(to.0, to.1), focus);
        s.activity = activity.into();
        t.app.today_sessions.insert(0, s);
    }
    let summary = t.app.day_summary();
    let row = focus_row(
        &summary,
        local(9, 0).with_timezone(&Utc),
        local(11, 0).with_timezone(&Utc),
        8,
    );
    let shown = row
        .iter()
        .map(|s| (s.content.as_ref(), s.style.fg))
        .collect::<Vec<_>>();
    let (red, gray, green) = (Some(Color::Red), Some(Color::DarkGray), Some(Color::Green));
    assert_eq!(
        shown,
        [
            ("▀", red),
            ("▀", red),
            (" ", None),
            (" ", None),
            ("▀", gray),
            ("▀", gray),
            ("▀", green),
            ("▀", green),
        ]
    );
}

#[test]
fn half_cells_pack_two_colors_per_cell() {
    let (b, g) = (Some(Color::LightBlue), Some(Color::LightGreen));
    let cells = half_cells(&[b, b, b, g, None, g, b, None, None, None]);
    let shown = cells
        .iter()
        .map(|s| (s.content.as_ref(), s.style.fg, s.style.bg))
        .collect::<Vec<_>>();
    assert_eq!(
        shown,
        [
            ("█", b, None),
            ("▌", b, g),
            ("▐", g, None),
            ("▌", b, None),
            (" ", None, None),
        ]
    );
}

#[test]
fn projects_keep_colors_in_the_order_they_were_first_used() {
    let mut t = TestApp::new();
    for project in ["zeta", "alpha", "zeta", "mid"] {
        t.app.close_active_session();
        t.app
            .start_session("Programming", Some(project.into()), None);
    }
    t.app.refresh_suggestions();
    let color = |t: &TestApp, p| t.app.project_color(Some(p));
    // Not alphabetical: zeta came first.
    assert_eq!(color(&t, "zeta"), theme::PROJECT_PALETTE[0]);
    assert_eq!(color(&t, "alpha"), theme::PROJECT_PALETTE[1]);
    assert_eq!(color(&t, "mid"), theme::PROJECT_PALETTE[2]);
    assert_eq!(t.app.project_color(None), theme::NO_PROJECT);
    // All distinct, and none an activity's.
    let palette = theme::PROJECT_PALETTE;
    let distinct = palette.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(distinct.len(), palette.len());
    for activity in ["Programming", MEETING, BREAK, DISRUPTION] {
        assert!(!palette.contains(&theme::activity_color(activity)));
    }
}

#[test]
fn projects_sharing_a_preferred_color_get_distinct_ones_on_the_same_day() {
    let mut t = TestApp::new();
    // Ranks 0..=19: "p2" and "p18" both prefer the palette's third color.
    t.app.project_order = (0..20).map(|i| format!("p{i}")).collect();
    let p = theme::PROJECT_PALETTE;
    // Alone on a day, each gets its usual color.
    assert_eq!(t.app.project_color(Some("p18")), p[2]);
    let start = chrono::Local::now() - chrono::Duration::hours(2);
    for name in ["p18", "p2"] {
        let mut s = finished_session(start, start + chrono::Duration::minutes(30), None);
        s.project = Some(name.into());
        t.app.today_sessions.push(s);
    }
    // Together, the older keeps it and the other moves on.
    assert_eq!(t.app.project_color(Some("p2")), p[2]);
    assert_eq!(t.app.project_color(Some("p18")), p[3]);
}

#[test]
fn the_share_bar_splits_by_time() {
    let minutes = |m| chrono::Duration::minutes(m);
    let row = share_row(
        &[
            (theme::activity_color("Programming"), minutes(30)),
            (theme::activity_color(BREAK), minutes(10)),
        ],
        4,
    );
    // 8 halves: 6 programming, 2 break.
    let symbols = row.iter().map(|s| s.content.as_ref()).collect::<String>();
    assert_eq!(symbols, "████");
    assert_eq!(row[2].style.fg, Some(Color::LightBlue));
    assert_eq!(row[3].style.fg, Some(Color::LightGreen));
}
