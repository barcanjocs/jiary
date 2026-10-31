use super::text::wrap_text;
use crate::theme;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, HighlightSpacing, List, ListItem, Paragraph};

/// Fixed size of the modals; centered in the content area and shrunk to fit
/// on tiny terminals (see `centered_rect`). The start form needs room for the
/// suggestion list; end/note forms only hold a tabs/context line plus one input.
const MODAL_WIDTH: u16 = 40;

pub(super) const START_MODAL_HEIGHT: u16 = 12;

pub(super) const SMALL_MODAL_HEIGHT: u16 = 6;

/// A picker `List` with the accent highlight symbol and style.
/// The symbol column is always reserved, so items don't shift right when
/// one gets selected.
pub(super) fn picker_list<'a>(items: Vec<ListItem<'a>>) -> List<'a> {
    List::new(items)
        .highlight_symbol(theme::picker_symbol())
        .highlight_style(theme::PICKER_HIGHLIGHT)
        .highlight_spacing(HighlightSpacing::Always)
}

/// A fixed-size rect centered in `area`, shrunk to fit when the area is too
/// small so tiny terminals don't break the layout.
fn centered_rect(width: u16, height: u16, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
    let y = area
        .y
        .saturating_add(area.height.saturating_sub(height) / 2);
    ratatui::layout::Rect::new(x, y, width.min(area.width), height.min(area.height))
}

/// A centered modal of the given size: (modal rect, content area inside the
/// border with a one-column padding).
pub(super) fn modal_layout(
    content: ratatui::layout::Rect,
    width: u16,
    height: u16,
) -> (ratatui::layout::Rect, ratatui::layout::Rect) {
    let modal = centered_rect(width, height, content);
    let inner = Block::bordered().inner(modal);
    let padded = ratatui::layout::Rect::new(
        inner.x.saturating_add(1),
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    (modal, padded)
}

/// Layout of the start-session modal: (modal rect, tabs row, input rows,
/// list). The modal grows by one row per input row beyond the first, so the
/// list keeps its room. Shared by the
/// renderer and `cursor_position` so the two can never disagree.
pub(super) fn start_modal_layout(
    content: ratatui::layout::Rect,
    input_rows: u16,
) -> (
    ratatui::layout::Rect,
    ratatui::layout::Rect,
    ratatui::layout::Rect,
    ratatui::layout::Rect,
) {
    let height = START_MODAL_HEIGHT + input_rows.saturating_sub(1);
    let (modal, inner) = modal_layout(content, MODAL_WIDTH, height);
    let [tabs_area, input_area, list_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(input_rows),
        Constraint::Min(0),
    ])
    .areas(inner);
    (modal, tabs_area, input_area, list_area)
}

/// Layout of the end/note modals: (modal rect, tabs/context row, body). The
/// body holds `body_rows` rows (the wrapped input, or the rating line) and
/// the modal grows to fit them. Shared by the renderers and
/// `cursor_position`.
pub(super) fn small_modal_layout(
    content: ratatui::layout::Rect,
    body_rows: u16,
) -> (
    ratatui::layout::Rect,
    ratatui::layout::Rect,
    ratatui::layout::Rect,
) {
    let height = SMALL_MODAL_HEIGHT + body_rows.saturating_sub(1);
    let (modal, inner) = modal_layout(content, MODAL_WIDTH, height);
    let [top_area, body_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    (modal, top_area, body_area)
}

/// Text width inside any modal on `content` (border plus padding on each
/// side), accounting for `centered_rect` shrinking narrow terminals.
pub(super) fn modal_inner_width(content: ratatui::layout::Rect) -> u16 {
    MODAL_WIDTH.min(content.width).saturating_sub(4)
}

/// A text input wrapped to fit its modal: an accent `›` prompt on the first
/// row, the value word-wrapped after it with later rows indented to line up,
/// a dimmed placeholder while empty, and the cursor right after the last
/// character.
pub(super) struct WrappedInput {
    lines: Vec<Line<'static>>,
    /// Cursor (column, row), relative to the input area.
    cursor: (u16, u16),
}

impl WrappedInput {
    /// Wraps `value` after the prompt within `width` columns. When that
    /// takes more than `max_rows` rows, only the last ones are kept: the
    /// cursor is always on the last row, so it stays visible.
    pub(super) fn new(value: &str, placeholder: &str, width: u16, max_rows: u16) -> Self {
        const PROMPT: &str = "› ";
        let indent = PROMPT.chars().count();
        let rows = wrap_text(value, (width as usize).saturating_sub(indent));
        let last = rows.len() - 1;
        let cursor_col = indent + rows[last].chars().count();

        let mut lines = rows
            .into_iter()
            .enumerate()
            .map(|(i, row)| match i {
                0 if value.is_empty() => Line::from(vec![
                    Span::styled(PROMPT, theme::PROMPT),
                    Span::styled(placeholder.to_string(), theme::PLACEHOLDER),
                ]),
                0 => Line::from(vec![Span::styled(PROMPT, theme::PROMPT), Span::raw(row)]),
                _ => Line::from(format!("{}{row}", " ".repeat(indent))),
            })
            .collect::<Vec<_>>();
        let hidden = lines.len().saturating_sub(max_rows as usize);
        lines.drain(..hidden);

        Self {
            lines,
            cursor: (cursor_col as u16, (last - hidden) as u16),
        }
    }

    pub(super) fn height(&self) -> u16 {
        self.lines.len() as u16
    }

    pub(super) fn paragraph(&self) -> Paragraph<'static> {
        Paragraph::new(self.lines.clone())
    }

    /// Absolute cursor position when drawn at `area`, kept inside it on
    /// terminals too narrow for the modal.
    pub(super) fn cursor_in(&self, area: ratatui::layout::Rect) -> (u16, u16) {
        let (col, row) = self.cursor;
        (area.x + col.min(area.width.saturating_sub(1)), area.y + row)
    }
}
