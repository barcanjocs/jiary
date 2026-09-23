use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};

use crate::session::Session;

pub struct Db {
    conn: Connection,
}

#[derive(Debug)]
pub enum DbError {
    NoDataDir,
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::NoDataDir => write!(f, "Could not determine application data directory."),
            DbError::Io(e) => write!(f, "Could not open Jiary database: {e}"),
            DbError::Sqlite(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl std::error::Error for DbError {}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    project TEXT,
    task TEXT,
    activity TEXT NOT NULL,
    notes TEXT,
    outcome TEXT,
    focus INTEGER,
    interruptions INTEGER
);

-- SQLite UNIQUE constraints ignore NULL values, so a trigger is used to
-- enforce that at most one session (ended_at IS NULL) is active.
CREATE TRIGGER IF NOT EXISTS enforce_single_active_session
BEFORE INSERT ON sessions
WHEN NEW.ended_at IS NULL
     AND (SELECT COUNT(*) FROM sessions WHERE ended_at IS NULL) > 0
BEGIN
    SELECT RAISE(ABORT, 'another session is already active');
END;
";

impl Db {
    pub fn open() -> Result<Self, DbError> {
        let dir = data_dir().ok_or(DbError::NoDataDir)?;
        std::fs::create_dir_all(&dir).map_err(DbError::Io)?;
        Self::open_at(&dir.join("jiary.db"))
    }

    pub(crate) fn open_at(path: &std::path::Path) -> Result<Self, DbError> {
        let conn = Connection::open(path).map_err(DbError::Sqlite)?;
        conn.execute_batch(SCHEMA).map_err(DbError::Sqlite)?;
        Ok(Self { conn })
    }

    pub fn create_session(
        &self,
        activity: &str,
        project: Option<&str>,
        task: Option<&str>,
    ) -> Result<i64, DbError> {
        // Explicit '+00:00' offset rather than 'Z': SQLite's date functions
        // parse the former but silently ignore the latter.
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
        self.conn
            .execute(
                "INSERT INTO sessions (started_at, project, task, activity)
                 VALUES (?1, ?2, ?3, ?4)",
                params![now, project, task, activity],
            )
            .map_err(DbError::Sqlite)?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_active_session(&self) -> Result<Option<Session>, DbError> {
        let result = self.conn.query_row(
            // ORDER BY + LIMIT: if duplicate active rows exist (legacy data or
            // external edits), deterministically pick the most recent one.
            "SELECT id, started_at, ended_at, project, task, activity, notes, outcome, focus, interruptions
             FROM sessions WHERE ended_at IS NULL
             ORDER BY started_at DESC LIMIT 1",
            [],
            |row| {
                Ok(Session {
                    id: row.get(0)?,
                    started_at: parse_timestamp(row.get(1)?)?,
                    ended_at: row.get::<_, Option<String>>(2)?.map(parse_timestamp).transpose()?,
                    project: row.get(3)?,
                    task: row.get(4)?,
                    activity: row.get(5)?,
notes: row.get(6)?,
                    outcome: row.get(7)?,
                    focus: row.get(8)?,
                    interruptions: row.get(9)?,
                })
            },
        );

        match result {
            Ok(session) => Ok(Some(session)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }

    pub fn complete_session(
        &self,
        id: i64,
        outcome: Option<&str>,
        focus: Option<i32>,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
        self.conn
            .execute(
                "UPDATE sessions SET ended_at = ?1, outcome = ?2, focus = ?3 WHERE id = ?4 AND ended_at IS NULL",
                params![now, outcome, focus, id],
            )
            .map_err(DbError::Sqlite)?;
        Ok(())
    }

    pub fn sessions_for_today(&self) -> Result<Vec<Session>, DbError> {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, started_at, ended_at, project, task, activity, notes, outcome, focus, interruptions
                 FROM sessions
                 WHERE date(started_at, 'localtime') = ?1
                 ORDER BY started_at DESC",
            )
            .map_err(DbError::Sqlite)?;
        let rows = stmt
            .query_map(params![today], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    started_at: parse_timestamp(row.get(1)?)?,
                    ended_at: row
                        .get::<_, Option<String>>(2)?
                        .map(parse_timestamp)
                        .transpose()?,
                    project: row.get(3)?,
                    task: row.get(4)?,
                    activity: row.get(5)?,
                    notes: row.get(6)?,
                    outcome: row.get(7)?,
                    focus: row.get(8)?,
                    interruptions: row.get(9)?,
                })
            })
            .map_err(DbError::Sqlite)?;
        let sessions = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::Sqlite)?;
        Ok(sessions)
    }

    pub fn recent_notes(&self, project: &str, task: &str) -> Result<Vec<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT COALESCE(notes, outcome) FROM sessions
         WHERE project = ?1 AND task = ?2 AND ended_at IS NOT NULL
           AND (notes IS NOT NULL OR outcome IS NOT NULL)
         ORDER BY started_at DESC LIMIT 5",
            )
            .map_err(DbError::Sqlite)?;

        let rows = stmt
            .query_map(params![project, task], |row| row.get::<_, String>(0))
            .map_err(DbError::Sqlite)?;

        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Sqlite)
    }

    pub fn append_note(&self, id: i64, note: &str) -> Result<(), DbError> {
        self.conn
            .execute(
                "UPDATE sessions SET notes = COALESCE(notes, '') || ?1 || char(10) WHERE id = ?2",
                params![note, id],
            )
            .map_err(DbError::Sqlite)?;
        Ok(())
    }

    pub fn distinct_projects(&self) -> Result<Vec<String>, DbError> {
        let mut stmt = self.conn.prepare(
        "SELECT DISTINCT project FROM sessions WHERE project IS NOT NULL AND project != '' ORDER BY project",
    ).map_err(DbError::Sqlite)?;

        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(DbError::Sqlite)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Sqlite)
    }

    pub fn distinct_tasks(&self) -> Result<Vec<String>, DbError> {
        let mut stmt = self.conn.prepare(
        "SELECT DISTINCT task FROM sessions WHERE task IS NOT NULL AND task != '' ORDER BY task",
    ).map_err(DbError::Sqlite)?;

        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(DbError::Sqlite)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Sqlite)
    }

    pub fn increment_interruptions(&self, id: i64) -> Result<(), DbError> {
        self.conn
        .execute(
            "UPDATE sessions SET interruptions = COALESCE(interruptions, 0) + 1 WHERE id = ?1 AND ended_at IS NULL",
            params![id],
        )
        .map_err(DbError::Sqlite)?;
        Ok(())
    }

    pub fn tasks_for_project(&self, project: &str) -> Result<Vec<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT task FROM sessions
         WHERE project = ?1 AND task IS NOT NULL AND task != ''
         ORDER BY task",
            )
            .map_err(DbError::Sqlite)?;

        let rows = stmt
            .query_map(params![project], |row| row.get::<_, String>(0))
            .map_err(DbError::Sqlite)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Sqlite)
    }

    pub fn latest_non_disruption(
        &self,
    ) -> Result<Option<(String, Option<String>, Option<String>)>, DbError> {
        let result = self.conn.query_row(
            "SELECT activity, project, task FROM sessions
         WHERE activity != 'Disruption' AND ended_at IS NOT NULL
         ORDER BY started_at DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        );

        match result {
            Ok(combo) => Ok(Some(combo)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }
}

fn parse_timestamp(s: String) -> Result<DateTime<Utc>, rusqlite::Error> {
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::InvalidPath(std::path::PathBuf::from(format!(
                "Bad timestamp in database: {s} ({e})"
            )))
        })
}

fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("jiary"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TmpDb {
        db: Db,
        dir: PathBuf,
    }

    impl TmpDb {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "jiary-test-{}-{}",
                std::process::id(),
                TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let db = Db::open_at(&dir.join("jiary.db")).unwrap();
            Self { db, dir }
        }
    }

    impl Drop for TmpDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn new_sessions_are_stored_with_explicit_utc_offset() {
        let tmp = TmpDb::new();
        let id = tmp
            .db
            .create_session("Programming", Some("proj"), None)
            .unwrap();
        let started_at: String = tmp
            .db
            .conn
            .query_row("SELECT started_at FROM sessions WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            started_at.ends_with("+00:00"),
            "expected '+00:00' suffix, got {started_at}"
        );
    }

    #[test]
    fn get_active_session_picks_most_recent_when_duplicates_exist() {
        let tmp = TmpDb::new();
        // Bypass the trigger to simulate legacy duplicate active rows.
        tmp.db
            .conn
            .execute("DROP TRIGGER enforce_single_active_session", [])
            .unwrap();
        for (ts, activity) in [
            ("2026-01-01T10:00:00+00:00", "Reading"),
            ("2026-01-02T10:00:00+00:00", "Writing"),
        ] {
            tmp.db
                .conn
                .execute(
                    "INSERT INTO sessions (started_at, activity) VALUES (?1, ?2)",
                    params![ts, activity],
                )
                .unwrap();
        }
        let active = tmp.db.get_active_session().unwrap().unwrap();
        assert_eq!(active.activity, "Writing");
    }

    #[test]
    fn only_one_active_session_is_allowed() {
        let tmp = TmpDb::new();
        tmp.db.create_session("Programming", None, None).unwrap();
        let err = tmp.db.create_session("Reading", None, None).unwrap_err();
        assert!(
            err.to_string()
                .contains("another session is already active"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn sessions_for_today_includes_current_session() {
        let tmp = TmpDb::new();
        tmp.db.create_session("Programming", None, None).unwrap();
        let sessions = tmp.db.sessions_for_today().unwrap();
        assert_eq!(sessions.len(), 1);
    }
}
