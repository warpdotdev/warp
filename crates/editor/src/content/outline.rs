use sum_tree::{Cursor, SeekBias};

use crate::content::text::BlockType;
use string_offset::CharOffset;

use super::{
    buffer::Buffer,
    text::{BlockCount, BufferBlockStyle, BufferText},
};

#[cfg(test)]
#[path = "outline_tests.rs"]
mod tests;

/// Outline of a block within the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockOutline {
    /// Offset of the block's start marker.
    pub start: CharOffset,
    /// Exclusive end of the block - the block marker that ends it.
    pub end: CharOffset,
    /// Style identifying the kind of block this is.
    pub block_type: BlockType,
}

/// Iterator over the blocks within a buffer.
struct BlockOutlines<'a> {
    count: BlockCount,
    cursor: Cursor<'a, BufferText, BlockCount, CharOffset>,
}

impl Buffer {
    /// Outlines the blocks in this buffer.
    ///
    /// This supports quickly finding blocks without the overhead of parsing all style and run
    /// information. It's useful for indexing (e.g. to show a table of contents) and building
    /// per-block state.
    pub fn outline_blocks(&self) -> impl Iterator<Item = BlockOutline> + '_ {
        BlockOutlines {
            cursor: self.content.cursor(),
            count: BlockCount::from(1),
        }
    }
}

impl Iterator for BlockOutlines<'_> {
    type Item = BlockOutline;

    fn next(&mut self) -> Option<Self::Item> {
        // The start marker for the current block will be at the cursor location where the block
        // count changes from `self.count - 1` to `self.count`.
        if !self.cursor.seek(&self.count, SeekBias::Left) {
            return None;
        };

        while let Some(BufferText::BlockMarker {
            marker_type: BufferBlockStyle::PlainText,
        }) = self.cursor.item()
        {
            self.count += 1;
            let found = self.cursor.seek(&self.count, SeekBias::Left);

            if !found {
                return None;
            }
        }

        let block_type = match self.cursor.item()? {
            BufferText::BlockMarker { marker_type }
                if *marker_type != BufferBlockStyle::PlainText =>
            {
                BlockType::Text(marker_type.clone())
            }
            BufferText::BlockItem { item_type } => BlockType::Item(item_type.clone()),
            other => {
                panic!("Invalid cursor state, expected a block-start marker but got {other:?}")
            }
        };
        let start_offset = *self.cursor.start();

        // The end of the current block is where the next one begins, so the count increases by 1.
        let end_count = self.count + 1;
        self.cursor.seek(&end_count, SeekBias::Left);

        // We sought to the start of the next block, so it's where we start on the next pass.
        self.count = end_count;
        Some(BlockOutline {
            start: start_offset,
            end: *self.cursor.start(),
            block_type,
        })
    }
}
