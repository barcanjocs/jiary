use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, Row, params};

use crate::session::Session;

/// The (activity, project, task) of a session, as returned by
/// [`Db::latest_non_disruption`] for resuming work.
type SessionCombo = (String, Option<String>, Option<String>);

pub struct Db {
    conn: Connection,
}

#[derive(Debug)]
pub enum DbError {
    NoDataDir,
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    /// A timestamp stored in the database is not valid RFC 3339.
    BadTimestamp {
        value: String,
        parse: chrono::ParseError,
    },
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::NoDataDir => write!(f, "Could not determine application data directory."),
            DbError::Io(e) => write!(f, "Could not open Jiary database: {e}"),
            DbError::Sqlite(e) => write!(f, "Database error: {e}"),
            DbError::BadTimestamp { value, parse } => {
                write!(f, "Bad timestamp in database: {value} ({parse})")
            }
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

/// Columns read into a [`SessionRow`], in the order [`read_session_row`]
/// expects.
const SESSION_COLUMNS: &str =
    "id, started_at, ended_at, project, task, activity, notes, focus, interruptions";

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

    /// Inserts a new active session and returns it as stored, so callers
    /// mirror the db's own `started_at` rather than a second clock reading.
    pub fn create_session(
        &self,
        activity: &str,
        project: Option<&str>,
        task: Option<&str>,
    ) -> Result<Session, DbError> {
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
        Ok(Session {
            id: self.conn.last_insert_rowid(),
            started_at: parse_timestamp(now)?,
            ended_at: None,
            project: project.map(str::to_string),
            task: task.map(str::to_string),
            activity: activity.to_string(),
            notes: None,
            focus: None,
            interruptions: None,
        })
    }

    pub fn get_active_session(&self) -> Result<Option<Session>, DbError> {
        let result = self.conn.query_row(
            // ORDER BY + LIMIT: if duplicate active rows exist (legacy data or
            // external edits), deterministically pick the most recent one.
            &format!(
                "SELECT {SESSION_COLUMNS} FROM sessions WHERE ended_at IS NULL
                 ORDER BY started_at DESC, id DESC LIMIT 1"
            ),
            [],
            read_session_row,
        );

        match result {
            Ok(row) => Ok(Some(row_to_session(row)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }

    pub fn complete_session(&self, id: i64, focus: Option<i32>) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
        self.conn
            .execute(
                "UPDATE sessions SET ended_at = ?1, focus = ?2 WHERE id = ?3 AND ended_at IS NULL",
                params![now, focus, id],
            )
            .map_err(DbError::Sqlite)?;
        Ok(())
    }

    pub fn sessions_for_today(&self) -> Result<Vec<Session>, DbError> {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions
                 WHERE date(started_at, 'localtime') = ?1
                 ORDER BY started_at DESC, id DESC"
            ))
            .map_err(DbError::Sqlite)?;
        let rows = stmt
            .query_map(params![today], read_session_row)
            .map_err(DbError::Sqlite)?;
        let rows = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::Sqlite)?;
        rows.into_iter().map(row_to_session).collect()
    }

    pub fn recent_notes(&self, project: &str, task: &str) -> Result<Vec<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT notes FROM sessions
         WHERE project = ?1 AND task = ?2 AND ended_at IS NOT NULL
           AND notes IS NOT NULL
         ORDER BY started_at DESC, id DESC LIMIT 5",
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

    pub fn latest_non_disruption(&self) -> Result<Option<SessionCombo>, DbError> {
        let result = self.conn.query_row(
            "SELECT activity, project, task FROM sessions
         WHERE activity != 'Disruption' AND ended_at IS NOT NULL
         ORDER BY started_at DESC, id DESC LIMIT 1",
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

/// A session row with timestamps still as raw strings. Row mappers can only
/// report [`rusqlite::Error`], so conversion to a [`Session`] (which parses
/// timestamps) happens in [`row_to_session`] instead.
struct SessionRow {
    id: i64,
    started_at: String,
    ended_at: Option<String>,
    project: Option<String>,
    task: Option<String>,
    activity: String,
    notes: Option<String>,
    focus: Option<i32>,
    interruptions: Option<i32>,
}

/// Reads a row selected with [`SESSION_COLUMNS`].
fn read_session_row(row: &Row) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        started_at: row.get(1)?,
        ended_at: row.get(2)?,
        project: row.get(3)?,
        task: row.get(4)?,
        activity: row.get(5)?,
        notes: row.get(6)?,
        focus: row.get(7)?,
        interruptions: row.get(8)?,
    })
}

fn row_to_session(row: SessionRow) -> Result<Session, DbError> {
    let SessionRow {
        id,
        started_at,
        ended_at,
        project,
        task,
        activity,
        notes,
        focus,
        interruptions,
    } = row;
    Ok(Session {
        id,
        started_at: parse_timestamp(started_at)?,
        ended_at: match ended_at {
            Some(s) => Some(parse_timestamp(s)?),
            None => None,
        },
        project,
        task,
        activity,
        notes,
        focus,
        interruptions,
    })
}

fn parse_timestamp(s: String) -> Result<DateTime<Utc>, DbError> {
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|parse| DbError::BadTimestamp { value: s, parse })
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
        let session = tmp
            .db
            .create_session("Programming", Some("proj"), None)
            .unwrap();
        let id = session.id;
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
        // The returned session mirrors the stored row exactly.
        assert_eq!(session.started_at, parse_timestamp(started_at).unwrap());
        assert_eq!(
            tmp.db.get_active_session().unwrap().unwrap().started_at,
            session.started_at
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
    fn unparseable_timestamp_yields_dedicated_error() {
        let tmp = TmpDb::new();
        tmp.db
            .conn
            .execute(
                "INSERT INTO sessions (started_at, activity) VALUES ('not-a-timestamp', 'Reading')",
                [],
            )
            .unwrap();
        let err = tmp.db.get_active_session().unwrap_err();
        assert!(
            matches!(err, DbError::BadTimestamp { .. }),
            "unexpected error: {err}"
        );
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
    fn sessions_started_in_the_same_second_sort_newest_first() {
        // Timestamps have one-second resolution; the id breaks ties so
        // "latest" means the last one inserted.
        let tmp = TmpDb::new();
        for activity in ["Reading", "Writing"] {
            let session = tmp.db.create_session(activity, None, None).unwrap();
            tmp.db.complete_session(session.id, None).unwrap();
        }
        tmp.db
            .conn
            .execute(
                "UPDATE sessions SET started_at = '2026-01-01T10:00:00+00:00'",
                [],
            )
            .unwrap();
        let (activity, _, _) = tmp.db.latest_non_disruption().unwrap().unwrap();
        assert_eq!(activity, "Writing");
    }

    #[test]
    fn sessions_for_today_includes_current_session() {
        let tmp = TmpDb::new();
        tmp.db.create_session("Programming", None, None).unwrap();
        let sessions = tmp.db.sessions_for_today().unwrap();
        assert_eq!(sessions.len(), 1);
    }
}
