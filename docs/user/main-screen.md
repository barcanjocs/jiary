# The main screen

![The main screen: the running session with its notes and the cat, today's sessions, and the day's summary with its graphs](../images/main.svg)

The main screen has three sections, stacked:

- **Active** (top): the running session. Its timer (with the
  [Pomodoro](recording.md#the-pomodoro)'s bar under it, when it's on),
  project, task, activity and interruptions are pinned at the top. Below them are its notes: notes
  from earlier sessions on the same project and task first, then this
  session's. The notes scroll as they grow.
- **Today** (middle): today's sessions, newest first, each with its times,
  length, focus badge and interruptions.
- **Summary** (bottom): the day so far.

A header row above the sections shows the date and today's total. A status
row below shows the keys you're most likely to need (or an
[error](data.md#errors)).

## The cat

A small pixel-art cat sits on the right of the active section when there's
room for it. It follows what you're doing: it idles when nothing is
running, types on a laptop during work (and dozes off over it after 25
minutes, a nudge to take a break; with the
[Pomodoro](recording.md#the-pomodoro) on, it has a coffee at the target
instead), sleeps on a break, chats in a meeting,
and plays video games when disrupted. It also reacts to what you do: it
flinches at an interruption, writes your note down, and reacts to your
focus rating. A short caption under it says what it's up to.

## The summary

The summary shows:

- the day's span, with tracked time split into work (meetings included),
  breaks and disruptions, and the gaps between sessions;
- time per activity and per project;
- average focus, interruptions and disruptions.

When the terminal is tall enough, two graphs join in:

- the day as a timeline, from the hour you started to now, with gaps left
  blank and the hours underneath. A thin line on top shows each session's
  focus: green, yellow or red, dimmed when unrated;
- each activity's and each project's share of the tracked time, as bars.

Focus averages weigh each session by its length, so a 3-hour session counts
more than a 20-minute one.

## Colors

Each activity and each project has its own color, used everywhere it
appears: the active session, today's list, the summary and the longer views.
A project keeps its color from day to day: projects take colors in the
order you first used them, and two projects on screen never share one.

## Arranging the sections

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Focus the next / previous section (it gets the accent border) |
| `Space` | Collapse or expand the focused section (Today and Summary) |
| `+` / `-` | Grow or shrink the focused section |
| `=` | Reset section sizes to fit their content |
| `↑` `↓` / `j` `k`, `PgUp` `PgDn`, `Home` `End` | Scroll the focused section: the notes in Active, otherwise today's sessions |

A collapsed section shrinks to one line that still sums it up, such as
`▸ TODAY · 6 sessions · 4h 12m`. Today's sessions take whatever space the
other sections leave. When Today is collapsed, the summary does.

Section sizes and collapsed sections are remembered between runs.

## The mouse

- Drag the line between two sections to resize them (either of its two
  border rows works).
- Click a section to focus it.
- Click the title of Today or Summary (`▾ TODAY …`) to collapse or expand it.
  A drag never collapses anything, even when it starts on a title.
- Use the wheel to scroll the section under the pointer.

Jiary captures the mouse, so to select text, hold `Shift` while you drag
(in most terminals).
