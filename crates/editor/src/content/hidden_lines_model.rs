use std::collections::HashMap;
use std::ops::Range;

use rangemap::RangeSet;
use string_offset::CharOffset;
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle};

use super::anchor::{Anchor, AnchorSide};
use crate::content::buffer::Buffer;
use crate::content::edit::EditDelta;
use crate::content::selection_model::BufferSelectionModel;
use crate::content::text::LineCount;
use crate::content::version::BufferVersion;

/// A model that tracks hidden line ranges independently of the buffer content.
/// This allows multiple editors to have different hidden line states for the same buffer.
///
/// Hidden ranges are stored as anchor pairs that automatically adjust their positions
/// when the buffer content changes.
pub struct HiddenLinesModel {
    hidden_ranges: Vec<(Anchor, Anchor)>,
    buffer: ModelHandle<Buffer>,
    buffer_selections: ModelHandle<BufferSelectionModel>,
    version_offsets: HashMap<BufferVersion, RangeSet<CharOffset>>,
}

impl HiddenLinesModel {
    pub fn new(
        buffer: ModelHandle<Buffer>,
        buffer_selections: ModelHandle<BufferSelectionModel>,
    ) -> Self {
        Self {
            hidden_ranges: Vec::new(),
            buffer,
            buffer_selections,
            version_offsets: HashMap::new(),
        }
    }

    pub fn set_hidden_lines(&mut self, ranges: RangeSet<LineCount>, ctx: &mut ModelContext<Self>) {
        self.hidden_ranges.clear();

        let buffer = self.buffer.as_ref(ctx);

        // We have to collect here given otherwise we will hold an immutable borrow to Buffer. And below we need a
        // mutable reference to the ctx.
        let offset_ranges: Vec<Range<CharOffset>> = ranges
            .into_iter()
            .map(|range| {
                let start_offset = buffer.line_start(range.start + LineCount::from(1));
                let end_offset = buffer.line_start(range.end + LineCount::from(1));

                start_offset..end_offset
            })
            .collect();

        let buffer_version = buffer.buffer_version();

        for range in offset_ranges {
            if range.start < range.end {
                let (start_anchor, end_anchor) =
                    self.buffer_selections.update(ctx, |selection_model, _| {
                        let start_anchor = selection_model
                            .anchors
                            .create_anchor(range.start, AnchorSide::Right);
                        // Note that we anchor to the left of the _next line_ after the hidden range.
                        // This avoids any insert after the hidden range from expanding it.
                        let end_anchor = selection_model
                            .anchors
                            .create_anchor(range.end, AnchorSide::Left);
                        (start_anchor, end_anchor)
                    });

                self.hidden_ranges.push((start_anchor, end_anchor));
            }
        }

        self.materialize_hidden_range_offsets(buffer_version, ctx);
    }

    /// Check if the given offset is within a hidden range
    pub fn is_hidden(&self, offset: CharOffset, ctx: &AppContext) -> bool {
        for (start_anchor, end_anchor) in &self.hidden_ranges {
            if let (Some(start), Some(end)) = (
                self.buffer_selections
                    .as_ref(ctx)
                    .resolve_anchor(start_anchor),
                self.buffer_selections
                    .as_ref(ctx)
                    .resolve_anchor(end_anchor),
            ) && offset >= start
                && offset < end
            {
                return true;
            }
        }
        false
    }

    /// Check if there are materialized offsets for a given buffer version.
    pub fn has_offsets_for_version(&self, version: BufferVersion) -> bool {
        self.version_offsets.contains_key(&version)
    }

    /// For a given content version, check if the incoming offset ranges intersect with any of the
    /// hidden ranges.
    pub fn range_intersects_with_hidden_range_at_version(
        &self,
        range: &Range<CharOffset>,
        version: BufferVersion,
    ) -> bool {
        // If there is no hidden range set, default to false.
        let Some(ranges) = self.version_offsets.get(&version) else {
            return false;
        };

        ranges.overlaps(range)
    }

    /// Convert anchors (stateful) to offsets (stateless) for a given buffer version. This allows
    /// us to maintain a stable hidden line range for each buffer state.
    pub fn materialize_hidden_range_offsets(
        &mut self,
        version: BufferVersion,
        ctx: &mut ModelContext<Self>,
    ) {
        let hidden_ranges = self.anchors_to_offsets(ctx);
        self.version_offsets.insert(version, hidden_ranges);
    }

    /// Set the following hidden range to be visible. No-op if the range is not hidden.
    /// IMPORTANT: This assumes the line range only overlaps with one hidden range for efficiency.
    pub fn set_visible_line_range(
        &mut self,
        line_range: Range<LineCount>,
        ctx: &mut ModelContext<Self>,
    ) -> Option<EditDelta> {
        let starting_visible = self.buffer.as_ref(ctx).line_start(line_range.start);
        let buffer_version = self.buffer.as_ref(ctx).buffer_version();
        let ending_visible = self
            .buffer
            .as_ref(ctx)
            .line_start(line_range.end + LineCount::from(1));

        let mut to_remove = None;
        let mut delta = None;

        for (idx, (start_anchor, end_anchor)) in self.hidden_ranges.iter().enumerate() {
            let Some(start) = self
                .buffer_selections
                .as_ref(ctx)
                .resolve_anchor(start_anchor)
            else {
                continue;
            };
            let Some(end) = self
                .buffer_selections
                .as_ref(ctx)
                .resolve_anchor(end_anchor)
            else {
                continue;
            };

            // I know this seems inefficient to start...why can't we just invalidate starting_visible to ending_visible?
            // This is because of a key constraint in the rendering layer. When we layout the blocks, we expect the boundary
            // of what we invalidate to be on the exact boundary of a block. However, with hidden blocks, we collapse them into
            // one block in the SumTree. This means we will end up in an invalid state if we try to slice in the middle of that
            // giant hidden block. By making sure the invalidation range is min(start, starting_visible)..max(end, ending_visible),
            // this works around that constraint. And it shouldn't be much more expensive as we don't layout hidden blocks.
            let invalidation_range = start.min(starting_visible)..end.max(ending_visible);

            if start >= starting_visible && end <= ending_visible {
                to_remove = Some(idx);
                delta = Some(
                    self.buffer
                        .as_ref(ctx)
                        .invalidate_layout_for_range(invalidation_range),
                );
                break;
            }

            // We can early break since the visible range can only overlap with one hidden range.
            if start < starting_visible && end > starting_visible {
                self.buffer_selections.update(ctx, |selections, _| {
                    selections
                        .anchors
                        .update_anchor(end_anchor, starting_visible);
                });
                delta = Some(
                    self.buffer
                        .as_ref(ctx)
                        .invalidate_layout_for_range(invalidation_range),
                );
                break;
            } else if ending_visible > start && end > ending_visible {
                self.buffer_selections.update(ctx, |selections, _| {
                    selections
                        .anchors
                        .update_anchor(start_anchor, ending_visible);
                });
                delta = Some(
                    self.buffer
                        .as_ref(ctx)
                        .invalidate_layout_for_range(invalidation_range),
                );
                break;
            }
        }

        if let Some(idx) = to_remove {
            self.hidden_ranges.remove(idx);
        }

        self.materialize_hidden_range_offsets(buffer_version, ctx);

        delta
    }

    /// Check if a character offset range contains any hidden sections.
    pub fn contains_hidden_section(&self, range: &Range<CharOffset>, ctx: &AppContext) -> bool {
        self.is_hidden(range.start, ctx)
            || self.is_hidden(range.end, ctx)
            || self
                .hidden_ranges_at_latest(ctx)
                .iter()
                .any(|hidden_range| {
                    hidden_range.start < range.end && hidden_range.end > range.start
                })
    }

    /// Check if any selection head is immediately after a hidden section.
    pub fn after_hidden_section(&self, ctx: &AppContext) -> bool {
        let selections = self.buffer_selections.as_ref(ctx).selection_heads();
        let hidden_ranges = self.hidden_ranges_at_latest(ctx);

        selections
            .iter()
            .any(|&head| hidden_ranges.iter().any(|range| range.end == head))
    }

    /// Check if any selection head is immediately before a hidden section.
    pub fn before_hidden_section(&self, ctx: &AppContext) -> bool {
        let selections = self.buffer_selections.as_ref(ctx).selection_heads();
        let hidden_ranges = self.hidden_ranges_at_latest(ctx);

        selections
            .iter()
            .any(|&head| hidden_ranges.iter().any(|range| range.start == head))
    }

    pub fn hidden_ranges_at_version(&self, version: BufferVersion) -> RangeSet<CharOffset> {
        self.version_offsets
            .get(&version)
            .cloned()
            .unwrap_or_default()
    }

    pub fn hidden_ranges_at_latest(&self, ctx: &AppContext) -> RangeSet<CharOffset> {
        let latest_version = self.buffer.as_ref(ctx).buffer_version();
        self.hidden_ranges_at_version(latest_version)
    }

    /// Get all current hidden ranges as resolved offsets
    fn anchors_to_offsets(&self, ctx: &mut ModelContext<Self>) -> RangeSet<CharOffset> {
        self.hidden_ranges
            .iter()
            .filter_map(|(start_anchor, end_anchor)| {
                let start = self
                    .buffer_selections
                    .as_ref(ctx)
                    .resolve_anchor(start_anchor)?;
                let end = self
                    .buffer_selections
                    .as_ref(ctx)
                    .resolve_anchor(end_anchor)?;

                if start < end { Some(start..end) } else { None }
            })
            .collect()
    }
}

impl Entity for HiddenLinesModel {
    type Event = ();
}
