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

- State (2026-10-31): **v1.0.0 released**
  (https://github.com/barcanjocs/jiary/releases/tag/v1.0.0). Tree green,
  171 tests, worktree clean, everything pushed. Since the October notes,
  on main: one-key `e` with a 5 s rating bar and `u` undo; recent picks in
  the start form; `[`/`]` move the start; away/idle detection (`idle.rs`);
  accentuated main screen with activity and project colors (16-color
  256-color project palette, distinct within a view); day summary graphs
  (timeline with a focus line, activity and project share bars); `/`
  command palette; `/weekly`, `/monthly`, `/yearly` (`period.rs`) with a
  GitHub-style focus grid; length-weighted focus averages; `--version` /
  `--help`; single-instance lock (`lock.rs`, backlog E); release workflow
  (`.github/workflows/release.yml`) and `install.sh` (curl one-liner in the
  README). Since then: `src/app.rs` split into `src/app/` (2026-10-31;
  one file per screen plus shared pieces, code moved unchanged, see the
  file map). Docs reorganized: a short README (what it is and isn't,
  install, a typical day), the user manual in `docs/user/` and the
  developer manual in `docs/dev/` (IMPLEMENTATION.md moved there).
- Next, in order:
  1. **Docs drift.** docs/dev/implementation.md has grown long
     paragraphs; tighten it to match the code (module map, screens,
     lifecycle, rendering).
  2. Then backlog H (rename labels, as a `/` command), J (sleep and mood,
     with its limits), A (browse past days, likely `/day` with ←→ like
     the period views) and K (soft Pomodoro, with `/settings`).
- Releasing: see `docs/dev/README.md` (bump `version`, commit, tag, push
  the tag; the workflow builds and publishes). Tag pushes are done by the
  maintainer, not an agent. Runner images age out: macOS builds moved to `macos-15` after
  `macos-14` was retired (Nov 2026).
- Blockers / open questions: the Intel macOS and Linux builds are built
  and tested in CI but not yet run by hand on real machines.

## Backlog

Ranked by value-per-effort as of 2026-09-23 (done items kept for
context; the order of what's left is in *Now*). Each item should be startable
from its scope note alone, without re-deriving context.

- [x] **G. TUI beautification** — done 2026-09-25: theme module, header/
      status chrome, main panel + timeline, start/end/note modals with tabs,
      pickers and terminal cursor on input lines, plus the full render-test
      suite (harness/chrome/main/forms). Taste decisions landed: cyan accent,
      rounded borders, centered modals. See git log `1f204c8..7349d6a`.
- [x] **C. Free-text activities** — done 2026-10-31: the activity step is an
      `Autocomplete` over `recent_activities` (most recent first);
      `DEFAULT_ACTIVITIES` only seed a fresh db, so unused built-ins drop out;
      activity is required (empty Enter stays on the step); 1–7 shortcuts
      removed.
- [ ] **H. Merge labels** (small). Free-text labels can't be deleted — they
      are derived from history, so a typo ("Progamming") is suggested as long
      as any session uses it. Add a way to rename every use of an activity,
      project or task to another value (`UPDATE sessions SET activity = ?new
      WHERE activity = ?old`, same for project/task), which removes the old
      label from suggestions while keeping history truthful. Decided
      2026-10-31: exposed as a `/` command (I is done, but palette commands
      take no arguments yet). Open: how the old and new label are picked
      (e.g. `/rename` then two `Autocomplete` steps, like the start form),
      and confirming the rename before writing. Renaming a project also
      moves its color (colors go by first use).
- [x] **I. `/` commands** — done 2026-10-31: `/` opens a palette of
      `COMMANDS` (fuzzy filter, Tab completes, Enter runs). A command's
      `Action` is a main-screen key or a view. Commands take no arguments
      yet; H will need them (e.g. `/rename` then a second input step).
- [x] **B. Weekly summary** — done 2026-10-31, grown into `/weekly`,
      `/monthly` and `/yearly` (calendar periods, ←→ to page): a row per
      day (or month), totals, activity/project bars, focus grid.
- [ ] **A. History view** (medium). Browse past days: navigate day by day
      with sessions rendered like the main screen, plus fuzzy search over
      notes/projects/tasks across all time. Read-only. The pieces exist:
      `db.sessions_between(day, day)`, `summarize` over any sessions, and
      the period views' ←→ paging; likely a `/day` command (or Enter on a
      day row in a week/month).
- [ ] **D. Stale-session handling** (small). If the app exits mid-session, the
      next launch shows a "live" timer that actually started before today.
      Detect `started_at` before local midnight and mark it clearly (e.g.
      "since yesterday"); make sure `e` still ends it sensibly. Partly
      covered: while jiary runs, the away check (`observe_idle`) catches
      idle time and sleep; a session left running with jiary closed is
      still unhandled at startup.
- [x] **E. Single-instance lock** — done 2026-10-31: `lock.rs` takes an
      exclusive `flock` (std `File::try_lock`, no dependency) on
      `jiary.lock` in the data dir before the db opens, writing our pid;
      a second jiary prints "already running (pid N)" and exits 1.
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
      activation helper, user manual (`docs/user/`).
- [ ] **J. Day context: sleep and mood** (medium; touches the db, the start
      flow and the period views).
      Two once-a-day ratings to set the day's work against. Discussed
      2026-10-31; the limits are the point, so keep them:
      - Exactly two, fixed: sleep and mood, on focus's 1–3 scale. No custom
        fields or "add a metric": that road ends in a habit tracker, out of
        scope. The README and the user manual already say Jiary is
        not a habit tracker; keep it so.
      - Asked once, cheaply: the day's first `s` opens a small prompt before
        the start form (`sleep 1/2/3 · mood 1/2/3`, two keys); Esc skips and
        it doesn't ask again that day. `/day` sets or changes them later.
        No reminders, no nagging.
      - Storage: a new table `days(date TEXT PRIMARY KEY, sleep INTEGER,
        mood INTEGER)`, added with `CREATE TABLE IF NOT EXISTS`, so old dbs
        just gain it. The first schema change after 1.0: note it in the
        release.
      - Showing the relation honestly (personal data is small and noisy):
        grouped averages in the period totals, e.g. `focus 2.6 after good
        sleep (12 days) · 1.9 after poor sleep (5 days)`, with the day
        counts, and "not enough data yet" under ~5 days per group; and
        sleep/mood as rows of squares beside the focus grid in the month
        and year views. No correlation coefficients, trend lines or
        predictions.
- [ ] **K. Soft Pomodoro** (small-medium). A target session length that
      Jiary shows you've reached, without ever interrupting you: you keep
      going while in flow, and when you glance at the screen it says "you
      can take a break now, if you want". Discussed 2026-10-31; the limits
      are the point, so keep them:
      - Never interrupts: no bell, no notification, no blinking or growing
        cue, nothing that stops or ends the session. The cue is only seen
        if you look at the main screen.
      - The cue: a thin bar under the timer filling up toward the target;
        on reaching it the timer turns green and the cat does something
        new (stretching, a coffee) with a caption like "break's earned, if
        you want one". Past the target, it just stays that way. Work
        sessions only: detours (breaks, disruptions, meetings) get no cue.
      - `/settings`: a small modal (the first settings screen; later
        settings go here too). Rows `Soft Pomodoro  on` and `Target  25m`;
        ↑↓ pick a row, Space/Enter toggle, ←→ change the target in 5 min
        steps, Esc closes. Changes apply at once. It opens a screen, so it
        needs no command arguments (not blocked on H).
      - `p` on the main screen turns it on or off (also `/pomodoro` in the
        palette, and in `?` help). Toggling mid-session just shows or
        hides the cue against the elapsed time.
      - Storage: `settings.conf` next to `layout.conf`, read and written
        like it (tolerating junk lines, see `layout.rs`). Nothing in the
        db: decided 2026-10-31 not to store the target per session. Hit
        counts mostly restate session length, and they pull toward a score
        or streak, which Jiary avoids (see J). Can be added later as a
        nullable `sessions.target` column if ever wanted.
      - When on, the cat dozes at the target instead of
        `DROWSY_AFTER_MINUTES` (25, already marked as a candidate for a
        setting); when off, nothing changes from today. Update the user
        manual (recording, main screen, keys) and the README's "Why I made
        this", which mentions the 25 minutes.
- [ ] **Nits** (tiny, do whenever):
  - `app/tests.rs` (~2,700 lines) is still one file; it could follow the
    code into one test file per screen, with the harness in `tests/mod.rs`.
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
  db.rs, `jiary-app-test-` in app/tests.rs) plus pid and an atomic counter, with
  Drop cleanup — parallel tests share one process and would otherwise delete
  each other's live databases.
- **Commits:** small, lowercase imperative, one concern each; keep the
  worktree clean so `git diff` always shows exactly the current task.
- **Docs:** user-facing behavior (keys, workflows, data, backup) lives in
  the user manual, `docs/user/` (keys in `keys.md`); keep it in sync when
  behavior changes. The README stays short: what Jiary is and isn't,
  install, a typical day. Building and releasing live in `docs/dev/`.
  After a visible change to the main screen or the longer views,
  regenerate the docs' pictures: `cargo test screenshot -- --ignored`.

## Verification

```sh
./check   # fmt --check, clippy -D warnings, build, test — must be green before committing
```

Requires Rust stable (edition 2024). No system dependencies (SQLite bundled).

## File map

- `docs/user/` — the user manual. `docs/dev/README.md` — building,
  checking, releasing.
- `docs/dev/implementation.md` — developer-facing implementation details: architecture,
  data layer, app state, screen state machine, session lifecycle, rendering,
  testing. Keep in sync when architecture changes.
- `src/main.rs` — entry point: open db, `ratatui::init()`/`restore()` (raw
  mode + alternate screen, restored on error and panic), 100 ms event loop
  calling `App::draw` / `App::handle_key`.
- `src/db.rs` — `Db` over rusqlite; schema + single-active trigger; all SQL
  lives here. Unit tests at the bottom.
- `src/session.rs` — `Session` struct: a db row with parsed timestamps.
- `src/layout.rs` — main-screen section geometry (`LayoutPrefs::resolve`,
  resizing with clamping) and the `layout.conf` file. Pure; unit tests at
  the bottom.
- `src/captions.rs` — sentences under the cat, per mood and per activity;
  add new ones here. Unit tests at the bottom.
- `src/mascot.rs` — the cat: half-block pixel-art sprites and their
  time-based frame schedule. Pure; unit tests at the bottom.
- `src/summary.rs` — `summarize`: the day summary over in-memory sessions.
  Pure; unit tests at the bottom.
- `src/theme.rs` — palette (16 named colors, plus 256-color project
  colors) and shared style helpers; all
  widgets take their styles from here so the look is tweaked in one place.
- `src/app/` — all UI, one `impl App` block per file: `mod.rs` (`App`
  state, `Screen`, session lifecycle, `handle_key`/`draw` dispatch), one
  file per screen (`main_screen`, `forms`, `palette`, `period_view`,
  `away`, `help`), shared pieces (`sections`, `chrome`, `cat`, `modal`,
  `items`, `graphs`, `text`), `tests.rs`, and `screenshot.rs` (the
  docs' pictures in `docs/images/`, ignored tests). See docs/dev/implementation.md.
