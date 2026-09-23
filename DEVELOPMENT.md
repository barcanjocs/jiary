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

- State: development pipeline in place — session protocol at the top of this
  file (`99dcaaa`) and one-command `./check` verification (`872a4d3`). Refactor
  phase complete (`bce0213`, `5fcc757`); 15 tests, all green.
- Next: choose a feature from the backlog below (suggested order C → B → A → D).
- Blockers / open questions: none.

## Backlog

Ranked by value-per-effort as of 2026-09-23. Each item should be startable
from its scope note alone, without re-deriving context.

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
- `src/app.rs` — all UI: `App` state, `Screen` enum, per-screen key handlers
  (`handle_main`, `handle_start_session` + `start_*_key`,
  `handle_end_session` + `end_*_key`, `handle_add_note`), draw methods
  (`draw_*`), session lifecycle helpers (`start_session`, `complete_active`,
  `close_active_session`, `append_active_note`), and `fuzzy_filter`. Unit
  tests at the bottom.
