//! The cat in the active section: pixel art drawn with half-block
//! characters, so each terminal cell shows two pixels stacked vertically,
//! animated by time.
//!
//! What it does follows the session (`Mood`): idling with nothing running,
//! typing on a laptop during work and dozing off over it in a long session,
//! asleep on a break, chatting with a friend in a meeting, gaming when
//! disrupted, and with a coffee once a session reaches the Pomodoro
//! target. Some actions trigger a short one-off `Reaction` on top: a
//! flinch on an interruption, writing the note down in a notebook when one
//! is added, and a mood that matches the focus rating when a session ends.
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

/// What the cat is up to, following the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    /// No session running: sitting around, blinking, flicking its tail.
    Idle,
    /// A work session: typing on a laptop.
    Working,
    /// A long work session: dozing off over the laptop.
    Drowsy,
    /// A work session past the Pomodoro target: a coffee beside it, a
    /// break earned.
    BreakEarned,
    /// A break: asleep, with z's floating up.
    Sleeping,
    /// A meeting: talking with another cat.
    Meeting,
    /// A disruption: playing a video game.
    Gaming,
}

/// A short one-off reaction, shown for its `duration`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    /// An interruption was counted.
    Flinch,
    /// A note was added: the cat writes it down.
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
// fur, `f` muzzle and belly; props: `a` laptop, `k` its screen, `c` code,
// `d` keys, `N` paper, `g` ink and spine, `O` pen, `D` controller, `r` / `G`
// buttons, `M` mug, `b` coffee.
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
    "............CC...CC.....",
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
    "............CC...CC.....",
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
    "............CC...CC.....",
]);

// --- working: typing on a laptop, code lines growing on its screen ----------
// The cat types with its tail (the "arm" leaves the body down by the legs).
// That's deliberate: it reads as a tail and it's cute; keep it.

const TYPE_A: Frame = frame([
    "............C.....C.....",
    "aaaaaaa....CPC...CPC....",
    "akkkkka....CCCCCCCCC....",
    "akcccka...CCWCCCWCCCC...",
    "akkkkka...CCKCPCKCCCC...",
    "akcckka....CCCLLLCCC....",
    "akkkkka.....CCLLLCC.....",
    "aaaaaaa.CCCCCCLLLCC.....",
    "dddddddC....CCLLLCC.....",
    "aaaaaaaaa...CC...CC.....",
]);
const TYPE_B: Frame = frame([
    "............C.....C.....",
    "aaaaaaa....CPC...CPC....",
    "akkkkka....CCCCCCCCC....",
    "akcccka...CCWCCCWCCCC...",
    "akkkkka...CCKCPCKCCCC...",
    "akcccka....CCCLLLCCC....",
    "akkkkka.CCCCCCLLLCC.....",
    "aaaaaaaC....CCLLLCC.....",
    "dddddddd....CCLLLCC.....",
    "aaaaaaaaa...CC...CC.....",
]);
const TYPE_C: Frame = frame([
    "............C.....C.....",
    "aaaaaaa....CPC...CPC....",
    "akkkkka....CCCCCCCCC....",
    "akcckka...CCWCCCWCCCC...",
    "akkkkka...CCKCPCKCCCC...",
    "akcccka....CCCLLLCCC....",
    "akkkkka.....CCLLLCC.....",
    "aaaaaaa.CCCCCCLLLCC.....",
    "dddddddC....CCLLLCC.....",
    "aaaaaaaaa...CC...CC.....",
]);
const TYPE_D: Frame = frame([
    "............C.....C.....",
    "aaaaaaa....CPC...CPC....",
    "akkkkka....CCCCCCCCC....",
    "akcccka...CCWCCCWCCCC...",
    "akkkkka...CCKCPCKCCCC...",
    "akcckka....CCCLLLCCC....",
    "akkkkka.CCCCCCLLLCC.....",
    "aaaaaaaC....CCLLLCC.....",
    "dddddddd....CCLLLCC.....",
    "aaaaaaaaa...CC...CC.....",
]);
const TYPE_BLINK: Frame = frame([
    "............C.....C.....",
    "aaaaaaa....CPC...CPC....",
    "akkkkka....CCCCCCCCC....",
    "akcccka...CCCCCCCCCCC...",
    "akkkkka...CCKCPCKCCCC...",
    "akcckka....CCCLLLCCC....",
    "akkkkka.....CCLLLCC.....",
    "aaaaaaa.CCCCCCLLLCC.....",
    "dddddddC....CCLLLCC.....",
    "aaaaaaaaa...CC...CC.....",
]);

/// Dozing: eyes shut, paw resting on the keys, the screen gone dark.
const DOZE_PIXELS: [&str; 10] = [
    "............C.....C.....",
    "aaaaaaa....CPC...CPC....",
    "akkkkka....CCCCCCCCC....",
    "akkkkka...CCCCCCCCCCC...",
    "akkkkka...CCKCPCKCCCC...",
    "akkkkka....CCCLLLCCC....",
    "akkkkka.....CCLLLCC.....",
    "aaaaaaa.CCCCCCLLLCC.....",
    "dddddddC....CCLLLCC.....",
    "aaaaaaaaa...CC...CC.....",
];
const DOZE: Frame = frame(DOZE_PIXELS);
const DOZE_Z: Frame = Frame {
    pixels: DOZE_PIXELS,
    text: &[(0, 22, "z", theme::CAT_TEXT)],
};

// --- break earned: content, a steaming coffee beside it ---------------------

/// The idle cat, eyes open, with a mug on the left.
const COFFEE_PIXELS: [&str; 10] = [
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCCWCCCWCCC...",
    "..........CCCKCPCKCCC...",
    "...........CCCLLLCCC..C.",
    "...MbbbM....CCLLLCC...C.",
    "...MMMMMMM..CCLLLCC..C..",
    "...MMMMM.M..CCLLLCCCC...",
    "...MMMMMMM..CC...CC.....",
];

/// The same with its eyes closed, savoring it.
const COFFEE_SHUT_PIXELS: [&str; 10] = [
    "............C.....C.....",
    "...........CPC...CPC....",
    "...........CCCCCCCCC....",
    "..........CCCCCCCCCCC...",
    "..........CCCKCPCKCCC...",
    "...........CCCLLLCCC..C.",
    "...MbbbM....CCLLLCC...C.",
    "...MMMMMMM..CCLLLCC..C..",
    "...MMMMM.M..CCLLLCCCC...",
    "...MMMMMMM..CC...CC.....",
];

const COFFEE_A: Frame = Frame {
    pixels: COFFEE_PIXELS,
    text: &[(2, 4, "~", theme::CAT_STEAM)],
};
const COFFEE_B: Frame = Frame {
    pixels: COFFEE_PIXELS,
    text: &[(1, 5, "~", theme::CAT_STEAM), (2, 5, "~", theme::CAT_STEAM)],
};
const COFFEE_SIP: Frame = Frame {
    pixels: COFFEE_SHUT_PIXELS,
    text: &[(1, 4, "~", theme::CAT_STEAM)],
};

// --- noting: writing in an open notebook ------------------------------------

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
    "NNNNNgNNNNN.CC...CC.....",
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
    "NNNNNgNNNNN.CC...CC.....",
]);

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
// Each has its tail on its outer side, clear of the faces in the middle.

const MEET_QUIET: Frame = frame([
    "..F.....F......C.....C..",
    ".FPF...FPF....CPC...CPC.",
    ".FFFFFFFFF....CCCCCCCCC.",
    "FFFFWFFFWFF..CCWCCCWCCCC",
    "FFFFKFPFKFF..CCKCPCKCCCC",
    ".FFFfffFFF....CCCLLLCCC.",
    "F.FFfffFF......CCLLLCC.C",
    "F.FFfffFF......CCLLLCC.C",
    ".FFFfffFF......CCLLLCCC.",
    "..FF...FF......CC...CC..",
]);

const MEET_WE_TALK: Frame = Frame {
    pixels: [
        "..F.....F......C.....C..",
        ".FPF...FPF....CPC...CPC.",
        ".FFFFFFFFF....CCCCCCCCC.",
        "FFFFWFFFWFF..CCWCCCWCCCC",
        "FFFFKFPFKFF..CCKCPCKCCCC",
        ".FFFfffFFF....CCCLKLCCC.",
        "F.FFfffFF......CCLLLCC.C",
        "F.FFfffFF......CCLLLCC.C",
        ".FFFfffFF......CCLLLCCC.",
        "..FF...FF......CC...CC..",
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
        "F.FFfffFF......CCLLLCC.C",
        "F.FFfffFF......CCLLLCC.C",
        ".FFFfffFF......CCLLLCCC.",
        "..FF...FF......CC...CC..",
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
    "............CC...CC.....",
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
    "............CC...CC.....",
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
    "............CC...CC.....",
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
    "............CC...CC.....",
];

const STARTLED: Frame = frame(STARTLED_PIXELS);
const STARTLED_BANG: Frame = Frame {
    pixels: STARTLED_PIXELS,
    text: &[(1, 22, "!", theme::CAT_ALERT)],
};

/// Written down: the notebook with a check mark.
const NOTED_DONE: Frame = Frame {
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
        "............CC...CC.....",
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

/// Paw tapping the keys while the code on screen grows, with a pause and a
/// blink now and then.
const WORK_LOOP: Animation = &[
    (220, &TYPE_A),
    (180, &TYPE_B),
    (220, &TYPE_C),
    (180, &TYPE_D),
    (220, &TYPE_A),
    (180, &TYPE_B),
    (220, &TYPE_C),
    (900, &TYPE_A),
    (150, &TYPE_BLINK),
    (220, &TYPE_C),
    (180, &TYPE_D),
    (600, &TYPE_A),
];

/// Eyes shut over a dark screen, a z, and a brief jolt awake.
const DROWSY_LOOP: Animation = &[
    (1800, &DOZE),
    (900, &DOZE_Z),
    (250, &TYPE_A),
    (1500, &DOZE),
    (900, &DOZE_Z),
];

/// Steam rising off the coffee, the cat closing its eyes now and then.
const COFFEE_LOOP: Animation = &[
    (900, &COFFEE_A),
    (900, &COFFEE_B),
    (900, &COFFEE_A),
    (1200, &COFFEE_SIP),
    (900, &COFFEE_B),
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

// Reactions play once. Most take 1.5 s; noting lasts a little longer so a
// few pen strokes show before the check mark.
const FLINCH: Animation = &[
    (200, &STARTLED_BANG),
    (150, &STARTLED),
    (200, &STARTLED_BANG),
    (950, &STARTLED),
];
const NOTED: Animation = &[
    (350, &WRITE_A),
    (350, &WRITE_B),
    (350, &WRITE_A),
    (350, &WRITE_B),
    (1000, &NOTED_DONE),
];
const HAPPY: Animation = &[(500, &HAPPY_A), (500, &HAPPY_B), (500, &HAPPY_C)];
const CONTENT: Animation = &[(750, &CONTENT_A), (750, &CONTENT_B)];
const DOWNCAST: Animation = &[(1500, &SAD)];

impl Mood {
    /// The canvas columns the cat itself spans in this mood (both cats in a
    /// meeting), leaving out props and its tail, so a caption can centre
    /// under the cat rather than the whole scene.
    pub fn columns(self) -> (u16, u16) {
        match self {
            Mood::Sleeping => (6, 20),
            Mood::Meeting => (0, WIDTH - 1),
            _ => (10, 20),
        }
    }

    fn animation(self) -> Animation {
        match self {
            Mood::Idle => IDLE_LOOP,
            Mood::Working => WORK_LOOP,
            Mood::Drowsy => DROWSY_LOOP,
            Mood::BreakEarned => COFFEE_LOOP,
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

    /// How long the reaction shows before the cat goes back to its mood.
    pub fn duration(self) -> u64 {
        self.animation().iter().map(|(ms, _)| ms).sum()
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
/// `reaction.duration()`).
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
        b'a' => Some(theme::CAT_LAPTOP),
        b'k' => Some(theme::CAT_SCREEN),
        b'c' => Some(theme::CAT_CODE),
        b'd' => Some(theme::CAT_KEYS),
        b'D' => Some(theme::CAT_CONTROLLER),
        b'r' => Some(theme::CAT_BUTTON_A),
        b'G' => Some(theme::CAT_BUTTON_B),
        b'M' => Some(theme::CAT_MUG),
        b'b' => Some(theme::CAT_COFFEE),
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
            Mood::BreakEarned,
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
    fn no_pixel_between_the_legs() {
        // Legs are two 2-pixel columns with a gap; a pixel in the middle
        // reads as something other than a tail.
        for (name, animation) in all_animations() {
            for (_, frame) in animation {
                for row in frame.pixels {
                    assert!(
                        !row.contains("CC.C.CC") && !row.contains("FF.F.FF"),
                        "{name}: {row:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn meeting_cats_have_tails_on_their_outer_sides() {
        for frame in [&MEET_QUIET, &MEET_WE_TALK, &MEET_THEY_TALK] {
            // Friend's tail on the far left, ours on the far right.
            assert_eq!(&frame.pixels[6][..2], "F.");
            assert_eq!(&frame.pixels[6][22..], ".C");
            assert_eq!(&frame.pixels[8][..2], ".F");
            assert_eq!(&frame.pixels[8][22..], "C.");
        }
    }

    #[test]
    fn reactions_are_short_and_noting_shows_a_few_strokes() {
        for reaction in [
            Reaction::Flinch,
            Reaction::Rated(1),
            Reaction::Rated(2),
            Reaction::Rated(3),
        ] {
            assert_eq!(reaction.duration(), 1500, "{reaction:?}");
        }
        assert_eq!(Reaction::Noted.duration(), 2400);
        // It writes before the check mark appears.
        assert!(std::ptr::eq(frame_at(NOTED, 0), &WRITE_A));
        assert!(std::ptr::eq(frame_at(NOTED, 350), &WRITE_B));
        assert!(std::ptr::eq(frame_at(NOTED, 1400), &NOTED_DONE));
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
            &IDLE,
            &TYPE_A,
            &TYPE_B,
            &DOZE_Z,
            &COFFEE_SIP,
            &WRITE_A,
            &GAME_A,
            &STARTLED,
            &HAPPY_A,
            &NOTED_DONE,
        ] {
            assert_eq!(frame.pixels[0].as_bytes()[12], b'C');
        }
    }

    #[test]
    fn animations_step_through_their_frames() {
        assert!(std::ptr::eq(frame_at(IDLE_LOOP, 2800), &IDLE_BLINK));
        assert!(std::ptr::eq(frame_at(WORK_LOOP, 220), &TYPE_B));
        assert!(std::ptr::eq(frame_at(DROWSY_LOOP, 1800), &DOZE_Z));
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
