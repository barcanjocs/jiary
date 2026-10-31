//! The README's picture of the main screen: a made-up day, drawn by the
//! real `App::draw` into a test buffer and written out as an SVG. Ignored
//! by `cargo test`; regenerate after UI changes with
//!
//! ```sh
//! cargo test screenshot -- --ignored
//! ```
//!
//! The day is laid out backwards from now, so run it during the day (its
//! first session starts six hours ago and must fall on today's date).

use std::fmt::Write;

use chrono::{Duration, Utc};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use crate::db::Db;

use super::App;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 40;
const OUTPUT: &str = "docs/screenshot.svg";

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
#[ignore = "writes docs/screenshot.svg; run with --ignored"]
fn screenshot() {
    let dir = std::env::temp_dir().join(format!("jiary-screenshot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = Db::open_at(&dir.join("jiary.db")).unwrap();
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
    app.set_size((WIDTH, HEIGHT));
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let svg = to_svg(terminal.backend().buffer());
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::write(OUTPUT, svg).unwrap();
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
        let flush = |out: &mut String, run: &mut String, start: u16, style: &str| {
            if !run.trim().is_empty() {
                let len = run.chars().count() as f64 * CELL_W;
                writeln!(
                    out,
                    r#"<text x="{}" y="{}" textLength="{len}" lengthAdjust="spacingAndGlyphs" xml:space="preserve"{style}>{}</text>"#,
                    PAD + start as f64 * CELL_W,
                    top + CELL_H * 0.74,
                    escape(run)
                )
                .unwrap();
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
