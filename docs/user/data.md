# Your data

## Where it lives

Everything is in one SQLite file, created on first run:

- **macOS:** `~/Library/Application Support/jiary/jiary.db`
- **Linux:** `~/.local/share/jiary/jiary.db`

Next to it are three small files:

- `layout.conf`: the main screen's section sizes and collapsed sections;
- `settings.conf`: your [settings](recording.md#settings): the Pomodoro,
  its target, and whether Jiary asks about sleep and mood;
- `jiary.lock`: makes sure only one Jiary runs at a time, so two can't write
  to the diary at once. A second Jiary says the first is already running
  (with its process id) and exits. The lock is released however Jiary
  ends, even on a crash.

## Backing up

Copy `jiary.db`. That's your whole diary.

## What's in it

Two tables. `sessions` has one row per session:

```sql
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    started_at TEXT NOT NULL,   -- RFC 3339 UTC
    ended_at TEXT,              -- NULL while active
    project TEXT,
    task TEXT,
    activity TEXT NOT NULL,
    notes TEXT,
    focus INTEGER,
    interruptions INTEGER
);
```

`days` has a row for each day you were asked about sleep and mood (added in
1.1.0; an older diary gains the table the first time 1.1.0 opens it):

```sql
CREATE TABLE days (
    date TEXT PRIMARY KEY,      -- local date, YYYY-MM-DD
    sleep INTEGER,              -- 1–3, NULL when skipped
    mood INTEGER
);
```

Times are stored in UTC with an explicit offset (such as
`2026-09-04T09:15:00+00:00`) and shown in local time. "Today" is the local
date a session started on.

To look at your data directly:

```sh
sqlite3 ~/Library/Application\ Support/jiary/jiary.db \
  "SELECT * FROM sessions ORDER BY started_at DESC LIMIT 10;"
```

## Errors

If the database fails (say, the file is locked or corrupt), the error shows
in red on the bottom line, in place of the key hints. The line always
reflects your most recent action: one that works clears it, and one that
fails shows the new error. An error while loading at startup stays until
your first action, and Jiary keeps running with whatever it could load.

If the database can't be opened at all, Jiary prints the error and exits.
