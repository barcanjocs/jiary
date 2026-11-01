# Implementation details

How Jiary works inside, and why. Read it before changing code, and keep it
in sync when the architecture changes. What Jiary does for the user lives
in the [user manual](../user/README.md); where things stand, the backlog
and the working conventions live in [`DEVELOPMENT.md`](../../DEVELOPMENT.md).

## Big picture

One binary, one thread for the UI, no network, no system dependencies
(SQLite is bundled). One SQLite file holds everything that matters; the rest
is UI state rebuilt from it on startup.

```
main.rs        flags → terminal check → lock → db → event loop
  ├─ lock      one jiary at a time                      (lock.rs)
  ├─ idle      system idle time, polled on a thread     (idle.rs)
  └─ App       all UI state + the Db handle             (app/)
       ├─ Db         rusqlite connection; all SQL       (db.rs)
       ├─ Session    one row, timestamps parsed         (session.rs)
       ├─ summarize  totals over in-memory sessions     (summary.rs)
       ├─ Period     week / month / year date ranges    (period.rs)
       ├─ LayoutPrefs  main-screen geometry + file      (layout.rs)
       ├─ mascot, captions  the cat and its sentences   (mascot.rs, captions.rs)
       └─ theme      colors and shared styles           (theme.rs)
```

The loop runs at about 10 fps: each turn sets the size, draws, places the
cursor, then waits up to 100 ms for an event, so the timer ticks with no
input. Key presses and mouse events are handled; key releases and repeats
(some terminals report them) and other events are ignored, and Ctrl
combinations never reach a form as plain letters. Quitting is a return
value: `App::handle_key` returns `true` for `q` on the main screen or Ctrl+C
anywhere.

**Data flows one way per frame: state → pixels.** `draw` never mutates and
never touches the db. Key handlers change state and write to the db; the
next frame shows the result. No event bus, no async, no shared state:
`App` alone owns the `Db`.

## Module map

- **`main.rs`**: `--help` / `--version`, refuses to run without a terminal,
  takes the lock, opens the db, then owns the terminal: `ratatui::init()`
  (raw mode, alternate screen, a panic hook that restores them, chained with
  one that shows the cursor and releases the mouse), mouse capture, and
  `ratatui::restore()` on every exit path. It calls only `App::new`,
  `set_size`, `draw`, `cursor_position`, `handle_key`, `handle_mouse` and
  `observe_idle`. A thread polls `idle::system_idle` every `idle::POLL` and
  sends the result over a channel the loop drains before each frame (the poll
  runs a command, so it stays off the draw loop).
- **`lock.rs`**: `acquire(path)`, an exclusive `flock` (std
  `File::try_lock`) on `jiary.lock` in the data dir, held for the process's
  lifetime and released by the OS however it ends. The holder writes its pid
  in it, so a second jiary can say who has it (`LockError::Running`) and
  exit 1.
- **`idle.rs`**: `system_idle()`, the time since the last keyboard or mouse
  input system-wide: `ioreg -c IOHIDSystem` on macOS, `xprintidle` on Linux,
  `None` elsewhere or when the tool fails.
- **`db.rs`**: `Db` over a `rusqlite::Connection`. All SQL lives here. See
  [Data layer](#data-layer).
- **`session.rs`**: `Session`, one row with timestamps parsed to
  `DateTime<Utc>`; the detour activities (`DISRUPTION`, `BREAK`, `MEETING`,
  `DETOURS`, `is_detour`) and `asks_focus` (not breaks or disruptions).
- **`summary.rs`**: `summarize(sessions, active, now) -> DaySummary`: time
  per activity and project, focus averaged weighted by length,
  interruptions, disruptions, breaks, span, gaps, and the sessions as
  timeline `Block`s. Takes any sessions, so the longer views use it too.
  Pure.
- **`search.rs`**: the `/rewind` query: `Query::parse` (words, prefixes,
  quotes, dates), `matches_date`, and `match_session`, which also says
  which note line matched; and `matcher()`, the one fuzzy matcher the
  pickers, the palette and searches share. Pure.
- **`period.rs`**: `Period` (week from Monday, calendar month, calendar
  year): `days(offset, today)` gives the first and last day `offset` periods
  from the current one, `title` names it. Pure.
- **`layout.rs`**: `LayoutPrefs`, the main screen's section heights and
  collapsed state: `resolve` splits the content area into the three
  sections, `resize` / `set_*_height` move borders with clamping, and
  `parse` / `load` / `save` handle `layout.conf`. Pure geometry, no
  rendering.
- **`settings.rs`**: `Settings` (the Pomodoro's `PomodoroMode` and
  target, `sleep_and_mood`), read and
  written like the layout file, as `settings.conf`. See
  [Settings file](#settings-file).
- **`mascot.rs`**: the cat. Sprites are grids of palette characters, two
  pixel rows per terminal row; `mood_lines` / `reaction_lines` pick the
  frame for the time and render it to half-block lines. Colors come from
  `theme::CAT_*`.
- **`captions.rs`**: the sentences under the cat, one list per `Mood` plus
  per-activity lists for work, and `caption(mood, activity, seed)`. Adding a
  caption means adding a sentence to a list.
- **`theme.rs`**: every color and shared style; widgets take their look
  from here. See [Colors](#colors).
- **`app/`**: everything else, one `impl App` block per file:
  - `mod.rs`: `App`, `Screen`, the session lifecycle, and the `handle_key` /
    `draw` dispatch on `Screen`.
  - One file per screen: `main_screen.rs`, `forms.rs` (start and note
    forms, `Autocomplete`, `fuzzy_filter`), `mood.rs` (sleep and mood),
    `settings_screen.rs` (`/settings`), `pomodoro.rs` (the cue and `p`),
    `rewind.rs` (`/rewind` and the past days it opens),
    `rename.rs`, `palette.rs`, `period_view.rs`, `away.rs`, `help.rs`.
  - Shared pieces: `sections.rs` (main-screen geometry, scrolling, mouse),
    `chrome.rs` (header, status line, cursor), `cat.rs` (mood, reactions,
    caption), `modal.rs` (modal geometry, `WrappedInput`, picker list),
    `items.rs` (the summary's styled items and how they wrap), `graphs.rs`
    (timeline, focus line, share bars, focus grids), `text.rs` (wrapping,
    plurals, times).
  - `tests/` (the UI's tests: the harness and shared helpers in `mod.rs`,
    then a file per screen or area) and `screenshot.rs` (the docs' pictures);
    see [Testing](#testing).

## Data layer

### Schema

Two tables. `sessions`, one row per session:

```sql
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    started_at TEXT NOT NULL,   -- RFC 3339 UTC, e.g. 2026-09-04T09:15:00+00:00
    ended_at TEXT,              -- NULL while active
    project TEXT,               -- free text, NULL when unset
    task TEXT,                  -- free text, NULL when unset
    activity TEXT NOT NULL,
    notes TEXT,                 -- newline-terminated entries, appended
    focus INTEGER,              -- 1..3, NULL when unrated
    interruptions INTEGER       -- small-interruption count
);
```

`days`, one row per day asked about sleep and mood (1.1.0, the first
schema change after 1.0):

```sql
CREATE TABLE days (
    date TEXT PRIMARY KEY,      -- local date, YYYY-MM-DD
    sleep INTEGER,              -- 1..3, NULL when skipped
    mood INTEGER                -- 1..3, NULL when skipped
);
```

A row means the day was asked, so a skip (both `NULL`) isn't asked again.
`day`, `set_day` (an upsert) and `days_between` read and write it as
`Day`s.

No other tables, by design: projects, tasks and activities are free-text
labels, not managed objects, and their "lists" are queries over history
(`distinct_projects`, `tasks_for_project`, `recent_activities`,
`projects_by_first_use`, `recent_combos`), and renaming one rewrites the
rows that use it (`rename_label`). The schema, with the trigger
below, is one `SCHEMA` string run on every open (`CREATE ... IF NOT
EXISTS`), so opening an old db also brings it up to date.

### One active session: a trigger, not a constraint

At most one row may have `ended_at IS NULL`. A `BEFORE INSERT` trigger
enforces it (`RAISE(ABORT, 'another session is already active')`); a UNIQUE
index can't, since SQLite's UNIQUE ignores NULLs. The UI never relies on
it, since it closes the active session first and stops if that fails (see
[Session lifecycle](#session-lifecycle)); the trigger is the backstop
against external edits. `get_active_session` also orders by
`started_at DESC, id DESC`, so duplicate active rows from older data or
external edits resolve to the most recent one.

### Timestamps

Stored as RFC 3339 UTC with an explicit `+00:00`. Older rows end in `Z`;
SQLite's date functions read both (pinned by
`legacy_z_suffixed_timestamps_still_count_as_today`), so there's no
migration. Resolution is one second, so every `ORDER BY started_at` also
orders by `id` (the most recent insert wins a tie). "Today" is the local
start date, `date(started_at, 'localtime') = ?`; a range of days
(`sessions_between`) works the same way. The UI converts to local time
for display (`fmt_time`), never SQL.

### Rows to sessions in two steps

rusqlite's row mappers can only fail with `rusqlite::Error`, but parsing a
timestamp should fail as `DbError::BadTimestamp`. So queries select
`SESSION_COLUMNS`, `read_session_row` maps a row into a `SessionRow` with
the timestamps still strings, and `row_to_session` parses them outside the
mapper. New queries that return sessions follow the same pattern.

`create_session` returns the `Session` it inserted, built from the same
timestamp string it stored, so memory never has its own clock reading.

### Data directory

`dirs::data_dir()/jiary/` (`~/Library/Application Support/jiary` on macOS,
`~/.local/share/jiary` on Linux) holds `jiary.db`, `layout.conf`,
`settings.conf` and `jiary.lock`. `Db::open_at(path)` is the test seam: tests open a db in a
temp dir and never touch the real one.

## App state

`App` (`app/mod.rs`) holds, in groups:

| Fields | What they are |
|---|---|
| `db` | the only `Db` |
| `active_session`, `today_sessions` | the db, mirrored: the running session and today's (including it) |
| `activities`, `projects`, `project_order`, `tasks`, `previous_notes` | suggestions, project color order, and the active project/task's earlier notes, cached |
| `screen` | which screen is up, with its form state |
| `error` | the last db failure, shown in one place |
| `notice` | a short message for the status line and when it was given (`NOTICE_TIME`, 4 s) |
| `settings`, `settings_path` | the settings and where they are saved |
| `layout`, `layout_path`, `focus`, `today_scroll`, `notes_scroll`, `size`, `drag`, `title_press` | the main screen's geometry, focus, scrolling and mouse state |
| `reaction`, `caption_seed` | the cat's current reaction and the seed that picks its caption |
| `rating`, `undo` | the open rating window and the last change, for `u` |
| `away_since`, `last_observed` | absence detection |

**Memory mirrors the db.** `draw` runs about ten times a second and must
never query the db, so everything it reads is a field. Operations that
change sessions call `refresh_after_session_change` (reloads today's
sessions and the previous notes, resets the notes' scroll, picks a new
caption seed); `refresh_suggestions` reloads the suggestion lists after a
start. In-place edits (an interruption, a note) update the db and the
in-memory `Session` in the same handler. `previous_notes` leaves out the
active session itself, so it only changes when the session does.

**Errors are data.** Every db failure goes through `set_error`, every
success calls `clear_error`, and the status line shows `error` when set.
So the line always reflects the most recent db operation, and errors render
in one place.

**`size`** is the terminal size of the last frame (`main.rs` calls
`set_size` before each draw), so key and mouse handlers lay the main screen
out exactly as it was drawn.

**Startup.** `App::new` attempts every load (active session, today,
suggestions, layout, settings) and records failures with `set_error` rather than
stopping, so a partly readable db still shows what it can. Only an
unopenable db exits, in `main.rs`, before `App::new`.

## Screens

```rust
enum Screen {
    Main,
    Help,
    StartSession(Box<StartSessionForm>), // boxed: by far the largest
    AddNote(AddNoteForm),
    Palette(PaletteForm),
    Period(PeriodView),
    Rename(Box<RenameForm>),             // boxed too
    Mood(MoodForm),                      // sleep and mood
    Settings(SettingsForm),              // /settings
    Rewind(Box<Rewind>),                 // /rewind: every day, searched
    PastDay(PastDay),                    // a day opened from it
    Away(DateTime<Utc>),                 // back from an absence: since when
}
```

**A screen's state lives in its variant.** Opening a screen builds it
fresh; leaving sets `Screen::Main` and drops it. No mode flags and no
persistent widget state (`ListState` is rebuilt every draw), so all UI
state is plain data tests can build directly.

### Borrowing patterns

Three patterns keep the borrow checker happy; the compiler catches most
slips, but knowing them makes new handlers easier:

1. `handle_key` matches `&self.screen` with patterns that bind nothing
   (`Screen::StartSession(_) =>`), so the borrow ends before the arm calls
   a `&mut self` method.
2. Handlers read a form's step by copy (`Step` is `Copy`) before
   dispatching to the per-step handler, which then re-borrows the form
   mutably with `let-else`.
3. Values a `&mut self` call needs are copied out of the form first, since
   call arguments keep their borrow alive across the call:
   ```rust
   let (activity, project, task) = combo.clone();
   self.begin(&activity, project, task); // `form` is no longer borrowed
   ```

### Main

`handle_main` handles the session keys (see
[Session lifecycle](#session-lifecycle)), the section keys (`Tab`,
`Space`, `+`/`-`/`=`, scrolling; see [Main screen](#main-screen)), `?`
(help), `/` (palette) and `q`.

### Start form

Three steps, `Activity → Project → Task`, each an `Autocomplete` (see
[Fuzzy autocomplete](#fuzzy-autocomplete)) whose candidates are copied from
`App` when the form opens; the step handlers only intercept Enter and pass
other keys on.

- **Recent picks.** The form opens with the last `RECENT_PICKS` (5)
  distinct combinations that aren't detours (`db.recent_combos`). While the
  activity input is empty they replace the suggestions (`draw_recent`), and
  `1`–`5` starts one at once; once anything is typed, digits are text.
- **Activities** are free text. `App::activities` comes from
  `recent_activities` (most recent first, so one Tab repeats the last), or
  `DEFAULT_ACTIVITIES` on a fresh db. The defaults only seed: once there is
  history they aren't merged in, so unused ones drop out. An activity is
  required, so Enter on that step only advances with a value.
- Enter on the Project step rebuilds the task field's candidates: every
  task for a blank project, else `tasks_for_project`.
- Submitting (a pick, or Enter on Task) goes through `begin`: close the
  active session, stopping if that fails (creating anyway would hit the
  trigger's generic error instead of the real one), `start_session`,
  refresh, back to Main. `value()` turns blank input into `None`, so the db
  stores NULL, not `""`.

### Note form

One wrapped input. Enter appends the note (when not blank) and returns to
Main; Esc discards it.

### Rename form

`/rename` (`rename.rs`) opens `Screen::Rename(Box<RenameForm>)`, built on
the start form's pieces: the same modal layout, `Autocomplete` fields and
`draw_suggestions`, with tabs What · From · To (and Project before From
for tasks). `RenameStep` runs What → (Project →) From → To → Confirm:

- **What** picks a `db::Label` from `KINDS` with ↑↓ / Tab.
- **Project** (tasks only): task names repeat across projects, so a task is
  renamed within one. Blank means no project; NULL and `""` are one group
  in the db (`IFNULL(project, '')`).
- **From** takes only a name in use (`Autocomplete::offers`); **To** any
  non-blank name other than From. Candidates come from the db at each step
  (`labels_in_use`), with the detour activities left out.
- **Confirm** shows the counts (`db.count_label` for both names: a new
  name in use means a merge) before anything is written; Enter runs
  `db.rename_label`, one `UPDATE`. There is no undo, by design: the
  confirmation is the safety, and `u` keeps meaning the last start or end.

A refused Enter sets `notice`, shown in red in place of the list until the
next key: a name not in use, a blank or unchanged new name, or a detour
activity on either side. Detours are recognized by name (`is_detour`), so
renaming one, or renaming to one, would change what counts as a break,
disruption or meeting. After a rename `apply_rename` reloads everything
`draw` reads: the active session, today, the suggestions and project
order, the previous notes. Merged projects keep the earlier first use, so
colors can shift. Notes aren't touched.

### Command palette

`/` opens `Screen::Palette`; its input starts as the `/` itself, so
Backspace on a bare `/` closes it. `COMMANDS` lists every command with a
name, a description and its `Action`:

- `Action::Key`: a main-screen key, shown beside the name. Running it
  returns to Main and calls `handle_main` with that key, so a command can't
  drift from its key, and one can quit.
- `Action::View`: a view with no key of its own (`/weekly`, `/monthly`,
  `/yearly`), listed first; `Action::Rename` opens the rename form,
  `Action::Mood` the sleep-and-mood prompt, and `Action::MoodOn(on)` turns
  sleep and mood on or off (`/mood-on`, `/mood-off`). Names stay short: the
  longest one sets the column every description starts in.

`palette_matches` filters by fuzzy score on the name (everything, in order,
for an empty query; ties keep the order). The modal reuses the start form's
layout: a dimmed match count where the tabs go, the input, then the matches
as `name  description  key`.

### Weeks, months and years

`Screen::Period(PeriodView)` holds the period, its offset, its first and
last day, its sessions as `open_period` loaded them
(`db.sessions_between`), its days' sleep and mood (`db.days_between`,
as `ratings`), and its scroll. `handle_period`: ←→ / `h` `l`
reopen the period before or after (never past the current one), `w` `m`
`y` switch to this week, month or year, the scroll keys scroll, Esc / `q`
go back.

`period_lines` builds the view, grouping sessions by day once (a year has
365 summaries to make). Every view has the same order, so switching between
them doesn't move things around:

1. **Rows.** A week or month has a row per day (a month adds a blank row
   after each Sunday): the label (accent for today, dimmed with nothing
   tracked), the day's `timeline_row` on an hour axis all the days share
   (earliest start to latest end; 9–17 when empty), and `day_total`
   (tracked time, focus average) in `DAY_TOTAL_WIDTH` columns; then the
   axis's hours. A year has a row per month instead: its `activity_bar`
   scaled to the busiest month, and its `day_total`.
2. **Totals** over the period, from the same pieces as today's summary
   (`time_split_items` plus the days tracked and the average per day, the
   activity and project bars and items, `focus_items`). With sleep and mood
   on and any answers in the period, `Sleep` and `Mood` groups follow:
   `grouped_focus_items` averages the days' focus averages per answer,
   with the day count, and says "not enough data yet" under `MIN_DAYS` (5).
   Plain grouped averages only: no correlations or trends, since the data
   is small and noisy.
3. **The focus grid** and its legend (`focus_legend`), centered.
   `focus_grid` (week, month) has a row per weekday and a column per week,
   a `██` per day; `year_focus_grid` has a column per week of each month,
   splitting the week a month ends in so each month is its own block under
   its name, with a blank column after a month that ends on a Sunday
   (which has no split week; 70–71 columns for any year), and two days per character row
   (`▀`: the upper day as foreground, the lower as background), each row
   labelled with both days (`Mo Tu`, `We Th`, `Fr Sa`, `Su`). See
   [Colors](#colors). Both take a `cell` function giving each day's
   `DayCell` (`Rated(value)`, `Unrated`, `Empty`; `focus_cell` makes the
   focus one), so sleep and mood draw with the same code. When those show,
   their grids follow, titled: beside the focus grid in a week or a month
   (`side_by_side`, the weekdays only on the first), stacked under it in a
   year (the title in the header row, over the weekdays).

The running session counts through its in-memory copy. `draw_period`
frames the view in an accent box titled with the period, with a scrollbar
on the border.

### Sleep and mood

`s` goes through `start_or_ask` (`mood.rs`): with `settings.sleep_and_mood`
on and no `days` row for today, it opens `Screen::Mood(MoodForm)` with
`then_start`, and the start form (`open_start_form`) follows; otherwise the
start form opens at once. `/mood` opens the same form on today's stored
answers, without `then_start`.

`MoodForm` holds the date (fixed when it opens), the step (`Sleep`,
`Mood`), both answers and `then_start`. `1`–`3` answer and move on, Enter
keeps the answer, Backspace goes back to sleep, and the answer to mood
saves the day (`set_day`). Esc saves a skip (both `NULL`) under
`then_start`, and otherwise cancels without writing. Answers are colored
like focus ratings (`answer_color`).

`/mood-off` and `/mood-on` (`set_sleep_and_mood`) save `settings.conf` at
once and leave a `notice` in the status line. Off, `s` never asks, `/mood`
only says it's off, and the longer views leave sleep and mood out; the
`days` rows stay.

### Pomodoro

`settings.pomodoro` is a `PomodoroMode` (`Off`, `Soft`) and
`settings.target_minutes` its target (5–120, 25 by default). Everything
asks `App::pomodoro()` (`pomodoro.rs`), which gives the running session's
`Pomodoro { mode, progress }` (elapsed over target) and is `None` when off,
with nothing running, or for a detour. From it:

- `pomodoro_line`: a row right under the timer, `━` filled toward the
  target (accent, then green and full) and `─` for the rest, then the
  target. It adds a row to the pinned info, so the section's height
  follows.
- `timer_style`: the accent, green once `reached()`.
- `App::mood`: `Mood::BreakEarned` (the cat with a steaming coffee,
  captions in `BREAK_EARNED`) from the target on, in place of the doze at
  `DROWSY_AFTER_MINUTES`; with it off, nothing changes.

`p` (`toggle_pomodoro`, also `/pomodoro`) switches between `Off` and
`Soft` and leaves a notice; `/settings` cycles the modes and steps the
target. Nothing goes in the db: reaching the target isn't recorded.

**Adding a stricter mode.** A `Hard` mode (one that interrupts) would be a
new `PomodoroMode` variant, in `ALL` and `name()`; the settings file,
`/settings` and the notices already go through those. What it does at
the target would branch on `Pomodoro::mode`; as `draw` takes `&self`,
something that has to happen once (a bell, ending the session) would need
a tick that can mutate, such as the loop that calls `observe_idle`. `p`
would then go back to the last mode used rather than to `Soft`.

### Rewind

`/rewind` (`open_rewind`, `rewind.rs`) loads every session once
(`db.all_sessions`) into `Screen::Rewind(Box<Rewind>)`: the sessions,
every day with sessions newest first (`RewindDay`: date, session indices,
its `DaySummary`, the running session counted through its in-memory copy),
and the hours the rows' timelines share. Nothing after that touches the
db.

Each keystroke re-parses the input (`search::Query`) and recomputes
`found` (the matching days, each with its matching sessions and the note
line that matched; no sessions when the query has only dates) and
`matched` (their dates, for the grid), and resets the pick. `rewind_lines`
builds the list from that: a row per day (marker, date, `timeline_row`,
`day_total`), then the matching sessions. The grid on top is
`year_focus_grid` for the picked day's year, `focus_cell` on matched
days and `Empty` on the rest. `rewind_follow` keeps the picked day's rows
in view after every move. The input is a one-row `WrappedInput`; the
chrome asks `rewind_input` for it and for the cursor, since its width and
place aren't a modal's.

Enter moves the list into `Screen::PastDay(PastDay { rewind, scroll })`.
The day shown is the list's pick, so ←→ just move the pick (through the
matching days, with a query) and Esc moves the list back as it was.
`past_day_lines` is `summary_body_of` (the main screen's summary, for any
`DaySummary`) with the graphs, then the day's sessions as the main screen
draws today's (`timeline_item_lines`) with all their notes, projects
colored distinct across that day (`project_colors_of`).

### Settings screen

`/settings` opens `Screen::Settings(SettingsForm)` (`settings_screen.rs`):
a small modal with a row per `Setting` (`Pomodoro`, `Target`,
`SleepAndMood`), `label  value`. ↑↓ pick, Space / Enter / → step forward
(`next` mode, +5 minutes, toggle), ← back; each change saves at once
(`save_settings`). A new setting is a `Setting` variant, its label, value
and step.

### Away

`observe_idle(idle, now)`, fed by `main`'s idle poll, sets `away_since`
once the idle time reaches `AWAY_AFTER` (15 min), to `now - idle`; or, when
the gap since the previous poll (`last_observed`) is that long (the
computer slept), to that poll's time. Idle time under the threshold (or
none after a sleep) means you're back: `came_back` opens
`Screen::Away(since)` when the main screen is up and a non-detour session
that started before `since` is running. A key that arrives before the poll
notices also calls `came_back`, and only brings the question up.

- `k` / Esc: keep the time.
- `e`: `close_active_session_at(since)`, with the usual undo record and
  rating window.
- `b` / `d`: end at `since`, insert a finished break or disruption from
  `since` to now (`db.insert_finished_session`), and start the same
  activity, project and task again.

Detours never ask: being away from the computer is what they're for.

**Left running while closed.** `heartbeat(now, force)`, called every loop
by `main` (and forced once on exit), writes the time to `last_seen` at
most every `HEARTBEAT` (a minute). `App::new` reads it before the first
heartbeat overwrites it (`ask_if_left_running`): a running session that
began before then, with Jiary closed for `AWAY_AFTER` or more, opens
`Screen::Away(Away { since: last_seen, closed: true })`, which reads
"Closed since …". This one asks for detours too, offering only `k` and
`e` (`away_offers_detours`). No `last_seen` (the first run of 1.1.0, or
tests) asks nothing. Files beside the db reach `App::new` as `Files`
(layout, settings, last seen; `None` keeps one in memory).

`fmt_when` writes a time with its day when it isn't today ("yesterday
17:42", "Fri 17:42", "23 Oct 17:42"): the timer's "since" and the away
question use it.

### Help

`draw_help` lists every key in two columns of groups (sessions; sections
and mouse), each under an accent heading with a rule. Any key closes it.

## Session lifecycle

The helpers on `App` share one shape: **take the session out of memory,
write to the db, put it back on failure**, so a db error never leaves the
UI thinking a session ended.

- `start_session(activity, project, task)`: `db.create_session`; the
  returned `Session` becomes the active one, and its id joins the undo
  record.
- `close_active_session() -> bool`: completes the session unrated, starts
  the undo record and the rating window. Returns `true` when there was
  nothing to close or the close worked; on failure it restores the
  session, records the error and returns `false`, so callers stop.
- `append_active_note(note)`: appends in the db (`notes || ? ||
  char(10)`) and mirrors the text in memory.

Every way a session ends (`e`, a new start, a detour, `r`, the away
question) goes through `close_active_session`, so they all behave alike.

**Detours.** `d` / `b` / `m` call `detour(kind, needs_session)`: close the
active session and start a `DISRUPTION` / `BREAK` / `MEETING` with no
project or task. The same key during that detour does nothing; another
switches kind. `d` needs a running session; `b` and `m` also start from
nothing. **`r`** works when nothing runs or a detour does: it looks up
`latest_work_session` (the most recent completed non-detour) *first*, so a
detour keeps running when there's nothing to resume, then closes it and
starts that activity, project and task again. Known limit: the activity
alone decides what's a detour, so a meeting started with `s` is skipped by
`r` too; fixing it would mean recording what each detour interrupted (a
schema change).

**`i`**: `db.increment_interruptions` (guarded by `ended_at IS NULL`) and
the in-memory count.

**Rating window.** When the closed session `asks_focus`, `rating` holds
its id and the time. For `RATING_WINDOW` (5 s), `1`/`2`/`3` rate it (`rate`
→ `db.set_focus`) and play the cat's rating reaction. Short on purpose:
the rating given right away is the honest one. While it's open,
`resolve_layout` takes a row under the active section for `rating_bar` (an
`ACCENT` line draining from the right) and the caption becomes
`captions::RATING`. Starting from nothing (`b` right after `e`) leaves the
window open; closing another session replaces it.

**Undo.** `close_active_session` starts an `Undo` with the id it ended;
`start_session` (and the away question's inserted detour) add to its
`created`. `u` deletes the created sessions, reopens the ended one
(`db.reopen_session`: `ended_at` and `focus` back to NULL), and reloads.
Only the last change can be undone. `e`, `u`, `n`, `e` adds a last note to
a session just ended.

**Moving the start.** `[` / `]` (`move_start`) move the active session's
start by `START_STEP` (5 min), never past now. The previous session
(`db.previous_session`) limits it: if it ended within 2 s of this start (a
switch), its end moves too (`db.set_ended_at`), keeping at least a minute;
otherwise the start stops at its end, and since they then touch, the next
press moves both. Only today's list reloads (`reload_today`), so the
caption and scroll stay.

## Rendering

### Chrome

`chrome_layout` splits every frame into a header row, the content and a
status row; `draw` and `cursor_position` both use it. The rows never
change, so nothing jumps when an error comes and goes.

- **Header**: `JIARY`, the local date, and today's total (from memory:
  completed sessions plus the running one's elapsed time).
- **Status line**: the red `✗ <error>` when there is one, otherwise the
  current screen's key hints, the only place either is drawn. Hints show in
  order while they fit; once one doesn't, the rest of the optional ones go
  too. `HELP_HINT` always shows, with room kept for it; hints after it in
  the list only appear on wide terminals. On the main screen the hints
  follow the focused section.

### Main screen

Three stacked sections, split by `LayoutPrefs::resolve`:

- **Active.** Left, pinned: the session (`active_lines`: the timer and
  `since HH:MM`, then project, task, activity, interruptions as `theme::kv`
  rows; a dimmed hint with no session). Below it the notes (`note_lines`):
  the cached previous notes, then this session's, as bullets wrapped by
  `bullet_lines`, scrolled by `notes_scroll` with a scrollbar when they
  don't fit. Right, pinned: the cat, a gap and its caption
  (`split_for_cat`, `CAT_BLOCK_HEIGHT`), each caption row centered under the
  cat's own columns for the mood (`Mood::columns`), since the scenes sit
  off center. The cat is left out when the section is shorter than the
  sprite or the text would get narrower than 24 columns. The section is as
  tall as its taller column and never collapses.
- **Today.** One item per session, newest first, blank rows between: the
  time range ("now" while running), the duration, a focus badge (none for
  breaks and disruptions), the interruption count, a `project · task ·
  activity` line, and the notes. One `Paragraph` scrolled by `today_scroll`.
- **Summary.** `Day` / `Activity` / `Project` / `Focus` groups from
  `summarize`, as styled items (`items.rs`) joined by a dimmed ` · `,
  packed by `wrap_items` and continued under the value by `hanging_lines`.
  The `Day` row splits tracked time into work (`work_time`: all but breaks
  and disruptions), breaks and disruptions, leaving out parts that didn't
  happen.

  When `summary_graphs_fit` (the graphs' `GRAPH_ROWS`, while today's list
  keeps `TODAY_ROOM_FOR_GRAPHS` rows and values get `MIN_GRAPH_WIDTH`
  columns), graphs join in, with a blank row between groups:
  - under `Day`, the timeline from the hour the day began
    (`start_of_hour`) to now (`timeline_row`: each half cell in the color of
    the block at its middle), the focus line above it (`focus_row`: a `▀`
    per cell in the rating's color, dimmed when unrated, blank over breaks,
    disruptions and gaps), and the hours under it (`hour_labels`, dropped
    where they'd collide);
  - under `Activity` and `Project`, share bars (`share_row`: half cells by
    cumulative time, longest first).

  `half_cells` packs two colors per cell with `▌` (left as foreground,
  right as background), as the cat's `▀` does vertically. Without room, the
  summary is text only, so a short terminal neither squeezes today's list
  nor clips the summary.

**Sizing.** Active and Summary default to their content height
(`content_heights`) and Today takes the rest; a resize swaps the content
height for a fixed one. With Today collapsed, the summary fills instead.
When space runs out, the summary and then the active section shrink to
their 3-row minimum; on tiny terminals the summary is clipped before
Today.

**Focus and collapsing.** The focused section has the accent border; a
collapsed one is a single unbordered `▸` line with a short summary
(`draw_collapsed`).

**Scrolling.** The scroll keys act on `scroll_target()`: the notes when the
active section has focus, today's list otherwise. Positions are clamped in
the handlers (`scroll`, `max_scroll`, `viewport`) and again at draw time,
since content can shrink between frames. Adding a note scrolls to the
bottom so it's in view. `draw_scrollbar` is shared by the notes, today's
list and the longer views.

### Mouse

`handle_mouse` acts on the main screen and, for the wheel only, the
longer views (`scroll_period`, 3 rows a notch, clamped like the keys).
On the main screen it uses the last frame's geometry (`main_areas`):

- The line between two sections is two rows (one section's bottom border,
  the next one's top). A press on either starts a drag (`border_at`),
  remembering which row was grabbed so the border doesn't jump. The line
  under Active sets its height; the one under Today sets the summary's
  (while both are expanded). Drags clamp like `+`/`-`; release saves the
  layout.
- A press focuses the section under the pointer. On the title text of
  Today or Summary (`title_hit`, using the same `section_title` the
  renderer draws) it's recorded in `title_press`, and toggles collapse on
  release only if no drag came in between.
- The wheel scrolls the section under the pointer by 3 rows.

Mouse capture stops plain drags from selecting text; Shift-drag still
selects in most terminals.

### Layout file

`layout.conf`: `key=value` lines (`active_height`, `summary_height` as a
number or `auto`, `today_collapsed`, `summary_collapsed`). Parsing is
lenient (unknown keys, junk and too-small values fall back to defaults), so
a stale or hand-edited file never blocks startup. It's saved on every
collapse, resize, reset and drag release, through a temp file and a rename;
failures go to the error line and the in-memory layout carries on.

### Settings file

`settings.conf`, beside `layout.conf` and handled the same way: `key=value`
lines (`pomodoro=off|soft`, `pomodoro_target`, `sleep_and_mood`), lenient
parsing (an unknown mode, say from a newer Jiary, leaves the default),
saved through a temp file and a rename whenever a setting changes. A
missing file means the defaults (Pomodoro off, 25 minutes, sleep and mood
on).

### Modals and inputs

The start form, the note form, the rename form, the palette and the away
question are centered modals, `MODAL_WIDTH` (40) wide:

- `centered_rect` centers a size in the content area and shrinks it to fit
  tiny terminals. `modal_layout` gives the modal and its inner area (the
  border plus a column of padding each side).
- `start_modal_layout(content, input_rows)` (tabs, input, list;
  `START_MODAL_HEIGHT` 12, also the palette's and the rename form's) and
  `small_modal_layout(content, body_rows)` (a context row, then the body;
  `SMALL_MODAL_HEIGHT` 6) grow by a row for each input row past the first.
  The renderers and `cursor_position` share them, so they can't disagree
  about where the input is.
- Each modal is a rounded `Block` with an accent title. The start form adds
  a `Tabs` row marking the step.

Every text input is a `WrappedInput`, built by `App::wrapped_input` from
`text_input()` (the screen's value and placeholder) and
`modal_inner_width`:

- No label (the tabs or context row name the field): a bold accent `› `
  prompt (`theme::PROMPT`), later rows indented 2 to line up.
- Empty, it shows a dimmed italic placeholder (`theme::PLACEHOLDER`) with
  the cursor on its first character.
- `wrap_text` breaks after the last space that fits (mid-word only for
  words longer than a row) and keeps every character. Widths count
  characters, not bytes. A full last row gets an empty one after it, so the
  cursor has somewhere to go.
- The cursor is always after the last character. If the text needs more
  rows than the modal can grow to, only the last rows show.

Picker lists (suggestions, recent picks, commands) use
`theme::picker_symbol()` (a bold accent `>`), `PICKER_HIGHLIGHT` and
`HighlightSpacing::Always`, so items don't shift when one is selected, and
are indented a column so their text sits under the typed text.

### Terminal cursor

Hidden by default. After each draw, `main.rs` asks
`app.cursor_position(size)`, a pure function of the size and the form
state, and shows the cursor only when it returns `Some`.
`WrappedInput::cursor_in` turns the input's cursor into a screen position,
clamped to the input area. Two quirks shape this:

1. `Terminal::draw` discards the closure's return value, so the position
   is computed after drawing, by a separate method.
2. `set_cursor_position` moves the cursor but doesn't show it, hence the
   explicit show and hide in the loop (pinned by
   `cursor_position_tracks_input_lines`).

### Colors

`theme.rs` holds every color and shared style:

- **Named colors** for the UI: `ACCENT` (cyan), `MUTED`, `GOOD` / `OK` /
  `BAD` (green, yellow, red), `UNRATED`; the activity colors
  (`activity_color`: work, meeting, break, disruption); the cat's
  `CAT_*`. With the 16 named colors the UI follows the user's terminal
  theme.
- **256-color hues** where 16 aren't enough: `PROJECT_PALETTE` (16 hues,
  the first 8 the most distinct, all apart from the activity, focus and
  accent colors; `NO_PROJECT` gray), `FOCUS_RAMP` (9 steps from red through
  yellow to green over 1–3, via `focus_gradient`) and `EMPTY_DAY`. No
  truecolor anywhere.
- **Styles**: `TITLE`, `DIMMED`, `VALUE`, `TIME`, `NONE`, `ERROR`,
  `PROMPT`, `PLACEHOLDER`, `CAPTION`, and helpers `kv`, `activity`,
  `focus_badge` (` F1 `–` F3 ` on red, yellow or green, ` F- ` dimmed),
  `picker_symbol`.

The rule: labels and words dimmed or plain, values bold, times bold
accent, activities and projects in their colors, missing values dimmed
italic. Focus averages are colored on the same ramp as the grids
(`focus_color`), so a number matches its day's square.

**Project colors** follow the order projects were first used
(`db.projects_by_first_use`, in `App::project_order`), so a project keeps
its color from day to day: its rank picks a preferred palette color. Each
view then makes its projects distinct (`theme::project_colors`): when two
would share a color, the one used first keeps it and the other takes the
next free one. First use, not time, decides, so colors don't swap as the
day goes on. The main screen does this over today's projects
(`project_color`), the longer views over the period's.

### ratatui gotchas

Learned the hard way; render tests pin them.

- `Tabs`' default divider is `│` with a space on each side
  (` Activity │ Project │ Task`).
- `List` content sits one column right of the highlight symbol (`>alpine`).
- Borders render in the default color unless given a style: only the
  focused main-screen section and the longer views' frame set the accent.
- `Block::bordered` adds no inner padding; `modal_layout` adds its own.

## Fuzzy autocomplete

`fuzzy_filter(query, candidates)`: an empty query gives every candidate in
order (so suggestions show as soon as a field opens); otherwise
`SkimMatcherV2` subsequence matches, best score first, at most 5.

`Autocomplete` holds `input` (the value), `query` (what was typed, which
suggestions filter on), `selection: Option<usize>` and its `candidates`.
Its invariant: **the input line shows exactly what Enter submits.**

- Tab / ↓ with nothing selected pick the best match (one Tab never skips
  it; pinned by `tab_accepts_best_match_first`); then Tab cycles with
  wrap-around and ↓ stops at the end.
- ↑ moves back; past the top it clears the selection and restores the
  typed text.
- Selecting copies the suggestion into `input`. Typing or Backspace edits
  `input`, sets `query` to it and clears the selection, so typing after
  accepting a suggestion extends it.
- Free text is always allowed; `value()` trims and turns blank into
  `None`.

## Testing

Unit tests at the bottom of each module, and the UI's in `app/tests/`:
the harness in `mod.rs`, then a file per screen or area (`forms.rs`,
`main_screen.rs`, `period_view.rs`, …), each starting with `use super::*;`.
A helper used by more than one file lives in `mod.rs`.
No integration-test directory and no shared fixture: each test opens its
own throwaway db.

**Temp dirs.** Tests run in parallel in one process, so each module's temp
dirs have their own prefix (`jiary-test-` in `db.rs`, `jiary-app-test-` in
`app/tests/`) plus the pid and an atomic counter, removed on `Drop`.
Shared dirs would let one test delete another's live db. In `TestApp`,
`app` is declared before `dir` so the connection drops before the dir is
removed.

**What's tested where:**

- *`db.rs`*: SQL behavior: the trigger, duplicate active rows, timestamp
  formats (`+00:00`, legacy `Z`), bad timestamps, today's local-midnight
  boundary, same-second ordering.
- *`layout.rs`, `summary.rs`, `mascot.rs`, `captions.rs`, `period.rs`*:
  pure logic: section heights for every collapsed and resized combination
  and tiny areas, the layout file's round trip and junk tolerance, summary
  totals and focus figures, frame schedules, date ranges.
- *`app/tests/`, behavior*: keys against a `TestApp` (a real db): key
  filtering, autocomplete, the forms, detours and resume, rating and undo,
  moving the start, the away question, the palette, the longer views'
  paging, section focus, collapse, resize and scroll, the mouse (`mouse`,
  `drag`, `click`), layout persistence across two `App`s. Keys go through
  `press(code)`, `TestApp::key` and `TestApp::typed`; helpers like
  `on_activity_step`, `on_project_step` and `on_screen` put the app straight
  on a screen. Pushing sessions into `today_sessions` (or
  `with_sessions(t, n)`) is the right level for what `draw` reads from
  memory.
- *`app/tests/`, rendering*: a `TestBackend` at a fixed 80×24
  (`RENDER_WIDTH` × `RENDER_HEIGHT`, also `TestApp`'s size, so handlers see
  the same geometry), drawn by `render(&app)`, read with char-based
  helpers (`line`, `line_from`, `col_of`, `row_with`), since rows hold
  multi-byte glyphs. `running_session` and `finished_session` make
  time-dependent output deterministic. `every_screen_survives_tiny_terminals`
  draws every screen at tiny sizes.

When adding a screen or widget, add a render test pinning its layout and
colors, and keep 80×24: wide enough for every status line, tall enough that
the chrome never overlaps content.

**The docs' pictures.** `app/screenshot.rs` holds ignored tests that fill
a throwaway db (a made-up day, or about two years of seeded history
written in one transaction), draw a screen into a `TestBackend` and write
the buffer to `docs/images/*.svg`: block and box-drawing characters become
shapes, each word is its own `<text>` at its column, stretched to its exact
width (viewers may collapse runs of spaces, so the gaps can't be spaces),
and the 16 named colors use a fixed dark palette. `cargo test screenshot -- --ignored`
regenerates them.

## Design decisions

- **The db is the source of truth, memory a cache.** Every change writes to
  the db first and mirrors on success; startup rebuilds from the db. So
  copying the `.db` file is a complete backup, and a crash loses at most
  the fact that a session was still running.
- **Fail visibly, not fatally.** Only an unopenable db exits; every other
  failure lands on the red status line and the app carries on with what it
  has.
- **No managed objects.** Projects, tasks and activities are strings, and
  history is their only list, which keeps the sessions to one table.
- **Sleep and mood stay two fixed questions.** No custom fields: that leads
  to a habit tracker, which Jiary isn't. They're asked once a day, never
  with a reminder, and can be turned off.
- **Named colors first.** The UI follows the terminal's theme; 256-color
  hues only where 16 aren't enough, no truecolor.
- **No system dependencies.** Bundled SQLite. Idle time comes from a
  command when there is one (`ioreg` on macOS, `xprintidle` on Linux if
  installed) and is simply unavailable otherwise. No X11 or Wayland
  integration, which also rules out in-process global hotkeys (backlog F).
