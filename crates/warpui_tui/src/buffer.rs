use crossterm::style::Color;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::TuiRect;
use crate::TuiSize;
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TuiStyle {
    foreground_color: Option<Color>,
}

impl TuiStyle {
    pub fn foreground_color(self) -> Option<Color> {
        self.foreground_color
    }

    pub fn with_foreground_color(mut self, color: Color) -> Self {
        self.foreground_color = Some(color);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cell {
    symbol: String,
    continuation: bool,
    style: TuiStyle,
}

impl Cell {
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn is_continuation(&self) -> bool {
        self.continuation
    }

    pub fn style(&self) -> TuiStyle {
        self.style
    }

    fn blank() -> Self {
        Self {
            symbol: " ".to_owned(),
            continuation: false,
            style: TuiStyle::default(),
        }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiBuffer {
    size: TuiSize,
    cells: Vec<Cell>,
}

impl TuiBuffer {
    pub fn new(size: TuiSize) -> Self {
        Self {
            size,
            cells: vec![Cell::default(); usize::from(size.width) * usize::from(size.height)],
        }
    }

    pub fn size(&self) -> TuiSize {
        self.size
    }

    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.cell_index(x, y).map(|index| &self.cells[index])
    }

    pub fn set_symbol(&mut self, x: u16, y: u16, symbol: char) {
        self.write_grapheme(x, y, &symbol.to_string(), TuiStyle::default());
    }

    pub fn set_symbol_with_style(&mut self, x: u16, y: u16, symbol: char, style: TuiStyle) {
        self.write_grapheme(x, y, &symbol.to_string(), style);
    }

    pub fn write_str(&mut self, x: u16, y: u16, max_width: u16, text: &str) {
        self.write_str_with_style(x, y, max_width, text, TuiStyle::default());
    }

    pub fn write_str_with_style(
        &mut self,
        x: u16,
        y: u16,
        max_width: u16,
        text: &str,
        style: TuiStyle,
    ) {
        if x >= self.size.width || y >= self.size.height {
            return;
        }
        let max_right = x.saturating_add(max_width).min(self.size.width);
        let mut column = x;
        for grapheme in text.graphemes(true) {
            let width = grapheme_width(grapheme);
            if column.saturating_add(width) > max_right {
                break;
            }
            self.write_grapheme(column, y, grapheme, style);
            column = column.saturating_add(width);
        }
    }

    pub(crate) fn display_string_for_range(&self, y: u16, start_x: u16, end_x: u16) -> String {
        let mut line = String::new();
        for x in start_x..end_x.min(self.size.width) {
            let Some(cell) = self.cell(x, y) else {
                continue;
            };
            if !cell.is_continuation() {
                line.push_str(cell.symbol());
            }
        }
        line
    }

    pub fn fill_rect(&mut self, rect: TuiRect, symbol: char) {
        for y in rect.y..rect.bottom().min(self.size.height) {
            for x in rect.x..rect.right().min(self.size.width) {
                self.set_symbol(x, y, symbol);
            }
        }
    }

    pub fn lines(&self) -> Vec<String> {
        (0..self.size.height)
            .map(|y| self.display_string_for_range(y, 0, self.size.width))
            .collect()
    }

    fn write_grapheme(&mut self, x: u16, y: u16, grapheme: &str, style: TuiStyle) {
        if x >= self.size.width || y >= self.size.height {
            return;
        }

        let width = grapheme_width(grapheme);
        if x.saturating_add(width) > self.size.width {
            return;
        }

        self.clear_grapheme_at(x, y);
        for continuation_x in x.saturating_add(1)..x.saturating_add(width) {
            self.clear_grapheme_at(continuation_x, y);
        }

        if let Some(index) = self.cell_index(x, y) {
            self.cells[index] = Cell {
                symbol: grapheme.to_owned(),
                continuation: false,
                style,
            };
        }

        for continuation_x in x.saturating_add(1)..x.saturating_add(width) {
            if let Some(index) = self.cell_index(continuation_x, y) {
                self.cells[index] = Cell::continuation(style);
            }
        }
    }

    fn clear_grapheme_at(&mut self, x: u16, y: u16) {
        let Some(index) = self.cell_index(x, y) else {
            return;
        };

        if self.cells[index].is_continuation() {
            let mut start_x = x;
            while start_x > 0 {
                let previous_x = start_x.saturating_sub(1);
                let Some(previous_index) = self.cell_index(previous_x, y) else {
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
            let Some(continuation_index) = self.cell_index(continuation_x, y) else {
                break;
            };
            if !self.cells[continuation_index].is_continuation() {
                break;
            }
            self.cells[continuation_index] = Cell::blank();
            continuation_x = continuation_x.saturating_add(1);
        }
    }

    fn cell_index(&self, x: u16, y: u16) -> Option<usize> {
        if x >= self.size.width || y >= self.size.height {
            return None;
        }
        Some(usize::from(y) * usize::from(self.size.width) + usize::from(x))
    }
}

fn grapheme_width(grapheme: &str) -> u16 {
    UnicodeWidthStr::width(grapheme)
        .max(1)
        .try_into()
        .unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_ascii_text() {
        let mut buffer = TuiBuffer::new(TuiSize::new(5, 1));

        buffer.write_str(0, 0, 5, "abc");

        assert_eq!(buffer.lines(), vec!["abc  "]);
    }

    #[test]
    fn writes_combining_graphemes_into_one_cell() {
        let mut buffer = TuiBuffer::new(TuiSize::new(4, 1));

        buffer.write_str(0, 0, 4, "e\u{301}x");

        assert_eq!(buffer.lines(), vec!["e\u{301}x  "]);
        assert_eq!(buffer.cell(0, 0).unwrap().symbol(), "e\u{301}");
        assert!(!buffer.cell(0, 0).unwrap().is_continuation());
        assert_eq!(buffer.cell(1, 0).unwrap().symbol(), "x");
    }

    #[test]
    fn writes_wide_graphemes_into_two_columns() {
        let mut buffer = TuiBuffer::new(TuiSize::new(4, 1));

        buffer.write_str(0, 0, 4, "界a");

        assert_eq!(buffer.lines(), vec!["界a "]);
        assert_eq!(buffer.cell(0, 0).unwrap().symbol(), "界");
        assert!(buffer.cell(1, 0).unwrap().is_continuation());
        assert_eq!(buffer.cell(2, 0).unwrap().symbol(), "a");
    }

    #[test]
    fn truncates_before_graphemes_that_would_cross_the_right_edge() {
        let mut buffer = TuiBuffer::new(TuiSize::new(3, 1));

        buffer.write_str(0, 0, 3, "ab界");

        assert_eq!(buffer.lines(), vec!["ab "]);
    }

    #[test]
    fn replacing_wide_graphemes_clears_continuation_cells() {
        let mut buffer = TuiBuffer::new(TuiSize::new(4, 1));
        buffer.write_str(0, 0, 4, "界a");

        buffer.write_str(0, 0, 1, "b");

        assert_eq!(buffer.lines(), vec!["b a "]);
        assert_eq!(buffer.cell(0, 0).unwrap().symbol(), "b");
        assert!(!buffer.cell(1, 0).unwrap().is_continuation());
        assert_eq!(buffer.cell(1, 0).unwrap().symbol(), " ");
    }

    #[test]
    fn writes_text_with_style() {
        let mut buffer = TuiBuffer::new(TuiSize::new(4, 1));
        let style = TuiStyle::default().with_foreground_color(Color::Yellow);

        buffer.write_str_with_style(0, 0, 4, "ab", style);

        assert_eq!(buffer.lines(), vec!["ab  "]);
        assert_eq!(buffer.cell(0, 0).unwrap().style(), style);
        assert_eq!(buffer.cell(1, 0).unwrap().style(), style);
        assert_eq!(buffer.cell(2, 0).unwrap().style(), TuiStyle::default());
    }

    #[test]
    fn writes_wide_grapheme_continuations_with_style() {
        let mut buffer = TuiBuffer::new(TuiSize::new(3, 1));
        let style = TuiStyle::default().with_foreground_color(Color::Blue);

        buffer.write_str_with_style(0, 0, 3, "界", style);

        assert_eq!(buffer.cell(0, 0).unwrap().style(), style);
        assert_eq!(buffer.cell(1, 0).unwrap().style(), style);
    }
}
