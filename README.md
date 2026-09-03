
# jiary

A small, local-first terminal work diary for understanding how I spend my working time.

`jiary` records what I work on throughout the day so I can later reflect on where my time goes, how much deep work I achieve, and how I might improve the way I work.

The name combines **Jeremy** and **diary**, inspired by a coach named Jeremy whose approach I particularly valued.

## What it is

jiary is a **work diary**, not a task or project management system.

The fundamental unit is a **work session**: a period of time spent doing something.

A session can record:

* Start and end time
* Activity (e.g. programming, reading, writing, meeting)
* Project
* Task
* Description and/or outcome
* Optional reflections such as focus or interruptions

Projects and tasks are lightweight labels, not managed entities. Previous values should be available through autocomplete/tab completion to make recording consistent without adding unnecessary rigidity.

The initial implementation will use **SQLite** for local storage and **Ratatui** for the terminal UI.

Run it with:

```bash
jiary
```

## What it isn't

jiary does not manage what I *should* do.

It is not intended to become a:

* Todo list
* Project management system
* Kanban board
* Planning or scheduling tool
* Team productivity tracker
* Billing/timesheet system

**The diary records what happened; it does not manage what should happen.**

## MVP

The first version should do only a few things well:

1. Start a work session
2. Stop a work session
3. Record session details
4. Review today's work
5. Provide simple historical summaries

The MVP should remain small, understandable, and quick to use.

## Future direction

jiary should eventually support **Pomodoro sessions** as an alternative to free-running sessions.

The motivation is to avoid common failures of conventional timers:

* A Pomodoro ends but I keep working and accidentally lose that additional work time.
* A free-running timer continues through a meeting because I forgot to stop it.

The underlying model should therefore treat the **diary as the source of truth**, with timers as a means of helping record work accurately.

Future versions may also support easy transitions such as starting a meeting or resuming previous work without losing accurate session history.

## Design principles

* **Low friction:** recording work should not interrupt the work itself.
* **Diary first:** record what happened rather than managing future work.
* **Soft structure:** use structured data where it helps later analysis, without unnecessary rigidity.
* **Historical truth:** past sessions should remain accurate and unchanged.
* **Local first:** personal work data should live locally.
* **Data before analysis:** collect useful data first; let real usage determine which analyses are worth building.
* **Keep it small:** prefer a simple tool that is easy to understand and maintain.

## Status

Early design / MVP.
