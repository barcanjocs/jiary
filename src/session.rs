use chrono::{DateTime, Utc};

/// Activities that pause work instead of being work: `d` starts a
/// disruption, `b` a break, and `r` ends either by resuming the last
/// session that is neither.
pub const DISRUPTION: &str = "Disruption";
pub const BREAK: &str = "Break";

pub fn is_pause(activity: &str) -> bool {
    activity == DISRUPTION || activity == BREAK
}

#[derive(Debug)]
pub struct Session {
    pub id: i64,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub project: Option<String>,
    pub task: Option<String>,
    pub activity: String,
    pub notes: Option<String>,
    pub focus: Option<i32>,
    pub interruptions: Option<i32>,
}
