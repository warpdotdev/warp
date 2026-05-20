use super::super::buffer::{AnchorRangeExt, TextSummary};
use super::buffer::StylizedChar;
use super::{buffer, Anchor, Buffer, DisplayPoint, Edit, Point, ToCharOffset};
use crate::util::extensions::SliceExt as _;
use anyhow::{anyhow, Result};
use std::{
    cmp::{self, Ordering},
    iter::Take,
    ops::Range,
};
use string_offset::CharOffset;
use sum_tree::{self, Cursor, Dimension, SeekBias, SumTree};
use warpui::text_layout::TextStyle;
use warpui::{AppContext, ModelHandle};

pub struct FoldMap {
    buffer: ModelHandle<Buffer>,
    transforms: SumTree<Transform>,
    folds: Vec<Range<Anchor>>,
}

impl FoldMap {
    pub fn new(buffer: ModelHandle<Buffer>, app: &AppContext) -> Self {
        let text_summary = buffer.as_ref(app).text_summary();
        Self {
            buffer,
            folds: Vec::new(),
            transforms: SumTree::from_item(Transform {
                summary: TransformSummary {
                    buffer: text_summary.clone(),
                    display: text_summary,
                },
                display_text: None,
            }),
        }
    }

    pub fn buffer_rows(&self, start_row: u32) -> Result<BufferRows<'_>> {
        if start_row > self.transforms.summary().display.lines.row {
            return Err(anyhow!("invalid display row {}", start_row));
        }

        let display_point = Point::new(start_row, 0);
        let mut cursor = self.transforms.cursor();
        cursor.seek(&DisplayPoint(display_point), SeekBias::Left);

        Ok(BufferRows {
            cursor,
            display_point,
        })
    }

    pub fn len(&self) -> CharOffset {
        self.transforms.summary().display.chars
    }

    pub fn line_len(&self, row: u32, ctx: &AppContext) -> Result<u32> {
        let line_start = self.to_display_offset(DisplayPoint::new(row, 0), ctx)?.0;
        let line_end = if row >= self.max_point().row() {
            self.len().as_usize()
        } else {
            self.to_display_offset(DisplayPoint::new(row + 1, 0), ctx)?
                .0
                - 1
        };

        Ok((line_end - line_start) as u32)
    }

    pub fn chars_with_style_at<'a>(
        &'a self,
        point: DisplayPoint,
        app: &'a AppContext,
    ) -> Result<CharsWithStyle<'a>> {
        let offset = self.to_display_offset(point, app)?;
        let mut cursor = self.transforms.cursor();
        cursor.seek(&offset, SeekBias::Right);
        let buffer = self.buffer.as_ref(app);
        Ok(CharsWithStyle {
            cursor,
            offset: CharOffset::from(offset.0),
            buffer,
            buffer_chars: None,
        })
    }

    pub fn chars_at<'a>(&'a self, point: DisplayPoint, app: &'a AppContext) -> Result<Chars<'a>> {
        Ok(Chars(self.chars_with_style_at(point, app)?))
    }

    pub fn max_point(&self) -> DisplayPoint {
        DisplayPoint(self.transforms.summary().display.lines)
    }

    pub fn rightmost_point(&self) -> DisplayPoint {
        DisplayPoint(self.transforms.summary().display.rightmost_point)
    }

    pub fn fold<T: ToCharOffset>(
        &mut self,
        ranges: impl IntoIterator<Item = Range<T>>,
        app: &AppContext,
    ) -> Result<()> {
        let mut edits = Vec::new();
        let buffer = self.buffer.as_ref(app);
        for range in ranges.into_iter() {
            let start = range.start.to_char_offset(buffer)?;
            let end = range.end.to_char_offset(buffer)?;
            edits.push(Edit {
                old_range: start..end,
                new_range: start..end,
            });

            let fold = buffer.anchor_after(start)?..buffer.anchor_before(end)?;
            let ix = self
                .folds
                .find_insertion_index(|probe| probe.cmp(&fold, buffer))?;
            self.folds.insert(ix, fold);
        }
        edits.sort_unstable_by(|a, b| {
            a.old_range
                .start
                .cmp(&b.old_range.start)
                .then_with(|| b.old_range.end.cmp(&a.old_range.end))
        });

        self.apply_edits(&edits, app)?;
        Ok(())
    }

    pub fn unfold<T: ToCharOffset>(
        &mut self,
        ranges: impl IntoIterator<Item = Range<T>>,
        app: &AppContext,
    ) -> Result<()> {
        let buffer = self.buffer.as_ref(app);

        let mut edits = Vec::new();
        for range in ranges.into_iter() {
            let start = buffer.anchor_before(range.start.to_char_offset(buffer)?)?;
            let end = buffer.anchor_after(range.end.to_char_offset(buffer)?)?;

            // Remove intersecting folds and add their ranges to edits that are passed to apply_edits
            self.folds.retain(|fold| {
                if fold.start.cmp(&end, buffer).unwrap() > Ordering::Equal
                    || fold.end.cmp(&start, buffer).unwrap() < Ordering::Equal
                {
                    true
                } else {
                    let start = fold.start.to_char_offset(buffer);
                    let end = fold.end.to_char_offset(buffer);
                    if let Ok((start, end)) = start.and_then(|start| Ok((start, end?))) {
                        let offset_range = start..end;
                        edits.push(Edit {
                            old_range: offset_range.clone(),
                            new_range: offset_range,
                        });
                    }

                    false
                }
            });
        }

        self.apply_edits(&edits, app)?;
        Ok(())
    }

    pub fn is_line_folded(&self, display_row: u32) -> bool {
        let mut cursor = self.transforms.cursor::<DisplayPoint, DisplayPoint>();
        cursor.seek(&DisplayPoint::new(display_row, 0), SeekBias::Right);
        while let Some(transform) = cursor.item() {
            if transform.display_text.is_some() {
                return true;
            }
            if cursor.end().row() == display_row {
                cursor.next()
            } else {
                break;
            }
        }
        false
    }

    pub fn to_display_offset(
        &self,
        point: DisplayPoint,
        app: &AppContext,
    ) -> Result<DisplayOffset> {
        let mut cursor = self.transforms.cursor::<DisplayPoint, TransformSummary>();
        cursor.seek(&point, SeekBias::Right);
        let overshoot = point.0 - cursor.start().display.lines;
        let mut offset = cursor.start().display.chars;
        if !overshoot.is_zero() {
            let transform = cursor
                .item()
                .ok_or_else(|| anyhow!("display point {:?} is out of range", point))?;
            assert!(transform.display_text.is_none());
            let end_buffer_offset = (cursor.start().buffer.lines + overshoot)
                .to_char_offset(self.buffer.as_ref(app))?;
            offset += end_buffer_offset - cursor.start().buffer.chars;
        }
        Ok(offset.into())
    }

    pub fn to_buffer_point(&self, display_point: DisplayPoint) -> Point {
        let mut cursor = self.transforms.cursor::<DisplayPoint, TransformSummary>();
        cursor.seek(&display_point, SeekBias::Right);
        let overshoot = display_point.0 - cursor.start().display.lines;
        cursor.start().buffer.lines + overshoot
    }

    pub fn to_display_point(&self, point: Point) -> DisplayPoint {
        let mut cursor = self.transforms.cursor::<Point, TransformSummary>();
        cursor.seek(&point, SeekBias::Right);
        let overshoot = point - cursor.start().buffer.lines;
        DisplayPoint(cmp::min(
            cursor.start().display.lines + overshoot,
            cursor.end().display.lines,
        ))
    }

    pub fn apply_edits(&mut self, edits: &[Edit], app: &AppContext) -> Result<()> {
        let buffer = self.buffer.as_ref(app);
        let mut edits = edits.iter().cloned().peekable();

        let mut new_transforms = SumTree::new();
        let mut cursor = self.transforms.cursor::<CharOffset, CharOffset>();
        cursor.seek(&0.into(), SeekBias::Right);

        while let Some(mut edit) = edits.next() {
            new_transforms.push_tree(cursor.slice(&edit.old_range.start, SeekBias::Left));
            edit.new_range.start -= edit.old_range.start - *cursor.start();
            edit.old_range.start = *cursor.start();

            cursor.seek(&edit.old_range.end, SeekBias::Right);
            cursor.next();

            let mut delta = edit.delta();
            loop {
                edit.old_range.end = *cursor.start();

                if let Some(next_edit) = edits.peek() {
                    if next_edit.old_range.start > edit.old_range.end {
                        break;
                    }

                    let next_edit = edits.next().unwrap();
                    delta += next_edit.delta();

                    if next_edit.old_range.end > edit.old_range.end {
                        edit.old_range.end = next_edit.old_range.end;
                        cursor.seek(&edit.old_range.end, SeekBias::Right);
                        cursor.next();
                    }
                } else {
                    break;
                }
            }

            edit.new_range.end = CharOffset::from(
                ((edit.new_range.start + edit.old_extent()).as_usize() as isize + delta) as usize,
            );

            let anchor = buffer.anchor_before(edit.new_range.start)?;
            let folds_start = self
                .folds
                .find_insertion_index(|probe| probe.start.cmp(&anchor, buffer))?;
            let mut folds = self.folds[folds_start..]
                .iter()
                .filter_map(|fold| {
                    Some(
                        fold.start.to_char_offset(buffer).ok()?
                            ..fold.end.to_char_offset(buffer).ok()?,
                    )
                })
                .peekable();

            while folds
                .peek()
                .is_some_and(|fold| fold.start < edit.new_range.end)
            {
                let mut fold = folds.next().unwrap();
                let sum = new_transforms.summary();

                assert!(fold.start >= sum.buffer.chars);

                while folds
                    .peek()
                    .is_some_and(|next_fold| next_fold.start <= fold.end)
                {
                    let next_fold = folds.next().unwrap();
                    if next_fold.end > fold.end {
                        fold.end = next_fold.end;
                    }
                }

                if fold.start > sum.buffer.chars {
                    let text_summary = buffer.text_summary_for_range(sum.buffer.chars..fold.start);
                    new_transforms.push(Transform {
                        summary: TransformSummary {
                            display: text_summary.clone(),
                            buffer: text_summary,
                        },
                        display_text: None,
                    });
                }

                if fold.end > fold.start {
                    new_transforms.push(Transform {
                        summary: TransformSummary {
                            display: TextSummary {
                                chars: 1.into(),
                                bytes: '…'.len_utf8().into(),
                                lines: Point::new(0, 1),
                                first_line_len: 1,
                                rightmost_point: Point::new(0, 1),
                            },
                            buffer: buffer.text_summary_for_range(fold.start..fold.end),
                        },
                        display_text: Some('…'),
                    });
                }
            }

            let sum = new_transforms.summary();
            if sum.buffer.chars < edit.new_range.end {
                let text_summary =
                    buffer.text_summary_for_range(sum.buffer.chars..edit.new_range.end);
                new_transforms.push(Transform {
                    summary: TransformSummary {
                        display: text_summary.clone(),
                        buffer: text_summary,
                    },
                    display_text: None,
                });
            }
        }

        new_transforms.push_tree(cursor.suffix());

        drop(cursor);
        self.transforms = new_transforms;

        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Transform {
    summary: TransformSummary,
    display_text: Option<char>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TransformSummary {
    display: TextSummary,
    buffer: TextSummary,
}

impl sum_tree::Item for Transform {
    type Summary = TransformSummary;

    fn summary(&self) -> Self::Summary {
        self.summary.clone()
    }
}

impl<'a> std::ops::AddAssign<&'a Self> for TransformSummary {
    fn add_assign(&mut self, other: &'a Self) {
        self.buffer += &other.buffer;
        self.display += &other.display;
    }
}

impl<'a> Dimension<'a, TransformSummary> for TransformSummary {
    fn add_summary(&mut self, summary: &'a TransformSummary) {
        *self += summary;
    }
}

pub struct BufferRows<'a> {
    cursor: Cursor<'a, Transform, DisplayPoint, TransformSummary>,
    display_point: Point,
}

impl Iterator for BufferRows<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        while self.display_point > self.cursor.end().display.lines {
            self.cursor.next();
            if self.cursor.item().is_none() {
                // TODO: Return a bool from next?
                break;
            }
        }

        if self.cursor.item().is_some() {
            let overshoot = self.display_point - self.cursor.start().display.lines;
            let buffer_point = self.cursor.start().buffer.lines + overshoot;
            self.display_point.row += 1;
            Some(buffer_point.row)
        } else {
            None
        }
    }
}

pub struct Chars<'a>(CharsWithStyle<'a>);

impl Iterator for Chars<'_> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(Into::into)
    }
}

pub struct CharsWithStyle<'a> {
    cursor: Cursor<'a, Transform, DisplayOffset, TransformSummary>,
    offset: CharOffset,
    buffer: &'a Buffer,
    buffer_chars: Option<Take<buffer::CharsWithStyle<'a>>>,
}

impl Iterator for CharsWithStyle<'_> {
    type Item = StylizedChar;

    fn next(&mut self) -> Option<Self::Item> {
        match self.buffer_chars.as_mut().map(Iterator::next) {
            // buffer_chars is set and has a value, return it
            Some(Some(c)) => {
                self.offset += 1;
                return Some(c);
            }
            // buffer_chars is set, but exhausted, so the current cursor item is complete
            Some(None) => {
                self.buffer_chars = None;
                self.cursor.next();
            }
            // buffer_chars is not set, if we've returned all of the current cursor item's
            // characters, then we should move to the next value
            None => {
                if self.offset == self.cursor.end().display.chars {
                    self.cursor.next();
                }
            }
        }

        self.cursor.item().and_then(|transform| {
            if let Some(c) = transform.display_text {
                self.offset += 1;
                Some(StylizedChar::new(c, TextStyle::default()))
            } else {
                let overshoot = self.offset - self.cursor.start().display.chars;
                let buffer_start = self.cursor.start().buffer.chars + overshoot;
                let char_count = self.cursor.end().buffer.chars - buffer_start;
                self.buffer_chars = Some(
                    self.buffer
                        .stylized_chars_at(buffer_start)
                        .unwrap()
                        .take(char_count.as_usize()),
                );
                self.next()
            }
        })
    }
}

impl<'a> Dimension<'a, TransformSummary> for DisplayPoint {
    fn add_summary(&mut self, summary: &'a TransformSummary) {
        self.0 += summary.display.lines;
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, Ord, PartialOrd, PartialEq)]
/// The number of _visible_ characters offset from the start of the buffer.
pub struct DisplayOffset(usize);

impl From<usize> for DisplayOffset {
    fn from(usize: usize) -> Self {
        DisplayOffset(usize)
    }
}

impl From<CharOffset> for DisplayOffset {
    fn from(char_offset: CharOffset) -> Self {
        DisplayOffset(char_offset.as_usize())
    }
}

impl<'a> Dimension<'a, TransformSummary> for DisplayOffset {
    fn add_summary(&mut self, summary: &'a TransformSummary) {
        self.0 += &summary.display.chars.as_usize();
    }
}

impl<'a> Dimension<'a, TransformSummary> for Point {
    fn add_summary(&mut self, summary: &'a TransformSummary) {
        *self += summary.buffer.lines;
    }
}

impl<'a> Dimension<'a, TransformSummary> for CharOffset {
    fn add_summary(&mut self, summary: &'a TransformSummary) {
        *self += summary.buffer.chars;
    }
}

#[cfg(test)]
#[path = "fold_map_tests.rs"]
mod tests;
