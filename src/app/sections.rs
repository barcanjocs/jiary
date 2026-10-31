use super::cat::{CAT_BLOCK_HEIGHT, cat_fits};
use super::graphs::{GRAPH_ROWS, MIN_GRAPH_WIDTH};
use super::items::SUMMARY_LABEL_WIDTH;
use super::main_screen::text_width;
use super::text::{fmt_hms, plural};
use super::{App, Screen, chrome_layout};
use crate::layout::{Areas, ContentHeights, Section};
use crate::theme;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

/// A scrollbar in `track` (a border column) when `total` rows don't fit in
/// `visible`; nothing otherwise.
pub(super) fn draw_scrollbar(
    frame: &mut Frame,
    track: Rect,
    total: usize,
    visible: usize,
    offset: usize,
) {
    if total <= visible || track.height == 0 {
        return;
    }
    let mut state = ScrollbarState::new(total.saturating_sub(visible))
        .position(offset)
        .viewport_content_length(visible);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_style(Style::new().fg(theme::ACCENT)),
        track,
        &mut state,
    );
}

/// Rows today's sessions keep for the summary to show its graphs: about two
/// sessions.
const TODAY_ROOM_FOR_GRAPHS: u16 = 8;

impl App {
    /// Handles a mouse event; only the main screen uses the mouse. A press
    /// focuses the section under the pointer. Dragging the line between two
    /// sections resizes; clicking a collapsible section's title text (press
    /// and release without moving) collapses or expands it. The wheel
    /// scrolls today's sessions.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.screen, Screen::Main) {
            return;
        }
        let [_, content, _] = chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        let areas = self.resolve_layout(content);
        let (column, row) = (mouse.column, mouse.row);

        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                // The section under the pointer; elsewhere, today's sessions.
                let pointer = ratatui::layout::Position::new(column, row);
                let target = if areas.active.contains(pointer) {
                    Section::Active
                } else {
                    Section::Today
                };
                let delta = if mouse.kind == MouseEventKind::ScrollUp {
                    -3
                } else {
                    3
                };
                self.scroll(target, delta);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let position = ratatui::layout::Position::new(column, row);
                if let Some(section) = [Section::Active, Section::Today, Section::Summary]
                    .into_iter()
                    .find(|&s| areas.get(s).contains(position))
                {
                    self.focus = section;
                }
                // A title can sit on a border row, so a press may be both;
                // the release (or a move) decides which it was.
                self.drag = self.border_at(&areas, row);
                self.title_press = [Section::Today, Section::Summary]
                    .into_iter()
                    .find(|&s| self.title_hit(s, areas.get(s), column, row));
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.title_press = None; // moved: not a click
                let Some((section, grabbed_row)) = self.drag else {
                    return;
                };
                self.focus = section;
                // Where the upper section's bottom border should now be.
                let border = row.saturating_sub(grabbed_row) as i32;
                let heights = self.content_heights(content.width);
                match section {
                    Section::Active => {
                        let height = border - areas.active.y as i32 + 1;
                        self.layout.set_active_height(height, content, heights);
                    }
                    _ => {
                        let height = content.bottom() as i32 - border - 1;
                        self.layout.set_summary_height(height, content, heights);
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let toggled = self.title_press.take();
                if let Some(section) = toggled {
                    self.layout.toggle_collapsed(section);
                }
                if self.drag.take().is_some() || toggled.is_some() {
                    self.save_layout();
                }
            }
            _ => {}
        }
    }

    /// The draggable line between two sections on `row`, as (the section
    /// whose height it sets, which of its rows: 0 or 1). The line is two
    /// rows (the upper section's bottom border and the lower one's top
    /// border) and either grabs it. The line under the active section sets
    /// the active section's height; the one under today's sessions sets the
    /// summary's, only while both are expanded (otherwise the summary just
    /// fills the space).
    fn border_at(&self, areas: &Areas, row: u16) -> Option<(Section, u16)> {
        let line = |upper: Rect, lower: Rect, lower_expanded: bool| {
            if upper.height > 0 && row + 1 == upper.bottom() {
                Some(0)
            } else if lower_expanded && lower.height > 0 && row == lower.y {
                Some(1)
            } else {
                None
            }
        };
        let today_expanded = !self.layout.today_collapsed;
        if let Some(grabbed) = line(areas.active, areas.today, today_expanded) {
            return Some((Section::Active, grabbed));
        }
        if today_expanded && !self.layout.summary_collapsed {
            return line(areas.today, areas.summary, true).map(|g| (Section::Summary, g));
        }
        None
    }

    /// The title of a section as drawn: in its top border when expanded, the
    /// whole row when collapsed. Shared with `title_hit`.
    pub(super) fn section_title(&self, section: Section) -> String {
        let sessions = plural(self.today_sessions.len(), "session");
        match (section, self.layout.collapsed(section)) {
            (Section::Active, _) => " ACTIVE ".to_string(),
            (Section::Today, false) => format!(" ▾ TODAY · {sessions} "),
            (Section::Today, true) => {
                format!("▸ TODAY · {sessions} · {}", fmt_hms(self.total_today()))
            }
            (Section::Summary, false) => " ▾ SUMMARY ".to_string(),
            (Section::Summary, true) => format!("▸ {}", self.summary_one_liner()),
        }
    }

    /// Whether (`column`, `row`) is on `section`'s title text: right after
    /// the border's corner when expanded, from the left edge when collapsed.
    fn title_hit(&self, section: Section, area: Rect, column: u16, row: u16) -> bool {
        if area.height == 0 || row != area.y {
            return false;
        }
        let start = area.x + if self.layout.collapsed(section) { 0 } else { 1 };
        let width = self.section_title(section).chars().count() as u16;
        (start..start + width).contains(&column)
    }

    fn main_areas(&self) -> Areas {
        let [_, content, _] = chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        self.resolve_layout(content)
    }

    /// The main screen's sections. While a session can be rated, a row
    /// below the active section is kept for the rating bar: the sections
    /// below move down and the one filling the rest gives up the row.
    pub(super) fn resolve_layout(&self, content: Rect) -> Areas {
        let heights = self.content_heights(content.width);
        if !self.rating_open() {
            return self.layout.resolve(content, heights);
        }
        let shorter = Rect {
            height: content.height.saturating_sub(1),
            ..content
        };
        let mut areas = self.layout.resolve(shorter, heights);
        areas.today.y += 1;
        areas.summary.y += 1;
        areas
    }

    /// Content-fitting heights of the active section and the summary,
    /// borders included.
    fn content_heights(&self, width: u16) -> ContentHeights {
        ContentHeights {
            active: self.active_content_height(width),
            summary: self.summary_lines(width.saturating_sub(2)).len() as u16 + 2,
        }
    }

    /// The active section's content-fitting height, borders included: the
    /// text column (info, then notes) or the cat column, whichever is
    /// taller. The cat column keeps its caption rows even without a caption,
    /// so the layout doesn't jump when one appears.
    fn active_content_height(&self, width: u16) -> u16 {
        let inner_width = width.saturating_sub(2);
        let notes = self.note_lines(text_width(inner_width)).len();
        let text = (self.active_lines().len() + notes) as u16;
        let cat = if cat_fits(inner_width) {
            CAT_BLOCK_HEIGHT
        } else {
            0
        };
        text.max(cat) + 2
    }

    /// Whether the summary (its interior `width` columns wide) shows its
    /// graphs: only when, with them, today's sessions keep
    /// `TODAY_ROOM_FOR_GRAPHS` rows, so on a short terminal the graphs give
    /// way rather than squeezing today's list or clipping the summary.
    pub(super) fn summary_graphs_fit(&self, width: u16) -> bool {
        if (width.saturating_sub(SUMMARY_LABEL_WIDTH) as usize) < MIN_GRAPH_WIDTH {
            return false;
        }
        let [_, content, _] = chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        let active = (self.layout.active_height)
            .unwrap_or_else(|| self.active_content_height(content.width));
        let summary = self.summary_text_lines(width).len() as u16 + 2 + GRAPH_ROWS;
        let today = if self.layout.today_collapsed {
            crate::layout::COLLAPSED_HEIGHT
        } else {
            TODAY_ROOM_FOR_GRAPHS
        };
        content.height >= active + summary + today
    }

    /// The section the scroll keys act on: the focused one if it scrolls,
    /// else today's sessions.
    pub(super) fn scroll_target(&self) -> Section {
        match self.focus {
            Section::Active => Section::Active,
            _ => Section::Today,
        }
    }

    pub(super) fn scroll_position(&mut self, section: Section) -> &mut usize {
        match section {
            Section::Active => &mut self.notes_scroll,
            _ => &mut self.today_scroll,
        }
    }

    /// Scrollable rows of a section: the active session's notes, or
    /// today's sessions.
    pub(super) fn scroll_lines(&self, section: Section) -> usize {
        match section {
            Section::Active => {
                let inner_width = self.main_areas().active.width.saturating_sub(2);
                self.note_lines(text_width(inner_width)).len()
            }
            _ => self.timeline_lines().len(),
        }
    }

    /// Rows of a section's scrollable part visible on screen.
    pub(super) fn viewport(&self, section: Section) -> usize {
        let areas = self.main_areas();
        match section {
            Section::Active => {
                // The notes fill the text column below the session info.
                let inner = areas.active.height.saturating_sub(2) as usize;
                inner.saturating_sub(self.active_lines().len())
            }
            _ if self.layout.today_collapsed => 0,
            _ => areas.today.height.saturating_sub(2) as usize,
        }
    }

    pub(super) fn max_scroll(&self, section: Section) -> usize {
        self.scroll_lines(section)
            .saturating_sub(self.viewport(section))
    }

    pub(super) fn scroll(&mut self, section: Section, delta: isize) {
        let max = self.max_scroll(section);
        let position = self.scroll_position(section);
        *position = position.saturating_add_signed(delta).min(max);
    }

    pub(super) fn resize_focused(&mut self, delta: i32) {
        let [_, content, _] = chrome_layout(Rect::new(0, 0, self.size.0, self.size.1));
        let heights = self.content_heights(content.width);
        self.layout.resize(self.focus, delta, content, heights);
        self.save_layout();
    }

    /// Writes the layout file, if there is one. A failure only costs the
    /// layout, so it is reported without stopping anything.
    pub(super) fn save_layout(&mut self) {
        let result = self.layout_path.as_ref().map(|path| self.layout.save(path));
        if let Some(Err(e)) = result {
            self.set_error(format!("Could not save layout: {e}"));
        }
    }
}
