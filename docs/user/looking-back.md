# Looking back

## The command palette

Press `/` to open the command palette. It lists every action by name, with
what it does and its key.

- Type to filter. Matching is fuzzy, so `/und` finds `undo`.
- `↑`/`↓` pick a command, `Tab` completes its name, and `Enter` runs it.
- `Esc` (or Backspace past the `/`) closes the palette.

A command does exactly what its key does, so the palette is also a way to
learn the keys. It's also how you open what has no key of its own: the
longer views below, `/rename` (see
[Renaming and merging names](recording.md#renaming-and-merging-names)), and
`/mood`, `/mood-off` and `/mood-on` (see
[Sleep and mood](recording.md#sleep-and-mood)), and `/settings` (see
[Settings](recording.md#settings)).

## Weeks and months

`/weekly` shows the current calendar week (Monday to Sunday) and `/monthly`
the current month, a row per day. Each row is the day's timeline in activity
colors, on an hour axis that all the days share so they line up, followed by
the day's tracked time and average focus. A month is grouped by week and
scrolls.

![The week view: a timeline row per day, the week's totals, activity and project bars, and its focus grid](../images/weekly.svg)

Under the days come the period's totals, like today's summary:

- tracked time, split into work, breaks and disruptions;
- the days you tracked, and the average per day;
- the activity and project bars;
- focus;
- with sleep and mood, your focus set against them (see
  [below](#sleep-and-mood)).

Focus also shows as a grid of squares: a row per weekday, a column per week.
Each day is colored on a gradient from red (1) through yellow to green (3)
by its average focus. Days you tracked without rating show dark gray, and
days with nothing tracked show a faint texture.

![The month view: a row per day grouped by week, the month's totals, and its focus grid](../images/monthly.svg)

## Years

`/yearly` shows the whole year, laid out like the other views:

1. a row per month, with its activity bar (scaled to your busiest month),
   its tracked time and its focus;
2. the year's totals;
3. its focus grid, a block per month with its name above it. A column is
   one week of that month, read top to bottom; a week that runs into the
   next month is split, its days shown in each month's block. To fit the
   year, each character holds two days, one above the other, so each row
   is labelled with both: `Mo Tu` is Monday on top and Tuesday below, then
   `We Th`, `Fr Sa`, and `Su` on its own.

![The year view: a row per month, the year's totals, and the focus grid for the whole year](../images/yearly.svg)

## Sleep and mood

When a week, month or year has [sleep and mood](recording.md#sleep-and-mood)
answers, its totals add two rows that set your focus against them:

```
Sleep     focus 2.6 after good sleep (12 days)
          focus 1.9 after poor sleep (5 days)
Mood      focus 2.4 with ok mood (9 days)
          with low mood: not enough data yet (2 days)
```

Each figure is the average focus of the days with that answer, and how many
days there were. Under 5 days, it says "not enough data yet" instead, since
so few days say more about chance than about your sleep. Jiary shows these
plain averages and nothing more: no correlations, trend lines or
predictions. A diary is small and noisy, and you're better placed to judge
it than a formula.

The answers also get grids of their own, colored like focus: beside the
focus grid in a week or a month, under it in a year. A day you didn't
answer shows the faint texture.

`/mood-off` hides all of this. Your answers stay in the diary, and
`/mood-on` shows them again.

## Keys in these views

| Key | Action |
|-----|--------|
| `←` `→` / `h` `l` | The week, month or year before / after (never into the future) |
| `w` `m` `y` | Switch to this week, month or year |
| `↑` `↓` / `j` `k`, `PgUp` `PgDn`, `Home` `End` | Scroll (the mouse wheel does too) |
| `Esc` / `q` | Back to the main screen |
