//! The docs' pictures (`docs/images/`): made-up data, drawn by the real
//! `App::draw` into a test buffer and written out as SVG. Ignored by
//! `cargo test`; regenerate after UI changes with
//!
//! ```sh
//! cargo test screenshot -- --ignored
//! ```
//!
//! The main screen's day is laid out backwards from now, so run it during
//! the day (its first session starts six hours ago and must fall on
//! today's date). The longer views get about two years of history before
//! today, from a fixed seed.

use std::fmt::Write;

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Utc, Weekday};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use crate::db::Db;
use crate::period::Period;
use crate::session::{BREAK, DISRUPTION, MEETING};

use super::{App, Screen};

const WIDTH: u16 = 100;
const HEIGHT: u16 = 40;
const IMAGES: &str = "docs/images";

/// Cell size in pixels, and the margin around the terminal.
const CELL_W: f64 = 9.0;
const CELL_H: f64 = 19.0;
const PAD: f64 = 14.0;

/// The terminal's default colors and its 16 named ones (a dark theme).
const BACKGROUND: &str = "#16181d";
const FOREGROUND: &str = "#d0d4dc";
const NAMED: [&str; 16] = [
    "#282c34", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
    "#5c6370", "#ff7b86", "#b5e890", "#f0d197", "#7cc4ff", "#de9bf0", "#7fd6e0", "#eceff4",
];

/// A session of the made-up day: minutes ago it started and ended (`None`:
/// still running), what it was, its focus, notes and interruptions.
struct Sample {
    from: i64,
    to: Option<i64>,
    activity: &'static str,
    project: Option<&'static str>,
    task: Option<&'static str>,
    focus: Option<i32>,
    notes: &'static [&'static str],
    interruptions: i32,
}

const DAY: &[Sample] = &[
    Sample {
        from: 360,
        to: Some(275),
        activity: "Programming",
        project: Some("jiary"),
        task: Some("week view"),
        focus: Some(3),
        notes: &[
            "days line up on one hour axis",
            "totals reuse the day summary",
        ],
        interruptions: 0,
    },
    Sample {
        from: 275,
        to: Some(262),
        activity: "Disruption",
        project: None,
        task: None,
        focus: None,
        notes: &[],
        interruptions: 0,
    },
    Sample {
        from: 262,
        to: Some(205),
        activity: "Programming",
        project: Some("jiary"),
        task: Some("week view"),
        focus: Some(2),
        notes: &["paging with ←→ works, scrolling next"],
        interruptions: 2,
    },
    Sample {
        from: 205,
        to: Some(175),
        activity: "Meeting",
        project: Some("team"),
        task: Some("planning"),
        focus: Some(2),
        notes: &[],
        interruptions: 0,
    },
    Sample {
        from: 175,
        to: Some(130),
        activity: "Break",
        project: None,
        task: None,
        focus: None,
        notes: &[],
        interruptions: 0,
    },
    Sample {
        from: 130,
        to: Some(70),
        activity: "Reading",
        project: Some("rust book"),
        task: Some("ch. 17, async"),
        focus: Some(3),
        notes: &[],
        interruptions: 0,
    },
    Sample {
        from: 70,
        to: Some(22),
        activity: "Writing",
        project: Some("blog"),
        task: Some("terminal UIs"),
        focus: Some(1),
        notes: &[],
        interruptions: 3,
    },
    Sample {
        from: 22,
        to: None,
        activity: "Programming",
        project: Some("jiary"),
        task: Some("week view"),
        focus: None,
        notes: &["month view: group days by week"],
        interruptions: 1,
    },
];

#[test]
#[ignore = "writes docs/images/main.svg; run with --ignored"]
fn screenshot_of_the_main_screen() {
    let (dir, db) = temp_db("main");
    let now = Utc::now();
    for s in DAY {
        let session = db.create_session(s.activity, s.project, s.task).unwrap();
        db.set_started_at(session.id, now - Duration::minutes(s.from))
            .unwrap();
        for note in s.notes {
            db.append_note(session.id, note).unwrap();
        }
        for _ in 0..s.interruptions {
            db.increment_interruptions(session.id).unwrap();
        }
        if let Some(to) = s.to {
            db.complete_session(session.id, s.focus).unwrap();
            db.set_ended_at(session.id, now - Duration::minutes(to))
                .unwrap();
        }
    }

    let mut app = App::new(db, None);
    app.caption_seed = 0;
    write_svg(&mut app, HEIGHT, "main");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "writes docs/images/{weekly,monthly,yearly}.svg; run with --ignored"]
fn screenshots_of_the_longer_views() {
    let (dir, db) = temp_db("history");
    drop(db);
    write_history(&dir.join("jiary.db"));
    let db = Db::open_at(&dir.join("jiary.db")).unwrap();
    let mut app = App::new(db, None);

    // Complete periods look best: last week, last month and last year
    // (this year's grid would end at today).
    for (period, offset, name) in [
        (Period::Week, -1, "weekly"),
        (Period::Month, -1, "monthly"),
        (Period::Year, -1, "yearly"),
    ] {
        // Tall enough to show the whole view without scrolling.
        app.set_size((WIDTH, HEIGHT));
        app.open_period(period, offset);
        let Screen::Period(view) = &app.screen else {
            panic!("expected a period view");
        };
        let rows = app.period_lines(view, WIDTH - 2).len() as u16 + 4;
        app.set_size((WIDTH, rows));
        app.open_period(period, offset);
        write_svg(&mut app, rows, name);
    }
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A fresh db in a temp dir of its own.
fn temp_db(name: &str) -> (std::path::PathBuf, Db) {
    let dir = std::env::temp_dir().join(format!("jiary-screenshot-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = Db::open_at(&dir.join("jiary.db")).unwrap();
    (dir, db)
}

/// Draws the app's current screen at `WIDTH` × `rows` into
/// `docs/images/<name>.svg`.
fn write_svg(app: &mut App, rows: u16, name: &str) {
    app.set_size((WIDTH, rows));
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, rows)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    std::fs::create_dir_all(IMAGES).unwrap();
    std::fs::write(
        format!("{IMAGES}/{name}.svg"),
        to_svg(terminal.backend().buffer()),
    )
    .unwrap();
}

/// A small deterministic random source (xorshift), so the made-up history
/// is the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number in `lo..=hi`.
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }

    /// True with probability `p`.
    fn chance(&mut self, p: f64) -> bool {
        (self.next() % 10_000) as f64 / 10_000.0 < p
    }
}

/// Projects by what part of the year they run in (months, inclusive),
/// with the tasks each one has.
const PROJECTS: &[(&str, u32, u32, &[&str])] = &[
    (
        "jiary",
        1,
        12,
        &["day summary", "week view", "palette", "release"],
    ),
    ("website", 1, 5, &["landing page", "blog engine"]),
    ("api", 3, 9, &["auth", "rate limits", "docs"]),
    ("rust book", 5, 9, &["ch. 16", "ch. 17"]),
    ("blog", 8, 12, &["terminal UIs", "sqlite tips"]),
    ("infra", 9, 12, &["backups", "ci"]),
];

/// Writes about two years of made-up work days before today straight into
/// the db file, in one transaction (thousands of single inserts would take
/// a while).
fn write_history(path: &std::path::Path) {
    let mut conn = rusqlite::Connection::open(path).unwrap();
    let tx = conn.transaction().unwrap();
    let mut rng = Rng(0x5eed_1234_abcd_9876);
    let today = Local::now().date_naive();
    let mut day = today - Duration::days(800);
    while day < today {
        for s in work_day(&mut rng, day) {
            tx.execute(
                "INSERT INTO sessions (started_at, ended_at, project, task, activity, focus, interruptions)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    s.0.to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
                    s.1.to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
                    s.2,
                    s.3,
                    s.4,
                    s.5,
                    s.6
                ],
            )
            .unwrap();
        }
        day += Duration::days(1);
    }
    tx.commit().unwrap();
}

/// A made-up session: start, end, project, task, activity, focus,
/// interruptions.
type Row = (
    chrono::DateTime<Utc>,
    chrono::DateTime<Utc>,
    Option<&'static str>,
    Option<&'static str>,
    &'static str,
    Option<i32>,
    i32,
);

/// One day of made-up sessions: weekdays from about nine to about six, with
/// a standup, lunch, the odd meeting, break and disruption, and focus that
/// is better midweek and in some weeks than others. Weekends are mostly
/// off, and so are two holidays and the odd sick day.
fn work_day(rng: &mut Rng, day: NaiveDate) -> Vec<Row> {
    let weekend = matches!(day.weekday(), Weekday::Sat | Weekday::Sun);
    let holiday = matches!(day.ordinal(), 95..=101 | 213..=226 | 358..);
    if holiday || rng.chance(0.03) || (weekend && !rng.chance(0.12)) {
        return Vec::new();
    }
    let at = |minutes: i64| {
        Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .unwrap()
            .with_timezone(&Utc)
            + Duration::minutes(minutes)
    };
    let projects = PROJECTS
        .iter()
        .filter(|p| (p.1..=p.2).contains(&day.month()))
        .collect::<Vec<_>>();
    // Some weeks go better than others; Mondays and Fridays a bit worse.
    let week_form = (day.iso_week().week() * 7 + 3) % 5;
    let day_form = match day.weekday() {
        Weekday::Tue | Weekday::Wed | Weekday::Thu => 1,
        _ => 0,
    };
    let form = week_form as i64 + day_form;

    let mut rows = Vec::new();
    let mut now = rng.range(8 * 60 + 20, 9 * 60 + 50);
    let end = if weekend {
        now + rng.range(60, 180)
    } else {
        rng.range(17 * 60, 18 * 60 + 40)
    };
    let mut lunch = false;
    if !weekend && rng.chance(0.75) {
        rows.push((
            at(now),
            at(now + 15),
            Some("team"),
            Some("standup"),
            MEETING,
            None,
            0,
        ));
        now += 15;
    }
    while now < end - 20 {
        if !lunch && now >= 12 * 60 + 15 {
            let length = rng.range(30, 60);
            rows.push((at(now), at(now + length), None, None, BREAK, None, 0));
            now += length;
            lunch = true;
            continue;
        }
        let roll = rng.next() % 100;
        let (activity, project, task, length) = if roll < 8 {
            let length = rng.range(30, 60);
            (MEETING, Some("team"), Some("planning"), length)
        } else if roll < 14 {
            (DISRUPTION, None, None, rng.range(5, 20))
        } else if roll < 24 {
            (BREAK, None, None, rng.range(10, 15))
        } else {
            let (name, _, _, tasks) = projects[rng.range(0, projects.len() as i64 - 1) as usize];
            let task = tasks[rng.range(0, tasks.len() as i64 - 1) as usize];
            let activity = match rng.next() % 10 {
                0..=5 => "Programming",
                6 => "Reading",
                7 | 8 => "Writing",
                _ => "Administration",
            };
            (activity, Some(*name), Some(task), rng.range(25, 110))
        };
        let length = length.min(end - now);
        let work = ![BREAK, DISRUPTION].contains(&activity);
        let focus = if !work || rng.chance(0.08) {
            None
        } else {
            let roll = rng.range(0, 9) + form;
            Some(match roll {
                ..=3 => 1,
                4..=8 => 2,
                _ => 3,
            })
        };
        let interruptions = if activity == MEETING || !work {
            0
        } else {
            rng.range(0, 4 - form.min(3)) as i32
        };
        rows.push((
            at(now),
            at(now + length),
            project,
            task,
            activity,
            focus,
            interruptions,
        ));
        now += length;
        // Now and then, a few minutes nobody tracked.
        if rng.chance(0.15) {
            now += rng.range(3, 15);
        }
    }
    rows
}

/// A color as CSS, `None` for the terminal's default.
fn css(color: Color) -> Option<String> {
    let named = |i: usize| Some(NAMED[i].to_string());
    match color {
        Color::Reset => None,
        Color::Black => named(0),
        Color::Red => named(1),
        Color::Green => named(2),
        Color::Yellow => named(3),
        Color::Blue => named(4),
        Color::Magenta => named(5),
        Color::Cyan => named(6),
        Color::Gray => named(7),
        Color::DarkGray => named(8),
        Color::LightRed => named(9),
        Color::LightGreen => named(10),
        Color::LightYellow => named(11),
        Color::LightBlue => named(12),
        Color::LightMagenta => named(13),
        Color::LightCyan => named(14),
        Color::White => named(15),
        Color::Indexed(i @ 0..=15) => named(i as usize),
        Color::Indexed(i @ 16..=231) => {
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            let i = i - 16;
            Some(rgb(level(i / 36), level(i / 6 % 6), level(i % 6)))
        }
        Color::Indexed(i) => {
            let v = 8 + (i - 232) * 10;
            Some(rgb(v, v, v))
        }
        Color::Rgb(r, g, b) => Some(rgb(r, g, b)),
    }
}

fn rgb(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Box-drawing characters as strokes, for lines that join up whatever the
/// viewer's font: which of the cell's edges each one reaches (up, right,
/// down, left), and whether its corner is rounded.
fn box_edges(symbol: &str) -> Option<([bool; 4], bool)> {
    Some(match symbol {
        "─" => ([false, true, false, true], false),
        "│" => ([true, false, true, false], false),
        "╭" => ([false, true, true, false], true),
        "╮" => ([false, false, true, true], true),
        "╰" => ([true, true, false, false], true),
        "╯" => ([true, false, false, true], true),
        "┌" => ([false, true, true, false], false),
        "┐" => ([false, false, true, true], false),
        "└" => ([true, true, false, false], false),
        "┘" => ([true, false, false, true], false),
        "├" => ([true, true, true, false], false),
        "┤" => ([true, false, true, true], false),
        "┬" => ([false, true, true, true], false),
        "┴" => ([true, true, false, true], false),
        "┼" => ([true, true, true, true], false),
        _ => return None,
    })
}

/// Block characters as rectangles (fractions of the cell: x, y, w, h).
fn block(symbol: &str) -> Option<(f64, f64, f64, f64)> {
    Some(match symbol {
        "█" => (0.0, 0.0, 1.0, 1.0),
        "▀" => (0.0, 0.0, 1.0, 0.5),
        "▄" => (0.0, 0.5, 1.0, 0.5),
        "▌" => (0.0, 0.0, 0.5, 1.0),
        "▐" => (0.5, 0.0, 0.5, 1.0),
        _ => return None,
    })
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn to_svg(buf: &Buffer) -> String {
    let (cols, rows) = (buf.area.width, buf.area.height);
    let (w, h) = (
        cols as f64 * CELL_W + 2.0 * PAD,
        rows as f64 * CELL_H + 2.0 * PAD,
    );
    let mut out = String::new();
    writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">"#
    )
    .unwrap();
    writeln!(
        out,
        r#"<rect width="100%" height="100%" rx="8" fill="{BACKGROUND}"/>
<g shape-rendering="crispEdges" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, 'DejaVu Sans Mono', monospace" font-size="15" fill="{FOREGROUND}">"#
    )
    .unwrap();

    for y in 0..rows {
        let top = PAD + y as f64 * CELL_H;
        // Text goes in runs of cells with the same style, each run
        // stretched to its exact width so columns stay aligned.
        let mut run = String::new();
        let mut run_start = 0;
        let mut run_style = String::new();
        // Each word on its own, at its own column: viewers may collapse
        // runs of spaces, which would pull the words of a run out of place.
        let flush = |out: &mut String, run: &mut String, start: u16, style: &str| {
            let chars = run.chars().collect::<Vec<_>>();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == ' ' {
                    i += 1;
                    continue;
                }
                let end = (i..chars.len())
                    .find(|&j| chars[j] == ' ')
                    .unwrap_or(chars.len());
                let word = chars[i..end].iter().collect::<String>();
                writeln!(
                    out,
                    r#"<text x="{}" y="{}" textLength="{}" lengthAdjust="spacingAndGlyphs"{style}>{}</text>"#,
                    PAD + (start as usize + i) as f64 * CELL_W,
                    top + CELL_H * 0.74,
                    (end - i) as f64 * CELL_W,
                    escape(&word)
                )
                .unwrap();
                i = end;
            }
            run.clear();
        };
        for x in 0..cols {
            let cell = &buf[(x, y)];
            let left = PAD + x as f64 * CELL_W;
            let (mut fg, mut bg) = (css(cell.fg), css(cell.bg));
            if cell.modifier.contains(Modifier::REVERSED) {
                (fg, bg) = (
                    bg.or(Some(BACKGROUND.into())),
                    fg.or(Some(FOREGROUND.into())),
                );
            }
            if let Some(bg) = &bg {
                writeln!(
                    out,
                    r#"<rect x="{left}" y="{top}" width="{CELL_W}" height="{CELL_H}" fill="{bg}"/>"#
                )
                .unwrap();
            }
            let color = fg.clone().unwrap_or(FOREGROUND.into());
            let symbol = cell.symbol();
            let mut style = String::new();
            if let Some(fg) = &fg {
                write!(style, r#" fill="{fg}""#).unwrap();
            }
            if cell.modifier.contains(Modifier::BOLD) {
                style.push_str(r#" font-weight="bold""#);
            }
            if cell.modifier.contains(Modifier::ITALIC) {
                style.push_str(r#" font-style="italic""#);
            }
            if cell.modifier.contains(Modifier::DIM) {
                style.push_str(r#" opacity="0.6""#);
            }

            let shape = block(symbol).is_some() || box_edges(symbol).is_some();
            if shape || style != run_style {
                flush(&mut out, &mut run, run_start, &run_style);
                run_start = x;
                run_style = style;
            }
            if let Some((bx, by, bw, bh)) = block(symbol) {
                writeln!(
                    out,
                    r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{color}"/>"#,
                    left + bx * CELL_W,
                    top + by * CELL_H,
                    bw * CELL_W,
                    bh * CELL_H
                )
                .unwrap();
                run_start = x + 1;
            } else if let Some((edges, rounded)) = box_edges(symbol) {
                let (cx, cy) = (left + CELL_W / 2.0, top + CELL_H / 2.0);
                let ends = [
                    (cx, top),
                    (left + CELL_W, cy),
                    (cx, top + CELL_H),
                    (left, cy),
                ];
                let reached = (0..4).filter(|&i| edges[i]).collect::<Vec<_>>();
                let path = if rounded {
                    let ((ax, ay), (bx, by)) = (ends[reached[0]], ends[reached[1]]);
                    format!("M{ax},{ay} Q{cx},{cy} {bx},{by}")
                } else {
                    reached
                        .iter()
                        .map(|&i| format!("M{cx},{cy} L{},{}", ends[i].0, ends[i].1))
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                writeln!(
                    out,
                    r#"<path d="{path}" stroke="{color}" stroke-width="1.2" fill="none"/>"#
                )
                .unwrap();
                run_start = x + 1;
            } else {
                run.push_str(symbol);
            }
        }
        flush(&mut out, &mut run, run_start, &run_style);
    }
    out.push_str("</g>\n</svg>\n");
    out
}
