//! The cat beside the active session: its mood, its short reactions to
//! what was just done, and the caption under it.

use std::time::Instant;

use chrono::Utc;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::captions;
use crate::mascot::{self, Mood, Reaction};
use crate::session::{BREAK, DISRUPTION, MEETING};
use crate::theme;

use super::App;
use super::text::wrap_text;

/// After this long in a work session the cat gets drowsy, a nudge to take a
/// break. With the Pomodoro on, its target takes over (the cat has a
/// coffee instead).
pub(super) const DROWSY_AFTER_MINUTES: i64 = 25;

/// Columns the cat takes beside the session text: its canvas plus a gap on
/// each side.
pub(super) const CAT_COLUMNS: u16 = mascot::WIDTH + 2;

/// Whether the cat fits beside at least this much session text in an active
/// section `inner_width` columns wide.
pub(super) fn cat_fits(inner_width: u16) -> bool {
    const MIN_TEXT_WIDTH: u16 = 24;
    inner_width >= MIN_TEXT_WIDTH + CAT_COLUMNS
}

/// Rows kept under the cat for its caption (wrapped to the cat's width),
/// after a blank gap row so the caption doesn't crowd the cat.
const CAPTION_GAP: u16 = 1;

const CAPTION_ROWS: u16 = 2;

/// The cat column's height: the cat, the gap and the caption rows.
pub(super) const CAT_BLOCK_HEIGHT: u16 = mascot::HEIGHT + CAPTION_GAP + CAPTION_ROWS;

/// Splits the active section's interior into the session text and, when it
/// fits, the cat on the right (top-aligned, one column in from the border)
/// with its caption rows below. The cat is left out rather than squeezing
/// the text below `MIN_TEXT_WIDTH` or being cut off vertically; the caption
/// is left out when only the cat fits.
pub(super) fn split_for_cat(inner: Rect) -> (Rect, Option<Rect>, Option<Rect>) {
    if !cat_fits(inner.width) || inner.height < mascot::HEIGHT {
        return (inner, None, None);
    }
    let text = Rect {
        width: inner.width - CAT_COLUMNS,
        ..inner
    };
    let cat = Rect::new(
        inner.right() - mascot::WIDTH - 1,
        inner.y,
        mascot::WIDTH,
        mascot::HEIGHT,
    );
    let caption = (inner.height >= CAT_BLOCK_HEIGHT).then(|| Rect {
        y: cat.bottom() + CAPTION_GAP,
        height: CAPTION_ROWS,
        ..cat
    });
    (text, Some(cat), caption)
}

/// The caption wrapped to the cat's canvas `width`, each row centred under
/// the cat's own columns in `mood` (the scenes aren't centred on the canvas:
/// a laptop or a curled tail would pull it to one side), or as close as the
/// canvas allows for rows too wide to centre there.
pub(super) fn caption_lines(caption: &str, mood: Mood, width: u16) -> Vec<Line<'static>> {
    let (left, right) = mood.columns();
    wrap_text(caption, width as usize)
        .iter()
        .map(|row| row.trim())
        .filter(|row| !row.is_empty())
        .map(|row| {
            let len = row.chars().count() as u16;
            let start = ((left + right + 1).saturating_sub(len) / 2).min(width.saturating_sub(len));
            Line::from(vec![
                Span::raw(" ".repeat(start as usize)),
                Span::styled(row.to_string(), theme::CAPTION),
            ])
        })
        .collect()
}

/// A random seed for picking captions, from std's randomly keyed hasher
/// (each `RandomState` gets new keys). Not the clock: on macOS it only
/// ticks in microseconds, so its nanoseconds are always a multiple of 1000
/// and would pick the same sentence from a list of 4 (or 8) all second.
pub(super) fn fresh_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

impl App {
    /// What the cat does for the current session: idle with none, asleep on
    /// a break, chatting in a meeting, gaming in a disruption, and writing
    /// in a work session: with the Pomodoro on, having a coffee once it
    /// reaches the target; otherwise drowsy once it runs past
    /// `DROWSY_AFTER_MINUTES`.
    pub(super) fn mood(&self) -> Mood {
        let Some(session) = &self.active_session else {
            return Mood::Idle;
        };
        match session.activity.as_str() {
            BREAK => Mood::Sleeping,
            MEETING => Mood::Meeting,
            DISRUPTION => Mood::Gaming,
            _ if let Some(pomodoro) = self.pomodoro() => match pomodoro.reached() {
                true => Mood::BreakEarned,
                false => Mood::Working,
            },
            _ if Utc::now() - session.started_at
                >= chrono::Duration::minutes(DROWSY_AFTER_MINUTES) =>
            {
                Mood::Drowsy
            }
            _ => Mood::Working,
        }
    }

    /// The caption under the cat for the current mood and session
    /// activity, if its list has any.
    pub(super) fn caption(&self) -> Option<&'static str> {
        if matches!(self.current_reaction(), Some((Reaction::Flinch, _))) {
            return Some(captions::FLINCH);
        }
        if self.rating_open() {
            return Some(captions::RATING);
        }
        let activity = self.active_session.as_ref().map(|s| s.activity.as_str());
        captions::caption(self.mood(), activity, self.caption_seed)
    }

    /// Picks another sentence for the caption: a different one when the
    /// list has more than one (a few random tries).
    pub(super) fn resample_caption(&mut self) {
        let before = self.caption();
        for _ in 0..8 {
            self.caption_seed = fresh_seed();
            if self.caption() != before {
                break;
            }
        }
    }

    /// Starts a one-off reaction, replacing any still playing.
    pub(super) fn react(&mut self, reaction: Reaction) {
        self.reaction = Some((reaction, Instant::now()));
    }

    /// The reaction still playing, with how far into it we are.
    pub(super) fn current_reaction(&self) -> Option<(Reaction, u64)> {
        let (reaction, started) = self.reaction?;
        let elapsed = started.elapsed().as_millis() as u64;
        (elapsed < reaction.duration()).then_some((reaction, elapsed))
    }

    /// The cat's current frame: a reaction while one plays, else its mood
    /// (animated by the wall clock, so every app instance stays in step).
    pub(super) fn cat_lines(&self) -> Vec<Line<'static>> {
        match self.current_reaction() {
            Some((reaction, elapsed)) => mascot::reaction_lines(reaction, elapsed),
            None => {
                let millis = Utc::now().timestamp_millis().max(0) as u64;
                mascot::mood_lines(self.mood(), millis)
            }
        }
    }
}
