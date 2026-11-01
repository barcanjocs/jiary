use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, Row, params};

use crate::session::{BREAK, DISRUPTION, MEETING, Session};

/// The (activity, project, task) of a session, as returned by
/// [`Db::latest_work_session`] for resuming work.
pub type SessionCombo = (String, Option<String>, Option<String>);

/// A free-text label a session carries, for renaming one everywhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Label {
    Activity,
    Project,
    Task,
}

impl Label {
    fn column(self) -> &'static str {
        match self {
            Label::Activity => "activity",
            Label::Project => "project",
            Label::Task => "task",
        }
    }

    /// The extra condition that keeps a task within its project (bound as
    /// `?2`); none for the others.
    fn scope(self) -> &'static str {
        match self {
            Label::Task => " AND IFNULL(project, '') = IFNULL(?2, '')",
            _ => "",
        }
    }
}

/// A day's sleep and mood, each on focus's 1–3 scale. A row with neither
/// means the day was asked about and skipped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Day {
    pub date: chrono::NaiveDate,
    pub sleep: Option<i32>,
    pub mood: Option<i32>,
}

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

-- Added in 1.1.0; older databases gain it on their next open.
CREATE TABLE IF NOT EXISTS days (
    date TEXT PRIMARY KEY,
    sleep INTEGER,
    mood INTEGER
);
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
        // Explicit '+00:00' offset by convention. Older rows use 'Z', which
        // SQLite's date functions parse just as well (see tests).
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

    /// Ends an active session at `at` instead of now (it was left running
    /// while nobody was there).
    pub fn complete_session_at(&self, id: i64, at: DateTime<Utc>) -> Result<(), DbError> {
        self.conn
            .execute(
                "UPDATE sessions SET ended_at = ?1 WHERE id = ?2 AND ended_at IS NULL",
                params![stamp(at), id],
            )
            .map_err(DbError::Sqlite)?;
        Ok(())
    }

    /// Records a session that already happened, from `start` to `end`.
    pub fn insert_finished_session(
        &self,
        activity: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<i64, DbError> {
        self.conn
            .execute(
                "INSERT INTO sessions (started_at, ended_at, activity) VALUES (?1, ?2, ?3)",
                params![stamp(start), stamp(end), activity],
            )
            .map_err(DbError::Sqlite)?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Moves a session's start.
    pub fn set_started_at(&self, id: i64, at: DateTime<Utc>) -> Result<(), DbError> {
        self.conn
            .execute(
                "UPDATE sessions SET started_at = ?1 WHERE id = ?2",
                params![stamp(at), id],
            )
            .map_err(DbError::Sqlite)?;
        Ok(())
    }

    /// Moves an ended session's end.
    pub fn set_ended_at(&self, id: i64, at: DateTime<Utc>) -> Result<(), DbError> {
        self.conn
            .execute(
                "UPDATE sessions SET ended_at = ?1 WHERE id = ?2 AND ended_at IS NOT NULL",
                params![stamp(at), id],
            )
            .map_err(DbError::Sqlite)?;
        Ok(())
    }

    /// The session just before session `id` (the latest one started before
    /// it), if any.
    pub fn previous_session(&self, id: i64) -> Result<Option<Session>, DbError> {
        let result = self.conn.query_row(
            &format!(
                "SELECT {SESSION_COLUMNS} FROM sessions
                 WHERE id != ?1 AND started_at <= (SELECT started_at FROM sessions WHERE id = ?1)
                 ORDER BY started_at DESC, id DESC LIMIT 1"
            ),
            params![id],
            read_session_row,
        );
        match result {
            Ok(row) => Ok(Some(row_to_session(row)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }

    /// Rates an ended session after the fact (the rating window after it
    /// ends).
    pub fn set_focus(&self, id: i64, focus: i32) -> Result<(), DbError> {
        self.conn
            .execute(
                "UPDATE sessions SET focus = ?1 WHERE id = ?2 AND ended_at IS NOT NULL",
                params![focus, id],
            )
            .map_err(DbError::Sqlite)?;
        Ok(())
    }

    /// Makes an ended session active again, as if it never ended: its end
    /// and focus rating are cleared. Undoes `complete_session`; the caller
    /// makes sure no other session is active.
    pub fn reopen_session(&self, id: i64) -> Result<(), DbError> {
        self.conn
            .execute(
                "UPDATE sessions SET ended_at = NULL, focus = NULL WHERE id = ?1",
                params![id],
            )
            .map_err(DbError::Sqlite)?;
        Ok(())
    }

    /// Removes a session outright. Undoes `create_session`.
    pub fn delete_session(&self, id: i64) -> Result<(), DbError> {
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![id])
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

    /// Sessions started (in local time) on the days from `first` to
    /// `last`, both included, earliest first: a week's or a month's.
    pub fn sessions_between(
        &self,
        first: chrono::NaiveDate,
        last: chrono::NaiveDate,
    ) -> Result<Vec<Session>, DbError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions
                 WHERE date(started_at, 'localtime') BETWEEN ?1 AND ?2
                 ORDER BY started_at, id"
            ))
            .map_err(DbError::Sqlite)?;
        let rows = stmt
            .query_map(params![day_key(first), day_key(last)], read_session_row)
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

    /// Distinct activities, most recently used first, so the activity step
    /// can offer the last one with a single Tab.
    pub fn recent_activities(&self) -> Result<Vec<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT activity FROM sessions WHERE activity != ''
                 GROUP BY activity
                 ORDER BY MAX(started_at) DESC, MAX(id) DESC",
            )
            .map_err(DbError::Sqlite)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(DbError::Sqlite)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Sqlite)
    }

    /// Every project, in the order it was first used: what gives each its
    /// color, so colors stay put as new projects come along.
    pub fn projects_by_first_use(&self) -> Result<Vec<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT project FROM sessions WHERE project IS NOT NULL AND project != ''
                 GROUP BY project ORDER BY MIN(id)",
            )
            .map_err(DbError::Sqlite)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(DbError::Sqlite)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Sqlite)
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

    /// The tasks used with `project`, or with no project for `None`
    /// (NULL and "" alike), alphabetically.
    pub fn tasks_in(&self, project: Option<&str>) -> Result<Vec<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT task FROM sessions
                 WHERE IFNULL(project, '') = IFNULL(?1, '') AND task IS NOT NULL AND task != ''
                 ORDER BY task",
            )
            .map_err(DbError::Sqlite)?;
        let rows = stmt
            .query_map(params![project], |row| row.get::<_, String>(0))
            .map_err(DbError::Sqlite)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Sqlite)
    }

    /// How many sessions use `value` as their `label`. Tasks count within
    /// `project` (`None`: no project); the others ignore it.
    pub fn count_label(
        &self,
        label: Label,
        value: &str,
        project: Option<&str>,
    ) -> Result<usize, DbError> {
        let sql = format!(
            "SELECT COUNT(*) FROM sessions WHERE {} = ?1{}",
            label.column(),
            label.scope()
        );
        let count = match label {
            Label::Task => self
                .conn
                .query_row(&sql, params![value, project], |row| row.get::<_, i64>(0)),
            _ => self
                .conn
                .query_row(&sql, params![value], |row| row.get::<_, i64>(0)),
        };
        count.map(|n| n as usize).map_err(DbError::Sqlite)
    }

    /// Renames every use of `from` as a `label` to `to`, merging the two
    /// when `to` is already in use, and returns how many sessions changed.
    /// Tasks are renamed within `project` (`None`: no project), since task
    /// names repeat across projects.
    pub fn rename_label(
        &self,
        label: Label,
        from: &str,
        to: &str,
        project: Option<&str>,
    ) -> Result<usize, DbError> {
        let column = label.column();
        let sql = format!(
            "UPDATE sessions SET {column} = ?1 WHERE {column} = ?2{}",
            label.scope().replace("?2", "?3")
        );
        match label {
            Label::Task => self.conn.execute(&sql, params![to, from, project]),
            _ => self.conn.execute(&sql, params![to, from]),
        }
        .map_err(DbError::Sqlite)
    }

    /// The last `limit` distinct activity/project/task combinations that
    /// aren't detours, most recently started first: the start form's
    /// one-key picks.
    pub fn recent_combos(&self, limit: usize) -> Result<Vec<SessionCombo>, DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT activity, project, task FROM sessions
                 WHERE activity NOT IN (?1, ?2, ?3)
                 GROUP BY activity, project, task
                 ORDER BY MAX(started_at) DESC, MAX(id) DESC LIMIT ?4",
            )
            .map_err(DbError::Sqlite)?;
        let rows = stmt
            .query_map(params![DISRUPTION, BREAK, MEETING, limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(DbError::Sqlite)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Sqlite)
    }

    /// The most recent completed session that isn't a detour (disruption,
    /// break or meeting): what `r` resumes.
    pub fn latest_work_session(&self) -> Result<Option<SessionCombo>, DbError> {
        let result = self.conn.query_row(
            "SELECT activity, project, task FROM sessions
         WHERE activity NOT IN (?1, ?2, ?3) AND ended_at IS NOT NULL
         ORDER BY started_at DESC, id DESC LIMIT 1",
            params![DISRUPTION, BREAK, MEETING],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        );

        match result {
            Ok(combo) => Ok(Some(combo)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }

    /// The day's sleep and mood, if it was asked about (a skipped day has
    /// a row with neither).
    pub fn day(&self, date: chrono::NaiveDate) -> Result<Option<Day>, DbError> {
        Ok(self.days_between(date, date)?.pop())
    }

    /// Sets the day's sleep and mood (`None`s for a skipped day),
    /// replacing what was there.
    pub fn set_day(&self, day: Day) -> Result<(), DbError> {
        self.conn
            .execute(
                "INSERT INTO days (date, sleep, mood) VALUES (?1, ?2, ?3)
                 ON CONFLICT (date) DO UPDATE SET sleep = ?2, mood = ?3",
                params![day_key(day.date), day.sleep, day.mood],
            )
            .map_err(DbError::Sqlite)?;
        Ok(())
    }

    /// The days from `first` to `last`, both included, that were asked
    /// about, earliest first.
    pub fn days_between(
        &self,
        first: chrono::NaiveDate,
        last: chrono::NaiveDate,
    ) -> Result<Vec<Day>, DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT date, sleep, mood FROM days
                 WHERE date BETWEEN ?1 AND ?2 ORDER BY date",
            )
            .map_err(DbError::Sqlite)?;
        let rows = stmt
            .query_map(params![day_key(first), day_key(last)], |row| {
                Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(DbError::Sqlite)?;
        let mut days = Vec::new();
        for row in rows {
            let (date, sleep, mood) = row.map_err(DbError::Sqlite)?;
            // Only Jiary writes this table; a date it can't read is skipped.
            if let Ok(date) = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d") {
                days.push(Day { date, sleep, mood });
            }
        }
        Ok(days)
    }
}

/// A local date as stored: `YYYY-MM-DD`, as SQLite's `date()` gives it.
fn day_key(date: chrono::NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// A timestamp as stored: RFC 3339 to the second with an explicit offset.
fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
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

/// Where the database (and the layout file next to it) lives.
pub fn data_dir() -> Option<PathBuf> {
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

    /// A finished session with these labels, straight into the table.
    fn insert(t: &TmpDb, activity: &str, project: Option<&str>, task: Option<&str>) {
        t.db.conn
            .execute(
                "INSERT INTO sessions (started_at, ended_at, project, task, activity)
                 VALUES ('2026-10-01T09:00:00+00:00', '2026-10-01T10:00:00+00:00', ?1, ?2, ?3)",
                params![project, task, activity],
            )
            .unwrap();
    }

    /// Every session's (activity, project, task), in insert order.
    fn labels(t: &TmpDb) -> Vec<(String, Option<String>, Option<String>)> {
        let mut stmt =
            t.db.conn
                .prepare("SELECT activity, project, task FROM sessions ORDER BY id")
                .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn renaming_a_label_changes_every_use_and_merges_into_an_existing_one() {
        let t = TmpDb::new();
        insert(&t, "Progamming", Some("jiary"), None);
        insert(&t, "Programming", Some("jiry"), None);
        insert(&t, "Progamming", Some("api"), None);
        assert_eq!(
            t.db.count_label(Label::Activity, "Progamming", None)
                .unwrap(),
            2
        );

        let changed =
            t.db.rename_label(Label::Activity, "Progamming", "Programming", None)
                .unwrap();
        assert_eq!(changed, 2);
        let changed =
            t.db.rename_label(Label::Project, "jiry", "jiary", None)
                .unwrap();
        assert_eq!(changed, 1);

        let all = labels(&t);
        assert!(all.iter().all(|(a, _, _)| a == "Programming"), "{all:?}");
        assert_eq!(t.db.count_label(Label::Project, "jiary", None).unwrap(), 2);
        assert_eq!(t.db.recent_activities().unwrap(), ["Programming"]);
        assert_eq!(t.db.distinct_projects().unwrap(), ["api", "jiary"]);
    }

    #[test]
    fn tasks_are_renamed_and_counted_within_their_project() {
        let t = TmpDb::new();
        insert(&t, "Writing", Some("api"), Some("docs"));
        insert(&t, "Writing", Some("website"), Some("docs"));
        insert(&t, "Writing", None, Some("docs"));
        insert(&t, "Writing", Some(""), Some("docs"));

        assert_eq!(
            t.db.count_label(Label::Task, "docs", Some("api")).unwrap(),
            1
        );
        // No project, NULL or blank, is one group.
        assert_eq!(t.db.count_label(Label::Task, "docs", None).unwrap(), 2);
        assert_eq!(t.db.tasks_in(None).unwrap(), ["docs"]);

        let changed =
            t.db.rename_label(Label::Task, "docs", "api docs", Some("api"))
                .unwrap();
        assert_eq!(changed, 1);
        let tasks = labels(&t)
            .into_iter()
            .map(|(_, _, task)| task)
            .collect::<Vec<_>>();
        assert_eq!(
            tasks,
            [Some("api docs"), Some("docs"), Some("docs"), Some("docs")]
                .map(|t| t.map(String::from))
        );
        assert_eq!(t.db.tasks_in(Some("api")).unwrap(), ["api docs"]);
        assert_eq!(t.db.tasks_in(Some("website")).unwrap(), ["docs"]);
    }

    #[test]
    fn days_are_set_replaced_and_read_back_by_range() {
        let t = TmpDb::new();
        let date = |d| chrono::NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
        let day = |d, sleep, mood| Day {
            date: date(d),
            sleep,
            mood,
        };
        assert_eq!(t.db.day(date(26)).unwrap(), None, "never asked");
        t.db.set_day(day(26, None, None)).unwrap();
        assert_eq!(t.db.day(date(26)).unwrap(), Some(day(26, None, None)));
        t.db.set_day(day(26, Some(3), Some(1))).unwrap();
        t.db.set_day(day(28, Some(2), None)).unwrap();
        t.db.set_day(day(30, Some(1), Some(1))).unwrap();
        assert_eq!(
            t.db.days_between(date(25), date(29)).unwrap(),
            [day(26, Some(3), Some(1)), day(28, Some(2), None)]
        );
    }

    #[test]
    fn sessions_between_takes_the_local_days_given() {
        use chrono::TimeZone;
        let t = TmpDb::new();
        let at = |day, h| {
            chrono::Local
                .with_ymd_and_hms(2026, 10, day, h, 0, 0)
                .unwrap()
                .with_timezone(&Utc)
        };
        for (day, activity) in [(25, "Before"), (27, "Late"), (26, "Early"), (28, "After")] {
            t.db.insert_finished_session(activity, at(day, 9), at(day, 10))
                .unwrap();
        }
        let day = |d| chrono::NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
        let sessions = t.db.sessions_between(day(26), day(27)).unwrap();
        let activities = sessions
            .iter()
            .map(|s| s.activity.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            activities,
            ["Early", "Late"],
            "both days included, earliest first"
        );
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
        let (activity, _, _) = tmp.db.latest_work_session().unwrap().unwrap();
        assert_eq!(activity, "Writing");
    }

    /// Inserts a completed session starting at `started_at` (stored as is).
    fn insert_started_at(db: &Db, started_at: &str) {
        db.conn
            .execute(
                "INSERT INTO sessions (started_at, ended_at, activity)
                 VALUES (?1, ?1, 'Reading')",
                params![started_at],
            )
            .unwrap();
    }

    #[test]
    fn sessions_for_today_compares_local_dates() {
        // Yesterday and today in local time, stored as UTC. In any non-UTC
        // zone at least one of these falls on a different UTC date, which
        // a comparison of UTC dates would get wrong.
        let tmp = TmpDb::new();
        let now = chrono::Local::now();
        let midnight = now
            .date_naive()
            .and_hms_opt(0, 0, 30)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .unwrap();
        let utc = |t: chrono::DateTime<chrono::Local>| {
            t.with_timezone(&Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
        };
        insert_started_at(&tmp.db, &utc(midnight - chrono::Duration::minutes(1)));
        insert_started_at(&tmp.db, &utc(midnight));
        let sessions = tmp.db.sessions_for_today().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].started_at, midnight.with_timezone(&Utc));
    }

    #[test]
    fn legacy_z_suffixed_timestamps_still_count_as_today() {
        // Rows written before the '+00:00' convention end in 'Z'; SQLite's
        // date functions must still place them on the right local day.
        let tmp = TmpDb::new();
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        assert!(now.ends_with('Z'));
        insert_started_at(&tmp.db, &now);
        assert_eq!(tmp.db.sessions_for_today().unwrap().len(), 1);
    }

    #[test]
    fn recent_activities_are_distinct_and_most_recent_first() {
        let tmp = TmpDb::new();
        for (ts, activity) in [
            ("2026-01-01T09:00:00+00:00", "Reading"),
            ("2026-01-01T10:00:00+00:00", "Writing"),
            ("2026-01-01T11:00:00+00:00", "Reading"),
            ("2026-01-01T12:00:00+00:00", "Gardening"),
        ] {
            tmp.db
                .conn
                .execute(
                    "INSERT INTO sessions (started_at, ended_at, activity) VALUES (?1, ?1, ?2)",
                    params![ts, activity],
                )
                .unwrap();
        }
        assert_eq!(
            tmp.db.recent_activities().unwrap(),
            ["Gardening", "Reading", "Writing"]
        );
    }

    #[test]
    fn latest_work_session_skips_detours() {
        let tmp = TmpDb::new();
        for activity in ["Writing", DISRUPTION, BREAK, MEETING, DISRUPTION] {
            let session = tmp.db.create_session(activity, Some("p"), None).unwrap();
            tmp.db.complete_session(session.id, None).unwrap();
        }
        let (activity, project, _) = tmp.db.latest_work_session().unwrap().unwrap();
        assert_eq!(activity, "Writing");
        assert_eq!(project.as_deref(), Some("p"));
    }

    #[test]
    fn sessions_for_today_includes_current_session() {
        let tmp = TmpDb::new();
        tmp.db.create_session("Programming", None, None).unwrap();
        let sessions = tmp.db.sessions_for_today().unwrap();
        assert_eq!(sessions.len(), 1);
    }
}
