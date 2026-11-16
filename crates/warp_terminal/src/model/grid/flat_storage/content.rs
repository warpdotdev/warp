//! Structures for storing the grid contents in a flat buffer.

use std::collections::BTreeMap;
use std::ops::{Index, Range};

use get_size::GetSize;
use string_offset::ByteOffset;

use super::grapheme::Grapheme;
use crate::model::char_or_str::CharOrStr;

/// A helper structure that wraps the underlying grid content and provides
/// higher-level APIs for accessing the data.
///
/// Internally, this stores content in a series of chunks.  By chunking up
/// the content, we can efficiently trim data off the _front_ of the content
/// string without having to make any (expensive) copies.
///
/// From the outside, this structure is best conceptualized as a non-circular
/// circular buffer, keyed by content offset.  The start of the buffer is
/// pointed to by `active_chunk.start_offset + active_chunk.len`, and the end
/// of the buffer is pointed to by the `content_offset` of the first entry in
/// the flat storage [`Index`](super::Index).  We never "re-zero" the offsets
/// in the buffer, and so the tail pointer only ever moves forward.  (The head
/// pointer can move backwards when we pop rows off of the end of flat storage,
/// and so remove content from the end of the buffer.)
///
/// [`Content::push_grapheme`] inserts new content and moves the head pointer
/// forwards; [`Content::truncate`] moves the head pointer backwards.  The tail
/// pointer isn't directly modified within this structure, as it is owned by
/// [`super::Index`].  When rows are dropped from the index, this structure is
/// notified via a call to [`Content::truncate_front`], which drops any chunks
/// which entirely precede the new tail pointer.
#[derive(Debug, Clone)]
pub struct Content {
    /// The already-full chunks of content, keyed by the offset of the final
    /// byte of the chunk, inclusive.  This offset is from the start of the
    /// content.
    ///
    /// Keying by the offset of the final byte of the chunk means we can
    /// easily find all chunks starting from a given offset using the
    /// [`BTreeMap::range`] API, passing an open range starting from the
    /// target offset.
    filled_chunks: BTreeMap<ByteOffset, Chunk>,

    /// The current chunk being built.
    active_chunk: Chunk,
}

impl Content {
    pub fn new() -> Self {
        Self {
            filled_chunks: Default::default(),
            active_chunk: Chunk::new(ByteOffset::zero()),
        }
    }

    /// Pushes the given grapheme onto the end of the content.
    pub fn push_grapheme(&mut self, grapheme: &Grapheme) {
        let grapheme_len = grapheme.len().as_usize();
        assert!(
            grapheme_len < Chunk::CHUNK_SIZE,
            "grapheme with length {grapheme_len} exceeds chunk size of {}",
            Chunk::CHUNK_SIZE
        );

        // Check to make sure there's enough room in the active chunk.
        if self.active_chunk.len() + grapheme_len > self.active_chunk.capacity() {
            // Set a new chunk as the last chunk.
            let new_start_offset = self.active_chunk.content_range().end;
            let full_chunk =
                std::mem::replace(&mut self.active_chunk, Chunk::new(new_start_offset));

            // Insert the now-full chunk into the map.
            let chunk_end_byte_offset = full_chunk.content_range().end - 1;
            self.filled_chunks.insert(chunk_end_byte_offset, full_chunk);
        }

        self.active_chunk.push_char_or_str(grapheme.content());
    }

    /// Truncates the content to the given length, in bytes.
    pub fn truncate(&mut self, new_len: ByteOffset) {
        // Split off any chunks that end after our new length.
        let mut truncated_chunks = self.filled_chunks.split_off(&new_len);
        // If we split off any chunks, the first removed chunk is our new active chunk.
        if let Some((_, active_chunk)) = truncated_chunks.pop_first() {
            self.active_chunk = active_chunk;
        }

        // Shorten the active chunk accordingly.
        self.active_chunk.truncate(new_len);

        debug_assert_eq!(self.end_offset(), new_len.as_usize());
    }

    /// Drops chunks that entirely precede the given start offset.
    pub fn truncate_front(&mut self, new_start_offset: ByteOffset) {
        // We use this approach as opposed to something like `split_off` because
        // in the common case, we remove 0 or 1 chunks, which is more efficient
        // to do explicitly.
        loop {
            match self.filled_chunks.first_entry() {
                Some(entry) if entry.key() < &new_start_offset => {
                    entry.remove();
                }
                _ => break,
            }
        }
    }

    /// Returns the offset of the end of the content (exclusive).
    ///
    /// If a new grapheme is pushed into the buffer, this is the offset at
    /// which it will start.
    ///
    /// This is somewhat similar to [`String::len`], but due to the circular
    /// nature of the buffer, it is not a true measure of the actual amount of
    /// content stored.
    pub fn end_offset(&self) -> usize {
        self.active_chunk.start_offset.as_usize() + self.active_chunk.len()
    }
}

impl Index<Range<ByteOffset>> for Content {
    type Output = str;

    fn index(&self, index: Range<ByteOffset>) -> &Self::Output {
        // Get the chunk that contains the start of the range, and the offset
        // of the start of that chunk.
        let chunk = self
            .filled_chunks
            .range(index.start..)
            .next()
            .map(|(_, v)| v)
            .unwrap_or(&self.active_chunk);

        // Compute the start and end of the range within the chunk.
        debug_assert!(
            index.start >= chunk.start_offset,
            "grapheme start offset ({}) must be >= chunk.start_offset ({})",
            index.start,
            chunk.start_offset
        );
        debug_assert!(
            index.end <= chunk.content_range().end,
            "grapheme end offset ({}) must be <= chunk.end_offset ({})",
            index.end,
            chunk.content_range().end
        );
        let start = index.start - chunk.start_offset;
        let end = index.end - chunk.start_offset;

        // Return the requested slice.
        &chunk[start.as_usize()..end.as_usize()]
    }
}

// Manually implement `GetSize` instead of using the derive macro because
// actually traversing the BTreeMap is expensive, and using this estimate
// is close enough for our purposes.
impl GetSize for Content {
    fn get_heap_size(&self) -> usize {
        self.active_chunk.get_heap_size() * (1 + self.filled_chunks.len())
    }
}

/// A single chunk of content, backed by a byte array.
#[derive(Debug, Clone, GetSize)]
struct Chunk {
    /// The number of bytes stored within the `content` array.
    len: usize,

    /// The actual content of the chunk.
    ///
    /// We store this inline as a byte array rather than using a
    /// dynamically-sized type to avoid the extra heap allocation
    /// and pointer dereference needed to access it.
    content: [u8; Chunk::CHUNK_SIZE],

    /// The offset of the start of this chunk, in bytes.
    start_offset: ByteOffset,
}

impl Chunk {
    const CHUNK_SIZE: usize = 1024;

    /// Creates a new chunk with the given start offset.
    fn new(start_offset: ByteOffset) -> Self {
        Self {
            len: 0,
            content: [0; Chunk::CHUNK_SIZE],
            start_offset,
        }
    }

    /// Pushes the given string into the chunk.
    ///
    /// Panics if there is not enough space left in the chunk.
    fn push_char_or_str(&mut self, char_or_str: CharOrStr<'_>) {
        match char_or_str {
            CharOrStr::Char(c) => {
                let len = c.len_utf8();
                match len {
                    1 => self.content[self.len] = c as u8,
                    len => self.content[self.len..self.len + len]
                        .copy_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
                }
                self.len += len;
            }
            CharOrStr::Str(s) => {
                let len = s.len();
                self.content[self.len..self.len + len].copy_from_slice(s.as_bytes());
                self.len += len;
            }
        }
    }

    /// Truncates the chunk to the given content offset.
    ///
    /// While the bytes stored in the chunk are interpreted as UTF-8, this
    /// function does not perform any validation, and so can end up producing
    /// a chunk that cannot parse as UTF-8.  It is already the caller's
    /// responsibility to not index into the chunk at non-UTF-8 offsets, so
    /// this does not introduce any additional safety concerns.
    fn truncate(&mut self, offset: ByteOffset) {
        debug_assert!(
            offset >= self.start_offset,
            "cannot apply truncate({offset:?}) to chunk with start_offset of {:?}",
            self.start_offset
        );
        let new_chunk_len = offset - self.start_offset;
        self.len = new_chunk_len.as_usize();
    }

    /// Returns the length of the chunk, in bytes.
    fn len(&self) -> usize {
        self.len
    }

    /// Returns the capacity of the chunk, in bytes.
    fn capacity(&self) -> usize {
        Self::CHUNK_SIZE
    }

    /// Returns the range of bytes covered by the chunk.
    fn content_range(&self) -> Range<ByteOffset> {
        self.start_offset..self.start_offset + self.len()
    }
}

impl Index<Range<usize>> for Chunk {
    type Output = str;

    fn index(&self, index: Range<usize>) -> &Self::Output {
        // SAFETY: We know that the content is valid UTF-8, and are trusting
        // the caller to ensure that the index is valid.  Validating that the
        // contents are valid UTF-8 is too expensive to do on every access.
        unsafe { std::str::from_utf8_unchecked(&self.content[index]) }
    }
}

#[cfg(test)]
#[path = "content_tests.rs"]
mod tests;
