# Implementation details

Developer reference for how jiary works internally. Read this before changing
code. User-facing behavior (keys, data model, backup) lives in `README.md`;
current session state, the backlog, and working conventions live in
`DEVELOPMENT.md`. This document explains *how* and *why*; keep it in sync when
architecture changes.

## Big picture

Single binary, single thread, no network, no system dependencies (SQLite is
bundled). One SQLite file is the entire persistence layer; everything else is
transient UI state that is rebuilt from the db on startup.

```
main.rs        event loop: size → draw → place cursor → poll(100 ms) → key/mouse
  └─ App       all UI state + a Db handle          (app.rs)
       ├─ Db   rusqlite connection; all SQL lives here (db.rs)
       ├─ Session  one db row with parsed timestamps (session.rs)
       ├─ LayoutPrefs  main-screen section geometry + layout file (layout.rs)
       ├─ summarize  day totals over in-memory sessions (summary.rs)
       └─ theme  palette + shared style helpers    (theme.rs)
```

The loop runs at ~10 fps: `event::poll` times out after 100 ms, so the timer
in the active-session panel ticks even when no key is pressed. Key *presses*
and mouse events are processed: key releases and repeats (reported by some
terminals, e.g. on Windows) and all other crossterm events are ignored, and
Ctrl combinations never reach a form as plain letters. Quitting is a return value,
not a state: `App::handle_key` returns `true` for `q` on the main screen or
Ctrl+C anywhere, which breaks the loop in `main.rs`.

Data flow is one-way per frame: **state → pixels**. `draw` never mutates and
never touches the db (it runs ~10×/sec). Key handlers mutate state and hit the
db; the next frame renders the result. There is no event bus, no async, and no
shared state — `Db`'s `Connection` is owned by `App` alone.

## Module map

- **`main.rs`** — owns the terminal lifecycle: `ratatui::init()` (raw mode +
  alternate screen + a panic hook that restores them, chained with one that
  re-shows the cursor and releases the mouse), mouse capture,
  `ratatui::restore()` on every other exit path, cursor show/hide, and the
  loop. It knows nothing about UI; it calls `app.set_size`, `app.draw`,
  `app.cursor_position`, `app.handle_key`, `app.handle_mouse` and nothing
  else. It also passes `App::new` the layout file path (`layout.conf` in the
  data dir).
- **`layout.rs`** — `LayoutPrefs` (section heights and collapsed state) and
  pure geometry: `resolve` splits the content area into the three main-screen
  sections, `resize` / `set_*_height` move borders with clamping, plus the
  lenient `key=value` layout file (`parse`, `load`, `save`). No rendering.
- **`summary.rs`** — `summarize(today, active, now) -> DaySummary`: time per
  activity and project, focus, interruptions, disruptions, span and gaps.
  Pure; computed from in-memory sessions every frame.
- **`db.rs`** — `Db` wraps a `rusqlite::Connection`. The schema (including the
  single-active-session trigger) is a `const SCHEMA` string applied with
  `execute_batch` on every open, so opening an existing db also migrates it.
  All SQL in the project lives in this file. Unit tests at the bottom.
- **`session.rs`** — `Session`: a plain data struct mirroring one db row, with
  timestamps already parsed to `DateTime<Utc>`. Nothing else.
- **`theme.rs`** — the 16-color palette (`ACCENT`, `MUTED`, `GOOD`, `OK`,
  `BAD`) and shared styles (`TITLE`, `DIMMED`, `ERROR`, `kv()`,
  `picker_symbol()`). All widgets take their styles from here so the look is
  tweaked in one place. No truecolor, so the app looks decent on any terminal.
- **`app.rs`** — everything else: `App` state, the `Screen` enum and forms,
  per-screen key handlers, draw methods, session lifecycle helpers,
  `fuzzy_filter`, layout helpers, and the test harness. Unit tests at the
  bottom.

## Data layer (`db.rs`)

### Schema

One table, `sessions`:

```sql
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    started_at TEXT NOT NULL,   -- RFC 3339 UTC, e.g. 2026-09-04T09:15:00+00:00
    ended_at TEXT,              -- NULL while active
    project TEXT,               -- free text, NULL when unset
    task TEXT,                  -- free text, NULL when unset
    activity TEXT NOT NULL,
    notes TEXT,                 -- newline-terminated entries appended over time
    focus INTEGER,              -- 1..3, set on completion
    interruptions INTEGER       -- micro-interruption counter
);
```

There are no other tables by design: projects and tasks are free-text labels,
not managed objects. "Lists" of projects/tasks are `SELECT DISTINCT` queries
over history (`distinct_projects`, `distinct_tasks`, `tasks_for_project`).

### Single active session — trigger, not constraint

At most one row may have `ended_at IS NULL`. This is enforced by a `BEFORE
INSERT` trigger that raises `ABORT`, **not** by a UNIQUE index, because
SQLite's UNIQUE constraints ignore NULL values (a UNIQUE index on `ended_at`
would allow unlimited active rows). Consequences:

- A second `create_session` while one is active fails with
  "another session is already active". The UI never relies on this — it closes
  the active session first and aborts if that fails (see lifecycle below) —
  but the trigger is the backstop against external edits.
- `get_active_session` additionally does `ORDER BY started_at DESC LIMIT 1`,
  so legacy/externally-created duplicate active rows degrade gracefully to the
  most recent one instead of being ambiguous.

### Timestamps

Stored as RFC 3339 UTC with an explicit `+00:00` offset. Rows written by
older versions end in `Z` instead; SQLite's date functions parse both, so
`date(started_at, 'localtime')` places either on the right local day (pinned
by `legacy_z_suffixed_timestamps_still_count_as_today`).
Timestamps have one-second resolution, so every `ORDER BY started_at` also
orders by `id` to break same-second ties (most recent insert wins).
"Today" is always the *local-time* start date:
`date(started_at, 'localtime') = ?`. Display converts to local time in the UI
(`fmt_time`), never in SQL.

### Row mapping indirection

rusqlite row-mapper closures can only report `rusqlite::Error`, but turning a
row into a `Session` requires parsing timestamps, which can fail in a way that
should surface as `DbError::BadTimestamp`. So queries select
`SESSION_COLUMNS`, map rows with `read_session_row` into an internal
`SessionRow` (timestamps still `String`), and `row_to_session` does the
parsing outside the mapper. Any new query returning sessions must follow this
two-step pattern.

`create_session` returns the `Session` it inserted (built from the same
timestamp string it stored), so the in-memory mirror never has its own clock
reading.

### Data directory

`dirs::data_dir()/jiary/jiary.db` (`~/.local/share/jiary` on Linux,
`~/Library/Application Support/jiary` on macOS), created with
`create_dir_all` on open. `Db::open_at` is the test seam: tests open a db at a
temp path and never touch the real data dir.

## App state (`app.rs`)

```rust
pub struct App {
    db: Db,
    active_session: Option<Session>,   // mirrored from db
    today_sessions: Vec<Session>,      // mirrored from db (today, incl. active)
    projects: Vec<String>,             // distinct history, refreshed after session changes
    tasks: Vec<String>,                // same
    screen: Screen,                    // which screen + its form state
    previous_notes: Vec<String>,       // cached "recent notes" for the active project/task
    error: Option<String>,             // last db failure, rendered in exactly one place
    // main screen
    layout: LayoutPrefs,               // section sizes + collapsed state (persisted)
    layout_path: Option<PathBuf>,      // where `layout` is saved; None in tests
    focus: Section,                    // section that Space and +/- act on
    scroll: usize,                     // first visible row of today's sessions
    size: (u16, u16),                  // terminal size as of the last frame
    drag: Option<Section>,             // section whose border the mouse is dragging
}
```

`size` exists so key and mouse handlers can lay the main screen out exactly
as the last frame did (`main_areas`): main.rs calls `set_size` before every
draw. `scroll` is clamped against the content in the handlers
(`scroll_today`, `max_scroll`) and again at draw time, since the content can
shrink between frames.

**The UI mirrors the db in memory.** `draw` runs ~10×/sec and must never query
the db, so everything it needs is a field on `App`. The mirror is refreshed by
`refresh_after_session_change` (reloads `today_sessions` + `previous_notes`),
called after every operation that changes the active session. In-place edits
(interruption count, appended notes) update both the db and the in-memory
`Session` in the same handler.

`previous_notes` is a cache with a deliberately narrow invalidation rule: the
query excludes the active session itself, so the list is static *during* a
session and only needs refreshing when the session changes.

`error` is plain data (`Option<String>`), not a callback or event: every db
failure goes through `App::set_error`, every success calls `clear_error`, and
the status line renders it (or the key hints when `None`). This gives one
render site for errors and makes "the line reflects the most recent db
operation" fall out of the design.

### Startup

`App::new` attempts **every** load — active session, today's sessions,
projects, tasks — recording failures via `set_error` instead of aborting, so a
partially readable db still shows partial data. The only condition that exits
the process is an unopenable database (handled in `main.rs` before `App::new`).

## Screen state machine

```rust
enum Screen {
    Main,
    StartSession(Box<StartSessionForm>),  // boxed: by far the largest variant
    EndSession(EndSessionForm),
    AddNote(AddNoteForm),
}
```

**Form state lives inside the `Screen` variant.** Entering a screen constructs
a fresh form; leaving (Esc, Enter, …) sets `screen = Screen::Main` and the
form is dropped. There is no separate "form mode" flag and no persistent
widget state — the activity index is a plain `usize`, autocomplete selections
are `Option<usize>` inside each `Autocomplete`, and `ListState` is
rebuilt from scratch on every draw via `render_stateful_widget`. All UI state
is therefore plain data that tests can construct directly.

### Key dispatch and the borrow dance

`handle_key` matches `&self.screen` with **unit patterns** (`Screen::Main =>`,
`Screen::StartSession(_) =>`) so nothing is bound and the shared borrow drops
before the arm body calls `&mut self` methods:

```rust
match &self.screen {
    Screen::Main => return self.handle_main(key),
    Screen::StartSession(_) => self.handle_start_session(key),
    ...
}
```

Per-screen handlers that need the current step read it **by copy** (`Step` and
`EndStep` are `Copy`) so the borrow drops before dispatching to the per-step
handler:

```rust
let step = match &self.screen {
    Screen::StartSession(form) => form.step,
    _ => return,
};
match step { Step::Activity => self.start_activity_key(key), ... }
```

Per-step handlers then re-match with `let-else` to get `&mut` access to the
form. And whenever a handler must call an `&mut self` method *while holding
form data*, it **copies the values out first** — call arguments keep their
borrow alive across the call:

```rust
let Some(activity) = form.activity.value() else { return };
let project = form.project.value();
let task = form.task.value();
...
self.start_session(&activity, project, task);  // borrow of `form` is dead here
```

These three patterns are load-bearing; the compiler rejects most violations,
but knowing them makes new handlers much easier to write.

### Start form

Steps: `Activity → Project → Task` (the tabs row shows progress).

- **Activity / Project / Task** — each an `Autocomplete` (see *Fuzzy
  autocomplete*); the step handlers only intercept Enter and forward every
  other key to it. The activity and project fields' candidates are copied from
  `App::activities` / `App::projects` when the form opens; one shared
  `draw_suggestions` renders every step's list.
- Activities are free text. `App::activities` comes from `recent_activities`
  (distinct, most recently used first, so one Tab repeats the last one), or
  `DEFAULT_ACTIVITIES` when the db has none yet. The defaults are only a seed:
  once there is history they are not merged in, so unused ones drop out.
- Activity is required (`activity` is `NOT NULL`): Enter on the Activity step
  advances only when `value()` is `Some`.
- On Enter at the Project step, the task field is rebuilt with its
  candidates: all known tasks if the project is blank, else
  `tasks_for_project(project)` (a failure goes to the error line).
- On Enter at the Task step: copy out activity/project/task (`value()` maps
  blank input to `None`, so the db stores NULL not `""`), close any active
  session first (aborting on failure — creating anyway would hit the
  trigger's generic error instead of the real one), `start_session`, refresh,
  `refresh_suggestions`, return to Main.

### End form

Steps: `Notes → Focus`.

- Notes: Enter advances; the typed notes stay in the form.
- Focus: `1`/`2`/`3` or Enter (no rating) commits: append the notes (if
  non-empty), then `complete_active(focus)`.
- Esc at either step cancels: back to Main with the session still running and
  nothing written, so an accidental `e` is harmless.

### Add-note form

Single (wrapping) input; Enter appends (if non-empty) and returns to Main, Esc
discards the input. Saved notes persist even if the app exits mid-session.

## Session lifecycle

All lifecycle helpers live on `App` and follow one shape: **take the session
out of memory, hit the db, restore it on failure** so a db error never leaves
the UI believing a session ended.

- `start_session(activity, project, task)` — `db.create_session`, whose
  returned `Session` becomes the active one.
- `complete_active(focus)` — `db.complete_session(id, focus)`, restore on
  error, then always `refresh_after_session_change`.
- `close_active_session() -> bool` — completes with no rating. Returns `true`
  when there was nothing to close or the close succeeded; on failure it
  restores `active_session`, records the error, and returns `false` so callers
  can abort (the start form uses this to avoid creating a second active
  session).
- `append_active_note(note)` — appends in the db (`notes || ? || char(10)`)
  and mirrors the newline-terminated text in memory.

Main-screen flows built from these:

- **`d` (disrupt)** — only if a non-Disruption session is active: close it,
  start a `Disruption` session with no project/task. During a Disruption it
  is a no-op rather than splitting it in two.
- **`r` (resume)** — proceeds only when there is no active session or the
  active one *is* a Disruption. Looks up `latest_non_disruption` (most recent
  completed non-Disruption row's activity/project/task) **first**, so the
  Disruption keeps running when there is nothing to resume; then closes it
  and starts a fresh session with that combo. The round trip `d` … `r` is two
  keystrokes.
- **`i`** — `db.increment_interruptions` (guarded by `ended_at IS NULL` in
  SQL) plus the in-memory counter bump.

## Rendering

### Global chrome

`draw` splits the frame into three fixed rows: header (1), content (rest),
status line (1). The rows are unconditional — no conditional layout — so the
UI never jumps when an error appears or disappears.

- **Header** — `JIARY` (bold accent) · local date · total time today (bold) ·
  "today". The total is computed from in-memory sessions only: completed
  durations plus the active session's elapsed time.
- **Status line** — the red `✗ <error>` when `error` is set, otherwise the
  key hints for the current screen. This is the *only* place errors and hints
  are rendered.

### Main screen

Three stacked sections, split by `LayoutPrefs::resolve` (`layout.rs`):

- **Active** (` ACTIVE `) — bold timer, dimmed label/value rows via
  `theme::kv`, interruptions red when > 0, cached previous notes, then the
  session's own notes as bullets; a dimmed hint when there is no session.
  Never collapses.
- **Today** (` ▾ TODAY · N sessions `) — timeline items, one per session
  (completed and the running one), newest first, separated by blank rows:
  bold time range + duration, a focus badge (`F1` red / `F2` yellow / `F3`
  green / `F-` dimmed), an interruption count (`I:N`, red when > 0), a bright
  `project · task · activity` line, and dimmed note bullets. Drawn as one
  `Paragraph` scrolled by `scroll`, with a `Scrollbar` on the right border
  when the items don't fit.
- **Summary** (` ▾ SUMMARY `) — `summary_lines`: `Activity` / `Project` /
  `Focus` / `Day` rows from `summarize`, items joined by ` · ` and packed by
  `wrap_items` (an item only splits when wider than a whole row), continuing
  under the value via `hanging_lines`.

Sizing: Active and Summary default to their content height
(`content_heights`), Today fills the rest; a fixed height replaces the
content height once the user resizes. When Today is collapsed the summary
fills instead; when both are collapsed the spare rows stay empty. When space
runs out, the summary and then the active section shrink to their 3-row
minimum, and on tiny terminals the summary is clipped before Today.

Focus is shown by the border color (accent vs dimmed). A collapsed section
is one unbordered `▸` line with a short summary (`draw_collapsed`).

Status-line hints depend on the focused section and are fitted to the
terminal width: hints are added in priority order while they fit, and
`[?] help` is always last. The `?` help screen (`Screen::Help`,
`draw_help`) lists every key; any key closes it.

### Mouse

`handle_mouse` acts only on the main screen, using the same `main_areas`
geometry as the last frame:

- The line between two sections is two rows (the upper section's bottom
  border, the lower one's top border); a left-press on either starts a drag
  (`border_at`), remembering which row was grabbed so the border follows the
  pointer without jumping. The line under the active section sets its
  height; the one under today's sessions sets the summary's (only while both
  are expanded). Drag events apply the same clamping as `+`/`-`; release
  saves the layout.
- A left-press focuses the section under the pointer. On the title text of
  Today or Summary (`title_hit`, using the same `section_title` the
  renderer draws) it is recorded in `title_press`, and toggles collapse on
  release only if no drag event came in between, so a drag that starts on a
  title (which sits on a border row) never collapses anything.
- The wheel scrolls today's sessions by 3 rows.

Mouse capture means terminals no longer select text on a plain drag;
Shift-drag still selects in most of them.

### Layout file

`layout.conf` in the data dir, `key=value` lines (`active_height`,
`summary_height` as a number or `auto`, `today_collapsed`,
`summary_collapsed`). Parsing is lenient (unknown keys, junk and values below
the minimum height fall back to defaults) so a stale or hand-edited file
never blocks startup. It is saved on every collapse, resize, reset and drag
release, via a temp file plus rename; a read or write failure goes to the
error line and the app carries on with the in-memory layout.

### Modals

The start/end/add-note screens are centered modals of fixed width that grow
in height only with a wrapped input:

- Start: 40×12 (`MODAL_WIDTH` × `START_MODAL_HEIGHT`) — room for the activity
  list. End/note: 40×6 (`SMALL_MODAL_HEIGHT`). Each grows by one row per
  input row beyond the first, so the suggestion list (start) and the two
  spare rows (end/note) keep their room.
- `chrome_layout` splits the frame into header / content / status rows; both
  `draw` and `cursor_position` use it, so modals are centered in the same
  content area.
- `centered_rect` centers a fixed size in the content area and **shrinks to
  fit** on tiny terminals instead of overflowing.
- `modal_layout` returns (modal rect, inner area) where the inner area is the
  block's interior with one extra column of padding left and right.
- `start_modal_layout(content, input_rows)` (tabs / input / list) and
  `small_modal_layout(content, body_rows)` (tabs-or-context / body) extend
  that. They are **shared by the renderers and `cursor_position`** so the two
  can never disagree about where the input is.

### Wrapped inputs

All five text inputs (Activity, Project, Task, Notes, Note) are drawn as a
`WrappedInput`, built by `App::wrapped_input` from `text_input()` (the
current screen's label and value) and the modal's inner width
(`modal_inner_width`):

- Row one is `theme::kv(label, …)`; the value continues on later rows with a
  hanging indent of `label + 2` columns, so it reads as one column.
- `wrap_text` breaks after the last space that fits (mid-word only for words
  longer than a row) and keeps every char, so the rows concatenate back to
  the value. Widths count chars, not bytes. A full last row gets an empty
  row after it so the cursor has somewhere to go.
- The cursor is always after the last char, i.e. on the last row. If the
  text needs more rows than the modal can grow to on this terminal, only the
  last rows are kept, so the cursor stays visible.

Modal structure: a `Block::bordered()` (rounded) with a titled accent span, a
`Tabs` row marking the current step, then the step body (a wrapped input plus
suggestion `List`, or the rating line). Picker lists use `theme::picker_symbol()`
(bold accent `>`) and `PICKER_HIGHLIGHT`.

### Terminal cursor

The cursor is hidden by default (`main.rs` hides it at startup). After each
draw, `main.rs` asks `app.cursor_position(size)` — a pure function of terminal
size + form state — and shows the cursor only when it returns `Some`, placing
it right after the typed text. Two ratatui/crossterm quirks drive this design:

1. `Terminal::draw` discards closure return values, so the position is
   computed *after* drawing via a separate method, not inside the draw
   closure.
2. `set_cursor_position` moves the cursor but does **not** show it — hence the
   explicit `show_cursor()`/`hide_cursor()` in the loop (regression-tested by
   `cursor_position_tracks_input_lines`).

`WrappedInput::cursor_in` turns the wrapped input's relative cursor into a
screen position, clamped to the input area on terminals narrower than the
modal.

### ratatui gotchas (learned the hard way; render tests pin these)

- `Tabs`' default divider is a box-drawing `│` **with space padding** on both
  sides (` Activity │ Project │ Task`).
- `List` content always sits **one column right of the highlight symbol**
  (`>alpine`, no space).
- Modal borders render in the **default color**; only titles carry the accent.
  The main screen's active panel is the unique widget with an accent border.
- Panels render inner text **at the border with no padding** — `Block::bordered`
  adds no interior padding; `modal_layout` adds its own one-column pad.

## Fuzzy autocomplete

`fuzzy_filter(query, candidates) -> Vec<String>`:

- Empty query → all candidates in original order (so suggestions appear as
  soon as the field is active).
- Otherwise: subsequence match via `SkimMatcherV2`, sorted by score descending,
  capped at **5** results.

`Autocomplete` (activity, project and task steps) holds `input` (the value), `query`
(what was typed; suggestions filter on it), `selection: Option<usize>` and its
`candidates`. Its invariant: **the input line shows exactly what Enter
submits.**

- Tab/↓ with nothing selected select the best match (a single Tab never skips
  past it — pinned by `tab_accepts_best_match_first`); after that Tab cycles
  with wrap-around and ↓ stops at the end.
- ↑ moves back; past the top it clears the selection and restores the typed
  text.
- Selecting copies the suggestion into `input`; nothing is highlighted until
  something is selected.
- Typing/Backspace edit `input`, set `query = input` and clear the
  selection, so typing after accepting a suggestion extends it.
- Free typing is always allowed; `value()` trims and maps blank to `None`.

(Performance nit: `fuzzy_filter` constructs a `SkimMatcherV2` per call, i.e.
per frame; see DEVELOPMENT.md nits.)

## Testing

All tests are plain unit tests at the bottom of their module; there is no
integration-test directory and no shared fixture — each test opens its own
throwaway db.

**Temp dirs.** Parallel tests share one process, so each test module uses a
unique prefix (`jiary-test-` in db.rs, `jiary-app-test-` in app.rs) plus the
pid and an atomic counter, with `Drop` cleanup. A shared directory would let
one test delete another's live database. In `TestApp`, the `app` field is
declared before `dir` so the `Db` connection drops before the directory is
removed.

**Levels of test:**

- *db.rs* — SQL behavior: trigger enforcement, duplicate-active tolerance,
  timestamp format (`+00:00` suffix, legacy `Z` rows), bad-timestamp error
  path, today query and its local-midnight boundary, same-second ordering.
- *layout.rs / summary.rs* — pure: section heights for every
  collapsed/resized combination and small or tiny areas, resize clamping,
  layout file round-trip and junk tolerance; summary totals, ordering, the
  running session's in-memory copy, focus/disruption figures.
- *app.rs state machine* — key handling against a `TestApp` (real db):
  key-event filtering, autocomplete semantics, end/note form commit and
  cancel, disrupt/resume edge cases, section focus/collapse/resize/scroll,
  mouse drag/click/wheel (`mouse`, `drag` helpers), layout persistence
  across two `App`s, status-line content and width fitting. Keys go through
  `press(code)` / `TestApp::key` / `TestApp::typed`. Setup helpers
  (`on_project_step`, `on_activity_step`, `on_screen`, `empty_form`) put the
  app directly on a screen/step; pushing sessions straight into
  `today_sessions` (or `with_sessions(t, n)`) is the right level for tests of
  things `draw` reads from memory. `TestApp` sets the 80×24 render size so
  handlers see the same geometry as `render`.
- *app.rs rendering* — a `TestBackend` at a fixed 80×24, drawn once via
  `render(&app) -> Buffer`, asserted with char-based row extractors:
  `line(buf, y)` (row text, trailing padding trimmed), `line_from(buf, y, x)`
  (row from char column `x`), `col_of(buf, y, needle)` (char column of first
  occurrence). Char-based because rows contain multi-byte glyphs (`│ – · ✗`).
  Fixtures `running_session(elapsed, interruptions)` and
  `finished_session(start, end, focus)` make time-dependent output
  deterministic.

When adding a screen or widget: add a render test that pins its exact layout
and colors (the gotchas list above exists because those tests caught real
bugs), and keep the fixed 80×24 size — it is wide enough for every status
line and tall enough that chrome rows never overlap content.

## Design decisions, in brief

- **The db is the source of truth; memory is a cache.** Every mutation writes
  through to the db first and mirrors on success; startup rebuilds everything
  from the db. This is what makes "copy the .db file" a complete backup and
  crash recovery trivial (worst case: one session left active).
- **Fail visible, not fatal.** Only an unopenable db exits. Every other
  failure lands in the red status line and the app keeps working with partial
  data.
- **No managed objects.** Projects/tasks/activities are strings; history is
  the only "database" of them. This keeps the schema at one table and the
  backup story at one file.
- **16 colors, no truecolor** — decent on any terminal; all styling flows
  through `theme.rs`.
- **No system dependencies** — bundled SQLite, no X11/Wayland interaction
  (which also rules out in-process global hotkeys; see backlog F).

## Known nits

Tracked in DEVELOPMENT.md's backlog: the per-frame `SkimMatcherV2`
allocation in `fuzzy_filter`.
