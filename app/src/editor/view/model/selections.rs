use std::cmp::Ordering;
use std::mem;
use std::ops::Range;

use pathfinder_geometry::vector::Vector2F;
use serde::{Deserialize, Serialize};
use string_offset::{ByteOffset, CharOffset};
use vec1::Vec1;
use warpui::text::point::Point;
use warpui::AppContext;

use super::buffer::{Anchor, Buffer, LamportValue, ToBufferOffset, ToCharOffset, ToPoint};
use super::display_map::{DisplayMap, ToDisplayPoint};
use super::{DisplayPoint, ReplicaId};
use crate::editor::soft_wrap::{ClampDirection, DisplayPointAndClampDirection};
use crate::editor::{CursorColors, RangeExt};
use crate::ui_components::avatar::Avatar;

/// This type encapsulates enough information about a selection to be able to
/// draw it. Compared to the `Selection` type, the points are converted based on
/// the `DisplayMap` to `DisplayPoint`s.
pub struct DrawableSelection {
    pub range: Range<DisplayPoint>,
    pub clamp_direction: ClampDirection,
    pub replica_id: ReplicaId,
}

/// This type holds additional information about how to draw a local peer's
/// selections and cursors.
pub struct LocalDrawableSelectionData {
    pub colors: CursorColors,
    pub should_draw_cursors: bool,
}

/// This type holds additional information about how to draw a remote peer's
/// selections and cursors.
pub struct RemoteDrawableSelectionData {
    pub colors: CursorColors,
    pub should_draw_cursors: bool,
    pub avatar: Avatar,
}

/// The minimal set of data to identify a selection
/// in the system.
///
/// For local selections, we need more information
/// so that we know how to extend them (see [`LocalSelection`]).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Selection {
    /// The start of the selection.
    /// The `start` is always <= the `end`,
    /// even if the selection is reversed.
    pub start: Anchor,
    pub end: Anchor,

    /// Whether or not the selection is reversed.
    /// If true, that means that the `start` is
    /// where the "head" of the selection is (i.e.
    /// where the cursor should be drawn, where the
    /// selection should be extended from, etc.).
    pub reversed: bool,
}

impl Selection {
    pub fn single_cursor(cursor: Anchor) -> Selection {
        Selection {
            start: cursor.clone(),
            end: cursor,
            reversed: false,
        }
    }

    pub fn head(&self) -> &Anchor {
        if self.reversed {
            &self.start
        } else {
            &self.end
        }
    }

    pub fn tail(&self) -> &Anchor {
        if self.reversed {
            &self.end
        } else {
            &self.start
        }
    }

    pub fn range(&self, buffer: &Buffer) -> Range<Point> {
        let start = self.start.to_point(buffer).unwrap();
        let end = self.end.to_point(buffer).unwrap();
        if self.reversed {
            end..start
        } else {
            start..end
        }
    }

    pub fn display_range(&self, map: &DisplayMap, app: &AppContext) -> Range<DisplayPoint> {
        let start = self.start.to_display_point(map, app).unwrap();
        let end = self.end.to_display_point(map, app).unwrap();
        if self.reversed {
            end..start
        } else {
            start..end
        }
    }

    pub fn is_cursor_only(&self, buffer: &Buffer) -> bool {
        let start = self.start.to_point(buffer).unwrap();
        let end = self.end.to_point(buffer).unwrap();
        start == end
    }

    /// Whether the selection encompasses the entire buffer.
    pub fn spans_entire_buffer(&self, buffer: &Buffer) -> bool {
        self.range(buffer).sorted() == (Point::zero(), buffer.max_point())
    }

    pub fn to_offset(&self, buffer: &Buffer) -> Range<CharOffset> {
        let start = self
            .head()
            .to_char_offset(buffer)
            .expect("Should be able to convert selection head to offset.");
        let end = self
            .tail()
            .to_char_offset(buffer)
            .expect("Should be able to convert selection tail to offset.");
        start..end
    }

    pub fn to_byte_offset(&self, buffer: &Buffer) -> Range<ByteOffset> {
        let start = self
            .head()
            .to_byte_offset(buffer)
            .expect("Should be able to convert selection head to offset.");
        let end = self
            .tail()
            .to_byte_offset(buffer)
            .expect("Should be able to convert selection tail to offset.");
        start..end
    }

    pub fn start_to_point(&self, buffer: &Buffer) -> Point {
        self.start
            .to_point(buffer)
            .expect("Selection start should be valid Point")
    }

    pub fn end_to_point(&self, buffer: &Buffer) -> Point {
        self.end
            .to_point(buffer)
            .expect("Selection end should be valid Point")
    }

    /// Returns the selections display range iff it intersects the provided `range`.
    fn intersects_display_range(
        &self,
        range: &Range<DisplayPoint>,
        map: &DisplayMap,
        app: &AppContext,
    ) -> Option<Range<DisplayPoint>> {
        let display_range = self.display_range(map, app);
        // TODO (suraj): this check is confusing because [`Selection::display_range`]
        // possibly returns a reversed range.
        let intersects = display_range.start <= range.end || display_range.end <= range.end;
        intersects.then_some(display_range)
    }
}

/// A selection made by the client itself.
/// Since the editor is CRDT-compliant, we distinguish
/// between local and remote (peer) selections.
///
/// A local selection contains enough information
/// to identify the selection as well as _extend_ it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LocalSelection {
    pub selection: Selection,
    pub clamp_direction: ClampDirection,

    // goal_*_column's represent the columns this selection really should take.
    // Sometimes, `start`/`end` might be less than their goal_*_column counterparts
    // so we store these to use them once enough characters are present. For empty
    // selections, goal_start_column == goal_end_column, similar to start == end.
    //
    // As a concrete example, suppose we had the following buffer:
    // "words
    //  wo
    //  words2"
    // If the existing selection is ds on the first line, then if we add a cursor below,
    // it becomes an empty selection at the end of the line. Specifically, the buffer now looks like:
    // "wor<ds>|
    //  wo|
    //  words2"
    // where <x> denotes a selection of string x. Now, if we add one more cursor below,
    // we want the buffer to look like the following
    // "wor<ds>|
    //  wo|
    //  wor<ds>|2"
    // That is, if we didn't keep track of the goal start _and_ end columns,
    // we would not be able to correctly make the selection on the last row.
    // Specifically, the empty selection on the middle row had the following information stored:
    // {goal_start_column: 3, goal_end_column: 5}, which is how we were able to make the selection
    // correctly on the last row. If we only kept a single goal_column, we would either lose
    // information about the start of the selection or the end of the selection.
    pub goal_start_column: Option<u32>,
    pub goal_end_column: Option<u32>,
}

impl LocalSelection {
    pub fn start(&self) -> &Anchor {
        &self.selection.start
    }

    pub fn end(&self) -> &Anchor {
        &self.selection.end
    }

    pub fn head(&self) -> &Anchor {
        self.selection.head()
    }

    pub fn tail(&self) -> &Anchor {
        self.selection.tail()
    }

    pub fn reversed(&self) -> bool {
        self.selection.reversed
    }

    pub fn range(&self, buffer: &Buffer) -> Range<Point> {
        self.selection.range(buffer)
    }

    pub fn display_range(&self, map: &DisplayMap, app: &AppContext) -> Range<DisplayPoint> {
        self.selection.display_range(map, app)
    }

    pub fn is_cursor_only(&self, buffer: &Buffer) -> bool {
        self.selection.is_cursor_only(buffer)
    }

    /// Whether the selection encompasses the entire buffer.
    pub fn spans_entire_buffer(&self, buffer: &Buffer) -> bool {
        self.selection.spans_entire_buffer(buffer)
    }

    pub fn to_offset(&self, buffer: &Buffer) -> Range<CharOffset> {
        self.selection.to_offset(buffer)
    }

    pub fn to_byte_offset(&self, buffer: &Buffer) -> Range<ByteOffset> {
        self.selection.to_byte_offset(buffer)
    }

    pub fn start_to_point(&self, buffer: &Buffer) -> Point {
        self.selection.start_to_point(buffer)
    }

    pub fn end_to_point(&self, buffer: &Buffer) -> Point {
        self.selection.end_to_point(buffer)
    }

    pub fn set_start(&mut self, new_start: Anchor) {
        self.selection.start = new_start;
    }

    pub fn set_end(&mut self, new_end: Anchor) {
        self.selection.end = new_end;
    }

    pub fn set_reversed(&mut self, reversed: bool) {
        self.selection.reversed = reversed;
    }

    pub fn set_selection(&mut self, selection: Selection) {
        self.selection = selection;
    }

    pub fn set_head(&mut self, buffer: &Buffer, cursor: Anchor) {
        if cursor
            .cmp(self.tail(), buffer)
            .expect("Anchors should be comparable")
            < Ordering::Equal
        {
            if !self.selection.reversed {
                mem::swap(&mut self.selection.start, &mut self.selection.end);
                self.selection.reversed = true;
            }
            self.selection.start = cursor;
        } else {
            if self.selection.reversed {
                mem::swap(&mut self.selection.start, &mut self.selection.end);
                self.selection.reversed = false;
            }
            self.selection.end = cursor;
        }
    }

    pub fn set_tail(&mut self, buffer: &Buffer, cursor: Anchor) {
        if cursor
            .cmp(self.head(), buffer)
            .expect("Anchors should be comparable")
            > Ordering::Equal
        {
            if !self.selection.reversed {
                mem::swap(&mut self.selection.start, &mut self.selection.end);
                self.selection.reversed = true;
            }
            self.selection.end = cursor;
        } else {
            if self.selection.reversed {
                mem::swap(&mut self.selection.start, &mut self.selection.end);
                self.selection.reversed = false;
            }
            self.selection.start = cursor;
        }
    }

    #[cfg(test)]
    pub fn new_for_test(start: Anchor, end: Anchor) -> Self {
        Self {
            selection: Selection {
                start,
                end,
                reversed: false,
            },
            clamp_direction: ClampDirection::Down,
            goal_end_column: None,
            goal_start_column: None,
        }
    }
}

/// A ongoing selection made by the client itself.
/// There is no concept of a pending remote selection.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LocalPendingSelection {
    pub selection: LocalSelection,
    /// Determines how to extend selection based on starting selection.
    pub selection_mode: SelectionMode,
    /// Saved selection to refer back to when updating new selection.
    /// Used when extending selection from an already selected word.
    pub starting_selection: LocalSelection,
    pub is_single_selection: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum MarkedTextState {
    /// There is currently marked text in the editor.
    Active {
        /// The current selection given by the IME within the marked text.
        selected_range: Range<usize>,
    },
    /// There is no marked text in the editor.
    #[default]
    Inactive,
}

/// The set of active and pending selections for this client.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LocalSelections {
    /// A pending selection state, if any. This is used
    /// when a selection is actively being changed (e.g. mouse down).
    pub pending: Option<LocalPendingSelection>,

    /// The set of established selections.
    /// There is always at least one selection.
    pub selections: Vec1<LocalSelection>,

    /// Indicates whether or not the selections are marked text.
    pub marked_text_state: MarkedTextState,
}

impl LocalSelections {
    pub fn new(selections: Vec1<LocalSelection>, marked_text_state: MarkedTextState) -> Self {
        Self {
            selections,
            pending: None,
            marked_text_state,
        }
    }

    pub fn first(&self) -> &LocalSelection {
        self.selections.first()
    }

    pub fn last(&self) -> &LocalSelection {
        self.selections.last()
    }

    pub fn selection_insertion_index(&self, start: &Anchor, buffer: &Buffer) -> usize {
        selection_insertion_index(&self.selections, start, buffer)
    }

    pub fn selections_intersecting_range<'a>(
        &'a self,
        range: Range<DisplayPoint>,
        map: &'a DisplayMap,
        app: &'a AppContext,
    ) -> impl Iterator<Item = (&'a LocalSelection, Range<DisplayPoint>)> + 'a {
        let pending_selection = self.pending.as_ref().and_then(|s| {
            // If there's a single pending selection, it's already in the selections vector.
            if s.is_single_selection {
                return None;
            }
            let selection = &s.selection;
            s.selection
                .selection
                .intersects_display_range(&range, map, app)
                .map(|r| (selection, r))
        });

        selections_intersecting_range(&self.selections, range, map, app).chain(pending_selection)
    }

    pub fn drawable_selections_intersecting_range<'a>(
        &'a self,
        range: Range<DisplayPoint>,
        replica_id: ReplicaId,
        map: &'a DisplayMap,
        app: &'a AppContext,
    ) -> impl Iterator<Item = DrawableSelection> + 'a {
        self.selections_intersecting_range(range, map, app).map(
            move |(selection, selection_range)| DrawableSelection {
                range: selection_range,
                clamp_direction: selection.clamp_direction,
                replica_id: replica_id.clone(),
            },
        )
    }

    pub fn marked_text_state(&self) -> MarkedTextState {
        self.marked_text_state.clone()
    }

    pub fn set_marked_text_state(&mut self, marked_text_state: MarkedTextState) {
        self.marked_text_state = marked_text_state;
    }
}

impl From<Vec1<LocalSelection>> for LocalSelections {
    fn from(selections: Vec1<LocalSelection>) -> Self {
        Self {
            selections,
            pending: None,
            marked_text_state: Default::default(),
        }
    }
}

/// Determines how to extend selection for text.
/// Either extending by characters, lines or words at a time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionMode {
    Chars,
    Lines,
    Words,
}

/// A selection range for a remote peer.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RemoteSelection {
    pub selection: Selection,
}

impl RemoteSelection {
    pub fn observed(&self, buffer: &Buffer) -> bool {
        self.selection.start.observed(buffer) && self.selection.end.observed(buffer)
    }
}

impl From<LocalSelection> for RemoteSelection {
    fn from(local: LocalSelection) -> Self {
        Self {
            selection: local.selection,
        }
    }
}

/// The set of selections for a remote client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSelections {
    /// The selections themselves.
    pub selections: Vec1<RemoteSelection>,

    /// The lamport timestamp of the update
    /// associated to this latest selection set.
    pub lamport: LamportValue,
}

impl RemoteSelections {
    pub fn drawable_selections_intersecting_range<'a>(
        &'a self,
        range: Range<DisplayPoint>,
        replica_id: ReplicaId,
        map: &'a DisplayMap,
        app: &'a AppContext,
    ) -> impl Iterator<Item = DrawableSelection> + 'a {
        selections_intersecting_range(&self.selections, range, map, app).map(
            move |(_, selection_range)| DrawableSelection {
                range: selection_range,
                clamp_direction: ClampDirection::default(),
                replica_id: replica_id.clone(),
            },
        )
    }
}

#[derive(Debug)]
pub enum SelectAction {
    Begin {
        position: DisplayPointAndClampDirection,
        add: bool,
    },
    Update {
        position: DisplayPoint,
        scroll_position: Vector2F,
    },
    Extend {
        position: DisplayPoint,
        scroll_position: Vector2F,
    },
    End,
}

impl SelectAction {
    /// Create an action for beginning a selection - use this in tests that need to simulate
    /// clicking on an editor.
    #[cfg(test)]
    pub fn begin(point: DisplayPoint) -> Self {
        Self::Begin {
            position: DisplayPointAndClampDirection {
                point,
                clamp_direction: Default::default(),
            },
            add: false,
        }
    }
}

pub trait AsSelection {
    fn as_selection(&self) -> &Selection;
    fn as_mut_selection(&mut self) -> &mut Selection;
}

impl AsSelection for LocalSelection {
    fn as_selection(&self) -> &Selection {
        &self.selection
    }

    fn as_mut_selection(&mut self) -> &mut Selection {
        &mut self.selection
    }
}

impl AsSelection for RemoteSelection {
    fn as_selection(&self) -> &Selection {
        &self.selection
    }

    fn as_mut_selection(&mut self) -> &mut Selection {
        &mut self.selection
    }
}

fn selection_insertion_index<S: AsSelection>(
    selections: &Vec1<S>,
    start: &Anchor,
    buffer: &Buffer,
) -> usize {
    match selections
        .binary_search_by(|probe| probe.as_selection().start.cmp(start, buffer).unwrap())
    {
        Ok(index) => index,
        Err(index) => {
            if index > 0
                && selections[index - 1]
                    .as_selection()
                    .end
                    .cmp(start, buffer)
                    .unwrap()
                    == Ordering::Greater
            {
                index - 1
            } else {
                index
            }
        }
    }
}

/// Returns an iterator over the selections
/// that intersect the given `range`.
///
/// Assumes that `selections` are sorted by their start point.
fn selections_intersecting_range<'a, S: AsSelection>(
    selections: &'a Vec1<S>,
    range: Range<DisplayPoint>,
    map: &'a DisplayMap,
    app: &'a AppContext,
) -> impl Iterator<Item = (&'a S, Range<DisplayPoint>)> + 'a {
    let start = map
        .anchor_before(range.start, super::Bias::Left, app)
        .unwrap();
    let start_index = selection_insertion_index(selections, &start, map.buffer(app));

    selections[start_index..]
        .iter()
        .map_while(move |selection| {
            selection
                .as_selection()
                .intersects_display_range(&range, map, app)
                .map(|r| (selection, r))
        })
}
