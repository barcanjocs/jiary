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
| `e` | End the active session (notes + focus rating) |
| `n` | Add a note to the active session |
| `i` | Increment interruption counter |
| `d` | Stop current session, start a Disruption session (no-op during one) |
| `b` | Stop current session, start a Break session (no-op during one) |
| `r` | Resume the last work session (ends a disruption or break) |
| `?` | Show all keys |
| `q` / `Ctrl+C` | Quit |

### The main screen

The main screen has three stacked sections:

- **Active** (top) — the running session: timer, project, task, activity, interruptions and notes, with a small pixel-art cat on the right (shown when the section has room for it).
- **Today** (middle) — today's sessions, newest first. Scrollable.
- **Summary** (bottom) — time per activity and per project, average focus, interruptions and disruptions, and the day's span with tracked time, breaks and gaps.

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Focus the next / previous section (accent border) |
| `Space` | Collapse or expand the focused section (Today and Summary) |
| `+` / `-` | Grow or shrink the focused section |
| `=` | Reset section sizes to fit their content |
| `↑` `↓` / `j` `k`, `PgUp` `PgDn`, `Home` `End` | Scroll today's sessions |

A collapsed section shrinks to one line that still shows a short summary (e.g. `▸ TODAY · 6 sessions · 4h 12m`). Today's sessions take whatever space the other sections leave; when Today is collapsed, the summary does.

With the mouse: drag the line between two sections to resize them (grab either of its two border rows), click a section to focus it, click the title text of Today or Summary (`▾ TODAY …`) to collapse or expand it, and use the wheel to scroll today's sessions. A drag never collapses anything, even when it starts on a title. Because Jiary captures the mouse, select text with `Shift` held (in most terminals).

Section sizes and collapsed sections are remembered between runs, in `layout.conf` next to the database.

### Starting a session

1. **Activity** — type to filter suggestions (fuzzy match), Tab or ↑/↓ to pick one, Enter to confirm
2. **Project** — same as activity
3. **Task** — same as activity (suggestions filtered by selected project)

Activity is required; project and task are optional (press Enter on an empty field to skip).

### Activities

Activities are free text, like projects: type any name to use a new one. Suggestions are the activities you have used, most recent first, so `s`, Tab, Enter repeats your last one. A new database suggests a starter set (Programming, Reading, Writing, Meeting, Administration, Disruption, Break, Other) until you have history; after that, built-ins you never use drop out of the list.

`Disruption` and `Break` are the activities `d` and `b` start and `r` resumes from.

### Autocomplete

Activity, project and task fields show all previous values as soon as the field is active. As you type, suggestions are filtered using fuzzy matching (subsequence-based, via `SkimMatcherV2`). Task suggestions are further filtered by the selected project.

Tab (or ↓) picks the best suggestion; further Tabs cycle through them, ↑/↓ move between them, and ↑ past the top returns to what you typed. The input line always shows what Enter will submit. Typing after picking a suggestion continues from it. You can always type a new value that doesn't match any suggestion.

### Notes

Press `n` at any time during an active session to append a note. Notes are saved immediately and persist even if the app exits before the session ends.

You can also add a note when ending a session (press `e`).

### Disruptions, Breaks & Resume

If you're interrupted (colleague conversation, phone call, context switch), press `d` to instantly stop your current session and start a "Disruption" session. When you're taking a break on purpose (coffee, lunch, a walk), press `b` instead to start a "Break" session. When you're ready to get back to work, press `r` to end the disruption or break and restart your previous activity/project/task.

The full round-trip is two keystrokes: `d` (or `b`) … `r`. `r` always goes back to the last session that was actual work, so a break that turns into a disruption (`b`, then `d`) still resumes your work. Both need a running session to pause.

### Focus rating

When ending a session (`e`), after the notes prompt you'll be asked to rate your focus:

- `1` — bad (fragmented, many context switches)
- `2` — ok
- `3` — good (deep flow)

Press Enter to skip rating. Esc at either step cancels: the session keeps running and the typed notes are discarded. The rating is shown in the daily history as `F1`–`F3`, or `F-` if not rated.

### Interruptions

Press `i` at any time during an active session to log a micro-interruption (phone glance, Slack ping, urge to check something). The count is shown in the active session view and in the daily history as `I:N`.

Significant interruptions (conversations, meetings) should be captured as Disruption sessions via `d` rather than as interruption counts.

### Errors

Database failures (e.g. a locked or corrupt file) are shown on the bottom line in red, replacing the key hints while they persist. The line reflects the outcome of your most recent database operation: a successful action clears it, a failure shows the new error. Errors from startup loads persist until you perform an action; Jiary keeps running with whatever data it could load. If the database cannot be opened at all, Jiary prints the error to the terminal and exits.

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
    focus INTEGER,
    interruptions INTEGER
);
```

Timestamps are stored as UTC RFC 3339 strings with an explicit offset (e.g. `2026-09-04T09:15:00+00:00`) and displayed in local time.

To inspect your data directly:

```sh
sqlite3 ~/Library/Application\ Support/jiary/jiary.db "SELECT * FROM sessions ORDER BY started_at DESC LIMIT 10;"
```

## Backing up

Copy the `.db` file. That's the entire dataset. (`layout.conf` beside it only holds the main screen's section sizes.)

## Design principles

- **Jiary records what happened; it does not manage what should happen.**
- **The diary is the source of truth; timers are merely tools for recording it accurately.**
- Sessions are historical observations, not workflow entities.
- Projects and tasks are free-text labels, not managed objects with state.
