//! The day summary: totals over today's sessions. Computed from the
//! in-memory sessions, because it is drawn every frame and draw must never
//! query the db.

use chrono::{DateTime, Duration, Utc};

use crate::session::{self, BREAK, DISRUPTION, Session};

#[derive(Debug, PartialEq)]
pub struct DaySummary {
    /// Time per activity, longest first.
    pub per_activity: Vec<(String, Duration)>,
    /// Time per project (`None`: sessions without one), longest first.
    pub per_project: Vec<(Option<String>, Duration)>,
    /// Mean focus rating over the sessions that have one, weighted by
    /// their length: a 3-hour session counts more than a 20-minute one.
    pub average_focus: Option<f64>,
    pub rated: usize,
    pub interruptions: i32,
    pub disruptions: usize,
    pub disruption_time: Duration,
    pub break_time: Duration,
    /// First start and last end (now, while a session runs).
    pub span: Option<(DateTime<Utc>, DateTime<Utc>)>,
    pub tracked: Duration,
    /// Each session as a block of time, earliest first: the day's
    /// timeline.
    pub blocks: Vec<Block>,
}

/// One session on the day's timeline.
#[derive(Debug, PartialEq)]
pub struct Block {
    pub start: DateTime<Utc>,
    /// Now, while it runs.
    pub end: DateTime<Utc>,
    pub activity: String,
    pub focus: Option<i32>,
}

impl Block {
    /// Whether this session asks for a focus rating (not breaks and
    /// disruptions), rated or not.
    pub fn rateable(&self) -> bool {
        session::asks_focus(&self.activity)
    }
}

impl DaySummary {
    /// Time inside the span not covered by any session.
    pub fn untracked(&self) -> Duration {
        match self.span {
            Some((start, end)) => (end - start - self.tracked).max(Duration::zero()),
            None => Duration::zero(),
        }
    }

    /// Time at work: everything tracked but breaks and disruptions
    /// (meetings count as work).
    pub fn work_time(&self) -> Duration {
        self.tracked - self.break_time - self.disruption_time
    }

    /// The session going on at `at`, if any (the latest started, should
    /// sessions overlap).
    pub fn block_at(&self, at: DateTime<Utc>) -> Option<&Block> {
        self.blocks
            .iter()
            .rev()
            .find(|block| block.start <= at && at < block.end)
    }
}

/// Summarizes `today` (the day's sessions as loaded from the db; or any
/// period's, for the longer views). The
/// running session's row is replaced by `active`, the in-memory copy, which
/// is kept current between reloads (interruptions, notes), and is counted up
/// to `now`.
pub fn summarize<'a>(
    today: impl IntoIterator<Item = &'a Session>,
    active: Option<&'a Session>,
    now: DateTime<Utc>,
) -> DaySummary {
    let sessions = today
        .into_iter()
        .filter(|s| s.ended_at.is_some())
        .chain(active)
        .collect::<Vec<_>>();
    let duration = |s: &Session| s.ended_at.unwrap_or(now) - s.started_at;

    let mut per_activity: Vec<(String, Duration)> = Vec::new();
    let mut per_project: Vec<(Option<String>, Duration)> = Vec::new();
    for session in &sessions {
        add(
            &mut per_activity,
            session.activity.clone(),
            duration(session),
        );
        add(&mut per_project, session.project.clone(), duration(session));
    }
    sort_longest_first(&mut per_activity);
    sort_longest_first(&mut per_project);

    let ratings = sessions
        .iter()
        .filter_map(|s| s.focus.map(|focus| (focus, duration(s))))
        .collect::<Vec<_>>();
    let disruptions = sessions
        .iter()
        .filter(|s| s.activity == DISRUPTION)
        .collect::<Vec<_>>();
    let span = sessions
        .iter()
        .map(|s| s.started_at)
        .min()
        .zip(sessions.iter().map(|s| s.ended_at.unwrap_or(now)).max());

    DaySummary {
        per_activity,
        per_project,
        average_focus: average_focus(&ratings),
        rated: ratings.len(),
        interruptions: sessions.iter().filter_map(|s| s.interruptions).sum(),
        disruptions: disruptions.len(),
        disruption_time: disruptions.iter().map(|s| duration(s)).sum(),
        break_time: sessions
            .iter()
            .filter(|s| s.activity == BREAK)
            .map(|s| duration(s))
            .sum(),
        span,
        tracked: sessions.iter().map(|s| duration(s)).sum(),
        blocks: {
            let mut blocks = sessions
                .iter()
                .map(|s| Block {
                    start: s.started_at,
                    end: s.ended_at.unwrap_or(now),
                    activity: s.activity.clone(),
                    focus: s.focus,
                })
                .collect::<Vec<_>>();
            blocks.sort_by_key(|block| block.start);
            blocks
        },
    }
}

/// The length-weighted mean of (rating, length) pairs; plain mean when the
/// rated sessions took no time at all.
fn average_focus(ratings: &[(i32, Duration)]) -> Option<f64> {
    if ratings.is_empty() {
        return None;
    }
    let seconds = |d: &Duration| d.num_seconds().max(0) as f64;
    let total = ratings.iter().map(|(_, d)| seconds(d)).sum::<f64>();
    Some(if total > 0.0 {
        ratings
            .iter()
            .map(|(f, d)| *f as f64 * seconds(d))
            .sum::<f64>()
            / total
    } else {
        ratings.iter().map(|(f, _)| *f as f64).sum::<f64>() / ratings.len() as f64
    })
}

fn add<K: PartialEq>(totals: &mut Vec<(K, Duration)>, key: K, duration: Duration) {
    match totals.iter_mut().find(|(k, _)| *k == key) {
        Some((_, total)) => *total += duration,
        None => totals.push((key, duration)),
    }
}

/// Longest first; ties keep first-seen order (the sort is stable).
fn sort_longest_first<K>(totals: &mut [(K, Duration)]) {
    totals.sort_by_key(|(_, d)| std::cmp::Reverse(*d));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 31, h, m, 0).unwrap()
    }

    fn session(
        start: DateTime<Utc>,
        end: Option<DateTime<Utc>>,
        activity: &str,
        project: Option<&str>,
    ) -> Session {
        Session {
            id: 0,
            started_at: start,
            ended_at: end,
            project: project.map(str::to_string),
            task: None,
            activity: activity.into(),
            notes: None,
            focus: None,
            interruptions: None,
        }
    }

    #[test]
    fn blocks_follow_the_day_and_tell_what_ran_at_a_time() {
        let active = session(at(11, 0), None, "Reading", None);
        let mut programming = session(at(9, 0), Some(at(10, 0)), "Programming", None);
        programming.focus = Some(3);
        let today = [
            session(at(10, 0), Some(at(10, 30)), BREAK, None),
            programming,
        ];
        let s = summarize(&today, Some(&active), at(12, 0));
        let activities = s
            .blocks
            .iter()
            .map(|b| b.activity.as_str())
            .collect::<Vec<_>>();
        assert_eq!(activities, ["Programming", BREAK, "Reading"]);
        assert_eq!(s.blocks[2].end, at(12, 0), "running: up to now");
        let activity = |t| s.block_at(t).map(|b| b.activity.as_str());
        assert_eq!(activity(at(9, 59)), Some("Programming"));
        assert_eq!(activity(at(10, 0)), Some(BREAK));
        assert_eq!(activity(at(10, 45)), None, "a gap");
        assert_eq!(activity(at(11, 30)), Some("Reading"));
        // With its rating, and whether it could have one.
        assert_eq!(s.block_at(at(9, 30)).unwrap().focus, Some(3));
        assert!(!s.block_at(at(10, 0)).unwrap().rateable());
        assert!(s.block_at(at(11, 30)).unwrap().rateable());
    }

    #[test]
    fn work_is_what_breaks_and_disruptions_leave() {
        let today = [
            session(at(9, 0), Some(at(10, 0)), "Programming", None),
            session(at(10, 0), Some(at(10, 30)), "Meeting", None),
            session(at(10, 30), Some(at(10, 45)), BREAK, None),
            session(at(10, 45), Some(at(10, 55)), DISRUPTION, None),
        ];
        let s = summarize(&today, None, at(12, 0));
        assert_eq!(s.tracked, Duration::minutes(115));
        assert_eq!(s.work_time(), Duration::minutes(90), "meetings count");
    }

    #[test]
    fn empty_day() {
        let s = summarize(&[], None, at(12, 0));
        assert!(s.per_activity.is_empty());
        assert_eq!(s.span, None);
        assert_eq!(s.average_focus, None);
        assert_eq!(s.tracked, Duration::zero());
        assert_eq!(s.untracked(), Duration::zero());
    }

    #[test]
    fn totals_per_activity_and_project_longest_first() {
        let today = [
            session(at(9, 0), Some(at(9, 30)), "Reading", Some("jiary")),
            session(at(9, 30), Some(at(10, 30)), "Programming", Some("jiary")),
            session(at(11, 0), Some(at(11, 45)), "Reading", None),
        ];
        let s = summarize(&today, None, at(12, 0));
        assert_eq!(
            s.per_activity,
            [
                ("Reading".to_string(), Duration::minutes(75)),
                ("Programming".to_string(), Duration::minutes(60)),
            ]
        );
        assert_eq!(
            s.per_project,
            [
                (Some("jiary".to_string()), Duration::minutes(90)),
                (None, Duration::minutes(45)),
            ]
        );
        assert_eq!(s.tracked, Duration::minutes(135));
        // 9:00–11:45 with a 30-minute gap.
        assert_eq!(s.span, Some((at(9, 0), at(11, 45))));
        assert_eq!(s.untracked(), Duration::minutes(30));
    }

    #[test]
    fn running_session_counts_to_now_from_its_in_memory_copy() {
        let mut running = session(at(10, 0), None, "Writing", None);
        let stale_row = Session {
            interruptions: Some(1),
            ..session(at(10, 0), None, "Writing", None)
        };
        running.interruptions = Some(3);
        let today = [
            session(at(9, 0), Some(at(9, 30)), "Reading", None),
            stale_row,
        ];
        let s = summarize(&today, Some(&running), at(11, 0));
        assert_eq!(
            s.per_activity[0],
            ("Writing".to_string(), Duration::hours(1))
        );
        assert_eq!(s.interruptions, 3); // not 1 + 3: the stale row is skipped
        assert_eq!(s.span, Some((at(9, 0), at(11, 0))));
    }

    #[test]
    fn focus_interruptions_and_disruptions() {
        let mut a = session(at(9, 0), Some(at(10, 0)), "Programming", None);
        a.focus = Some(3);
        a.interruptions = Some(2);
        let mut b = session(at(10, 0), Some(at(10, 20)), DISRUPTION, None);
        b.interruptions = Some(1);
        let mut c = session(at(10, 20), Some(at(11, 0)), "Programming", None);
        c.focus = Some(2);
        let d = session(at(11, 0), Some(at(11, 5)), DISRUPTION, None);
        let e = session(at(11, 5), Some(at(11, 50)), BREAK, None);

        let s = summarize(&[a, b, c, d, e], None, at(12, 0));
        // Weighted by length: 60 min of 3, 40 min of 2.
        assert_eq!(s.average_focus, Some(2.6));
        assert_eq!(s.rated, 2);
        assert_eq!(s.interruptions, 3);
        assert_eq!(s.disruptions, 2);
        assert_eq!(s.disruption_time, Duration::minutes(25));
        assert_eq!(s.break_time, Duration::minutes(45));
    }
}
