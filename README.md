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
| `[` / `]` | Move the active session's start 5 minutes earlier / later |
| `n` | Add a note to the active session |
| `i` | Increment interruption counter |
| `d` | Stop current session, start a Disruption session (no-op during one) |
| `b` | Stop current session (if any), start a Break session (no-op during one) |
| `m` | Stop current session (if any), start a Meeting session (no-op during one) |
| `r` | Resume the last work session (ends a disruption, break or meeting) |
| `/` | Open the command palette |
| `?` | Show all keys |
| `q` / `Ctrl+C` | Quit |

### The main screen

The main screen has three stacked sections:

- **Active** (top) — the running session: timer, project, task, activity and interruptions, with a small blue pixel-art cat on the right (shown when the section has room for it), pinned at the top; below them, the notes (previous notes for the project/task, then this session's), which scroll as they grow.

The cat follows what you're doing: it idles when nothing is running, types on a laptop during work (and dozes off over it after 25 minutes), sleeps on a break, chats with a friend in a meeting, and plays video games when disrupted. It also flinches when you count an interruption (saying *oups!*, then moving on to a new caption), writes your note down in a notebook when you add one, and reacts to your focus rating when you end a session. Under it, a short caption in faded italic says what it's up to (e.g. *working on world domination...*), picked from a list per state and, for work sessions, per activity (see `src/captions.rs`).
- **Today** (middle) — today's sessions, newest first. Scrollable.
- **Summary** (bottom) — the day's span with tracked time split into work (meetings included), breaks and disruptions, and the gaps between sessions, time per activity and per project, average focus, interruptions and disruptions. When the terminal is tall enough, two graphs join in, in the activities' colors: the day as a timeline from the hour you started to now (gaps left blank, hours underneath, and a thin line on top in each session's focus rating: green, yellow or red, dimmed when unrated), and each activity's and each project's share of the tracked time as a bar.

Projects each get a color of their own, kept from day to day (they take them in the order you first used them; two projects on screen never share one), and show in it everywhere: the active session, today's list and the summary.

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

### Command palette

Press `/` to open the command palette: every action by name, with what it does and its key. Type to filter (fuzzy, so `/und` finds `undo`), `↑`/`↓` to pick, `Tab` to complete the name, `Enter` to run it, `Esc` (or Backspace past the `/`) to close. A command does exactly what its key does, so the palette doubles as a way to learn the keys. The palette also opens the longer views, which have no key of their own.

### Weeks and months

`/weekly` and `/monthly` show the current calendar week (Monday to Sunday) or month, a row per day: the day's timeline in activity colors, on an hour axis all the days share so they line up, with the day's tracked time and focus average at the end. Under the days come the period's totals, like today's summary: tracked time split into work, breaks and disruptions, the days you tracked and the average per day, the activity and project bars, and focus. A month is grouped by week and scrolls.

Focus also shows as a grid of squares, GitHub style: a row per weekday, a column per week, each day colored on a gradient from red (1) through yellow to green (3) by its average focus. Days you tracked without rating show dark gray, days with nothing tracked a faint texture. Focus averages, here and in today's summary, weigh each session by its length, so a 3-hour session counts more than a 20-minute one.

`←`/`→` (or `h`/`l`) page to the week or month before or after, `w` and `m` switch to this week or this month, `↑`/`↓` (or `j`/`k`), `PgUp`/`PgDn` and `Home`/`End` scroll, and `Esc` (or `q`) goes back.

### Starting a session

The form opens with your 5 most recent activity/project/task combinations listed (detours left out): press `1`–`5` to start one straight away. Type anything to fill the form in instead:

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

`e` ends a session right away, with no prompts. For the next 5 seconds a bar in the accent color appears under the active section, draining as time runs out (the cat asks too), and you can rate your focus with a single key:

- `1` — bad (fragmented, many context switches)
- `2` — ok
- `3` — good (deep flow)

Or ignore it and carry on: starting the next thing is fine, and the session simply stays unrated. The window is short on purpose, since the rating you give right away is the honest one. It also opens when you switch straight to something else (`s`, `d`, `b`, `m`), so ratings don't depend on whether you pressed `e` first. Breaks and disruptions aren't focused work, so ending one doesn't open it.

Ended by mistake? `u` undoes the last start or end: the session you ended is running again (its rating cleared), and any session that replaced it is removed.

The rating is shown in the daily history as a colored badge, `F1` (red) to `F3` (green), or `F-` if not rated.

### Fixing the times

Started late? `[` moves the running session's start 5 minutes earlier, and `]` 5 minutes later. It never goes into the future. If you switched from another session (pressed `s`, `d`, `b`, `m` or `r`), the switch moves with it, so the session before ends earlier too (keeping at least a minute). If there was a gap, the start stops at the end of the session before; pressing on from there moves both.

Walked away and left a session running? While Jiary is open, it watches how long the computer has gone without keyboard or mouse input (macOS, or Linux with `xprintidle` installed), and notices when the computer slept. After 15 minutes or more away during a work session, coming back asks what that time was:

- `k` — count it as the session (say, you were reading on paper)
- `b` — it was a break: the session ends when you left, a break covers the time away, and the same work picks up again now
- `d` — it was a disruption: the same, with a disruption covering the time away
- `e` — end the session when you left

`u` undoes whichever you choose. Detours (breaks, meetings, disruptions) are never asked about, since being away from the computer is what they're for.

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
