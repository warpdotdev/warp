//! The in-memory cell grid that elements paint into and the renderer flushes to
//! the terminal.
//!
//! [`TuiBuffer`] is the headless assertion surface for the whole crate: every
//! element/presenter test paints into a buffer and compares
//! [`to_lines`](TuiBuffer::to_lines) (and, where style matters, individual
//! [`get`](TuiBuffer::get) cells) against expected values. All writes are
//! clipped to the buffer bounds and are wide- and combining-grapheme aware, so
//! callers never have to bounds-check or measure text themselves.

use crossterm::style::Color;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{TuiRect, TuiSize};

/// The visual styling applied to a [`Cell`]. Cheap to copy; equality is exact,
/// which is what makes style-sensitive buffer assertions possible.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TuiStyle {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reversed: bool,
}

impl TuiStyle {
    pub fn with_foreground(mut self, color: Color) -> Self {
        self.foreground = Some(color);
        self
    }

    pub fn with_background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub fn with_bold(mut self, bold: bool) -> Self {
        self.bold = bold;
        self
    }

    pub fn with_dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    pub fn with_italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    pub fn with_underline(mut self, underline: bool) -> Self {
        self.underline = underline;
        self
    }

    pub fn with_reversed(mut self, reversed: bool) -> Self {
        self.reversed = reversed;
        self
    }
}

/// A single terminal cell: one grapheme cluster plus its style.
///
/// A grapheme that occupies more than one column (e.g. a CJK character) is
/// stored as a leading cell holding the symbol followed by one or more
/// *continuation* cells; the renderer skips continuation cells so the wide
/// glyph is emitted exactly once. Construct printable cells with [`Cell::new`];
/// continuation cells are produced internally by the buffer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cell {
    symbol: String,
    continuation: bool,
    style: TuiStyle,
}

impl Cell {
    /// A printable cell holding `symbol` (a single grapheme cluster) styled
    /// with `style`.
    pub fn new(symbol: impl Into<String>, style: TuiStyle) -> Self {
        Self {
            symbol: symbol.into(),
            continuation: false,
            style,
        }
    }

    /// A blank (space) cell with default styling. This is the fill value of a
    /// freshly constructed buffer.
    pub fn blank() -> Self {
        Self {
            symbol: " ".to_owned(),
            continuation: false,
            style: TuiStyle::default(),
        }
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn style(&self) -> TuiStyle {
        self.style
    }

    /// Whether this cell is the trailing column of a preceding wide grapheme.
    /// The renderer emits nothing for continuation cells.
    pub fn is_continuation(&self) -> bool {
        self.continuation
    }

    fn continuation(style: TuiStyle) -> Self {
        Self {
            symbol: String::new(),
            continuation: true,
            style,
        }
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::blank()
    }
}

/// A fixed-size grid of [`Cell`]s addressed by `(x, y)` in column/row order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiBuffer {
    size: TuiSize,
    cells: Vec<Cell>,
}

impl TuiBuffer {
    /// A blank buffer of the given size.
    pub fn new(size: TuiSize) -> Self {
        Self {
            size,
            cells: vec![Cell::default(); size.area()],
        }
    }

    pub fn size(&self) -> TuiSize {
        self.size
    }

    /// The cell at `(x, y)`, or `None` if out of bounds.
    pub fn get(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index(x, y).map(|index| &self.cells[index])
    }

    /// Writes a single printable cell at `(x, y)`. Out-of-bounds positions are
    /// silently ignored. Continuation/empty cells are ignored, since the buffer
    /// manages continuations itself; use [`set_str`](TuiBuffer::set_str) for
    /// wide graphemes.
    pub fn set_cell(&mut self, x: u16, y: u16, cell: Cell) {
        if cell.continuation || cell.symbol.is_empty() {
            return;
        }
        self.write_grapheme(x, y, &cell.symbol, cell.style);
    }

    /// Writes `text` starting at `(x, y)`, clipped to the lesser of `max_width`
    /// columns and the buffer's right edge. Grapheme clusters that would cross
    /// the limit are dropped whole (a wide glyph is never split). Returns the
    /// number of columns actually advanced.
    pub fn set_str(&mut self, x: u16, y: u16, max_width: u16, text: &str, style: TuiStyle) -> u16 {
        if y >= self.size.height || x >= self.size.width {
            return 0;
        }
        let limit = x.saturating_add(max_width).min(self.size.width);
        let mut column = x;
        for grapheme in text.graphemes(true) {
            let width = grapheme_width(grapheme);
            if column.saturating_add(width) > limit {
                break;
            }
            self.write_grapheme(column, y, grapheme, style);
            column = column.saturating_add(width);
        }
        column - x
    }

    /// Fills every cell of `rect` (intersected with the buffer) with a clone of
    /// `cell`. Typically used to paint a styled background.
    pub fn fill(&mut self, rect: TuiRect, cell: Cell) {
        for y in rect.y..rect.bottom().min(self.size.height) {
            for x in rect.x..rect.right().min(self.size.width) {
                self.set_cell(x, y, cell.clone());
            }
        }
    }

    /// Renders the buffer to one `String` per row, omitting continuation cells
    /// so wide glyphs appear once. This is the primary headless assertion hook.
    pub fn to_lines(&self) -> Vec<String> {
        (0..self.size.height)
            .map(|y| {
                let mut line = String::new();
                for x in 0..self.size.width {
                    if let Some(cell) = self.get(x, y) {
                        if !cell.is_continuation() {
                            line.push_str(cell.symbol());
                        }
                    }
                }
                line
            })
            .collect()
    }

    fn write_grapheme(&mut self, x: u16, y: u16, grapheme: &str, style: TuiStyle) {
        let width = grapheme_width(grapheme);
        if x >= self.size.width
            || y >= self.size.height
            || x.saturating_add(width) > self.size.width
        {
            return;
        }

        // Clear any wide grapheme we are about to partially overwrite, both at
        // the head and across the columns the new grapheme will occupy.
        self.clear_grapheme_at(x, y);
        for continuation_x in (x + 1)..(x + width) {
            self.clear_grapheme_at(continuation_x, y);
        }

        if let Some(index) = self.index(x, y) {
            self.cells[index] = Cell {
                symbol: grapheme.to_owned(),
                continuation: false,
                style,
            };
        }
        for continuation_x in (x + 1)..(x + width) {
            if let Some(index) = self.index(continuation_x, y) {
                self.cells[index] = Cell::continuation(style);
            }
        }
    }

    /// Blanks the full grapheme covering `(x, y)`, walking left to the leading
    /// cell if `(x, y)` is a continuation, then clearing its trailing cells.
    fn clear_grapheme_at(&mut self, x: u16, y: u16) {
        let Some(index) = self.index(x, y) else {
            return;
        };

        if self.cells[index].is_continuation() {
            let mut start_x = x;
            while start_x > 0 {
                let previous_x = start_x - 1;
                let Some(previous_index) = self.index(previous_x, y) else {
                    break;
                };
                start_x = previous_x;
                if !self.cells[previous_index].is_continuation() {
                    break;
                }
            }
            self.clear_grapheme_at(start_x, y);
            return;
        }

        self.cells[index] = Cell::blank();
        let mut continuation_x = x.saturating_add(1);
        while continuation_x < self.size.width {
            let Some(continuation_index) = self.index(continuation_x, y) else {
                break;
            };
            if !self.cells[continuation_index].is_continuation() {
                break;
            }
            self.cells[continuation_index] = Cell::blank();
            continuation_x += 1;
        }
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        if x >= self.size.width || y >= self.size.height {
            return None;
        }
        Some(usize::from(y) * usize::from(self.size.width) + usize::from(x))
    }
}

/// The column width of a grapheme cluster, floored at 1 so zero-width clusters
/// (e.g. a lone combining mark) still occupy a cell.
fn grapheme_width(grapheme: &str) -> u16 {
    UnicodeWidthStr::width(grapheme)
        .max(1)
        .try_into()
        .unwrap_or(u16::MAX)
}

#[cfg(test)]
#[path = "buffer_tests.rs"]
mod tests;
