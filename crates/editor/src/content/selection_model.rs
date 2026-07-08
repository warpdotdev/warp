use std::ops::Range;

use itertools::Itertools;
use string_offset::CharOffset;
use vec1::{Vec1, vec1};
use warp_core::report_error;
use warpui_core::{AppContext, Entity, ModelHandle};

use crate::content::anchor::{Anchor, AnchorSide, AnchorUpdate, Anchors};
use crate::content::buffer::{Buffer, SelectionOffsets, ToBufferPoint};
use crate::content::selection::{Selection, SelectionSet};
use crate::content::text::{BlockType, TextStylesWithMetadata};

/// A snapshot of the selection state. This includes all data reported by [`BufferEvent::SelectionChanged`].
#[derive(PartialEq, Eq)]
pub(super) struct SelectionSnapshot {
    /// We must snapshot the resolved selections, rather than their anchors, because anchors are
    /// updated in-place and won't directly reflect the update.
    pub(super) selections: Vec1<SelectionOffsets>,
    pub(super) active_text_styles: TextStylesWithMetadata,
    pub(super) active_block_type: BlockType,
}

pub struct BufferSelectionModel {
    selections: SelectionSet,
    pub(crate) anchors: Anchors,
    buffer: ModelHandle<Buffer>,
}

impl BufferSelectionModel {
    pub fn new(buffer: ModelHandle<Buffer>) -> Self {
        let mut anchors = Anchors::new();
        let head_anchor = anchors.create_anchor(CharOffset::from(1), AnchorSide::Right);
        let tail_anchor = anchors.create_anchor(CharOffset::from(1), AnchorSide::Right);

        let selections = SelectionSet::new(Selection::new(head_anchor, tail_anchor));
        Self {
            selections,
            buffer,
            anchors,
        }
    }

    pub fn update_anchors(&mut self, anchor_updates: Vec<AnchorUpdate>) {
        for update in anchor_updates {
            self.anchors.update(update);
        }
    }

    pub(super) fn truncate(&mut self) {
        self.selections.truncate();
    }

    /// Create a new anchor at the given offset.
    pub fn anchor(&mut self, offset: CharOffset, ctx: &AppContext) -> Anchor {
        self.anchors
            .create_anchor(offset.min(self.buffer.as_ref(ctx).len()), AnchorSide::Right)
    }

    pub(super) fn create_anchor(&mut self, offset: CharOffset, side: AnchorSide) -> Anchor {
        self.anchors.create_anchor(offset, side)
    }

    /// Resolve an anchor to its current offset.
    pub fn resolve_anchor(&self, anchor: &Anchor) -> Option<CharOffset> {
        self.anchors.resolve(anchor)
    }

    /// Resolve an anchor to its 0-based line number in the buffer.
    pub fn line_number_from_anchor(&self, anchor: &Anchor, ctx: &AppContext) -> Option<usize> {
        let char_offset = self.resolve_anchor(anchor)?;
        Some(char_offset.to_buffer_point(self.buffer.as_ref(ctx)).row as usize)
    }

    // The following two are temporary methods until multiple selections are supported.
    // Todo (kc CLD-1018): This should no longer be needed once we move to multi-selection.
    pub fn selection(&self) -> &Selection {
        self.selections.first()
    }

    pub fn selections(&self) -> &SelectionSet {
        &self.selections
    }

    pub fn selections_mut(&mut self) -> &mut SelectionSet {
        &mut self.selections
    }

    /// Returns the index of all lines that have active selection.
    pub fn selected_lines(&self, ctx: &AppContext) -> Vec1<usize> {
        Vec1::try_from_vec(
            self.selections
                .iter()
                .flat_map(|s| {
                    let range = self.selection_to_offset_range(s);
                    let start_idx =
                        range.start.to_buffer_point(self.buffer.as_ref(ctx)).row as usize;
                    let end_idx = ((range.end - 1).to_buffer_point(self.buffer.as_ref(ctx)).row
                        as usize)
                        .max(start_idx);

                    start_idx..end_idx + 1
                })
                .sorted()
                .dedup()
                .collect_vec(),
        )
        .expect("Should have more than 1 element")
    }

    pub fn selection_to_offset_range(&self, selection: &Selection) -> Range<CharOffset> {
        // Selection should always be valid when we have single selections.
        // TODO(kevin): migrate this when moving to multi-selection.
        let head = self
            .resolve_anchor(selection.head())
            .expect("anchor should exist");
        let tail = self
            .resolve_anchor(selection.tail())
            .expect("anchor should exist");

        if head >= tail { tail..head } else { head..tail }
    }

    // Todo (kc CLD-1018): This should no longer be needed once we move to multi-selection.
    pub fn selection_to_first_offset_range(&self) -> Range<CharOffset> {
        self.selection_to_offset_range(self.selection())
    }

    pub fn selections_to_offset_ranges(&self) -> Vec1<Range<CharOffset>> {
        self.selections
            .selection_map(|s| self.selection_to_offset_range(s))
    }

    fn create_selection(&mut self, head: CharOffset, tail: CharOffset) -> Selection {
        Selection::new(
            self.create_anchor(head, AnchorSide::Right),
            self.create_anchor(tail, AnchorSide::Right),
        )
    }

    pub fn set_selections(&mut self, selections: SelectionSet) {
        self.selections = selections;
    }

    /// Checks for overlapping selections.  If any are found, we merge them together.
    ///
    /// Before:
    /// | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 |
    /// |     S1    |       |     S3    |
    /// |           S2          |
    ///                                     |    S4    |
    ///
    /// After:
    /// | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 |
    /// |                S1             |
    ///                                     |    S2    |
    pub fn merge_overlapping_selections(&mut self) {
        let (overlap_indices, new_ranges) =
            Buffer::overlapping_ranges(self.selections_to_offset_ranges().to_vec());

        // Return early if there are no overlapping selections.
        if overlap_indices.is_empty() {
            return;
        }

        // To determine whether the head of new selections should be before or after the tail,
        // we check whether overlapping selections have the head before the tail.
        // This seems to work in the common cases.
        let forwards_selection = self
            .selections
            .iter()
            .enumerate()
            .filter(|(i, _)| overlap_indices.contains(i))
            .any(|(_, selection)| self.selection_head(selection) > self.selection_tail(selection));

        // Keep any non-overlapping selections.
        let mut new_selections: Vec<Selection> = self
            .selections
            .iter()
            .enumerate()
            .flat_map(|(i, selection)| {
                if !overlap_indices.contains(&i) {
                    Some(selection.clone())
                } else {
                    None
                }
            })
            .collect();

        // Create any new selections from the overlapping selections.
        for range in new_ranges {
            let new_selection = if forwards_selection {
                self.create_selection(range.end, range.start)
            } else {
                self.create_selection(range.start, range.end)
            };

            new_selections.push(new_selection);
        }

        match SelectionSet::try_from(new_selections) {
            Ok(selections) => self.selections = selections,
            Err(_) => {
                report_error!(
                    "After removing overlapping selections, there were no selections left!"
                )
            }
        }
    }

    // Todo (kc CLD-1018): This should no longer be needed once we move to multi-selection.
    pub fn first_selection_head(&self) -> CharOffset {
        self.resolve_anchor(self.selections.first().head())
            .expect("anchor should exist")
    }

    /// The current location of the selection head.
    pub fn selection_head(&self, selection: &Selection) -> CharOffset {
        self.resolve_anchor(selection.head())
            .expect("anchor should exist")
    }

    pub fn selection_heads(&self) -> Vec1<CharOffset> {
        self.selections
            .selection_map(|s| self.resolve_anchor(s.head()).expect("anchor should exist"))
    }

    // Todo (kc CLD-1018): This should no longer be needed once we move to multi-selection.
    pub fn first_selection_tail(&self) -> CharOffset {
        self.resolve_anchor(self.selection().tail())
            .expect("anchor should exist")
    }

    /// The current location of the selection tail.
    pub fn selection_tail(&self, selection: &Selection) -> CharOffset {
        self.resolve_anchor(selection.tail())
            .expect("anchor should exist")
    }

    /// Return all selected ranges.
    pub fn selection_offsets(&self) -> Vec1<SelectionOffsets> {
        self.selections.selection_map(|s| SelectionOffsets {
            head: self.resolve_anchor(s.head()).expect("anchor should exist"),
            tail: self.resolve_anchor(s.tail()).expect("anchor should exist"),
        })
    }

    pub fn is_single_selection(&self) -> bool {
        self.selections.len() == 1
    }

    /// Query for the set of text styles that are fully active over the current
    /// selection.
    pub fn selection_text_styles(&self, ctx: &AppContext) -> TextStylesWithMetadata {
        self.selections_to_offset_ranges()
            .into_iter()
            .map(|range| self.buffer.as_ref(ctx).range_text_styles(range))
            .reduce(|style1, style2| style1.mutual_styles(style2))
            .expect("At least one selection style should exist")
    }

    pub fn active_block_type_at_selection(
        &self,
        selection: &Selection,
        ctx: &AppContext,
    ) -> BlockType {
        let range = self.selection_to_offset_range(selection);
        self.buffer.as_ref(ctx).block_type_at_point(range.start)
    }

    /// Check whether the block type for every selection allow formatting.
    pub fn all_selections_allow_formatting(&self, ctx: &AppContext) -> bool {
        self.selections.iter().all(|selection| {
            match self.active_block_type_at_selection(selection, ctx) {
                BlockType::Text(block_style) => block_style.allows_formatting(),
                // If pasting content directly after a rich block item, we should keep the content's original
                // styling.
                BlockType::Item(_) => true,
            }
        })
    }

    // Todo: kc (CLD1018) Temporary until multiselect is implemented.
    pub fn first_selection_is_single_cursor(&self) -> bool {
        // Selection should always be valid when we have single selections.
        // TODO(kevin): migrate this when moving to multi-selection.
        self.first_selection_head() == self.first_selection_tail()
    }

    pub fn selection_is_single_cursor(&self, selection: &Selection) -> bool {
        self.selection_head(selection) == self.selection_tail(selection)
    }

    /// Return true if all selections are single cursors.
    pub fn all_single_cursors(&self) -> bool {
        self.selections
            .iter()
            .all(|s| self.selection_is_single_cursor(s))
    }

    pub fn cursors_at_line_start(&self, ctx: &AppContext) -> bool {
        self.all_single_cursors();
        for selection in self.selections.iter() {
            let cursor_offset = self.selection_head(selection);
            if cursor_offset != self.buffer.as_ref(ctx).containing_line_start(cursor_offset) {
                return false;
            }
        }
        true
    }

    /// Validate the buffer content with this selection model's anchors.
    pub fn validate_buffer(&self, ctx: &impl warpui_core::ModelAsRef) {
        self.buffer.as_ref(ctx).validate(&self.anchors);
    }

    pub(super) fn shift_selections_after_offset(&mut self, offset: CharOffset, delta: usize) {
        for selection in self.selections.iter_mut() {
            let head_offset = self
                .anchors
                .resolve(selection.head())
                .expect("Anchor should be valid");

            if head_offset >= offset && head_offset < offset + delta {
                selection.set_head(&mut self.anchors, offset + delta);
            }

            let tail_offset = self
                .anchors
                .resolve(selection.tail())
                .expect("Anchor should be valid");

            if tail_offset >= offset && tail_offset < offset + delta {
                selection.set_tail(&mut self.anchors, offset + delta);
            }
        }
    }

    pub(super) fn set_single_cursor(&mut self, offset: CharOffset) {
        self.set_selection_offsets(vec1![SelectionOffsets {
            head: offset,
            tail: offset
        }]);
    }

    pub(super) fn set_clamped_selection_head(
        &mut self,
        selection: &mut Selection,
        offset: CharOffset,
    ) {
        selection.set_head(&mut self.anchors, offset);
    }

    pub(super) fn set_clamped_selection_tail(
        &mut self,
        selection: &mut Selection,
        offset: CharOffset,
    ) {
        selection.set_tail(&mut self.anchors, offset);
    }

    pub(super) fn update_selection_offsets(&mut self, clamped_selections: Vec1<SelectionOffsets>) {
        debug_assert!(
            clamped_selections.len() == self.selections().len(),
            "To update selection offsets, you must provide the same number of offset pairs as there are currently active selections"
        );
        for (selection, offsets) in self.selections.iter_mut().zip(clamped_selections.iter()) {
            selection.set_head(&mut self.anchors, offsets.head);
            selection.set_tail(&mut self.anchors, offsets.tail);
        }
    }

    /// Clear the current selections and replace with this new set of selections.
    /// All biases will be cleared.
    pub fn set_selection_offsets(&mut self, selections: Vec1<SelectionOffsets>) {
        let new_selections = selections.mapped(|offsets| {
            let head_anchor = self.create_anchor(offsets.head, AnchorSide::Right);
            let tail_anchor = self.create_anchor(offsets.tail, AnchorSide::Right);
            Selection::new(head_anchor, tail_anchor)
        });
        self.set_selections(new_selections.into());
    }
}

impl Entity for BufferSelectionModel {
    type Event = ();
}
