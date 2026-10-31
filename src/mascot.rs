//! The cat in the active section: pixel art drawn with half-block
//! characters, so each terminal cell shows two pixels stacked vertically,
//! animated by time.
//!
//! What it does follows the session (`Mood`): idling with nothing running,
//! writing in a notebook during work and getting drowsy in a long session,
//! asleep on a break, chatting with a friend in a meeting, gaming when
//! disrupted. Some actions trigger a short one-off `Reaction` on top: a
//! flinch on an interruption, a check mark on a note, and a mood that
//! matches the focus rating when a session ends.
//!
//! A frame is a grid of palette characters (two pixel rows per terminal
//! row) plus optional colored text over transparent cells, for things that
//! read better as characters than as pixels (`z`, `♥`, `!`). Every frame
//! has the same size, and the cat's body sits in the same columns whenever
//! it is alone, so switching scenes never makes it jump.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::theme;

/// Canvas size in terminal cells.
pub const WIDTH: u16 = 24;
pub const HEIGHT: u16 = 5;

/// How long a reaction shows before the cat goes back to its mood.
pub const REACTION_MS: u64 = 1500;

/// What the cat is up to, following the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    /// No session running: sitting around, blinking, flicking its tail.
    Idle,
    /// A work session: writing in a notebook.
    Working,
    /// A long work session: nodding off over the notebook.
    Drowsy,
    /// A break: asleep, with z's floating up.
    Sleeping,
    /// A meeting: talking with another cat.
    Meeting,
    /// A disruption: playing a video game.
    Gaming,
}

/// A short one-off reaction, shown for `REACTION_MS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    /// An interruption was counted.
    Flinch,
    /// A note was added.
    Noted,
    /// A session ended with this focus rating (1–3).
    Rated(i32),
}

struct Frame {
    pixels: [&'static str; HEIGHT as usize * 2],
    /// (row, column, text, color) in cells, drawn over the pixels.
    text: &'static [(usize, usize, &'static str, Color)],
}

const fn frame(pixels: [&'static str; HEIGHT as usize * 2]) -> Frame {
    Frame { pixels, text: &[] }
}

// Palette: `.` transparent; the cat: `C` fur, `L` muzzle and belly, `P`
// inner ears and nose, `K` eyes and mouth, `W` eye shine; its friend: `F`
// fur, `f` muzzle and belly; props: `N` paper, `g` ink and spine, `O` pen,
// `D` controller, `r` / `G` buttons.
//
// The cat's body is on columns 10–20 in every scene where it is alone.

// --- the cat on its own --------------------------------------------------------

const IDLE_PIXELS: [&str; 10] = [
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCCWCCCWCCC...",
    "..........CCCKCPCKCCC...",
    "...........CCCLLLCCC..C.",
    "............CCLLLCC...C.",
    "............CCLLLCC..C..",
    "............CCLLLCCCC...",
    "............CC.C.CC.....",
];

/// Eyes closed: blinking, or content.
const SHUT_PIXELS: [&str; 10] = [
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCCCCCCCCCC...",
    "..........CCCKCPCKCCC...",
    "...........CCCLLLCCC..C.",
    "............CCLLLCC...C.",
    "............CCLLLCC..C..",
    "............CCLLLCCCC...",
    "............CC.C.CC.....",
];

const IDLE: Frame = frame(IDLE_PIXELS);
const IDLE_BLINK: Frame = frame(SHUT_PIXELS);
const IDLE_TAIL: Frame = frame([
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCCWCCCWCCC...",
    "..........CCCKCPCKCCC...",
    "...........CCCLLLCCC....",
    "............CCLLLCC.....",
    "............CCLLLCC...CC",
    "............CCLLLCCCCC..",
    "............CC.C.CC.....",
]);

// --- working: a paw on a pen over an open notebook -------------------------

const WRITE_A_PIXELS: [&str; 10] = [
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCWCCCWCCCC...",
    ".....O....CCKCPCKCCCC...",
    "......O....CCCLLLCCC....",
    ".......OCCCCCCLLLCC.....",
    "NNNNNgNNNNN.CCLLLCC.....",
    "NgggNgNNggN.CCLLLCC.....",
    "NNNNNgNNNNN.CC.C.CC.....",
];

const WRITE_SHUT_PIXELS: [&str; 10] = [
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCCCCCCCCCC...",
    ".....O....CCKCPCKCCCC...",
    "......O....CCCLLLCCC....",
    ".......OCCCCCCLLLCC.....",
    "NNNNNgNNNNN.CCLLLCC.....",
    "NgggNgNNggN.CCLLLCC.....",
    "NNNNNgNNNNN.CC.C.CC.....",
];

const WRITE_A: Frame = frame(WRITE_A_PIXELS);
const WRITE_B: Frame = frame([
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCWCCCWCCCC...",
    "....O.....CCKCPCKCCCC...",
    ".....O.....CCCLLLCCC....",
    "......OCCCCCCCLLLCC.....",
    "NNNNNgNNNNN.CCLLLCC.....",
    "NgggNgNgggN.CCLLLCC.....",
    "NNNNNgNNNNN.CC.C.CC.....",
]);
const WRITE_BLINK: Frame = frame(WRITE_SHUT_PIXELS);
const DROWSY_Z: Frame = Frame {
    pixels: WRITE_SHUT_PIXELS,
    text: &[(0, 22, "z", theme::CAT_TEXT)],
};

// --- sleeping: curled into a loaf, z's drifting up --------------------------

const SLEEP_PIXELS: [&str; 10] = [
    "........................",
    "........................",
    "........................",
    "..........C.....C.......",
    ".........CPC...CPC......",
    "........CCCCCCCCCCC.....",
    ".......CCCKKCPCKKCCC.CC.",
    ".......CCCCCLLLCCCCCC..C",
    "......CCCCCCCCCCCCCCC..C",
    ".......CCCCCCCCCCCCCCCC.",
];

const SLEEP_A: Frame = Frame {
    pixels: SLEEP_PIXELS,
    text: &[(1, 19, "z", theme::CAT_TEXT)],
};
const SLEEP_B: Frame = Frame {
    pixels: SLEEP_PIXELS,
    text: &[(1, 19, "z", theme::CAT_TEXT), (0, 21, "Z", theme::CAT_TEXT)],
};
const SLEEP_C: Frame = Frame {
    pixels: SLEEP_PIXELS,
    text: &[(0, 21, "Z", theme::CAT_TEXT)],
};

// --- meeting: a friend (left) and the cat (right) take turns talking --------

const MEET_QUIET: Frame = frame([
    "..F.....F......C.....C..",
    ".FPF...FPF....CPC...CPC.",
    ".FFFFFFFFF....CCCCCCCCC.",
    "FFFFWFFFWFF..CCWCCCWCCCC",
    "FFFFKFPFKFF..CCKCPCKCCCC",
    ".FFFfffFFF....CCCLLLCCC.",
    "..FFfffFF......CCLLLCC..",
    "..FFfffFF......CCLLLCC..",
    "..FFfffFF......CCLLLCC..",
    "..FF.F.FF......CC.C.CC..",
]);

const MEET_WE_TALK: Frame = Frame {
    pixels: [
        "..F.....F......C.....C..",
        ".FPF...FPF....CPC...CPC.",
        ".FFFFFFFFF....CCCCCCCCC.",
        "FFFFWFFFWFF..CCWCCCWCCCC",
        "FFFFKFPFKFF..CCKCPCKCCCC",
        ".FFFfffFFF....CCCLKLCCC.",
        "..FFfffFF......CCLLLCC..",
        "..FFfffFF......CCLLLCC..",
        "..FFfffFF......CCLLLCC..",
        "..FF.F.FF......CC.C.CC..",
    ],
    text: &[(0, 13, "…", theme::CAT_TEXT)],
};

const MEET_THEY_TALK: Frame = Frame {
    pixels: [
        "..F.....F......C.....C..",
        ".FPF...FPF....CPC...CPC.",
        ".FFFFFFFFF....CCCCCCCCC.",
        "FFFFWFFFWFF..CCWCCCWCCCC",
        "FFFFKFPFKFF..CCKCPCKCCCC",
        ".FFFfKfFFF....CCCLLLCCC.",
        "..FFfffFF......CCLLLCC..",
        "..FFfffFF......CCLLLCC..",
        "..FFfffFF......CCLLLCC..",
        "..FF.F.FF......CC.C.CC..",
    ],
    text: &[(0, 10, "…", theme::CAT_TEXT)],
};

// --- gaming: holding a controller, buttons flashing ------------------------

const GAME_A: Frame = frame([
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCCWCCCWCCC...",
    "..........CCCKCPCKCCC...",
    "...........CCCLLLCCC..C.",
    "...........DDDDDDDDDD.C.",
    "..........DDrDDDDDGDDDC.",
    "...........DDDDDDDDDDC..",
    "............CC.C.CC.....",
]);

const GAME_B: Frame = frame([
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCCWCCCWCCC...",
    "..........CCCKCPCKCCC...",
    "...........CCCLLLCCC..C.",
    "...........DDDDDDDDDD.C.",
    "..........DDDDDDDDGDDDC.",
    "...........DDDDDDDDDDC..",
    "............CC.C.CC.....",
]);

const GAME_C: Frame = frame([
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCCWCCCWCCC...",
    "..........CCCKCPCKCCC...",
    "...........CCCLLLCCC..C.",
    "...........DDDDDDDDDD.C.",
    "..........DDrDDDDDDDDDC.",
    "...........DDDDDDDDDDC..",
    "............CC.C.CC.....",
]);

// --- reactions ---------------------------------------------------------------

/// Wide eyes and the tail straight up.
const STARTLED_PIXELS: [&str; 10] = [
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCCKCCCKCCC...",
    "..........CCCKCPCKCCC...",
    "...........CCCLLLCCC..C.",
    "............CCLLLCC...C.",
    "............CCLLLCC...C.",
    "............CCLLLCCCCC..",
    "............CC.C.CC.....",
];

const STARTLED: Frame = frame(STARTLED_PIXELS);
const STARTLED_BANG: Frame = Frame {
    pixels: STARTLED_PIXELS,
    text: &[(1, 22, "!", theme::CAT_ALERT)],
};

const NOTED_A: Frame = Frame {
    pixels: WRITE_SHUT_PIXELS,
    text: &[(0, 22, "✓", theme::CAT_CHECK)],
};
const NOTED_B: Frame = Frame {
    pixels: WRITE_A_PIXELS,
    text: &[(0, 22, "✓", theme::CAT_CHECK)],
};

/// Focus 3: eyes closed happily, hearts floating up.
const HAPPY_A: Frame = Frame {
    pixels: SHUT_PIXELS,
    text: &[(1, 22, "♥", theme::CAT_HEART)],
};
const HAPPY_B: Frame = Frame {
    pixels: SHUT_PIXELS,
    text: &[
        (0, 22, "♥", theme::CAT_HEART),
        (1, 9, "♥", theme::CAT_HEART),
    ],
};
const HAPPY_C: Frame = Frame {
    pixels: SHUT_PIXELS,
    text: &[(0, 9, "♥", theme::CAT_HEART)],
};

/// Focus 2: a content little tune.
const CONTENT_A: Frame = Frame {
    pixels: SHUT_PIXELS,
    text: &[(1, 22, "♪", theme::CAT_MUSIC)],
};
const CONTENT_B: Frame = Frame {
    pixels: IDLE_PIXELS,
    text: &[(0, 22, "♪", theme::CAT_MUSIC)],
};

/// Focus 1: ears drooped, eyes down, tail low.
const SAD: Frame = Frame {
    pixels: [
        "........................",
        "..........CC.......CC...",
        "..........CPCCCCCCCPC...",
        "..........CCCCCCCCCCC...",
        "..........CCCKCPCKCCC...",
        "...........CCCLLLCCC....",
        "............CCLLLCC...C.",
        "............CCLLLCC...CC",
        "............CCLLLCCCCC..",
        "............CC.C.CC.....",
    ],
    text: &[(0, 22, "…", theme::CAT_TEXT)],
};

/// (milliseconds, frame) steps.
type Animation = &'static [(u64, &'static Frame)];

/// Mostly still, with blinks and tail flicks, so it reads as alive without
/// pulling attention from the timer next to it.
const IDLE_LOOP: Animation = &[
    (2800, &IDLE),
    (150, &IDLE_BLINK),
    (1200, &IDLE),
    (500, &IDLE_TAIL),
    (300, &IDLE),
    (500, &IDLE_TAIL),
    (2400, &IDLE),
    (150, &IDLE_BLINK),
    (120, &IDLE),
    (150, &IDLE_BLINK),
];

/// The pen goes back and forth, with a pause and a blink now and then.
const WORK_LOOP: Animation = &[
    (400, &WRITE_A),
    (400, &WRITE_B),
    (400, &WRITE_A),
    (400, &WRITE_B),
    (400, &WRITE_A),
    (400, &WRITE_B),
    (900, &WRITE_A),
    (150, &WRITE_BLINK),
    (600, &WRITE_A),
];

/// Eyes mostly shut over the notebook, a z, and a brief jolt awake.
const DROWSY_LOOP: Animation = &[
    (1800, &WRITE_BLINK),
    (900, &DROWSY_Z),
    (250, &WRITE_A),
    (1500, &WRITE_BLINK),
    (900, &DROWSY_Z),
];

/// z, then z Z, then Z: the z's float up and away.
const SLEEP_LOOP: Animation = &[(900, &SLEEP_A), (900, &SLEEP_B), (900, &SLEEP_C)];

/// The cat talks, a pause, the friend talks, a pause; mouths open and
/// close while talking.
const MEET_LOOP: Animation = &[
    (300, &MEET_WE_TALK),
    (200, &MEET_QUIET),
    (300, &MEET_WE_TALK),
    (200, &MEET_QUIET),
    (300, &MEET_WE_TALK),
    (700, &MEET_QUIET),
    (300, &MEET_THEY_TALK),
    (200, &MEET_QUIET),
    (300, &MEET_THEY_TALK),
    (200, &MEET_QUIET),
    (300, &MEET_THEY_TALK),
    (700, &MEET_QUIET),
];

/// Buttons mashed in a quick, uneven rhythm.
const GAME_LOOP: Animation = &[
    (250, &GAME_A),
    (200, &GAME_B),
    (250, &GAME_A),
    (300, &GAME_C),
    (200, &GAME_A),
    (400, &GAME_B),
];

// Reactions play once over `REACTION_MS`.
const FLINCH: Animation = &[
    (200, &STARTLED_BANG),
    (150, &STARTLED),
    (200, &STARTLED_BANG),
    (950, &STARTLED),
];
const NOTED: Animation = &[(250, &NOTED_A), (1250, &NOTED_B)];
const HAPPY: Animation = &[(500, &HAPPY_A), (500, &HAPPY_B), (500, &HAPPY_C)];
const CONTENT: Animation = &[(750, &CONTENT_A), (750, &CONTENT_B)];
const DOWNCAST: Animation = &[(1500, &SAD)];

impl Mood {
    fn animation(self) -> Animation {
        match self {
            Mood::Idle => IDLE_LOOP,
            Mood::Working => WORK_LOOP,
            Mood::Drowsy => DROWSY_LOOP,
            Mood::Sleeping => SLEEP_LOOP,
            Mood::Meeting => MEET_LOOP,
            Mood::Gaming => GAME_LOOP,
        }
    }
}

impl Reaction {
    fn animation(self) -> Animation {
        match self {
            Reaction::Flinch => FLINCH,
            Reaction::Noted => NOTED,
            Reaction::Rated(3) => HAPPY,
            Reaction::Rated(2) => CONTENT,
            Reaction::Rated(_) => DOWNCAST,
        }
    }
}

/// The frame of `animation` shown `millis` milliseconds in, looping.
fn frame_at(animation: Animation, millis: u64) -> &'static Frame {
    let period: u64 = animation.iter().map(|(ms, _)| ms).sum();
    let mut t = millis % period;
    for (ms, frame) in animation {
        if t < *ms {
            return frame;
        }
        t -= ms;
    }
    animation[0].1
}

/// The cat in `mood`, `millis` milliseconds into the animation (any clock
/// works: only the position within the loop matters).
pub fn mood_lines(mood: Mood, millis: u64) -> Vec<Line<'static>> {
    render(frame_at(mood.animation(), millis))
}

/// The cat `millis` milliseconds into `reaction` (meant for the first
/// `REACTION_MS`).
pub fn reaction_lines(reaction: Reaction, millis: u64) -> Vec<Line<'static>> {
    render(frame_at(reaction.animation(), millis))
}

fn color(pixel: u8) -> Option<Color> {
    match pixel {
        b'C' => Some(theme::CAT_FUR),
        b'L' => Some(theme::CAT_LIGHT),
        b'P' => Some(theme::CAT_PINK),
        b'K' => Some(theme::CAT_EYE),
        b'W' => Some(theme::CAT_SHINE),
        b'F' => Some(theme::CAT_FRIEND),
        b'f' => Some(theme::CAT_FRIEND_LIGHT),
        b'N' => Some(theme::CAT_PAPER),
        b'g' => Some(theme::CAT_INK),
        b'O' => Some(theme::CAT_PEN),
        b'D' => Some(theme::CAT_CONTROLLER),
        b'r' => Some(theme::CAT_BUTTON_A),
        b'G' => Some(theme::CAT_BUTTON_B),
        _ => None,
    }
}

/// Two pixel rows per terminal row: `▀` takes the top pixel's color as
/// foreground and the bottom one's as background; a transparent half leaves
/// the terminal's own background showing. Text is then drawn over its
/// cells.
fn render(frame: &Frame) -> Vec<Line<'static>> {
    let mut rows: Vec<Vec<Span<'static>>> = frame
        .pixels
        .chunks(2)
        .map(|pair| {
            let (top, bottom) = (pair[0].as_bytes(), pair[1].as_bytes());
            top.iter()
                .zip(bottom)
                .map(|(&t, &b)| match (color(t), color(b)) {
                    (None, None) => Span::raw(" "),
                    (Some(t), None) => Span::styled("▀", Style::new().fg(t)),
                    (None, Some(b)) => Span::styled("▄", Style::new().fg(b)),
                    (Some(t), Some(b)) if t == b => Span::styled("█", Style::new().fg(t)),
                    (Some(t), Some(b)) => Span::styled("▀", Style::new().fg(t).bg(b)),
                })
                .collect()
        })
        .collect();
    for &(row, col, text, color) in frame.text {
        rows[row][col] = Span::styled(text, Style::new().fg(color));
    }
    rows.into_iter().map(Line::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_animations() -> Vec<(String, Animation)> {
        let moods = [
            Mood::Idle,
            Mood::Working,
            Mood::Drowsy,
            Mood::Sleeping,
            Mood::Meeting,
            Mood::Gaming,
        ]
        .map(|m| (format!("{m:?}"), m.animation()));
        let reactions = [
            Reaction::Flinch,
            Reaction::Noted,
            Reaction::Rated(1),
            Reaction::Rated(2),
            Reaction::Rated(3),
        ]
        .map(|r| (format!("{r:?}"), r.animation()));
        moods.into_iter().chain(reactions).collect()
    }

    #[test]
    fn every_frame_fills_the_canvas_with_palette_pixels() {
        for (name, animation) in all_animations() {
            for (_, frame) in animation {
                for row in frame.pixels {
                    assert_eq!(row.len(), WIDTH as usize, "{name}: {row:?}");
                    assert!(
                        row.bytes().all(|p| p == b'.' || color(p).is_some()),
                        "{name}: {row:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn text_sits_on_transparent_cells_inside_the_canvas() {
        for (name, animation) in all_animations() {
            for (_, frame) in animation {
                for &(row, col, text, _) in frame.text {
                    assert!(row < HEIGHT as usize && col < WIDTH as usize, "{name}");
                    assert_eq!(text.chars().count(), 1, "{name}: one cell each");
                    let (top, bottom) = (frame.pixels[row * 2], frame.pixels[row * 2 + 1]);
                    assert_eq!(top.as_bytes()[col], b'.', "{name}: text over pixels");
                    assert_eq!(bottom.as_bytes()[col], b'.', "{name}: text over pixels");
                }
            }
        }
    }

    #[test]
    fn reactions_last_exactly_the_reaction_time() {
        for reaction in [
            Reaction::Flinch,
            Reaction::Noted,
            Reaction::Rated(1),
            Reaction::Rated(2),
            Reaction::Rated(3),
        ] {
            let total: u64 = reaction.animation().iter().map(|(ms, _)| ms).sum();
            assert_eq!(total, REACTION_MS, "{reaction:?}");
        }
    }

    #[test]
    fn half_blocks_combine_two_pixels_per_cell() {
        let lines = render(&IDLE);
        assert_eq!(lines.len(), HEIGHT as usize);
        assert!(lines.iter().all(|l| l.spans.len() == WIDTH as usize));
        let cell = |row: usize, col: usize| lines[row].spans[col].clone();
        // Ear: only the bottom pixel → lower half in fur.
        assert_eq!(cell(0, 11).content, "▄");
        assert_eq!(cell(0, 11).style, Style::new().fg(theme::CAT_FUR));
        // Ear tip over inner ear → upper half in fur on a pink background.
        assert_eq!(cell(0, 12).content, "▀");
        assert_eq!(
            cell(0, 12).style,
            Style::new().fg(theme::CAT_FUR).bg(theme::CAT_PINK)
        );
        // Two fur pixels → full block; transparent pixels stay blank.
        assert_eq!(cell(1, 11).content, "█");
        assert_eq!(cell(0, 0).content, " ");
    }

    #[test]
    fn text_is_drawn_over_the_pixels_in_its_color() {
        let lines = render(&HAPPY_B);
        assert_eq!(lines[0].spans[22].content, "♥");
        assert_eq!(lines[1].spans[9].content, "♥");
        assert_eq!(lines[0].spans[22].style, Style::new().fg(theme::CAT_HEART));
    }

    #[test]
    fn the_cat_alone_keeps_its_place_across_scenes() {
        // The left ear tip is at pixel (0, 12) whenever the cat is alone, so
        // a reaction never makes it jump.
        for frame in [
            &IDLE, &WRITE_A, &DROWSY_Z, &GAME_A, &STARTLED, &HAPPY_A, &NOTED_B,
        ] {
            assert_eq!(frame.pixels[0].as_bytes()[12], b'C');
        }
    }

    #[test]
    fn animations_step_through_their_frames() {
        assert!(std::ptr::eq(frame_at(IDLE_LOOP, 2800), &IDLE_BLINK));
        assert!(std::ptr::eq(frame_at(WORK_LOOP, 400), &WRITE_B));
        assert!(std::ptr::eq(frame_at(SLEEP_LOOP, 1000), &SLEEP_B));
        assert!(std::ptr::eq(frame_at(MEET_LOOP, 2000), &MEET_THEY_TALK));
        assert!(std::ptr::eq(frame_at(HAPPY, 600), &HAPPY_B));
        assert!(std::ptr::eq(frame_at(FLINCH, 0), &STARTLED_BANG));
        // Moods loop forever.
        let period: u64 = SLEEP_LOOP.iter().map(|(ms, _)| ms).sum();
        assert!(std::ptr::eq(
            frame_at(SLEEP_LOOP, period * 1000 + 1000),
            &SLEEP_B
        ));
    }
}
