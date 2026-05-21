use arrayvec::ArrayVec;
use num_traits::SaturatingSub;
use std::{
    cmp,
    fmt::{self, Debug},
    ops::{Bound, Index, Range, RangeBounds},
    rc::Rc,
};
use string_offset::{ByteOffset, CharOffset};
use sum_tree::{self, SeekBias, SumTree};
use warpui::text::point::Point;
use warpui::text_layout::TextStyle;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Run {
    Newline,
    Chars { len: usize, char_size: u8 },
}

impl sum_tree::Item for Run {
    type Summary = TextSummary;

    fn summary(&self) -> Self::Summary {
        match *self {
            Run::Newline => TextSummary {
                chars: 1.into(),
                bytes: 1.into(),
                lines: Point::new(1, 0),
                first_line_len: 0,
                rightmost_point: Point::new(0, 0),
            },
            Run::Chars { len, char_size } => TextSummary {
                chars: len.into(),
                bytes: (len * char_size as usize).into(),
                lines: Point::new(0, len as u32),
                first_line_len: len as u32,
                rightmost_point: Point::new(0, len as u32),
            },
        }
    }
}

impl Run {
    fn char_size(&self) -> u8 {
        match self {
            Run::Newline => 1,
            Run::Chars { char_size, .. } => *char_size,
        }
    }
}

/// A summary of text locations.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextSummary {
    pub chars: CharOffset,
    pub bytes: ByteOffset,
    pub lines: Point,
    pub first_line_len: u32,
    pub rightmost_point: Point,
}

impl<'a> std::ops::AddAssign<&'a Self> for TextSummary {
    fn add_assign(&mut self, other: &'a Self) {
        let joined_line_len = self.lines.column + other.first_line_len;
        if joined_line_len > self.rightmost_point.column {
            self.rightmost_point = Point::new(self.lines.row, joined_line_len);
        }
        if other.rightmost_point.column > self.rightmost_point.column {
            self.rightmost_point = self.lines + other.rightmost_point;
        }

        if self.lines.row == 0 {
            self.first_line_len += other.first_line_len;
        }

        self.chars += other.chars;
        self.bytes += other.bytes;
        self.lines += other.lines;
    }
}

impl std::ops::AddAssign<Self> for TextSummary {
    fn add_assign(&mut self, other: Self) {
        *self += &other;
    }
}

impl sum_tree::Dimension<'_, TextSummary> for TextSummary {
    fn add_summary(&mut self, summary: &TextSummary) {
        *self += summary;
    }
}

impl sum_tree::Dimension<'_, TextSummary> for Point {
    fn add_summary(&mut self, summary: &TextSummary) {
        *self += summary.lines;
    }
}

impl sum_tree::Dimension<'_, TextSummary> for ByteOffset {
    fn add_summary(&mut self, summary: &TextSummary) {
        *self += summary.bytes
    }
}

impl sum_tree::Dimension<'_, TextSummary> for CharOffset {
    fn add_summary(&mut self, summary: &TextSummary) {
        *self += summary.chars;
    }
}

#[derive(Clone)]
pub struct Text {
    text: Rc<str>,
    runs: SumTree<Run>,
    range: Range<CharOffset>,
    pub text_style: Option<TextStyle>,
}

impl Text {
    pub fn new(text: impl Into<String>, text_style: Option<TextStyle>) -> Self {
        let mut text = Text::from(text.into());
        text.text_style = text_style;
        text
    }

    pub fn with_text_style(mut self, text_style: impl Into<Option<TextStyle>>) -> Self {
        self.text_style = text_style.into();
        self
    }

    pub fn fallback_text_style_with<F>(&mut self, fallback: F)
    where
        F: FnOnce() -> Option<TextStyle>,
    {
        if self.text_style.is_none() {
            self.text_style = fallback();
        }
    }

    pub fn text_style(&self) -> Option<TextStyle> {
        self.text_style
    }
}

impl From<String> for Text {
    fn from(text: String) -> Self {
        let mut runs = Vec::new();

        let mut chars_len = 0;
        let mut run_char_size = 0;
        let mut run_chars = 0;

        let mut chars = text.chars();
        loop {
            let ch = chars.next();
            let ch_size = ch.map_or(0, |ch| ch.len_utf8());
            if run_chars != 0 && (ch.is_none() || ch == Some('\n') || run_char_size != ch_size) {
                runs.push(Run::Chars {
                    len: run_chars,
                    char_size: run_char_size as u8,
                });
                run_chars = 0;
            }
            run_char_size = ch_size;

            match ch {
                Some('\n') => runs.push(Run::Newline),
                Some(_) => run_chars += 1,
                None => break,
            }
            chars_len += 1;
        }

        let mut tree = SumTree::new();
        tree.extend(runs);
        Text {
            text: text.into(),
            runs: tree,
            range: 0.into()..chars_len.into(),
            text_style: None,
        }
    }
}

impl<'a> From<&'a str> for Text {
    fn from(text: &'a str) -> Self {
        Self::from(String::from(text))
    }
}

impl Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Text")
            .field("text", &self.text)
            .field("range", &self.range)
            .field("text_style", &self.text_style)
            .finish()
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for Text {}

impl<T: RangeBounds<CharOffset>> Index<T> for Text {
    type Output = str;

    fn index(&self, range: T) -> &Self::Output {
        let start = match range.start_bound() {
            Bound::Included(start) => cmp::min(self.range.start + *start, self.range.end),
            Bound::Excluded(_) => unimplemented!(),
            Bound::Unbounded => self.range.start,
        };
        let end = match range.end_bound() {
            Bound::Included(end) => cmp::min(self.range.start + *end + 1, self.range.end),
            Bound::Excluded(end) => cmp::min(self.range.start + *end, self.range.end),
            Bound::Unbounded => self.range.end,
        };

        let byte_start = self.abs_byte_offset_for_offset(start);
        let byte_end = self.abs_byte_offset_for_offset(end);
        &self.text[byte_start.as_usize()..byte_end.as_usize()]
    }
}

impl Text {
    pub fn range(&self) -> Range<CharOffset> {
        self.range.clone()
    }

    pub fn as_str(&self) -> &str {
        &self[..]
    }

    pub fn slice<T: RangeBounds<CharOffset>>(&self, range: T) -> Text {
        let start = match range.start_bound() {
            Bound::Included(start) => cmp::min(self.range.start + *start, self.range.end),
            Bound::Excluded(_) => unimplemented!(),
            Bound::Unbounded => self.range.start,
        };
        let end = match range.end_bound() {
            Bound::Included(end) => cmp::min(self.range.start + *end + 1, self.range.end),
            Bound::Excluded(end) => cmp::min(self.range.start + *end, self.range.end),
            Bound::Unbounded => self.range.end,
        };

        Text {
            text: self.text.clone(),
            runs: self.runs.clone(),
            range: start..end,
            text_style: self.text_style,
        }
    }

    pub fn line_len(&self, row: u32) -> u32 {
        let mut cursor = self.runs.cursor::<CharOffset, Point>();
        cursor.seek(&self.range.start, SeekBias::Right);
        let absolute_row = cursor.start().row + row;

        let mut cursor = self.runs.cursor::<Point, CharOffset>();
        cursor.seek(&Point::new(absolute_row, 0), SeekBias::Right);
        let prefix_len = self.range.start.saturating_sub(cursor.start());
        let line_len =
            cursor.summary::<CharOffset>(&Point::new(absolute_row + 1, 0), SeekBias::Left);
        let suffix_len = cursor.start().saturating_sub(&self.range.end);

        line_len
            .saturating_sub(&prefix_len)
            .saturating_sub(&suffix_len)
            .as_usize() as u32
    }

    pub fn len(&self) -> CharOffset {
        self.range.end - self.range.start
    }

    pub fn byte_len(&self) -> ByteOffset {
        self.as_str().len().into()
    }

    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }

    pub fn lines(&self) -> Point {
        self.abs_point_for_offset(self.range.end) - self.abs_point_for_offset(self.range.start)
    }

    pub fn rightmost_point(&self) -> Point {
        let lines = self.lines();

        let mut candidates = ArrayVec::<Point, 3>::new();
        candidates.push(lines);
        if lines.row > 0 {
            candidates.push(Point::new(0, self.line_len(0)));
            if lines.row > 1 {
                let mut cursor = self.runs.cursor::<CharOffset, Point>();
                cursor.seek(&self.range.start, SeekBias::Right);
                let absolute_start_row = cursor.start().row;

                let mut cursor = self.runs.cursor::<Point, CharOffset>();
                cursor.seek(&Point::new(absolute_start_row + 1, 0), SeekBias::Right);
                let summary = cursor.summary::<TextSummary>(
                    &Point::new(absolute_start_row + lines.row, 0),
                    SeekBias::Left,
                );

                candidates.push(Point::new(1, 0) + summary.rightmost_point);
            }
        }

        candidates.into_iter().max_by_key(|p| p.column).unwrap()
    }

    pub fn point_for_offset(&self, offset: CharOffset) -> Point {
        self.abs_point_for_offset(self.range.start + offset)
            - self.abs_point_for_offset(self.range.start)
    }

    pub fn offset_for_point(&self, point: Point) -> CharOffset {
        let mut cursor = self.runs.cursor::<Point, TextSummary>();
        let abs_point = self.abs_point_for_offset(self.range.start) + point;
        cursor.seek(&abs_point, SeekBias::Right);
        let overshoot = abs_point - cursor.start().lines;
        let abs_offset = cursor.start().chars.as_usize() + overshoot.column as usize;
        CharOffset::from(abs_offset) - self.range.start
    }

    pub fn byte_offset_for_point(&self, point: Point) -> ByteOffset {
        // Compute the number of characters the `point` is from the start of the text.
        let character_offset = self.offset_for_point(point);

        let num_bytes_to_point =
            self.abs_byte_offset_for_offset(character_offset + self.range.start);
        let num_bytes_to_start = self.abs_byte_offset_for_offset(self.range.start);

        num_bytes_to_point - num_bytes_to_start
    }

    pub fn summary(&self) -> TextSummary {
        TextSummary {
            chars: self.range.end - self.range.start,
            bytes: self.abs_byte_offset_for_offset(self.range.end)
                - self.abs_byte_offset_for_offset(self.range.start),
            lines: self.abs_point_for_offset(self.range.end)
                - self.abs_point_for_offset(self.range.start),
            first_line_len: self.line_len(0),
            rightmost_point: self.rightmost_point(),
        }
    }

    /// Computes the number of equivalent chars from the start of the `Text` given the number of
    /// bytes from the start of the `Text`.
    pub fn char_offset_for_byte_offset(&self, byte_offset: ByteOffset) -> CharOffset {
        let mut cursor = self.runs.cursor::<ByteOffset, TextSummary>();
        let abs_byte_offset = self.abs_byte_offset_for_offset(self.range.start) + byte_offset;
        cursor.seek(&abs_byte_offset, SeekBias::Right);

        let overshoot = abs_byte_offset - cursor.start().bytes;

        // Determine the number of characters based on the char size of the fragment.
        let absolute_chars = cursor.start().chars
            + ((overshoot).as_usize() / (cursor.item().map_or(1, |run| run.char_size()) as usize));

        // Convert character offset from the start of the text back to a relative offset back to the
        // the start of the range.
        absolute_chars - self.range.start
    }

    fn abs_point_for_offset(&self, offset: CharOffset) -> Point {
        let mut cursor = self.runs.cursor::<CharOffset, TextSummary>();
        cursor.seek(&offset, SeekBias::Right);
        let overshoot = (offset - cursor.start().chars).as_usize() as u32;
        cursor.start().lines + Point::new(0, overshoot)
    }

    /// Computes the byte offset from the start of the `self.text` given a character offset from
    /// the start of `self.text`.
    fn abs_byte_offset_for_offset(&self, offset: CharOffset) -> ByteOffset {
        let mut cursor = self.runs.cursor::<CharOffset, TextSummary>();
        cursor.seek(&offset, SeekBias::Right);
        let overshoot = (offset - cursor.start().chars).as_usize();
        cursor.start().bytes + (overshoot * cursor.item().map_or(0, |run| run.char_size()) as usize)
    }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
