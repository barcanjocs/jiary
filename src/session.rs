use chrono::{DateTime, Utc};

#[derive(Debug)]
pub struct Session {
    pub id: i64,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub project: Option<String>,
    pub task: Option<String>,
    pub activity: String,
    pub notes: Option<String>,
    pub outcome: Option<String>,
    pub focus: Option<i32>,
    pub interruptions: Option<i32>,
}
