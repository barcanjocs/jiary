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
";

impl Db {
    pub fn open() -> Result<Self, DbError> {
        let dir = data_dir().ok_or(DbError::NoDataDir)?;
        std::fs::create_dir_all(&dir).map_err(DbError::Io)?;
        let path = dir.join("jiary.db");
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
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
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
            "SELECT id, started_at, ended_at, project, task, activity, notes, outcome, focus, interruptions
             FROM sessions WHERE ended_at IS NULL",
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
        interruptions: Option<i32>,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.conn
            .execute(
                "UPDATE sessions SET ended_at = ?1, outcome = ?2, focus = ?3, interruptions = ?4 WHERE id = ?5 AND ended_at IS NULL",
                params![now, outcome, focus, interruptions, id],
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
                 WHERE date(started_at) = ?1
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
