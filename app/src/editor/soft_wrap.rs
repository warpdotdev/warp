use anyhow::anyhow;
use parking_lot::Mutex;
use std::sync::Arc;

use warpui::text_layout;

use crate::editor::{view::DisplayPoint, Point};

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
pub struct SoftWrapPoint(Point);

impl SoftWrapPoint {
    pub fn new(row: u32, col: u32) -> Self {
        Self(Point::new(row, col))
    }

    pub fn row(self) -> u32 {
        self.0.row
    }

    pub fn column(self) -> u32 {
        self.0.column
    }

    pub fn column_mut(&mut self) -> &mut u32 {
        &mut self.0.column
    }
}

/// When a line is soft-wrapped, there can be ambiguity about where the cursor
/// should be drawn.  For example, if I had the text "hello world" which became
/// soft wrapped to "hello \nworld", then if the cursor is just before "w",
/// then the cursor could either be at the end of the first line or the very
/// beginning of the next line. This field keeps track of whether the cursor
/// should be clamped above or below.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum ClampDirection {
    Up,
    #[default]
    Down,
}

#[derive(Default, Clone)]
/// Wrapper around `FrameLayouts` to help manage the levels of indirection around
/// the Arc, Mutex and Option.
///
/// When this is None, we haven't yet laid out the text.
pub struct SoftWrapState(Arc<Mutex<Option<FrameLayouts>>>);

impl SoftWrapState {
    pub fn update(&self, new_value: FrameLayouts) {
        *self.0.lock() = Some(new_value);
    }

    pub fn read<T, F: FnOnce(anyhow::Result<&FrameLayouts>) -> T>(&self, callback: F) -> T {
        let frame_layouts = self.0.lock();
        match &*frame_layouts {
            Some(frame_layouts) => callback(Ok(frame_layouts)),
            None => callback(Err(anyhow!("No frame layout. This should only happen if we attempt to read before a frame of the editor has been laid out"))),
        }
    }
}

/// We layout more frames than we need to display on the screen because
/// we need to know the number of lines in previous frames to calculate
/// any row position for any given character.
///
/// In particular, we always layout from zero up to the number of lines in
/// viewport + number of lines in scrollback because this is the worst case
/// and we don't know how many rows we'll need to be laid out until we've laid
/// them out.
#[derive(Clone)]
pub struct FrameLayouts {
    frames: Vec<Arc<text_layout::TextFrame>>,
    /// The first line that is displayed. The very first line in the editor
    /// is 0, and each displayed line is incremented by 1. So if soft wrapping
    /// is off, this is one-to-one with the rows of the buffer, but if soft
    /// wrapping is on, the line number is the number of lines from the top
    /// given soft wrapping.
    start_line: u32,
    end_line: u32,
}

impl FrameLayouts {
    pub fn new(frames: Vec<Arc<text_layout::TextFrame>>, start_line: u32, end_line: u32) -> Self {
        Self {
            frames,
            start_line,
            end_line,
        }
    }

    pub fn frames(&self) -> impl Iterator<Item = &Arc<text_layout::TextFrame>> {
        self.frames.iter()
    }

    pub fn last_frame(&self) -> Option<&Arc<text_layout::TextFrame>> {
        self.frames.last()
    }

    pub fn displayed_lines(&self) -> impl Iterator<Item = &text_layout::Line> {
        FrameLayoutDisplayedLines::zero(self).skip(self.start_line as usize)
    }

    /// Number of lines displayed in the editor.
    pub fn num_displayed_lines(&self) -> u32 {
        self.displayed_lines().count() as u32
    }

    pub fn num_lines(&self) -> usize {
        self.frames().map(|frame| frame.lines().len()).sum()
    }

    /// Returns the index of the row that represents the end of some logical row
    pub fn end_of_logical_row(&self, logical_row: usize) -> usize {
        self.frames
            .iter()
            // adding 1 to include the TextFrame that represents this row (the queried one)
            .take(logical_row + 1)
            .fold(0, |acc, text_frame| acc + text_frame.lines().len())
            // subtracting one to convert a count of rows into an index
            - 1
    }

    /// Returns the line at the given index, where the first line is 0 if it exists.
    pub fn get_line(&self, desired_index: usize) -> Option<&text_layout::Line> {
        self.frames
            .iter()
            .flat_map(|frame| frame.lines())
            .nth(desired_index)
    }

    /// Given a `DisplayPoint`, converts the row number such that soft wrapping
    /// is considered. This function will yield a result in the case where the
    /// provided point is within the bounds of the last laid out text. Otherwise,
    /// it will return `None`.
    pub fn to_soft_wrap_point(
        &self,
        point: DisplayPoint,
        clamp_direction: ClampDirection,
    ) -> Option<SoftWrapPoint> {
        let rows_before_frame: u32 = self
            .frames
            .iter()
            .take(point.row() as usize)
            .fold(0, |acc, text_frame| acc + text_frame.lines().len() as u32);

        let frame = self.frames.get(point.row() as usize)?;

        let rows_within_frame = frame.row_within_frame(
            point.column() as usize,
            matches!(clamp_direction, ClampDirection::Up),
        ) as u32;
        Some(SoftWrapPoint::new(
            rows_before_frame + rows_within_frame,
            point.column(),
        ))
    }

    /// Given a `SoftWrapPoint`, converts the row number to a value irrespective
    /// of soft-wrapping.
    pub fn to_display_point(&self, point: SoftWrapPoint) -> DisplayPointAndClampDirection {
        let mut acc_lines = 0;
        let mut row = 0;
        let mut clamp_direction = Default::default();
        for (i, frame) in self.frames.iter().enumerate() {
            if acc_lines + frame.lines().len() > point.row() as usize {
                row = i;
                break;
            }
            acc_lines += frame.lines().len()
        }
        if let Some(line) = self.get_line(point.row() as usize) {
            if let Some(glyph) = line.last_glyph() {
                if point.column() as usize == glyph.index + 1 {
                    clamp_direction = ClampDirection::Up;
                }
            } else if let Some(glyph) = line.first_glyph() {
                if point.column() as usize == glyph.index {
                    clamp_direction = ClampDirection::Down;
                }
            }
        }

        DisplayPointAndClampDirection {
            point: DisplayPoint::new(row as u32, point.column()),
            clamp_direction,
        }
    }
}

#[derive(Debug)]
pub struct DisplayPointAndClampDirection {
    pub point: DisplayPoint,
    pub clamp_direction: ClampDirection,
}

pub struct FrameLayoutDisplayedLines<'a> {
    frames: &'a FrameLayouts,
    current_row: usize,
    current_line_of_row: usize,
    lines_yielded: u32,
}

impl<'a> FrameLayoutDisplayedLines<'a> {
    pub fn zero(frames: &'a FrameLayouts) -> Self {
        Self {
            frames,
            current_row: 0,
            current_line_of_row: 0,
            lines_yielded: 0,
        }
    }
}

impl<'a> Iterator for FrameLayoutDisplayedLines<'a> {
    type Item = &'a text_layout::Line;

    fn next(&mut self) -> Option<Self::Item> {
        if self.lines_yielded >= self.frames.end_line {
            return None;
        }
        if let Some(frame) = self.frames.frames.get(self.current_row) {
            if frame.lines().len() > self.current_line_of_row {
                let line = &frame.lines()[self.current_line_of_row];
                self.current_line_of_row += 1;
                self.lines_yielded += 1;
                Some(line)
            } else {
                self.current_row += 1;
                self.current_line_of_row = 0;
                self.next()
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "soft_wrap_tests.rs"]
mod tests;
