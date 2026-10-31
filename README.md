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
| `e` | End the active session |
| `1` `2` `3` | Rate your focus, within 5 seconds of a session ending |
| `u` | Undo the last start or end |
| `n` | Add a note to the active session |
| `i` | Increment interruption counter |
| `d` | Stop current session, start a Disruption session (no-op during one) |
| `b` | Stop current session (if any), start a Break session (no-op during one) |
| `m` | Stop current session (if any), start a Meeting session (no-op during one) |
| `r` | Resume the last work session (ends a disruption, break or meeting) |
| `?` | Show all keys |
| `q` / `Ctrl+C` | Quit |

### The main screen

The main screen has three stacked sections:

- **Active** (top) — the running session: timer, project, task, activity and interruptions, with a small blue pixel-art cat on the right (shown when the section has room for it), pinned at the top; below them, the notes (previous notes for the project/task, then this session's), which scroll as they grow.

The cat follows what you're doing: it idles when nothing is running, types on a laptop during work (and dozes off over it after 25 minutes), sleeps on a break, chats with a friend in a meeting, and plays video games when disrupted. It also flinches when you count an interruption, writes your note down in a notebook when you add one, and reacts to your focus rating when you end a session. Under it, a short caption in faded italic says what it's up to (e.g. *working on world domination...*), picked from a list per state and, for work sessions, per activity (see `src/captions.rs`).
- **Today** (middle) — today's sessions, newest first. Scrollable.
- **Summary** (bottom) — time per activity and per project, average focus, interruptions and disruptions, and the day's span with tracked time, breaks and gaps.

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Focus the next / previous section (accent border) |
| `Space` | Collapse or expand the focused section (Today and Summary) |
| `+` / `-` | Grow or shrink the focused section |
| `=` | Reset section sizes to fit their content |
| `↑` `↓` / `j` `k`, `PgUp` `PgDn`, `Home` `End` | Scroll the focused section: the notes in Active, otherwise today's sessions |

A collapsed section shrinks to one line that still shows a short summary (e.g. `▸ TODAY · 6 sessions · 4h 12m`). Today's sessions take whatever space the other sections leave; when Today is collapsed, the summary does.

With the mouse: drag the line between two sections to resize them (grab either of its two border rows), click a section to focus it, click the title text of Today or Summary (`▾ TODAY …`) to collapse or expand it, and use the wheel to scroll the section under the pointer (notes or today's sessions). A drag never collapses anything, even when it starts on a title. Because Jiary captures the mouse, select text with `Shift` held (in most terminals).

Section sizes and collapsed sections are remembered between runs, in `layout.conf` next to the database.

### Starting a session

1. **Activity** — type to filter suggestions (fuzzy match), Tab or ↑/↓ to pick one, Enter to confirm
2. **Project** — same as activity
3. **Task** — same as activity (suggestions filtered by selected project)

Activity is required; project and task are optional (press Enter on an empty field to skip).

### Activities

Activities are free text, like projects: type any name to use a new one. Suggestions are the activities you have used, most recent first, so `s`, Tab, Enter repeats your last one. A new database suggests a starter set (Programming, Reading, Writing, Meeting, Administration, Disruption, Break, Other) until you have history; after that, built-ins you never use drop out of the list.

`Disruption`, `Break` and `Meeting` are the activities `d`, `b` and `m` start and `r` resumes from.

### Autocomplete

Activity, project and task fields show all previous values as soon as the field is active. As you type, suggestions are filtered using fuzzy matching (subsequence-based, via `SkimMatcherV2`). Task suggestions are further filtered by the selected project.

Tab (or ↓) picks the best suggestion; further Tabs cycle through them, ↑/↓ move between them, and ↑ past the top returns to what you typed. The input line always shows what Enter will submit. Typing after picking a suggestion continues from it. You can always type a new value that doesn't match any suggestion.

### Notes

Press `n` at any time during an active session to append a note. Notes are saved immediately and persist even if the app exits before the session ends.

To add a last note to a session you just ended, press `u` to reopen it, `n` to add the note, and `e` again.

### Detours: disruptions, breaks, meetings & resume

Three keys step you out of what you're doing with a single keystroke:

- `d` — you're interrupted (colleague conversation, phone call, context switch): starts a "Disruption" session.
- `b` — you're taking a break on purpose (coffee, lunch, a walk): starts a "Break" session. It also works when nothing is running, e.g. right after ending a session with `e`.
- `m` — you're jumping into a meeting: starts a "Meeting" session. It also works when nothing is running (e.g. the morning standup), and meeting time counts as work in the summary.

When you're done, press `r` to end the detour and restart your previous activity/project/task. The status line groups the three as `[d/b/m] away`; `?` lists them separately.

The full round-trip is two keystrokes: `d`, `b` or `m` … `r`. `r` always goes back to the last session that wasn't a detour, so a meeting followed by a break (`m`, then `b`) still resumes your work. Only a disruption needs a running session to interrupt.

Meetings you start with `s` (activity "Meeting") are detours too, so `r` never resumes into a meeting.

### Focus rating

`e` ends a session right away, with no prompts. For the next 5 seconds the status line counts down, and you can rate your focus with a single key:

- `1` — bad (fragmented, many context switches)
- `2` — ok
- `3` — good (deep flow)

Or ignore it and carry on: starting the next thing is fine, and the session simply stays unrated. The window is short on purpose, since the rating you give right away is the honest one. It also opens when you switch straight to something else (`s`, `d`, `b`, `m`), so ratings don't depend on whether you pressed `e` first. Breaks and disruptions aren't focused work, so ending one doesn't open it.

Ended by mistake? `u` undoes the last start or end: the session you ended is running again (its rating cleared), and any session that replaced it is removed.

The rating is shown in the daily history as `F1`–`F3`, or `F-` if not rated.

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
