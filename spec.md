
# Jiary — Technical Specification

**Status:** Draft / MVP
**Language:** Rust
**UI:** Ratatui
**Persistence:** SQLite
**Interface:** Terminal (TUI)
**Executable:** `jiary`

---

## 1. Purpose

Jiary is a local-first terminal application for maintaining a personal **work diary**.

The application records periods of work as sessions, together with enough structured and unstructured information to allow the user to analyse their working habits retrospectively.

The primary user is an individual researcher who spends substantial time programming, reading academic papers, writing, and attending meetings.

The application is explicitly **not** a project-management or task-management system.

The central design goal is:

> **Capture an accurate record of what happened during the working day with minimal cognitive overhead.**

The data collected should subsequently support analysis of time allocation, deep-work patterns, interruptions, meetings, projects, and tasks.

---

# 2. MVP Goals

The MVP must support:

1. Starting a work session.
2. Stopping a work session.
3. Recording project, task, and activity information.
4. Recording optional free-text notes.
5. Reviewing the current day's sessions.
6. Persisting all session data locally in SQLite.
7. Providing autocomplete for previously used project and task names.
8. Providing basic historical/weekly summaries.
9. Operating entirely offline.
10. Being simple enough for a single developer to understand and maintain.

The MVP should establish a sound foundation for future timing modes, particularly Pomodoro, without implementing the full Pomodoro feature initially.

---

# 3. Explicit Non-Goals

The MVP must not become a task-management application.

The following are out of scope:

* Task completion/status tracking
* Due dates
* Priorities
* Task assignment
* Kanban boards
* Project plans
* Project hierarchies
* Team collaboration
* Authentication
* Cloud synchronisation
* Web UI
* Mobile application
* Billing
* Employee monitoring
* Automatic productivity scoring
* AI productivity recommendations
* Complex reporting
* Sophisticated dashboards
* Full Pomodoro implementation

Features should not be added merely because they are common in time-tracking or productivity applications.

---

# 4. Core Conceptual Model

The fundamental entity is a **work session**.

A session represents a period of time during which the user was engaged in a particular activity.

Conceptually:

```text
Session
├── start time
├── end time
├── project (optional)
├── task (optional)
├── activity
├── description (optional)
├── outcome (optional)
├── focus rating (optional)
└── interruptions (optional)
```

The application should treat sessions as **historical observations**, not as entities in a workflow.

A task is therefore metadata describing what the user worked on.

It is not a managed object with state.

---

# 5. Session Semantics

A session begins when the user starts tracking an activity.

A session ends when the user explicitly stops it or when a future session-transition mechanism ends it.

The initial MVP should support one active session at a time.

An active session should be recoverable after application restart.

For example, if the application exits while a session is active, the next invocation of Jiary should detect the active session and provide an appropriate recovery/resume interface rather than silently losing it.

### Historical immutability

Once a completed session has been recorded, its historical values should not be silently changed as a side effect of changing autocomplete values or future project/task naming.

For example:

```text
Session A
Project: Neural PDEs
Task: Implement solver
```

If the user later starts using:

```text
Project: Neural PDE Research
```

existing sessions should continue to contain `Neural PDEs`.

---

# 6. Session Fields

The MVP session model should contain approximately the following fields.

| Field           | Required                   | Purpose                   |
| --------------- | -------------------------- | ------------------------- |
| `id`            | Yes                        | Unique session identifier |
| `started_at`    | Yes                        | Session start timestamp   |
| `ended_at`      | Yes for completed sessions | Session end timestamp     |
| `project`       | No                         | Project/research context  |
| `task`          | No                         | Work being performed      |
| `activity`      | Yes                        | Broad category of work    |
| `description`   | No                         | What was done             |
| `outcome`       | No                         | What was accomplished     |
| `focus`         | No                         | Subjective focus rating   |
| `interruptions` | No                         | Number of interruptions   |

The exact schema may evolve during implementation.

The specification intentionally does not prescribe every SQLite type or Rust type at this stage.

---

# 7. Activities

Activity is a controlled vocabulary because consistent activity categories are important for later analysis.

The initial vocabulary should be small.

Suggested values:

```text
Programming
Reading
Writing
Meeting
Administration
Other
```

The vocabulary should be easy to extend in the future.

Activities should not initially be user-defined free-text values, because inconsistent activity names would make aggregate analysis unnecessarily difficult.

---

# 8. Projects

Projects are optional textual metadata.

There is deliberately no `projects` table in the MVP.

A project value is stored directly on the session:

```text
project TEXT
```

Previous project values should be available through autocomplete.

The user may always enter a new project value.

The application must not require a project to exist before it can be assigned to a session.

---

# 9. Tasks

Tasks are also optional textual metadata.

There is deliberately no `tasks` table in the MVP.

A task value is stored directly on the session:

```text
task TEXT
```

Previous task values should be available through autocomplete.

The user may always enter a new task.

Tasks have no lifecycle.

There is no concept of:

```text
todo
in progress
completed
cancelled
overdue
```

A task is simply a label describing what the user was working on.

This distinction is fundamental to the application.

---

# 10. Autocomplete

Autocomplete is intended to provide **soft structure**.

The user should be able to enter a project or task while seeing values previously used in the diary.

For example:

```text
Project: Neu▌

  Neural PDEs
  Neuromorphic Computing
```

or:

```text
Task: Implement▌

  Implement sparse solver
  Implement optimiser
```

Autocomplete should:

* Search historical session values.
* Prefer exact/prefix matches.
* Support tab completion.
* Allow selection with keyboard navigation.
* Allow the user to enter a completely new value.
* Avoid requiring the creation or management of task/project entities.

Suggestions may eventually be ranked by frequency and/or recency.

The MVP should begin with a simple implementation.

Potential query:

```sql
SELECT DISTINCT project
FROM sessions
WHERE project IS NOT NULL
  AND project != ''
ORDER BY project;
```

The exact matching algorithm should be determined during implementation based on the capabilities of the chosen TUI/input components.

---

# 11. User Interface

Jiary is a TUI-first application.

The interface should prioritise keyboard interaction and rapid operation.

The primary screen should be the **current day** rather than a task list.

Conceptually:

```text
┌─────────────────────────────────────────────────┐
│ JIARY — TODAY                                   │
├─────────────────────────────────────────────────┤
│                                                 │
│ 09:15 ─────────────── 11:07                    │
│      Programming · Neural PDEs                  │
│      Implement sparse solver                    │
│      Fixed boundary-condition bug               │
│                                                 │
│ 11:07 ─────────────── 11:52                    │
│      Meeting · Neural PDEs                      │
│                                                 │
│ 13:05 ─────────────── 14:42                    │
│      Reading · Neural PDEs                      │
│      Smith et al. (2026)                        │
│                                                 │
├─────────────────────────────────────────────────┤
│ Total: 4h 24m       Deep work: 3h 12m           │
└─────────────────────────────────────────────────┘
```

This is illustrative rather than a fixed UI specification.

The exact layout should be developed interactively.

---

# 12. Starting a Session

The primary action should be starting a new session.

The user should be able to:

1. Select an activity.
2. Optionally select/enter a project.
3. Optionally select/enter a task.
4. Start the session.

The application should make repeated combinations fast to select.

For example, recently used combinations may eventually be available:

```text
Programming · Neural PDEs · Implement sparse solver
Reading · Neural PDEs · Literature review
Writing · Paper X · Results section
Meeting · Neural PDEs
```

This is a convenience feature, not a required MVP abstraction.

---

# 13. Stopping a Session

Stopping the active session should be a simple keyboard action.

The user may optionally be prompted for:

* Description
* Outcome
* Focus
* Interruptions

These fields should not all be mandatory.

The application should avoid turning the end of every session into a lengthy questionnaire.

The user should be able to stop a session and continue immediately.

---

# 14. Active Session

At most one session should normally be active.

The UI should make the active session visually obvious.

For example:

```text
ACTIVE
────────────────────────────
Programming · Neural PDEs
Implement sparse solver

01:42:17
```

The application should always provide a clear way to stop or transition the active session.

---

# 15. Application Restart

The application must not lose an active session merely because the process exits.

The database should contain sufficient information to identify an unfinished session:

```text
ended_at IS NULL
```

On startup, Jiary should detect such a session.

The exact recovery UX can be decided during implementation, but reasonable behaviour would include:

```text
An active session was found.

Programming · Neural PDEs
Started 09:15

[R] Resume
[S] Stop now
[E] Edit
```

The implementation should avoid creating duplicate sessions accidentally.

---

# 16. SQLite

SQLite is the initial persistence layer.

The database should be local to the user.

The application should not require:

* A server
* An account
* Network access
* External services

The database should be stored in an appropriate user-specific application/data directory rather than the current working directory.

The exact platform-specific location should be decided during implementation.

---

# 17. Initial Database Schema

The MVP should use a single primary table:

```sql
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,

    started_at TEXT NOT NULL,
    ended_at TEXT,

    project TEXT,
    task TEXT,
    activity TEXT NOT NULL,

    description TEXT,
    outcome TEXT,

    focus INTEGER,
    interruptions INTEGER
);
```

This is a starting point, not a requirement to preserve the exact schema indefinitely.

The schema should remain deliberately simple.

A separate task/project table should only be introduced if real usage demonstrates a concrete need.

---

# 18. Time Representation

Timestamps must represent actual points in time rather than merely durations.

The database should store:

* Session start timestamp
* Session end timestamp

Duration should normally be calculated from those timestamps rather than stored redundantly.

The application should handle timezone consistently.

The implementation should make a deliberate choice about timestamp representation rather than relying on implicit local-time behaviour.

---

# 19. Historical Views

The MVP should provide at least a basic view of today's sessions.

Eventually the application should support historical periods such as:

```text
Today
Yesterday
This week
Last week
```

The first implementation does not need a sophisticated reporting system.

The purpose is to make the recorded data immediately useful and to validate that the diary is being captured correctly.

---

# 20. Basic Analysis

The MVP should provide simple aggregate information.

Potential initial metrics:

* Total tracked time
* Time by activity
* Time by project
* Time by task
* Number of sessions
* Average session duration
* Longest session

Deep-work analysis can initially be very simple.

For example, a future rule might classify programming, reading, and writing sessions above a certain duration as potential deep-work sessions.

The exact definition of "deep work" should remain configurable or, preferably, analytically derived later rather than becoming a hard-coded productivity judgement.

---

# 21. Pomodoro — Future Capability

Pomodoro support is explicitly planned but is **not part of the MVP implementation**.

The architecture should nevertheless accommodate it.

The motivation is to solve two practical problems with conventional timers.

### Problem 1: Pomodoro overruns

A conventional Pomodoro timer may switch from focus to break while the user continues working.

This can result in the additional work not being accurately recorded.

Jiary should eventually allow:

```text
25m focus period
      ↓
Pomodoro boundary
      ↓
User continues working
      ↓
Additional work remains recorded
```

The timer should not silently cause the diary to classify ongoing work as a break.

### Problem 2: Activity transitions

A free-running timer may continue running when the user unexpectedly has to attend a meeting.

Jiary should eventually make transitioning from:

```text
Programming
```

to:

```text
Meeting
```

and subsequently back to:

```text
Programming
```

extremely cheap.

The programming time should be split accurately around the meeting.

For example:

```text
09:00–10:15  Programming
10:15–11:00  Meeting
11:00–12:20  Programming
```

rather than recording one misleading 3h20m programming session.

---

# 22. Timing Architecture

The conceptual model should distinguish between:

**Work sessions**

and

**mechanisms/events that control or annotate sessions.**

The MVP only needs basic start/stop behaviour.

Future functionality may introduce:

```text
Start session
    ↓
Pomodoro focus period
    ↓
Pomodoro boundary
    ↓
Continue / break / switch activity
```

or:

```text
Programming
    ↓
Meeting
    ↓
Resume programming
```

The data model should not make these future behaviours impossible.

However, this does **not** justify prematurely implementing an event-sourcing architecture or complex state machine.

The MVP should use the simplest model that works.

---

# 23. Data Integrity

The diary represents personal historical data and should be treated as durable.

The application should:

* Avoid silently losing sessions.
* Avoid silently creating duplicate sessions.
* Preserve completed session timestamps.
* Preserve historical project/task strings.
* Handle application termination gracefully.
* Provide a straightforward way to back up the SQLite database.

Automatic cloud backup is out of scope.

---

# 24. Export

A full export system is not required for the initial MVP.

However, the data should remain accessible through SQLite and should not be stored in a proprietary or opaque format.

Future export formats could include:

* CSV
* JSON
* Markdown
* Direct analysis through SQLite/DuckDB/Python/R

The database itself should be considered the canonical historical dataset.

---

# 25. Rust Architecture

The implementation should favour a small number of clearly separated responsibilities.

A possible high-level structure is:

```text
Application
├── UI
│   └── Ratatui views/widgets
│
├── Application state
│   └── Current screen/session/input state
│
├── Domain
│   └── Session/activity concepts
│
└── Persistence
    └── SQLite queries and migrations
```

This is intentionally conceptual.

The project should not introduce layers, traits, or abstractions unless they solve a real problem.

The goal is a codebase that remains easy to follow.

---

# 26. Error Handling

Errors should be handled explicitly.

User-facing errors should be understandable.

Examples:

```text
Could not open Jiary database.
```

rather than exposing raw SQLite errors directly where avoidable.

Database errors should not result in silently discarded diary entries.

The application should fail safely when persistence is unavailable.

---

# 27. Testing

Testing should focus on behaviour that protects the integrity of the diary.

At minimum, tests should cover:

### Persistence

* Creating a session.
* Completing a session.
* Reading sessions back.
* Detecting an active session.
* Preventing accidental duplicate active sessions.

### Time calculations

* Session duration.
* Sessions spanning boundaries such as midnight.
* Daily totals.

### Autocomplete

* Retrieving previous projects.
* Retrieving previous tasks.
* Filtering suggestions.
* Handling empty histories.

### Domain behaviour

* Starting a session.
* Stopping a session.
* Correctly representing active versus completed sessions.

The TUI itself does not need exhaustive snapshot testing in the first iteration unless that proves useful.

---

# 28. Development Approach

Jiary should be built incrementally.

Do not generate the entire application in one pass.

Development should proceed approximately as:

```text
Project setup
    ↓
SQLite connection
    ↓
Session persistence
    ↓
Basic CLI/TUI
    ↓
Start/stop workflow
    ↓
Today's timeline
    ↓
Autocomplete
    ↓
Basic summaries
    ↓
Real-world usage
    ↓
Refinement
```

Each step should result in something understandable and testable.

Implementation decisions should be revisited based on actual use rather than theoretical requirements.

---

# 29. Pair Programming Expectations

Jiary is intended to be developed collaboratively through deliberate pair programming.

Changes should be made **one file at a time where practical**, with each change reviewed and understood before proceeding.

The developer should favour explaining:

* Why a particular design is being chosen.
* What alternatives were considered.
* What assumptions are being made.
* What the smallest viable implementation is.

The objective is not to maximise development speed.

The objective is to produce a small codebase that the project owner understands and can confidently modify.

Avoid large generated implementations or introducing dependencies without discussing their purpose.

---

# 30. Dependency Philosophy

Dependencies should be kept to a minimum.

Likely core dependencies include:

* Ratatui
* Crossterm
* SQLite library
* Time/date library

The exact crate choices should be discussed before adding them.

A dependency should have a clear benefit over implementing the required functionality directly.

---

# 31. MVP Definition of Done

The MVP can be considered complete when the following workflow works reliably:

```text
$ jiary
```

The application opens today's diary.

The user can:

```text
Start session
    ↓
Choose activity
    ↓
Choose/type project
    ↓
Choose/type task
    ↓
Start working
    ↓
Stop session
    ↓
Optionally record notes
    ↓
See the completed session in today's timeline
```

On a subsequent session, previously used project and task values can be selected through autocomplete.

After multiple days of use, the user can review basic totals and see where their working time has gone.

The application remains entirely local and does not attempt to manage the user's work.

---

# 32. Guiding Principle

The most important architectural and product principle is:

> **Jiary records what happened; it does not manage what should happen.**

The second is:

> **The diary is the source of truth; timers are merely tools for recording it accurately.**

Every proposed feature should be evaluated against these principles.

If a feature makes Jiary better at recording and understanding the user's working life, it may belong in the product.

If it starts telling the user what they should do, managing unfinished work, or demanding attention unrelated to recording the work, it probably belongs somewhere else.
