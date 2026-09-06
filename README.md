# Jiary

A local-first terminal application for maintaining a personal work diary.

Jiary records what happened during your working day — sessions of programming, reading, writing, meetings — with minimal cognitive overhead. It does not manage tasks, set priorities, or tell you what to do next.

## Building

```sh
cargo build --release
```

The binary will be at `target/release/jiary`.

Requires Rust stable. No system dependencies needed (SQLite is bundled).

## Running

```sh
jiary
```

On first run, a database is created at:

- **macOS:** `~/Library/Application Support/jiary/jiary.db`
- **Linux:** `~/.local/share/jiary/jiary.db`

## Usage

| Key | Action |
|-----|--------|
| `s` | Start a new session |
| `e` | End the active session (optional note prompt) |
| `n` | Add a note to the active session |
| `q` | Quit |

### Starting a session

1. **Activity** — navigate with ↑/↓ or press 1–6, confirm with Enter
2. **Project** — type to filter suggestions (fuzzy match), Tab to cycle, Enter to confirm
3. **Task** — same as project

All fields after activity are optional; press Enter on an empty field to skip.

### Autocomplete

Project and task fields show suggestions from your session history as you type. Matching is fuzzy (subsequence-based, via `SkimMatcherV2`). You can always type a new value that doesn't match any suggestion.

### Notes

Press `n` at any time during an active session to append a note. Notes are saved immediately and persist even if the app exits before the session ends.

You can also add a note when ending a session (press `e`).

## Data

All data is stored in a single SQLite file. The schema:

```sql
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    started_at TEXT NOT NULL,   -- RFC 3339 UTC
    ended_at TEXT,              -- NULL while active
    project TEXT,
    task TEXT,
    activity TEXT NOT NULL,
    notes TEXT,
    outcome TEXT,
    focus INTEGER,
    interruptions INTEGER
);
```

Timestamps are stored as UTC RFC 3339 strings (e.g. `2026-09-04T09:15:00Z`) and displayed in local time.

To inspect your data directly:

```sh
sqlite3 ~/Library/Application\ Support/jiary/jiary.db "SELECT * FROM sessions ORDER BY started_at DESC LIMIT 10;"
```

## Backing up

Copy the `.db` file. That's the entire dataset.

## Design principles

- **Jiary records what happened; it does not manage what should happen.**
- **The diary is the source of truth; timers are merely tools for recording it accurately.**
- Sessions are historical observations, not workflow entities.
- Projects and tasks are free-text labels, not managed objects with state.
