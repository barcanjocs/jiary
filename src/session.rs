use chrono::{DateTime, Utc};

/// Detours: activities you step into from your work and come back from.
/// `d` starts a disruption, `b` a break, `m` a meeting, and `r` ends any of
/// them by resuming the last session that isn't a detour. A meeting counts
/// as work time in the summary; the others don't.
pub const DISRUPTION: &str = "Disruption";
pub const BREAK: &str = "Break";
pub const MEETING: &str = "Meeting";
pub const DETOURS: [&str; 3] = [DISRUPTION, BREAK, MEETING];

pub fn is_detour(activity: &str) -> bool {
    DETOURS.contains(&activity)
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
