mod fold_map;

use super::buffer::{self, Anchor, Buffer, Edit, StylizedChar, ToCharOffset, ToPoint};
use crate::editor::soft_wrap::{self, DisplayPointAndClampDirection, SoftWrapPoint, SoftWrapState};
use anyhow::{Context, Result};
pub use fold_map::BufferRows;
use fold_map::FoldMap;
use std::cmp;
use std::ops::Range;
use string_offset::CharOffset;
use warpui::text::point::Point;
use warpui::{AppContext, Entity, ModelContext, ModelHandle};

#[derive(Copy, Clone)]
pub enum Bias {
    Left,
    Right,
}

pub struct DisplayMap {
    buffer: ModelHandle<Buffer>,
    fold_map: FoldMap,
    soft_wrap_state: SoftWrapState,
    tab_size: usize,
}

pub struct MovementResult {
    pub point_and_clamp_direction: DisplayPointAndClampDirection,
    /// True if the resulting point is the same row as the original row.
    pub is_same_row: bool,
    /// The desired column based on the original point. This could be higher than
    /// the returned column in point because the current row may not have had as
    /// many columns as the previous row.
    pub goal_column: u32,
}

pub enum Event {
    Folded,
    Unfolded,
}

impl Entity for DisplayMap {
    type Event = Event;
}

impl DisplayMap {
    fn new_internal(
        buffer: ModelHandle<Buffer>,
        subscribe_to_buffer: bool,
        tab_size: usize,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        if subscribe_to_buffer {
            ctx.subscribe_to_model(&buffer, Self::handle_buffer_event);
        }

        DisplayMap {
            buffer: buffer.clone(),
            fold_map: FoldMap::new(buffer, ctx),
            soft_wrap_state: Default::default(),
            tab_size,
        }
    }

    pub fn new(buffer: ModelHandle<Buffer>, tab_size: usize, ctx: &mut ModelContext<Self>) -> Self {
        Self::new_internal(buffer, true, tab_size, ctx)
    }

    pub fn recreate(&mut self, tab_size: usize, ctx: &mut ModelContext<Self>) {
        *self = Self::new_internal(self.buffer.clone(), false, tab_size, ctx);
    }

    pub fn buffer<'a>(&self, app: &'a AppContext) -> &'a Buffer {
        self.buffer.as_ref(app)
    }

    pub fn soft_wrap_state(&self) -> SoftWrapState {
        self.soft_wrap_state.clone()
    }

    pub fn fold<T: ToCharOffset>(
        &mut self,
        ranges: impl IntoIterator<Item = Range<T>>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        self.fold_map.fold(ranges, ctx)?;
        ctx.emit(Event::Folded);
        Ok(())
    }

    pub fn unfold<T: ToCharOffset>(
        &mut self,
        ranges: impl IntoIterator<Item = Range<T>>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        self.fold_map.unfold(ranges, ctx)?;
        ctx.emit(Event::Unfolded);
        Ok(())
    }

    /// TODO(zheng) Consolidate logic with `down`.
    pub fn up(
        &self,
        point: DisplayPoint,
        goal_column: Option<u32>,
        clamp_direction: soft_wrap::ClampDirection,
    ) -> Result<MovementResult> {
        self.soft_wrap_state.read(|frame_layouts| {
            let frame_layouts = frame_layouts.map_err(|err| anyhow::anyhow!("Could not read Frame Layouts {:?}", err))?;
            let point = frame_layouts.to_soft_wrap_point(point, clamp_direction).context("Point is out of bounds of laid out text")?;

            let line = frame_layouts
                .get_line(point.row() as usize)
                .context("Should have current line as it should not be possible for it to be beyond bounds, but was not able to retrieve it")?;
            // Note that the index here will correspond to that of the original text so if
            // this line is soft wrapped, it will be higher than 0.
            let line_first_index = line.first_glyph().map_or(0, |glyph| glyph.index);

            let relative_index = point.column() - line_first_index as u32;

            // Note that, as also mentioned in the rustdoc of this function,
            // the goal column is a relative index to the first point of the row
            // so it starts at 0 and is at most the length of a row (but it could
            // be higher than the current row as it is preserved across rows).
            let goal_column = match goal_column {
                Some(goal_column) => cmp::max(goal_column, relative_index),
                None => relative_index,
            };
            let (point, is_same_row) = if point.row() > 0 {
                let prev_line = frame_layouts
                    .get_line(point.row() as usize - 1)
                    .context("Should have next line based on max point bounds check, but was not able to retrieve it")?;
                let prev_line_first_column = prev_line.first_glyph().map_or(0, |glyph| glyph.index as u32);
                // Note that goal column is a relative value, rather than based
                // on the original string.
                let new_column = prev_line_first_column + goal_column;

                let last_index_in_line = prev_line.last_glyph().map(|glyph| glyph.index as u32 + 1)
                    .unwrap_or(0);

                (SoftWrapPoint::new(
                    point.row() - 1,
                    cmp::min(new_column, last_index_in_line),
                ), false)
            } else {
                (SoftWrapPoint::new(0, 0), true)
            };
            Ok(MovementResult {
                point_and_clamp_direction: frame_layouts.to_display_point(point),
                goal_column,
                is_same_row,
            })
        })
    }

    /// Given a `DisplayPoint` and the goal column (i.e. the rightmost column
    /// that we _want_ to be at which could be higher than the current point's
    /// column) returns the point and new goal column we would be at if we were
    /// to navigate one row down from the point with soft wrap in consideration.
    ///
    /// Note that while `goal_column` is the column number of the row, the column
    /// numbers in `SoftWrapPoint`, are relative to the original string.
    /// TODO(zheng) Consolidate logic with `up`.
    pub fn down(
        &self,
        point: DisplayPoint,
        goal_column: Option<u32>,
        clamp_direction: soft_wrap::ClampDirection,
    ) -> Result<MovementResult> {
        self.soft_wrap_state.read(|frame_layouts| {
            let frame_layouts = frame_layouts.map_err(|err| anyhow::anyhow!("Could not read Frame Layouts {:?}", err))?;
            let point = frame_layouts.to_soft_wrap_point(point, clamp_direction).ok_or_else(|| anyhow::anyhow!("Point is out of bounds of laid out text"))?;

            let line = frame_layouts
                .get_line(point.row() as usize)
                .context("Should have current line as it should not be possible for it to be beyond bounds, but was not able to retrieve it")?;
            // Note that the index here will correspond to that of the original text so if
            // this line is soft wrapped, it will be higher than 0.
            let line_first_index = line.first_glyph().map_or(0, |glyph| glyph.index);

            let relative_index = point.column() - line_first_index as u32;

            // Note that, as also mentioned in the rustdoc of this function,
            // the goal column is a relative index to the first point of the row
            // so it starts at 0 and is at most the length of a row (but it could
            // be higher than the current row as it is preserved across rows).
            let goal_column = match goal_column {
                Some(goal_column) => cmp::max(goal_column, relative_index),
                None => relative_index,
            };
            let max_row = frame_layouts.num_lines() - 1;
            let max_col = frame_layouts
                .get_line(max_row)
                .and_then(|line| line.last_glyph().map(|glyph| glyph.index + 1))
                .unwrap_or(0);
            let max_point = SoftWrapPoint::new(max_row as u32, max_col as u32);
            let (point, is_same_row) = if point.row() < max_point.row() {
                let next_line = frame_layouts
                    .get_line(point.row() as usize + 1)
                    .context("Should have next line based on max point bounds check, but was not able to retrieve it")?;
                let next_line_first_column = next_line.first_glyph().map_or(0, |glyph| glyph.index as u32);
                // Note that goal column is a relative value, rather than based
                // on the original string.
                let new_column = next_line_first_column + goal_column;

                let last_index_in_line = next_line.last_glyph().map(|glyph| glyph.index as u32 + 1)
                    .unwrap_or(0);

                (SoftWrapPoint::new(
                    point.row() + 1,
                    cmp::min(new_column, last_index_in_line),
                ), false)
            } else {
                (max_point, true)
            };
            Ok(MovementResult {
                point_and_clamp_direction: frame_layouts.to_display_point(point),
                goal_column,
                is_same_row,
            })
        })
    }

    pub fn to_soft_wrap_point(
        &self,
        point: DisplayPoint,
        clamp_direction: soft_wrap::ClampDirection,
    ) -> Option<SoftWrapPoint> {
        self.soft_wrap_state
            .read(|frame_layouts| match frame_layouts {
                Ok(frame_layouts) => frame_layouts.to_soft_wrap_point(point, clamp_direction),
                Err(err) => {
                    log::warn!("Error attempting to get soft wrap point {err:?}");
                    None
                }
            })
    }

    pub fn is_line_folded(&self, display_row: u32) -> bool {
        self.fold_map.is_line_folded(display_row)
    }

    pub fn tab_size(&self) -> usize {
        self.tab_size
    }

    #[cfg(test)]
    pub fn text(&self, app: &AppContext) -> String {
        self.chars_at(DisplayPoint::zero(), app).unwrap().collect()
    }

    pub fn line(&self, display_row: u32, app: &AppContext) -> Result<String> {
        let chars = self.chars_at(DisplayPoint::new(display_row, 0), app)?;
        Ok(chars.take_while(|c| *c != '\n').collect())
    }

    pub fn chars_with_styles_at<'a>(
        &'a self,
        point: DisplayPoint,
        app: &'a AppContext,
    ) -> Result<impl 'a + Iterator<Item = StylizedChar>> {
        let column = point.column() as usize;
        let (point, to_next_stop) = point.collapse_tabs(self, Bias::Left, app)?;
        let mut fold_chars = self.fold_map.chars_with_style_at(point, app)?;
        if to_next_stop > 0 {
            fold_chars.next();
        }

        Ok(CharsWithStyles {
            fold_chars,
            column,
            to_next_stop,
            tab_size: self.tab_size,
        })
    }

    pub fn chars_at<'a>(
        &'a self,
        point: DisplayPoint,
        app: &'a AppContext,
    ) -> Result<impl 'a + Iterator<Item = char>> {
        Ok(self.chars_with_styles_at(point, app)?.map(Into::into))
    }

    pub fn buffer_rows(&self, start_row: u32) -> Result<BufferRows<'_>> {
        self.fold_map.buffer_rows(start_row)
    }

    pub fn line_len(&self, row: u32, ctx: &AppContext) -> Result<u32> {
        DisplayPoint::new(row, self.fold_map.line_len(row, ctx)?)
            .expand_tabs(self, ctx)
            .map(|point| point.column())
    }

    pub fn max_point(&self, app: &AppContext) -> DisplayPoint {
        self.fold_map.max_point().expand_tabs(self, app).unwrap()
    }

    pub fn rightmost_point(&self) -> DisplayPoint {
        self.fold_map.rightmost_point()
    }

    pub fn anchor_before(
        &self,
        point: DisplayPoint,
        bias: Bias,
        app: &AppContext,
    ) -> Result<Anchor> {
        self.buffer
            .as_ref(app)
            .anchor_before(point.to_buffer_point(self, bias, app)?)
    }

    #[allow(dead_code)]
    pub fn anchor_after(
        &self,
        point: DisplayPoint,
        bias: Bias,
        app: &AppContext,
    ) -> Result<Anchor> {
        self.buffer
            .as_ref(app)
            .anchor_after(point.to_buffer_point(self, bias, app)?)
    }

    pub fn apply_edits(&mut self, edits: &[Edit], ctx: &AppContext) -> Result<()> {
        self.fold_map.apply_edits(edits, ctx)
    }

    fn handle_buffer_event(&mut self, event: &buffer::Event, ctx: &mut ModelContext<Self>) {
        match event {
            buffer::Event::Edited { edits, .. } => self.apply_edits(edits, ctx).unwrap(),
            buffer::Event::StylesUpdated
            | buffer::Event::UpdatePeers { .. }
            | buffer::Event::SelectionsChanged => {}
        }
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, Ord, PartialOrd, PartialEq)]
pub struct DisplayPoint(Point);

impl DisplayPoint {
    pub fn new(row: u32, column: u32) -> Self {
        Self(Point::new(row, column))
    }

    #[allow(dead_code)]
    pub fn zero() -> Self {
        Self::new(0, 0)
    }

    pub fn row(self) -> u32 {
        self.0.row
    }

    pub fn column(self) -> u32 {
        self.0.column
    }

    pub fn row_mut(&mut self) -> &mut u32 {
        &mut self.0.row
    }

    pub fn column_mut(&mut self) -> &mut u32 {
        &mut self.0.column
    }

    pub fn to_buffer_point(self, map: &DisplayMap, bias: Bias, app: &AppContext) -> Result<Point> {
        Ok(map
            .fold_map
            .to_buffer_point(self.collapse_tabs(map, bias, app)?.0))
    }

    pub fn to_char_offset(
        self,
        map: &DisplayMap,
        bias: Bias,
        buffer: &Buffer,
        app: &AppContext,
    ) -> Result<CharOffset> {
        let point = self.to_buffer_point(map, bias, app)?;
        point.to_char_offset(buffer)
    }

    fn expand_tabs(mut self, map: &DisplayMap, app: &AppContext) -> Result<Self> {
        let chars = map
            .fold_map
            .chars_at(DisplayPoint(Point::new(self.row(), 0)), app)?;
        let expanded = expand_tabs(chars, self.column() as usize, map.tab_size);
        *self.column_mut() = expanded as u32;

        Ok(self)
    }

    fn collapse_tabs(
        mut self,
        map: &DisplayMap,
        bias: Bias,
        app: &AppContext,
    ) -> Result<(Self, usize)> {
        let chars = map
            .fold_map
            .chars_at(DisplayPoint(Point::new(self.0.row, 0)), app)?;
        let expanded = self.column() as usize;
        let (collapsed, to_next_stop) = collapse_tabs(chars, expanded, bias, map.tab_size);
        *self.column_mut() = collapsed as u32;

        Ok((self, to_next_stop))
    }
}

/// Trait for types that can map onto a display point, accounting for soft-wrapping
/// and text folding.
pub trait ToDisplayPoint {
    fn to_display_point(self, map: &DisplayMap, app: &AppContext) -> Result<DisplayPoint>;
}

impl ToDisplayPoint for Point {
    fn to_display_point(self, map: &DisplayMap, app: &AppContext) -> Result<DisplayPoint> {
        let mut display_point = map.fold_map.to_display_point(self);
        let chars = map
            .fold_map
            .chars_at(DisplayPoint::new(display_point.row(), 0), app)?;
        *display_point.column_mut() =
            expand_tabs(chars, display_point.column() as usize, map.tab_size) as u32;
        Ok(display_point)
    }
}

impl ToDisplayPoint for &Anchor {
    fn to_display_point(self, map: &DisplayMap, app: &AppContext) -> Result<DisplayPoint> {
        self.to_point(map.buffer.as_ref(app))?
            .to_display_point(map, app)
    }
}

pub struct CharsWithStyles<'a> {
    fold_chars: fold_map::CharsWithStyle<'a>,
    column: usize,
    to_next_stop: usize,
    tab_size: usize,
}

impl Iterator for CharsWithStyles<'_> {
    type Item = StylizedChar;

    fn next(&mut self) -> Option<Self::Item> {
        if self.to_next_stop > 0 {
            self.to_next_stop -= 1;
            self.column += 1;
            Some(StylizedChar::new(' ', Default::default()))
        } else {
            self.fold_chars.next().map(|c| match c.char() {
                '\t' => {
                    self.to_next_stop = self.tab_size - self.column % self.tab_size - 1;
                    self.column += 1;
                    StylizedChar::new(' ', c.style())
                }
                '\n' => {
                    self.column = 0;
                    c
                }
                _ => {
                    self.column += 1;
                    c
                }
            })
        }
    }
}

pub fn expand_tabs(chars: impl Iterator<Item = char>, column: usize, tab_size: usize) -> usize {
    let mut expanded = 0;
    for c in chars.take(column) {
        if c == '\t' {
            expanded += tab_size - expanded % tab_size;
        } else {
            expanded += 1;
        }
    }
    expanded
}

pub fn collapse_tabs(
    chars: impl Iterator<Item = char>,
    column: usize,
    bias: Bias,
    tab_size: usize,
) -> (usize, usize) {
    let mut expanded = 0;
    let mut collapsed = 0;
    for c in chars {
        if expanded == column {
            break;
        }

        if c == '\t' {
            expanded += tab_size - (expanded % tab_size);
            if expanded > column {
                return match bias {
                    Bias::Left => (collapsed, expanded - column),
                    Bias::Right => (collapsed + 1, 0),
                };
            }
        } else {
            expanded += 1;
        }
        collapsed += 1;
    }
    (collapsed, 0)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
