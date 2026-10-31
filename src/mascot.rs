//! The cat in the active section: pixel art drawn with half-block
//! characters, so each terminal cell shows two pixels stacked vertically,
//! animated by wall-clock time.
//!
//! Sprites are grids of palette characters, two pixel rows per terminal
//! row. Frames of an animation must share the sprite size so switching
//! between them never shifts anything.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::theme;

/// Sprite size in terminal cells.
pub const WIDTH: u16 = 14;
pub const HEIGHT: u16 = 5;

type Sprite = [&'static str; HEIGHT as usize * 2];

// Palette: `.` transparent, `C` fur, `L` muzzle and belly, `P` inner ears
// and nose, `K` eyes, `W` eye shine. A slim kitten: round head, narrow
// body, thin upright tail.
const IDLE: Sprite = [
    "..C.....C.....",
    ".CPC...CPC....",
    ".CCCCCCCCC....",
    "CCCWCCCWCCC...",
    "CCCKCPCKCCC...",
    ".CCCLLLCCC..C.",
    "..CCLLLCC...C.",
    "..CCLLLCC..C..",
    "..CCLLLCCCC...",
    "..CC.C.CC.....",
];

const BLINK: Sprite = [
    "..C.....C.....",
    ".CPC...CPC....",
    ".CCCCCCCCC....",
    "CCCCCCCCCCC...",
    "CCCKCPCKCCC...",
    ".CCCLLLCCC..C.",
    "..CCLLLCC...C.",
    "..CCLLLCC..C..",
    "..CCLLLCCCC...",
    "..CC.C.CC.....",
];

const TAIL_DOWN: Sprite = [
    "..C.....C.....",
    ".CPC...CPC....",
    ".CCCCCCCCC....",
    "CCCWCCCWCCC...",
    "CCCKCPCKCCC...",
    ".CCCLLLCCC....",
    "..CCLLLCC.....",
    "..CCLLLCC...CC",
    "..CCLLLCCCCC..",
    "..CC.C.CC.....",
];

/// The idle loop: (milliseconds, frame) steps, repeated. Mostly still, with
/// a quick blink and an occasional tail flick, so it reads as alive without
/// pulling attention from the timer next to it.
const IDLE_LOOP: &[(u64, &Sprite)] = &[
    (2800, &IDLE),
    (150, &BLINK),
    (1200, &IDLE),
    (500, &TAIL_DOWN),
    (300, &IDLE),
    (500, &TAIL_DOWN),
    (2400, &IDLE),
    (150, &BLINK),
    (120, &IDLE),
    (150, &BLINK),
];

/// The idle frame shown `millis` milliseconds into the animation.
fn idle_frame(millis: u64) -> &'static Sprite {
    let period: u64 = IDLE_LOOP.iter().map(|(ms, _)| ms).sum();
    let mut t = millis % period;
    for (ms, frame) in IDLE_LOOP {
        if t < *ms {
            return frame;
        }
        t -= ms;
    }
    &IDLE
}

/// The cat as it should look `millis` milliseconds into the animation
/// (any monotonic clock works: only the position within the loop matters).
pub fn lines(millis: u64) -> Vec<Line<'static>> {
    render(idle_frame(millis))
}

fn color(pixel: u8) -> Option<Color> {
    match pixel {
        b'C' => Some(theme::CAT_FUR),
        b'L' => Some(theme::CAT_LIGHT),
        b'P' => Some(theme::CAT_PINK),
        b'K' => Some(theme::CAT_EYE),
        b'W' => Some(theme::CAT_SHINE),
        _ => None,
    }
}

/// Two pixel rows per terminal row: `▀` takes the top pixel's color as
/// foreground and the bottom one's as background; a transparent half leaves
/// the terminal's own background showing.
fn render(sprite: &Sprite) -> Vec<Line<'static>> {
    sprite
        .chunks(2)
        .map(|pair| {
            let (top, bottom) = (pair[0].as_bytes(), pair[1].as_bytes());
            let cells = top
                .iter()
                .zip(bottom)
                .map(|(&t, &b)| match (color(t), color(b)) {
                    (None, None) => Span::raw(" "),
                    (Some(t), None) => Span::styled("▀", Style::new().fg(t)),
                    (None, Some(b)) => Span::styled("▄", Style::new().fg(b)),
                    (Some(t), Some(b)) if t == b => Span::styled("█", Style::new().fg(t)),
                    (Some(t), Some(b)) => Span::styled("▀", Style::new().fg(t).bg(b)),
                });
            Line::from(cells.collect::<Vec<_>>())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_is_the_sprite_size_and_uses_the_palette() {
        for (_, frame) in IDLE_LOOP {
            for row in frame.iter() {
                assert_eq!(row.len(), WIDTH as usize, "{row:?}");
                assert!(
                    row.bytes().all(|p| p == b'.' || color(p).is_some()),
                    "{row:?}"
                );
            }
        }
    }

    #[test]
    fn half_blocks_combine_two_pixels_per_cell() {
        let lines = render(&IDLE);
        assert_eq!(lines.len(), HEIGHT as usize);
        let cell = |row: usize, col: usize| lines[row].spans[col].clone();
        // Ear: only the bottom pixel → lower half in fur.
        assert_eq!(cell(0, 1).content, "▄");
        assert_eq!(cell(0, 1).style, Style::new().fg(theme::CAT_FUR));
        // Ear tip over inner ear → upper half in fur on a pink background.
        assert_eq!(cell(0, 2).content, "▀");
        assert_eq!(
            cell(0, 2).style,
            Style::new().fg(theme::CAT_FUR).bg(theme::CAT_PINK)
        );
        // Fur over the eye's shine.
        assert_eq!(cell(1, 3).content, "▀");
        assert_eq!(
            cell(1, 3).style,
            Style::new().fg(theme::CAT_FUR).bg(theme::CAT_SHINE)
        );
        // Two fur pixels → full block; transparent pixels stay blank.
        assert_eq!(cell(1, 1).content, "█");
        assert_eq!(cell(0, 0).content, " ");
    }

    #[test]
    fn idle_loop_blinks_and_flicks_its_tail() {
        assert_eq!(idle_frame(0), &IDLE);
        assert_eq!(idle_frame(2800), &BLINK);
        assert_eq!(idle_frame(2950), &IDLE);
        assert_eq!(idle_frame(4150), &TAIL_DOWN);
        // Loops forever.
        let period: u64 = IDLE_LOOP.iter().map(|(ms, _)| ms).sum();
        assert_eq!(idle_frame(period * 1000 + 2800), &BLINK);
    }
}
