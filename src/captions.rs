//! Captions under the cat: short lines in faded italic, picked from the
//! lists below by what the cat is doing and, during work, by the session's
//! activity.
//!
//! To add a caption, add a sentence to the matching list. A mood (or
//! activity) whose list is empty shows no caption. One sentence is picked
//! at random whenever the session changes and stays put until the next
//! change, so captions don't flicker.

use crate::mascot::Mood;

/// Captions for work sessions of a specific activity, matched by name
/// (ignoring case). When an activity has sentences here they are used
/// instead of `WORKING`, e.g. `("Programming", &["..."])`.
const BY_ACTIVITY: &[(&str, &[&str])] = &[];

/// No session running.
const IDLE: &[&str] = &["what's next, boss?"];
/// A work session (unless its activity has its own list above).
const WORKING: &[&str] = &[
    "working on world domination...",
    "it worked on my machine...",
    "and now the nuke codes...",
    "don't be evil, don't be evil, don't be evil",
];
/// A work session past the drowsy threshold.
const DROWSY: &[&str] = &["where is that pomodoro timer?"];
/// A work session past the Pomodoro target.
const BREAK_EARNED: &[&str] = &[
    "break's earned, if you want one",
    "a coffee? no rush",
    "good place for a stretch",
];
/// A break.
const SLEEPING: &[&str] = &["rest is productive", "stress doesn't ship"];
/// A meeting.
const MEETING: &[&str] = &["teams, am I right?"];
/// A disruption.
const GAMING: &[&str] = &["probably footabling with Jimmy"];

/// While the cat flinches at an interruption (`i`); a new sentence from
/// the mood's list follows.
pub const FLINCH: &str = "oups!";

/// While the session that just ended can be rated (a few seconds), the cat
/// asks for it instead.
pub const RATING: &str = "how was your focus?";

/// The sentences for `mood`; for a work session, `activity`'s own list
/// when it has one.
fn sentences(mood: Mood, activity: Option<&str>) -> &'static [&'static str] {
    if mood == Mood::Working
        && let Some(activity) = activity
        && let Some((_, list)) = BY_ACTIVITY
            .iter()
            .find(|(name, list)| name.eq_ignore_ascii_case(activity) && !list.is_empty())
    {
        return list;
    }
    match mood {
        Mood::Idle => IDLE,
        Mood::Working => WORKING,
        Mood::Drowsy => DROWSY,
        Mood::BreakEarned => BREAK_EARNED,
        Mood::Sleeping => SLEEPING,
        Mood::Meeting => MEETING,
        Mood::Gaming => GAMING,
    }
}

/// The caption to show, if the list has any: the one `seed` picks. The same
/// seed always picks the same sentence, so a caption only changes when the
/// seed does (on a session change) or the list does (a new mood).
pub fn caption(mood: Mood, activity: Option<&str>, seed: u64) -> Option<&'static str> {
    let list = sentences(mood, activity);
    (!list.is_empty()).then(|| list[(seed % list.len() as u64) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_caption_is_a_non_empty_single_line() {
        let lists = [
            IDLE,
            WORKING,
            DROWSY,
            BREAK_EARNED,
            SLEEPING,
            MEETING,
            GAMING,
        ]
        .into_iter()
        .chain(BY_ACTIVITY.iter().map(|(_, list)| *list));
        for list in lists {
            for sentence in list {
                assert!(!sentence.trim().is_empty());
                assert!(!sentence.contains('\n'), "{sentence:?}");
            }
        }
    }

    #[test]
    fn empty_lists_show_no_caption() {
        assert_eq!(
            sentences(Mood::Idle, None).is_empty(),
            caption(Mood::Idle, None, 7).is_none()
        );
    }

    #[test]
    fn the_seed_picks_a_sentence_from_the_moods_list() {
        for seed in 0..10 {
            let picked = caption(Mood::Working, Some("Anything"), seed).unwrap();
            assert!(WORKING.contains(&picked));
            // Stable: the same seed picks the same sentence.
            assert_eq!(caption(Mood::Working, Some("Anything"), seed), Some(picked));
        }
    }
}
