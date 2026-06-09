//! Text segmentation (splitting on words and grapheme clusters) for rich-text buffers.
//! Mostly, this relies on the [`crate::text::word_boundaries`] module by implementing the
//! [`TextBuffer`] API.

use anyhow::anyhow;
use string_offset::CharOffset;
use warpui_core::text::TextBuffer;
use warpui_core::text::point::Point;
use warpui_core::text::word_boundaries::WordBoundariesPolicy;

use super::buffer::{Buffer, ToBufferCharOffset, ToBufferPoint};
use super::cursor::BufferCursor;

#[cfg(test)]
#[path = "segmentation_tests.rs"]
mod tests;

impl Buffer {
    /// Get the offset of the start of the word closest to the given position.
    pub fn word_start(&self, offset: CharOffset, policy: &WordBoundariesPolicy) -> CharOffset {
        self.word_starts_backward_from_offset_inclusive(offset)
            .ok()
            .and_then(|word_starts| word_starts.with_policy(policy).next())
            .map(|point| point.to_buffer_char_offset(self))
            .unwrap_or(offset)
    }

    /// Get the offset of the end of the word closest to the given position.
    pub fn word_end(&self, offset: CharOffset, policy: &WordBoundariesPolicy) -> CharOffset {
        self.word_ends_from_offset_inclusive(offset)
            .ok()
            .and_then(|word_ends| word_ends.with_policy(policy).next())
            .map(|point| point.to_buffer_char_offset(self))
            .unwrap_or(offset)
    }
}

impl TextBuffer for Buffer {
    type Chars<'a>
        = Chars<'a>
    where
        Self: 'a;

    type CharsReverse<'a>
        = Chars<'a>
    where
        Self: 'a;

    fn chars_at(&self, offset: CharOffset) -> anyhow::Result<Self::Chars<'_>> {
        Chars::new(self, offset)
    }

    fn chars_rev_at(&self, offset: CharOffset) -> anyhow::Result<Self::CharsReverse<'_>> {
        Chars::new_reversed(self, offset)
    }

    fn to_point(&self, offset: CharOffset) -> anyhow::Result<Point> {
        Ok(offset.to_buffer_point(self))
    }

    fn to_offset(&self, point: Point) -> anyhow::Result<CharOffset> {
        Ok(point.to_buffer_char_offset(self))
    }
}

pub struct Chars<'a> {
    /// Underlying cursor into the buffer.
    cursor: BufferCursor<'a, CharOffset>,
    /// Whether we're iterating in the reverse direction.
    reversed: bool,
    is_first: bool,
}

impl<'a> Chars<'a> {
    /// Returns a new char iterator starting at `offset`, or an error if the offset is out of bounds.
    fn new(buffer: &'a Buffer, offset: CharOffset) -> anyhow::Result<Self> {
        if offset > buffer.max_charoffset() {
            return Err(anyhow!("char offset {offset} out of bounds"));
        }

        let cursor = buffer.content.cursor();
        let mut buffer_cursor = BufferCursor::new(cursor);
        buffer_cursor.seek_to_offset_before_markers(offset);

        Ok(Self {
            cursor: buffer_cursor,
            reversed: false,
            is_first: true,
        })
    }

    /// Returns a new reversed char iterator starting at `offset`, or an error if the offset is out
    /// of bounds.
    fn new_reversed(buffer: &'a Buffer, offset: CharOffset) -> anyhow::Result<Self> {
        if offset > buffer.max_charoffset() {
            return Err(anyhow!("char offset {offset} out of bounds"));
        }
        let cursor = buffer.content.cursor();
        let mut buffer_cursor = BufferCursor::new(cursor);
        buffer_cursor.seek_to_offset_after_markers(offset);

        Ok(Self {
            cursor: buffer_cursor,
            reversed: true,
            is_first: true,
        })
    }

    /// Move the underlying cursor to the next item (in the current direction).
    fn advance(&mut self) {
        // If this is the first `next` call and the direction is not reversed,
        // we don't need to advance the cursor as it is already in the right position.
        if self.is_first && !self.reversed {
            self.is_first = false;
            return;
        }

        if self.reversed {
            self.cursor.prev_char_position();
        } else {
            self.cursor.next_char_position();
        }
    }
}

impl Iterator for Chars<'_> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.advance();
            if self.cursor.item().is_none() {
                break None;
            }

            match self.cursor.char() {
                // This treats placeholders as invisible, like style markers. We may want to return
                // a fake boundary character instead so that placeholders split word boundaries.
                None => continue,
                Some(character) => break Some(character),
            }
        }
    }
}
