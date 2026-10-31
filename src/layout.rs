//! Main-screen layout: three stacked sections (the active session, today's
//! sessions, the day summary), their heights and collapsed state, and the
//! preferences persisted between runs.
//!
//! Pure geometry plus a tiny `key=value` file format; nothing here renders.
//! Today's sessions is the flexible section: it takes whatever the other two
//! leave. When it is collapsed the summary takes the space instead, and when
//! both are collapsed the spare rows stay empty below them.

use std::io;
use std::path::Path;

use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Active,
    Today,
    Summary,
}

impl Section {
    pub fn next(self) -> Self {
        match self {
            Section::Active => Section::Today,
            Section::Today => Section::Summary,
            Section::Summary => Section::Active,
        }
    }

    pub fn prev(self) -> Self {
        self.next().next()
    }
}

/// Smallest height of an expanded section: top border, one row, bottom
/// border.
pub const MIN_HEIGHT: u16 = 3;
/// A collapsed section is a single unbordered row.
pub const COLLAPSED_HEIGHT: u16 = 1;

/// Layout preferences, persisted between runs. `None` heights fit the
/// section's content; `=` resets to that.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutPrefs {
    pub active_height: Option<u16>,
    pub summary_height: Option<u16>,
    pub today_collapsed: bool,
    pub summary_collapsed: bool,
}

/// Content-fitting heights (borders included) of the sections whose size
/// can follow their content.
#[derive(Clone, Copy, Debug)]
pub struct ContentHeights {
    pub active: u16,
    pub summary: u16,
}

/// The resolved rect of each section, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Areas {
    pub active: Rect,
    pub today: Rect,
    pub summary: Rect,
}

impl Areas {
    pub fn get(&self, section: Section) -> Rect {
        match section {
            Section::Active => self.active,
            Section::Today => self.today,
            Section::Summary => self.summary,
        }
    }
}

impl LayoutPrefs {
    pub fn collapsed(&self, section: Section) -> bool {
        match section {
            Section::Active => false,
            Section::Today => self.today_collapsed,
            Section::Summary => self.summary_collapsed,
        }
    }

    /// Toggles a collapsible section. The active section can't collapse.
    pub fn toggle_collapsed(&mut self, section: Section) {
        match section {
            Section::Active => {}
            Section::Today => self.today_collapsed = !self.today_collapsed,
            Section::Summary => self.summary_collapsed = !self.summary_collapsed,
        }
    }

    /// The section that takes the rows the others leave, if any.
    fn filler(&self) -> Option<Section> {
        if !self.today_collapsed {
            Some(Section::Today)
        } else if !self.summary_collapsed {
            Some(Section::Summary)
        } else {
            None
        }
    }

    fn min_height(&self, section: Section) -> u16 {
        if self.collapsed(section) {
            COLLAPSED_HEIGHT
        } else {
            MIN_HEIGHT
        }
    }

    /// Splits `area` into the three sections.
    ///
    /// Sections other than the filler get their preferred height (or their
    /// content's), the filler gets the rest. When the filler would drop
    /// below its minimum, the summary and then the active section give rows
    /// back, down to their own minimums. On terminals too small even for
    /// that, sections are clipped at the bottom.
    pub fn resolve(&self, area: Rect, content: ContentHeights) -> Areas {
        let filler = self.filler();
        let wanted = |section: Section| -> u16 {
            if self.collapsed(section) {
                COLLAPSED_HEIGHT
            } else {
                let preferred = match section {
                    Section::Active => self.active_height.unwrap_or(content.active),
                    Section::Summary => self.summary_height.unwrap_or(content.summary),
                    Section::Today => MIN_HEIGHT,
                };
                preferred.max(MIN_HEIGHT)
            }
        };

        let mut active = wanted(Section::Active);
        let mut today = wanted(Section::Today);
        let mut summary = wanted(Section::Summary);
        let filler_min = filler.map_or(0, |s| self.min_height(s));
        let fixed = |active: u16, today: u16, summary: u16| -> u16 {
            let mut total = active;
            if filler != Some(Section::Today) {
                total += today;
            }
            if filler != Some(Section::Summary) {
                total += summary;
            }
            total
        };

        // Give rows back until the filler fits (summary first, then active).
        let mut excess = (fixed(active, today, summary) + filler_min).saturating_sub(area.height);
        if filler != Some(Section::Summary) && !self.summary_collapsed {
            let give = excess.min(summary - MIN_HEIGHT);
            summary -= give;
            excess -= give;
        }
        let give = excess.min(active - MIN_HEIGHT);
        active -= give;

        let mut rest = area.height.saturating_sub(fixed(active, today, summary));
        // Still short (tiny terminal): today's sessions outrank the summary,
        // which then gets clipped away at the bottom instead.
        if filler == Some(Section::Today) && rest < filler_min {
            let take = (filler_min - rest).min(summary);
            summary -= take;
            rest += take;
        }
        match filler {
            Some(Section::Today) => today = rest,
            Some(Section::Summary) => summary = rest,
            _ => {}
        }

        // Stack top to bottom, clipping at the bottom of `area`.
        let mut y = area.y;
        let mut next = |height: u16| {
            let height = height.min(area.bottom().saturating_sub(y));
            let rect = Rect::new(area.x, y, area.width, height);
            y += height;
            rect
        };
        Areas {
            active: next(active),
            today: next(today),
            summary: next(summary),
        }
    }

    /// Grows (`delta > 0`) or shrinks the section by `delta` rows, starting
    /// from its current resolved height. Growing the active section or the
    /// summary takes rows from today's sessions; resizing today's sessions
    /// moves rows to or from the summary (or the active section when the
    /// summary is collapsed). No-op for a collapsed section or one that
    /// fills the space anyway.
    pub fn resize(&mut self, section: Section, delta: i32, area: Rect, content: ContentHeights) {
        let areas = self.resolve(area, content);
        match section {
            Section::Active => {
                let height = areas.active.height as i32 + delta;
                self.set_active_height(height, area, content);
            }
            Section::Summary if self.filler() == Some(Section::Today) => {
                let height = areas.summary.height as i32 + delta;
                self.set_summary_height(height, area, content);
            }
            Section::Today if !self.today_collapsed => {
                if !self.summary_collapsed {
                    let height = areas.summary.height as i32 - delta;
                    self.set_summary_height(height, area, content);
                } else {
                    let height = areas.active.height as i32 - delta;
                    self.set_active_height(height, area, content);
                }
            }
            _ => {}
        }
    }

    /// Sets the active section's height, clamped so every section below it
    /// keeps its minimum (or, when the summary has a fixed height, its
    /// current height).
    pub fn set_active_height(&mut self, height: i32, area: Rect, content: ContentHeights) {
        let areas = self.resolve(area, content);
        let below = match self.filler() {
            Some(Section::Today) => MIN_HEIGHT + areas.summary.height,
            _ => self.min_height(Section::Today) + self.min_height(Section::Summary),
        };
        let max = area.height.saturating_sub(below).max(MIN_HEIGHT);
        self.active_height = Some(clamp(height, MIN_HEIGHT, max));
    }

    /// Sets the summary's height, clamped so today's sessions keep their
    /// minimum. Only meaningful while today's sessions fill the space.
    pub fn set_summary_height(&mut self, height: i32, area: Rect, content: ContentHeights) {
        let areas = self.resolve(area, content);
        let max = area
            .height
            .saturating_sub(areas.active.height + MIN_HEIGHT)
            .max(MIN_HEIGHT);
        self.summary_height = Some(clamp(height, MIN_HEIGHT, max));
    }

    /// Back to content-fitting heights; collapsed state is kept.
    pub fn reset_sizes(&mut self) {
        self.active_height = None;
        self.summary_height = None;
    }

    /// Parses the `key=value` layout file. Lenient: unknown keys, malformed
    /// lines and bad values are ignored, falling back to defaults, so an old
    /// or hand-edited file never stops the app from starting.
    pub fn parse(text: &str) -> Self {
        let mut prefs = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            let height = || value.parse::<u16>().ok().filter(|&h| h >= MIN_HEIGHT);
            let flag = || value.parse::<bool>().ok();
            match key {
                "active_height" => prefs.active_height = height(),
                "summary_height" => prefs.summary_height = height(),
                "today_collapsed" => prefs.today_collapsed = flag().unwrap_or_default(),
                "summary_collapsed" => prefs.summary_collapsed = flag().unwrap_or_default(),
                _ => {}
            }
        }
        prefs
    }

    pub fn to_file_contents(&self) -> String {
        let height = |h: Option<u16>| h.map_or("auto".to_string(), |h| h.to_string());
        format!(
            "# jiary main-screen layout; written automatically\n\
             active_height={}\n\
             summary_height={}\n\
             today_collapsed={}\n\
             summary_collapsed={}\n",
            height(self.active_height),
            height(self.summary_height),
            self.today_collapsed,
            self.summary_collapsed,
        )
    }

    /// Reads the layout file; a missing file means defaults.
    pub fn load(path: &Path) -> io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Writes the layout file via a temp file and rename, so a crash
    /// mid-write never leaves a truncated file behind.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.to_file_contents())?;
        std::fs::rename(&tmp, path)
    }
}

fn clamp(value: i32, min: u16, max: u16) -> u16 {
    value.clamp(min as i32, max.max(min) as i32) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 1, 80, 22);
    const CONTENT: ContentHeights = ContentHeights {
        active: 7,
        summary: 6,
    };

    fn heights(areas: Areas) -> [u16; 3] {
        [
            areas.active.height,
            areas.today.height,
            areas.summary.height,
        ]
    }

    #[test]
    fn defaults_fit_content_and_today_fills_the_rest() {
        let areas = LayoutPrefs::default().resolve(AREA, CONTENT);
        assert_eq!(heights(areas), [7, 9, 6]);
        // Stacked without gaps, starting at the area's top.
        assert_eq!(areas.active.y, 1);
        assert_eq!(areas.today.y, 8);
        assert_eq!(areas.summary.y, 17);
        assert_eq!(areas.summary.bottom(), AREA.bottom());
    }

    #[test]
    fn collapsed_sections_take_one_row() {
        let prefs = LayoutPrefs {
            summary_collapsed: true,
            ..Default::default()
        };
        assert_eq!(heights(prefs.resolve(AREA, CONTENT)), [7, 14, 1]);

        // With today collapsed, the summary fills instead.
        let prefs = LayoutPrefs {
            today_collapsed: true,
            ..Default::default()
        };
        assert_eq!(heights(prefs.resolve(AREA, CONTENT)), [7, 1, 14]);

        // Both collapsed: spare rows stay empty at the bottom.
        let prefs = LayoutPrefs {
            today_collapsed: true,
            summary_collapsed: true,
            ..Default::default()
        };
        assert_eq!(heights(prefs.resolve(AREA, CONTENT)), [7, 1, 1]);
    }

    #[test]
    fn small_area_shrinks_summary_then_active_to_keep_today_visible() {
        let small = Rect::new(0, 0, 80, 12);
        // 7 + 3 + 6 = 16 > 12: summary gives 3 rows, active 1.
        assert_eq!(
            heights(LayoutPrefs::default().resolve(small, CONTENT)),
            [6, 3, 3]
        );
    }

    #[test]
    fn tiny_area_clips_at_the_bottom() {
        let tiny = Rect::new(0, 0, 80, 5);
        let areas = LayoutPrefs::default().resolve(tiny, CONTENT);
        assert_eq!(heights(areas), [3, 2, 0]);
        assert!(areas.summary.bottom() <= tiny.bottom());
    }

    #[test]
    fn resizing_active_moves_the_border_below_it() {
        let mut prefs = LayoutPrefs::default();
        prefs.resize(Section::Active, 2, AREA, CONTENT);
        assert_eq!(prefs.active_height, Some(9));
        assert_eq!(heights(prefs.resolve(AREA, CONTENT)), [9, 7, 6]);
        // Can't grow past today's minimum.
        prefs.resize(Section::Active, 100, AREA, CONTENT);
        assert_eq!(heights(prefs.resolve(AREA, CONTENT)), [13, 3, 6]);
        // Or shrink below its own.
        prefs.resize(Section::Active, -100, AREA, CONTENT);
        assert_eq!(prefs.active_height, Some(MIN_HEIGHT));
    }

    #[test]
    fn resizing_today_trades_rows_with_the_summary() {
        let mut prefs = LayoutPrefs::default();
        prefs.resize(Section::Today, 2, AREA, CONTENT);
        assert_eq!(heights(prefs.resolve(AREA, CONTENT)), [7, 11, 4]);
        prefs.resize(Section::Today, -4, AREA, CONTENT);
        assert_eq!(heights(prefs.resolve(AREA, CONTENT)), [7, 7, 8]);

        // With the summary collapsed, rows come from the active section.
        let mut prefs = LayoutPrefs {
            summary_collapsed: true,
            ..Default::default()
        };
        prefs.resize(Section::Today, 2, AREA, CONTENT);
        assert_eq!(heights(prefs.resolve(AREA, CONTENT)), [5, 16, 1]);
    }

    #[test]
    fn resizing_a_collapsed_or_filling_section_is_a_noop() {
        let mut prefs = LayoutPrefs {
            today_collapsed: true,
            ..Default::default()
        };
        let before = prefs.clone();
        prefs.resize(Section::Today, 2, AREA, CONTENT);
        prefs.resize(Section::Summary, 2, AREA, CONTENT); // fills the space
        assert_eq!(prefs, before);
    }

    #[test]
    fn reset_restores_content_heights_but_keeps_collapsed_state() {
        let mut prefs = LayoutPrefs {
            active_height: Some(10),
            summary_height: Some(4),
            summary_collapsed: true,
            ..Default::default()
        };
        prefs.reset_sizes();
        assert_eq!(
            prefs,
            LayoutPrefs {
                summary_collapsed: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn file_round_trips_and_tolerates_junk() {
        let prefs = LayoutPrefs {
            active_height: Some(9),
            summary_height: None,
            today_collapsed: false,
            summary_collapsed: true,
        };
        assert_eq!(LayoutPrefs::parse(&prefs.to_file_contents()), prefs);

        let junk =
            "active_height=2\nsummary_height=lots\nnonsense\ncolour=blue\ntoday_collapsed=yes\n";
        assert_eq!(LayoutPrefs::parse(junk), LayoutPrefs::default());
    }

    #[test]
    fn missing_file_loads_defaults_and_save_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("jiary-layout-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("layout.conf");
        let _ = std::fs::remove_file(&path);
        assert_eq!(LayoutPrefs::load(&path).unwrap(), LayoutPrefs::default());

        let prefs = LayoutPrefs {
            active_height: Some(5),
            today_collapsed: true,
            ..Default::default()
        };
        prefs.save(&path).unwrap();
        assert_eq!(LayoutPrefs::load(&path).unwrap(), prefs);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
