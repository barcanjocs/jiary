# Development notes

Handoff document for work sessions (human or agent). The repo is the source of
truth for state — this file plus `git log` is all that is needed to resume.

## Session protocol

**Starting a session** (when asked to read this file and continue):

1. Read this file in full, then run `git log --oneline -15`.
2. Check *Now* below for state and next step; confirm the worktree is clean
   (`git status`).
3. Run `./check` to confirm the tree is green before changing anything.
4. State briefly what you found and what you will do, then proceed.

**Finishing a session** (when told the session is ending):

1. Update *Now* below (state, next step, blockers).
2. Run `./check` — must be green.
3. Commit everything, including this file's update.

## Now

- State: TUI beautification (item G) in progress — steps 1–4 landed: theme
  module (`1f204c8`), header/status layout (`52491b8`), main screen as
  bordered active panel + timeline List (`749ea5c`), start form as centered
  40×12 modal with `Tabs` + picker `List`s + real terminal cursor on input
  steps (`fdb334d`); tree green, 16 tests. Taste decisions confirmed: cyan
  accent, rounded borders, centered modals.
  Cursor note: `Terminal::draw` discards closure returns and this ratatui
  (0.30) has no `Frame::hide_cursor`, so `App::cursor_position(size)` is a
  pure fn shared with the renderer via `start_modal_layout`; main.rs shows/
  hides the terminal cursor each frame from it.
- Next: G step 5 — end + note forms: same modal/tabs/cursor treatment;
  focus step as a colored `[1] Bad [2] OK [3] Good` line.
- Blockers / open questions: none.

## Backlog

Ranked by value-per-effort as of 2026-09-23. Each item should be startable
from its scope note alone, without re-deriving context.

- [ ] **G. TUI beautification** (medium, chosen next). The UI is one
      full-area Paragraph per screen with manual `>` markers and no colors.
      Plan, in order of small commits (each keeps `./check` green):
      1. `src/theme.rs` — palette (16 named colors only: cyan accent,
         gray/dimmed secondary, green/yellow/red semantics) + style helpers
         (title, dimmed label/value line builder).
      2. `draw()` layout skeleton — `Layout::vertical`: header row (name bold,
         date, today's total time computed from in-memory sessions), content,
         bottom status line showing either the red error (`✗ …`) or key hints
         for the current screen (bold keys, dimmed descriptions); inline hint
         lines removed from forms since the status line is their single home.
      3. Main screen — active session as rounded bordered panel (accent
         border/title, bold timer, interruptions colored when >0, notes as
         dimmed bullets); no-active state centered and dimmed; today's
         timeline as multi-line `List` (one item per session: bold time range
         + duration, focus badge F1 red / F2 yellow / F3 green, project·task
         bright, notes dimmed).
      4. Start form — centered fixed-size modal block; `Tabs` for
         Activity·Project·Task with current step selected; activity picker and
         suggestions as real `List`s with `highlight_symbol`/`highlight_style`
         (temp per-frame `ListState` built from the existing index fields);
         input lines get the real terminal cursor (`Frame::set_cursor_position`,
         explicit `hide_cursor` at startup, shown only on input screens).
      5. End + note forms — same modal/tabs/cursor treatment; focus step as a
         colored `[1] Bad [2] OK [3] Good` line.
      6. Render tests via ratatui `TestBackend` (no feature gate, has
         `assert_buffer_lines`) covering header/footer/error rows, active vs
         no-active state, form step tabs and highlight position.
      Constraints: no new deps; no db queries from draw; follow existing
      borrow patterns; README unchanged (keys don't change). Open taste
      decisions (defaults in parens): cyan accent (vs green), rounded borders
      (vs double), centered modal dialogs (vs full-screen).
- [ ] **C. Free-text activities** (small). Activities are a hardcoded list of
      7 (`ACTIVITIES` in app.rs); anything that doesn't fit forces "Other".
      Make the activity step work like project/task: fuzzy autocomplete seeded
      from distinct past activities plus the defaults, free typing allowed.
      Touches: `Step::Activity` handling and its rendering, plus a new db query
      (`distinct_activities`, patterned on `distinct_projects`).
- [ ] **B. Weekly summary** (small-medium). A key (e.g. `w`) showing the last
      7 days: total time per activity and per project, average focus rating,
      interruption/disruption counts. Pure SQL aggregation plus one new screen;
      no schema change.
- [ ] **A. History view** (medium). A key (e.g. `h`) to browse past days:
      navigate day by day with sessions rendered like the main screen, plus
      fuzzy search over notes/projects/tasks across all time. Read-only; needs
      a "sessions for local date" db query — `sessions_for_today` is the
      pattern.
- [ ] **D. Stale-session handling** (small). If the app exits mid-session, the
      next launch shows a "live" timer that actually started before today.
      Detect `started_at` before local midnight and mark it clearly (e.g.
      "since yesterday"); make sure `e` still ends it sensibly.
- [ ] **E. Single-instance lock** (small). Nothing prevents two jiary
      processes from running at once and interleaving writes. At startup,
      before opening the db, acquire an exclusive `flock` on a dedicated
      lock file in the data dir (e.g. `jiary.lock`); on contention exit(1)
      with "already running (pid N)" — same philosophy as the unopenable-db
      exit (our own pid written into the lockfile, best-effort). flock is
      kernel-managed: released on crash/kill, no stale-lock logic, and a
      separate namespace from SQLite's POSIX locks. Dep: `fs2` or raw libc.
      Test headlessly by holding the lock and asserting a second acquire
      fails. Touches: main.rs startup, new error path, README.
- [ ] **F. Relaunch-as-focus** (small-medium, builds on E). Re-running
      `jiary` while an instance is live should focus its terminal window
      instead of erroring: the first instance records pid + `$WINDOWID` in
      the lockfile; the second reads it on lock failure and activates the
      window (X11 only, e.g. `wmctrl -ia <id>`), then exits 0. No
      `$WINDOWID` or non-X → fall back to E's "already running" message.
      The global-key part stays in the user's environment (desktop
      shortcut / terminal keybind that runs `jiary`); no in-process global
      hotkey registration — Wayland forbids it and it breaks the
      no-system-deps property. Touches: E's lockfile format, one X11
      activation helper, README.
- [ ] **Nits** (tiny, do whenever):
  - In-memory `started_at` (`Utc::now()` in `start_session`) differs from the
    db's by microseconds; have `create_session` return (or re-read) the stored
    value so the db stays authoritative.
  - Two `unwrap_or_default()` calls swallow DB errors: `tasks_for_project` in
    the start form and the `distinct_projects`/`distinct_tasks` refresh after
    session creation. Inconsistent with the error-line philosophy.
  - `fuzzy_filter` constructs a `SkimMatcherV2` per call (per frame); make it
    static.

## Conventions (don't drift)

- **Errors:** every DB failure goes through `App::set_error`; successful
  operations call `clear_error`. The ERROR line at the bottom reflects the
  most recent db operation. Only an unopenable database exits at startup; all
  startup loads are attempted so partial data still shows.
- **Timestamps:** stored as RFC 3339 UTC with explicit `+00:00`, never `Z`
  (SQLite date functions parse the former and silently ignore the latter).
  "Today" means local-time start date (`date(started_at, 'localtime')`).
- **Single active session:** enforced by a db trigger (a UNIQUE index can't —
  SQLite ignores NULLs in UNIQUE constraints). `get_active_session`
  additionally picks the most recent row to tolerate legacy duplicates.
- **UI state mirrors the db** in memory (notes, interruptions, previous-notes
  cache); refresh via `refresh_after_session_change`; never query the db from
  `draw` (it runs ~10x/sec).
- **Borrow patterns:** match `&self.screen` with unit patterns (binds nothing,
  so the borrow drops before the arm body) to keep calling `&mut self`
  methods; read `form.step` by copy when dispatching; copy values out of
  `form` before any `&mut self` call (call arguments keep their borrow alive
  across the call).
- **Tests:** temp dirs need a unique prefix per module (`jiary-test-` in
  db.rs, `jiary-app-test-` in app.rs) plus pid and an atomic counter, with
  Drop cleanup — parallel tests share one process and would otherwise delete
  each other's live databases.
- **Commits:** small, lowercase imperative, one concern each; keep the
  worktree clean so `git diff` always shows exactly the current task.
- User-facing docs (keys, data model, backup) live in README.md — keep it in
  sync when behavior changes.

## Verification

```sh
./check   # fmt --check, clippy -D warnings, build, test — must be green before committing
```

Requires Rust stable (edition 2024). No system dependencies (SQLite bundled).

## File map

- `src/main.rs` — entry point: open db, raw mode + alternate screen, 100 ms
  event loop calling `App::draw` / `App::handle_key`.
- `src/db.rs` — `Db` over rusqlite; schema + single-active trigger; all SQL
  lives here. Unit tests at the bottom.
- `src/session.rs` — `Session` struct: a db row with parsed timestamps.
- `src/theme.rs` — palette (16 named colors) and shared style helpers; all
  widgets take their styles from here so the look is tweaked in one place.
- `src/app.rs` — all UI: `App` state, `Screen` enum, per-screen key handlers
  (`handle_main`, `handle_start_session` + `start_*_key`,
  `handle_end_session` + `end_*_key`, `handle_add_note`), draw methods
  (`draw_*`), session lifecycle helpers (`start_session`, `complete_active`,
  `close_active_session`, `append_active_note`), and `fuzzy_filter`. Unit
  tests at the bottom.
