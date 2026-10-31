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

- State: fix pass on branch `fix/input-and-resume` (2026-10-31), one commit
  per fix: terminal restored on error/panic; key presses only + Ctrl+C;
  add-note Esc discards; end-session Esc cancels (session keeps running);
  `r` keeps the disruption when there is nothing to resume, `d` is a no-op
  during one; shared `Autocomplete` (input line = what Enter submits);
  two nits done (swallowed errors, `create_session` returns the stored
  session); shared row mapper; same-second `id` tiebreak; corrected the
  "SQLite ignores `Z`" claim (it parses it; legacy rows are fine); multi-line
  previous notes. README and IMPLEMENTATION.md synced. Tree green, 51 tests.
- Next: backlog item C (free-text activities).
- Blockers / open questions: none.

## Backlog

Ranked by value-per-effort as of 2026-09-23. Each item should be startable
from its scope note alone, without re-deriving context.

- [x] **G. TUI beautification** — done 2026-09-25: theme module, header/
      status chrome, main panel + timeline, start/end/note modals with tabs,
      pickers and terminal cursor on input lines, plus the full render-test
      suite (harness/chrome/main/forms). Taste decisions landed: cyan accent,
      rounded borders, centered modals. See git log `1f204c8..7349d6a`.
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
  - `fuzzy_filter` constructs a `SkimMatcherV2` per call (per frame); make it
    static.

## Conventions (don't drift)

- **Errors:** every DB failure goes through `App::set_error`; successful
  operations call `clear_error`. The ERROR line at the bottom reflects the
  most recent db operation. Only an unopenable database exits at startup; all
  startup loads are attempted so partial data still shows.
- **Timestamps:** stored as RFC 3339 UTC with explicit `+00:00`. Older rows
  end in `Z`; SQLite's date functions parse both (covered by db tests), so
  no migration is needed.
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

- `IMPLEMENTATION.md` — developer-facing implementation details: architecture,
  data layer, app state, screen state machine, session lifecycle, rendering,
  testing. Keep in sync when architecture changes.
- `src/main.rs` — entry point: open db, `ratatui::init()`/`restore()` (raw
  mode + alternate screen, restored on error and panic), 100 ms event loop
  calling `App::draw` / `App::handle_key`.
- `src/db.rs` — `Db` over rusqlite; schema + single-active trigger; all SQL
  lives here. Unit tests at the bottom.
- `src/session.rs` — `Session` struct: a db row with parsed timestamps.
- `src/theme.rs` — palette (16 named colors) and shared style helpers; all
  widgets take their styles from here so the look is tweaked in one place.
- `src/app.rs` — all UI: `App` state, `Screen` enum, per-screen key handlers
  (`handle_main`, `handle_start_session` + `start_*_key`,
  `handle_end_session` + `end_*_key`, `handle_add_note`), draw methods
  (`draw_*`), session lifecycle helpers (`start_session`, `complete_active`,
  `close_active_session`, `append_active_note`), the `Autocomplete` field
  used by the project/task steps, and `fuzzy_filter`. Unit tests at the
  bottom.
