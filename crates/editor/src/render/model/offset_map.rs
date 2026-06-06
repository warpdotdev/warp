use std::ops::{Add, Sub};

use itertools::Itertools;
use num_traits::SaturatingSub;
use string_offset::CharOffset;

use super::FrameOffset;

#[cfg(test)]
#[path = "offset_map_tests.rs"]
mod tests;

/// Mapping between visible character offsets and interactive content character offsets. Due to
/// placeholder text, not all characters painted on the screen correspond to interactive characters
/// in the content model.
///
/// The mapping model assumes that, within the overall set of characters in a text frame, only
/// certain character runs are interactive. Within those runs, there's a 1:1 mapping between
/// `char`s in the content model and `char`s in the text frame. It translates in two directions:
/// * From a [`CharOffset`] relative to the start of the content model block (a [`super::Paragraph`])
///   to the character index in the [`warpui_core::text_layout::TextFrame`].
/// * From a [`FrameOffset`] in the `TextFrame` to the closest `CharOffset` in the content model
///   block (for example, clicking within a placeholder should snap the cursor to a regular content
///   character).
///
/// An interactive character is one that the user can select and edit.
#[derive(Debug, Clone)]
pub struct OffsetMap {
    /// Sorted list of interactive content runs.
    runs: Vec<SelectableTextRun>,
}

/// A run of selectable/interactive content in the [`OffsetMap`]. This includes all user-written
/// characters, but not placeholder text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectableTextRun {
    /// The offset of the first content-character in this run.
    pub content_start: CharOffset,
    /// The offset of the first [`TextFrame`] visible character in this run.
    pub frame_start: FrameOffset,
    /// The total number of characters in this run. The run length is the same for both content
    /// characters and text frame characters. That is, this run represents:
    /// * Characters `content_start..content_start+length` within the containing paragraph
    /// * Characters `frame_start..frame_start+length` within the containing `TextFrame`
    pub length: usize,
}

impl OffsetMap {
    /// Creates a new [`OffsetMap`] that translates 1:1 between visible character offsets and
    /// interactive content character offsets.
    pub fn direct(length: usize) -> Self {
        Self::new(vec![SelectableTextRun {
            content_start: CharOffset::zero(),
            frame_start: FrameOffset::zero(),
            length,
        }])
    }

    pub fn new(mut runs: Vec<SelectableTextRun>) -> Self {
        runs.sort_unstable_by_key(|r| r.frame_start);
        if cfg!(debug_assertions) {
            // Content- and frame-offsets should both increase from run to run.
            for (a, b) in runs.iter().tuple_windows() {
                assert!(a.content_start < b.content_start, "Runs must be ascending");
            }
        }
        Self { runs }
    }

    /// Translate a visible character to the closest content character.
    pub fn to_content(&self, offset: FrameOffset) -> CharOffset {
        self.translate(offset)
    }

    /// Translate a content character to the corresponding visible character.
    pub fn to_frame(&self, offset: CharOffset) -> FrameOffset {
        // When going from content to frame offsets, the character is almost always going to be
        // inside an interactive run. The exception is the placeholder marker character, and so
        // it's simpler to reuse the translation algorithm from the other direction rather than
        // detect and handle this special case.
        self.translate(offset)
    }

    /// Translate from one kind of offset to the other. If the source offset is within an interactive
    /// run, this returns the exact location of the character within that run. Otherwise, it rounds
    /// up or down to the nearest interactive run.
    fn translate<T: ParagraphOffset, U: ParagraphOffset>(&self, offset: T) -> U {
        // Find the runs before and after `offset`, and calculate for each:
        // - The absolute distance from it to `offset`
        // - The translated offset according to it
        let after_idx = self.runs.partition_point(|run| T::start(run) < offset);

        let after = self.runs.get(after_idx).map(|run| {
            let distance = T::start(run) - offset;
            // If the run after `offset` is closer, then snap to the start of that run (e.g. the
            // first regular character after a placeholder).
            (distance, U::start(run))
        });

        let before = after_idx
            .checked_sub(1)
            .and_then(|idx| self.runs.get(idx))
            .map(|run| {
                // For the before run, we measure the distance from its end to offset. However, the
                // offset could be within the run, so we clamp down to 0.
                let run_end = T::start(run) + run.length;
                let distance = offset.saturating_sub(&run_end);
                // If we pick the before run, we essentially rebase it onto the destination start, but then
                // clamp to stay within the run.
                let translation =
                    U::start(run) + (offset - T::start(run)).as_usize().min(run.length);
                (distance, translation)
            });

        match (before, after) {
            (None, None) => U::zero(), // This should only happen if the frame is empty.
            (None, Some((_, translation))) => translation,
            (Some((_, translation)), None) => translation,
            (
                Some((before_distance, before_translation)),
                Some((after_distance, after_translation)),
            ) => {
                if before_distance < after_distance {
                    before_translation
                } else {
                    after_translation
                }
            }
        }
    }
}

/// Abstracts over [`FrameOffset`] and [`CharOffset`] so that we can use the same conversion
/// algorithm for both.
trait ParagraphOffset:
    Add<usize, Output = Self> + Ord + Sub<Output = Self> + SaturatingSub + Copy + Sized
{
    /// Gets the start offset of a [`SelectableTextRun`] for this offset/coordinate system.
    fn start(run: &SelectableTextRun) -> Self;

    fn zero() -> Self;

    fn as_usize(self) -> usize;
}

impl ParagraphOffset for CharOffset {
    fn start(run: &SelectableTextRun) -> Self {
        run.content_start
    }

    fn zero() -> Self {
        CharOffset::zero()
    }

    fn as_usize(self) -> usize {
        CharOffset::as_usize(self)
    }
}

impl ParagraphOffset for FrameOffset {
    fn start(run: &SelectableTextRun) -> Self {
        run.frame_start
    }

    fn zero() -> Self {
        FrameOffset::zero()
    }

    fn as_usize(self) -> usize {
        FrameOffset::as_usize(self)
    }
}
