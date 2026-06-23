//! [`TuiInputLine`]: a single-row text field that owns the terminal cursor and
//! horizontally clips so the cursor stays visible.
//!
//! # Construction
//! Build with [`TuiInputLine::new`], passing the current text and the cursor's
//! position as a Unicode-scalar (char) index into that text. Style the text with
//! [`with_style`](TuiInputLine::with_style); supply an empty-state placeholder
//! with [`with_placeholder`](TuiInputLine::with_placeholder).
//!
//! # Layout policy
//! The element is always one row tall. It renders its text left-aligned, and
//! when the text is wider than the area it scrolls horizontally so the cursor
//! cell remains on screen (anchoring the cursor to the right edge once the text
//! overflows). Column widths defer to ratatui's grapheme-aware buffer writer, so
//! a wide (CJK) glyph occupies two cells and is never split. The cursor cell is
//! reported through [`cursor_position`](TuiElement::cursor_position) so the
//! presenter can place the real terminal cursor.
//!
//! This is a small, reusable building block: pair it with a
//! [`TuiContainer`](super::TuiContainer) for a bordered prompt rather than
//! drawing a frame here.

use ratatui::text::Line;

use super::{TuiBuffer, TuiConstraint, TuiElement, TuiLayoutContext, TuiRect, TuiSize, TuiStyle};

pub struct TuiInputLine {
    text: String,
    /// Cursor position as a char index into `text`, in `[0, text char count]`.
    cursor: usize,
    style: TuiStyle,
    placeholder: Option<String>,
    placeholder_style: TuiStyle,
}

impl TuiInputLine {
    pub fn new(text: impl Into<String>, cursor: usize) -> Self {
        Self {
            text: text.into(),
            cursor,
            style: TuiStyle::default(),
            placeholder: None,
            placeholder_style: TuiStyle::default(),
        }
    }

    pub fn with_style(mut self, style: TuiStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the placeholder rendered (in `style`) when the text is empty.
    pub fn with_placeholder(mut self, placeholder: impl Into<String>, style: TuiStyle) -> Self {
        self.placeholder = Some(placeholder.into());
        self.placeholder_style = style;
        self
    }

    fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    fn clamped_cursor(&self) -> usize {
        self.cursor.min(self.char_count())
    }

    /// Cumulative display widths at every char boundary: `widths[i]` is the
    /// display width of the first `i` characters (so `widths[0] == 0`).
    fn cumulative_widths(&self) -> Vec<u16> {
        let mut widths = Vec::with_capacity(self.char_count() + 1);
        widths.push(0);
        let mut acc = 0u16;
        for ch in self.text.chars() {
            acc = acc.saturating_add(char_width(ch));
            widths.push(acc);
        }
        widths
    }

    /// The first visible char index and the cursor's on-screen column for an
    /// area `width` cells wide. Scrolls only as far as needed to keep the cursor
    /// visible, anchoring it to the right edge once the text overflows.
    fn scroll(&self, widths: &[u16], width: u16) -> (usize, u16) {
        let cursor = self.clamped_cursor();
        let cursor_col = widths[cursor];
        if width == 0 || cursor_col < width {
            return (0, cursor_col);
        }
        // Smallest start whose remaining width fits the cursor in the last cell.
        let min_width = cursor_col - (width - 1);
        let start = widths.iter().position(|&w| w >= min_width).unwrap_or(0);
        (start, cursor_col - widths[start])
    }
}

impl TuiElement for TuiInputLine {
    fn layout(&mut self, constraint: TuiConstraint, _ctx: &mut TuiLayoutContext) -> TuiSize {
        let content = self
            .cumulative_widths()
            .last()
            .copied()
            .unwrap_or(0)
            .max(self.placeholder.as_deref().map_or(0, display_width));
        TuiSize::new(
            constraint.constrain_width(content),
            constraint.constrain_height(1),
        )
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {
        if area.is_empty() {
            return;
        }
        let width = area.width as usize;
        if self.text.is_empty() {
            if let Some(placeholder) = &self.placeholder {
                buffer.set_stringn(area.x, area.y, placeholder, width, self.placeholder_style);
            }
            return;
        }
        let widths = self.cumulative_widths();
        let (start, _) = self.scroll(&widths, area.width);
        let start_byte = self
            .text
            .char_indices()
            .nth(start)
            .map_or(self.text.len(), |(byte, _)| byte);
        buffer.set_stringn(area.x, area.y, &self.text[start_byte..], width, self.style);
    }

    fn cursor_position(&self, area: TuiRect, _ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        if area.is_empty() {
            return None;
        }
        let widths = self.cumulative_widths();
        let (_, x) = self.scroll(&widths, area.width);
        Some((x.min(area.width.saturating_sub(1)), 0))
    }
}

/// The display width (in cells) of `text`, deferring to ratatui's measurement so
/// it matches what the buffer paints.
fn display_width(text: &str) -> u16 {
    u16::try_from(Line::raw(text).width()).unwrap_or(u16::MAX)
}

/// The display width (in cells) of a single character.
fn char_width(ch: char) -> u16 {
    let mut buf = [0u8; 4];
    display_width(ch.encode_utf8(&mut buf))
}

#[cfg(test)]
#[path = "input_line_tests.rs"]
mod tests;
