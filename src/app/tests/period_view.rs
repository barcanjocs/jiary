//! `/weekly`, `/monthly` and `/yearly`.

use super::*;

// --- weeks and months -------------------------------------------------

fn period_view(t: &TestApp) -> &PeriodView {
    match &t.app.screen {
        Screen::Period(view) => view,
        _ => panic!("no period on screen"),
    }
}

#[test]
fn weekly_shows_this_week_day_by_day() {
    let mut t = TestApp::new();
    t.app.start_session("Reading", Some("alpha".into()), None);
    backdate(&mut t, 30);
    t.key(KeyCode::Char('e'));
    open(&mut t, "weekly");

    let today = chrono::Local::now().date_naive();
    let (first, last) = Period::Week.days(0, today);
    let view = period_view(&t);
    assert_eq!((view.first, view.last, view.offset), (first, last, 0));
    let buf = render(&t.app);
    assert!(line(&buf, 1).contains(&Period::Week.title(first, last)));
    // Monday to Sunday, one row each, from the box's first row.
    for (i, day) in first.iter_days().take(7).enumerate() {
        let label = day.format("%a %-d").to_string();
        assert!(
            line_from(&buf, 2 + i as u16, 1).starts_with(&label),
            "{label}"
        );
    }
    // Today: the accent label, its timeline in the activity's color,
    // its time.
    // (By position: the header line names today too.)
    let y = 2 + (today - first).num_days() as u16;
    assert_eq!(buf[(1, y)].fg, Color::Cyan);
    assert!(line(&buf, y).contains("30m"));
    assert!((11..67).any(|x| buf[(x, y)].fg == Color::LightBlue));
    // Then the totals.
    assert!(line(&buf, row_with(&buf, "Total ")).contains("tracked 30m · work 30m · 1 day"));
    row_with(&buf, "Activity  ");
    row_with(&buf, "Project   ");
    assert_eq!(
        line(&buf, RENDER_HEIGHT - 1),
        "[←→] previous/next  [w/m/y] week/month/year  [↑↓] scroll  [Esc] back"
    );
}

#[test]
fn the_focus_grid_colors_each_day_by_its_focus() {
    let day = |d| chrono::NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
    let at = |d, h| {
        chrono::Local
            .with_ymd_and_hms(2026, 10, d, h, 0, 0)
            .unwrap()
    };
    let (first, last, today) = (
        day(26),
        chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        day(29),
    );
    let summary_of = |sessions: Vec<Session>| summarize(&sessions, None, Utc::now());
    let days = vec![
        (
            day(26),
            summary_of(vec![finished_session(at(26, 9), at(26, 11), Some(3))]),
        ),
        (
            day(27),
            summary_of(vec![finished_session(at(27, 9), at(27, 11), None)]),
        ),
        (day(28), summary_of(Vec::new())),
        (
            day(29),
            summary_of(vec![finished_session(at(29, 9), at(29, 10), Some(1))]),
        ),
    ];
    let grid = focus_grid(&days, first, last, today);
    // A header (the week's Monday), then Mon–Sun.
    assert_eq!(grid.len(), 8);
    assert_eq!(grid[0][1].content, "26");
    // Every row as wide as the others, so the block centers evenly.
    let widths = grid
        .iter()
        .map(item_width)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(widths.len(), 1, "{widths:?}");
    let cell = |weekday: usize| &grid[1 + weekday][1];
    let fg = |weekday: usize| cell(weekday).style.fg;
    assert_eq!(
        (cell(0).content.as_ref(), fg(0)),
        ("██", Some(theme::FOCUS_RAMP[8]))
    );
    assert_eq!(
        (cell(1).content.as_ref(), fg(1)),
        ("██", Some(theme::UNRATED))
    );
    assert_eq!(
        (cell(2).content.as_ref(), fg(2)),
        ("░░", Some(theme::UNRATED))
    );
    assert_eq!(
        (cell(3).content.as_ref(), fg(3)),
        ("██", Some(theme::FOCUS_RAMP[0]))
    );
    // Friday on: still to come.
    assert_eq!(cell(4).content, "  ");
    // Today's weekday in the accent.
    assert_eq!(grid[4][0].style, theme::TITLE);
    assert_eq!(grid[1][0].style, theme::DIMMED);
    // The legend: 1, the ramp, 3.
    let legend = focus_legend()
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();
    assert_eq!(legend, "1 █████████ 3");
}

#[test]
fn yearly_has_a_row_per_month_then_its_totals_then_the_focus_grid() {
    use chrono::Datelike;
    let mut t = TestApp::new();
    t.app.start_session("Reading", None, None);
    backdate(&mut t, 60);
    t.key(KeyCode::Char('e'));
    t.key(KeyCode::Char('3'));
    open(&mut t, "yearly");
    let today = chrono::Local::now().date_naive();
    let view = period_view(&t);
    assert_eq!(view.period, Period::Year);
    let lines = t.app.period_lines(view, 78);
    let text = |l: &Line| {
        l.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    };
    // It opens on the months, like the other views open on their days.
    assert!(text(&lines[0]).starts_with("Jan "), "{}", text(&lines[0]));
    // Under the focus text, as in the other views: the month names over
    // the grid, then four rows of weekday pairs.
    let focus = lines
        .iter()
        .position(|l| text(l).starts_with("Focus"))
        .unwrap();
    assert!(
        text(&lines[focus + 2]).contains("Jan"),
        "{}",
        text(&lines[focus + 2])
    );
    for (row, weekday) in ["Mo Tu", "We Th", "Fr Sa", "Su"].iter().enumerate() {
        assert!(
            text(&lines[focus + 3 + row])
                .trim_start()
                .starts_with(weekday)
        );
    }
    // A row per month, this one with its hour and rating.
    let months = lines
        .iter()
        .map(text)
        .filter(|l| {
            [
                "Jan ", "Feb ", "Mar ", "Apr ", "May ", "Jun ", "Jul ", "Aug ", "Sep ", "Oct ",
                "Nov ", "Dec ",
            ]
            .iter()
            .any(|m| l.starts_with(m))
        })
        .collect::<Vec<_>>();
    assert_eq!(months.len(), 12, "{months:#?}");
    let this_month = &months[today.month0() as usize];
    assert!(this_month.contains("1h 0m 3.0"), "{this_month}");
    // Paging by year; y from another view.
    t.key(KeyCode::Left);
    assert_eq!(period_view(&t).first, Period::Year.days(-1, today).0);
    t.key(KeyCode::Char('w'));
    t.key(KeyCode::Char('y'));
    assert_eq!(period_view(&t).period, Period::Year);
}

#[test]
fn the_year_grid_stacks_two_days_per_cell() {
    let day = |m, d| chrono::NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let at = |d, h| chrono::Local.with_ymd_and_hms(2026, 1, d, h, 0, 0).unwrap();
    let summary_of = |sessions: Vec<Session>| summarize(&sessions, None, Utc::now());
    // 2026 starts on a Thursday: its first week's column holds Thu 1
    // (bottom of the Wed/Thu row) to Sun 4.
    let days = vec![
        (
            day(1, 1),
            summary_of(vec![finished_session(at(1, 9), at(1, 10), Some(3))]),
        ),
        (
            day(1, 2),
            summary_of(vec![finished_session(at(2, 9), at(2, 10), None)]),
        ),
        (day(1, 3), summary_of(Vec::new())),
        (
            day(1, 5),
            summary_of(vec![finished_session(at(5, 9), at(5, 10), Some(1))]),
        ),
        (
            day(1, 31),
            summary_of(vec![finished_session(at(31, 9), at(31, 10), Some(2))]),
        ),
        (
            day(2, 1),
            summary_of(vec![finished_session(
                chrono::Local.with_ymd_and_hms(2026, 2, 1, 9, 0, 0).unwrap(),
                chrono::Local
                    .with_ymd_and_hms(2026, 2, 1, 10, 0, 0)
                    .unwrap(),
                Some(3),
            )]),
        ),
    ];
    let grid = year_focus_grid(&days, day(1, 1), day(12, 31), day(2, 1));
    assert_eq!(grid.len(), 5, "month names, then four weekday-pair rows");
    assert!(grid[0][0].content.starts_with("       Jan"));
    // Each row names both its days, top then bottom.
    let labels = grid[1..]
        .iter()
        .map(|row| row[0].content.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(labels, ["Mo Tu ", "We Th ", "Fr Sa ", "Su    "]);
    let cell = |row: usize, week: usize| &grid[row][1 + week];
    let colors = |row, week| {
        (
            cell(row, week).content.as_ref(),
            cell(row, week).style.fg,
            cell(row, week).style.bg,
        )
    };
    // Week 0: Wed (before the year) blank over Thu 1 (rated 3).
    assert_eq!(colors(2, 0), ("▄", Some(theme::FOCUS_RAMP[8]), None));
    // Fri 2 (unrated) over Sat 3 (nothing tracked).
    assert_eq!(
        colors(3, 0),
        ("▀", Some(theme::UNRATED), Some(theme::EMPTY_DAY))
    );
    // Week 1: Mon 5 (rated 1) over Tue 6 (nothing recorded).
    assert_eq!(colors(1, 1), ("▀", Some(theme::FOCUS_RAMP[0]), None));
    // January's last week, Mon 26 to Sat 31 (rated 2), is its fifth
    // column; Sun Feb 1 (rated 3) starts February's block next to it.
    assert_eq!(colors(3, 4), ("▄", Some(theme::focus_gradient(2.0)), None));
    assert_eq!(colors(4, 4), (" ", None, None));
    assert_eq!(colors(4, 5), ("▀", Some(theme::FOCUS_RAMP[8]), None));
    // So each month's name sits centered over its own block: January's and
    // February's are five columns wide, from columns 0 and 5.
    assert!(
        grid[0][0].content.starts_with("       Jan  Feb"),
        "{:?}",
        grid[0][0].content
    );
    // May 2026 ends on a Sunday: no split week sets it apart from June, so
    // a blank column does (May's five weeks, the gap, then June).
    let header = grid[0][0].content.as_ref();
    let may = header.find("May").unwrap();
    assert_eq!(header.find("Jun").unwrap(), may + 6, "{header}");
    let gap = (may - 1) - 6 + 5; // May's name is centered over its 5 columns
    for row in &grid[1..] {
        assert_eq!(row[1 + gap].content, " ");
    }
    // 53 weeks fit in 80 columns.
    assert!(item_width(&grid[1]) <= 78);
}

#[test]
fn arrows_page_through_weeks_but_not_into_the_future() {
    let mut t = TestApp::new();
    open(&mut t, "weekly");
    t.key(KeyCode::Right); // already the current week
    assert_eq!(period_view(&t).offset, 0);
    t.key(KeyCode::Left);
    t.key(KeyCode::Char('h'));
    let view = period_view(&t);
    assert_eq!(view.offset, -2);
    let today = chrono::Local::now().date_naive();
    assert_eq!(view.first, Period::Week.days(-2, today).0);
    let buf = render(&t.app);
    row_with(&buf, "Nothing tracked this week.");
    t.key(KeyCode::Char('l'));
    assert_eq!(period_view(&t).offset, -1);
    t.key(KeyCode::Esc);
    assert!(matches!(t.app.screen, Screen::Main));
}

#[test]
fn monthly_has_a_row_per_day_in_weeks_and_scrolls() {
    let mut t = TestApp::new();
    open(&mut t, "monthly");
    let today = chrono::Local::now().date_naive();
    let (first, last) = Period::Month.days(0, today);
    let view = period_view(&t);
    assert_eq!(view.period, Period::Month);
    let lines = t.app.period_lines(view, 78);
    let labels = lines
        .iter()
        .filter_map(|l| l.spans.first())
        .map(|span| span.content.trim().to_string())
        .collect::<Vec<_>>();
    for day in first.iter_days().take_while(|d| *d <= last) {
        let label = day.format("%a %-d").to_string();
        assert_eq!(labels.iter().filter(|l| **l == label).count(), 1, "{label}");
    }
    // More than fits: it scrolls, and not past its end.
    t.key(KeyCode::End);
    let end = period_view(&t).scroll;
    assert!(end > 0);
    t.key(KeyCode::Down);
    assert_eq!(period_view(&t).scroll, end);
    t.key(KeyCode::Home);
    assert_eq!(period_view(&t).scroll, 0);
    // w switches to this week; q goes back too.
    t.key(KeyCode::Char('w'));
    assert_eq!(period_view(&t).period, Period::Week);
    t.key(KeyCode::Char('q'));
    assert!(matches!(t.app.screen, Screen::Main));
}

#[test]
fn a_months_total_keeps_its_focus_in_the_column() {
    // A whole month of round-the-clock work: the longest total a row shows.
    let summary = crate::summary::DaySummary {
        per_activity: Vec::new(),
        per_project: Vec::new(),
        average_focus: Some(2.5),
        rated: 1,
        interruptions: 0,
        disruptions: 0,
        disruption_time: chrono::Duration::zero(),
        break_time: chrono::Duration::zero(),
        span: None,
        tracked: chrono::Duration::hours(744) + chrono::Duration::minutes(59),
        blocks: Vec::new(),
    };
    let total = day_total(&summary);
    assert_eq!(item_width(&total), DAY_TOTAL_WIDTH);
    let text: String = total.iter().map(|span| span.content.as_ref()).collect();
    assert_eq!(text, " 744h 59m 2.5");
}
