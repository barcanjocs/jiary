# Recording your day

## Starting a session

Press `s`. The form opens with your 5 most recent activity/project/task
combinations (detours left out). Press `1`–`5` to start one straight away.

Or type to fill the form in, one step at a time:

1. **Activity**: required.
2. **Project**: optional. Press Enter on an empty field to skip it.
3. **Task**: optional. Suggestions are limited to the chosen project's tasks.

`Esc` closes the form without starting anything. Starting a session while
another runs ends the running one.

### Suggestions

Each step lists the values you've used before as soon as it opens. Typing
filters them with fuzzy matching, so `prg` finds `Programming`.

- `Tab` (or `↓`) picks the best suggestion. More `Tab`s cycle through the
  rest.
- `↑`/`↓` move between suggestions. `↑` past the top goes back to what you
  typed.
- The input line always shows what Enter will submit. Typing after picking
  a suggestion carries on from it.
- You can always type a new value that matches nothing.

### Activities

Activities are free text, like projects. Suggestions are the activities
you've used, most recent first, so `s`, `Tab`, `Enter` repeats your last
one. A new diary suggests a starter set (Programming, Reading, Writing,
Meeting, Administration, Disruption, Break, Other) until you have some
history. After that, starter activities you never use drop out of the
list.

## Ending a session and rating focus

`e` ends the running session at once. For the next 5 seconds, a bar drains
under the active section (and the cat asks too). In that time you can rate
your focus with one key:

- `1`: bad (fragmented, many context switches)
- `2`: ok
- `3`: good (deep flow)

You can also ignore it and carry on. The session just stays unrated. The
window is short on purpose: the rating you give right away is the honest
one.

The window also opens when you switch straight to something else (`s`, `d`,
`b`, `m`), so you don't have to press `e` first to rate. Breaks and
disruptions aren't focused work, so ending one doesn't open it.

Today's list shows each rating as a badge: `F1` (red) to `F3` (green), or
`F-` when unrated.

## Undo

`u` undoes the last start or end. A session you ended runs again (its rating
cleared), and anything that replaced it is removed.

## Notes

Press `n` while a session runs to add a note, then Enter to save it or `Esc`
to discard it. Notes are saved at once, so
they survive Jiary quitting before the session ends. The active section
shows the session's notes, under the notes from earlier sessions on the
same project and task.

To add a last note to a session you just ended, press `u` to reopen it, `n`
to add the note, then `e` again.

## Interruptions

Press `i` while a session runs to count a small interruption: a glance at
your phone, a chat ping, the urge to check something. The active section
shows the count, and today's list shows it as `I:N`.

Bigger interruptions, such as a conversation or a call, are better recorded
as a disruption with `d`.

## Detours: disruptions, breaks and meetings

Three keys step you out of what you're doing:

- `d`: you've been interrupted (a colleague, a call, a context switch).
  Starts a Disruption. It needs a running session to interrupt.
- `b`: you're taking a break on purpose (coffee, lunch, a walk). Starts a
  Break. It also works with nothing running, for example right after `e`.
- `m`: you're joining a meeting. Starts a Meeting. It also works with
  nothing running (the morning standup). Meeting time counts as work.

When you're done, `r` ends the detour and restarts the activity, project and
task you left. So the round trip is two keys, such as `d` … `r`.

`r` always goes back to the last session that wasn't a detour, so a meeting
followed by a break (`m`, then `b`) still resumes your work. A meeting you
start with `s` is a detour too, so `r` never resumes into a meeting.
Pressing a detour's key during that same detour does nothing.

## Renaming and merging names

Activities, projects and tasks are whatever you typed, so a typo
("Progamming") keeps being suggested for as long as a session uses it.
`/rename` fixes that everywhere at once:

1. **What**: pick Activity, Project or Task.
2. **Project** (tasks only): the project the task belongs to. Leave it
   blank for tasks without a project. Task names repeat across projects,
   so a task is only renamed within one.
3. **From**: the name to change, picked from the names in use.
4. **To**: the new name. Pick a name already in use to **merge** the two,
   or type a new one.

Jiary then asks before writing anything, with the numbers:
`Rename "jiry" to "jiary" in 2 sessions?`, or
`Merge "Progamming" into "Programming" (3 sessions; it has 41)?`. `Enter`
applies it and `Esc` cancels; there's no undo afterwards.

Your sessions keep their times and notes; only the name changes, the
running session's included, and the old name stops being suggested. A
few things to know:

- Disruption, Break and Meeting can't be renamed, and nothing can be
  renamed to them: Jiary recognizes detours by those names.
- The new name can't be blank. Changing only its case ("programming" to
  "Programming") works.
- Notes aren't changed, even if they mention the old name.
- Merged projects keep the color of whichever was used first.

## Fixing the times

**Started late?** `[` moves the running session's start 5 minutes earlier,
and `]` 5 minutes later, but never into the future. If you switched from
another session (with `s`, `d`, `b`, `m` or `r`), the switch moves too, so
the session before ends earlier (it keeps at least a minute). If there was
a gap before this session, the start stops at the end of the session
before. Press again to move both.

**Walked away with a session running?** While Jiary is open, it watches how
long the computer has gone without keyboard or mouse input, and it notices
when the computer slept. This works on macOS, and on Linux with
`xprintidle` installed. If you come back after 15 minutes or more away
during a work session, Jiary asks what that time was:

- `k`: keep it as part of the session (say, you were reading on paper);
- `b`: it was a break. The session ends when you left, a break covers the
  time away, and the same work starts again now;
- `d`: it was a disruption. The same, but with a disruption covering the
  time away;
- `e`: end the session when you left.

`u` undoes whichever you pick. Detours are never asked about, since being
away from the computer is what they're for.

Jiary only notices an absence while it's open. A session left running
while Jiary was closed isn't caught yet.
