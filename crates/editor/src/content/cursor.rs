use std::ops::Range;

use arrayvec::ArrayString;
use num_traits::SaturatingSub;
use pathfinder_color::ColorU;
use string_offset::CharOffset;
use sum_tree::{Cursor, Dimension, SeekBias, SumTree};

use super::text::{
    BufferSummary, BufferText, ColorMarker, LinkCount, LinkMarker, SyntaxColorId,
    TEXT_FRAGMENT_SIZE,
};

/// Cursor that provides utility function to traverse the buffer tree. Note that
/// this is always preferred over traversing buffer directly using a SumTree cursor given
/// the items could have different length (e.g. markers take 0 offset while text fragment
/// could take more than 1 offset).
#[derive(Clone, Debug)]
pub struct BufferCursor<'a, U: Dimension<'a, BufferSummary>> {
    cursor: Cursor<'a, BufferText, CharOffset, U>,
    offset: CharOffset,
}

impl<'a, U> BufferCursor<'a, U>
where
    U: Dimension<'a, BufferSummary>,
{
    pub fn new(cursor: Cursor<'a, BufferText, CharOffset, U>) -> Self {
        Self {
            cursor,
            offset: CharOffset::zero(),
        }
    }

    /// The current offset of the cursor.
    pub fn offset(&self) -> CharOffset {
        self.offset
    }

    /// Return the BufferText item at the given cursor position.
    pub fn item(&self) -> Option<&'a BufferText> {
        self.cursor.item()
    }

    /// Return the char at the given cursor position. This could be none
    /// if the cursor is at a style marker. Note that this is different from
    /// Self::item which will return the entire text fragment when cursor is on
    /// one.
    pub fn char(&self) -> Option<char> {
        match &self.item() {
            Some(BufferText::Text { fragment, .. }) => {
                let cursor_offset = *self.cursor.seek_position();
                let ix = self.offset - cursor_offset;

                fragment.chars().nth(ix.as_usize())
            }
            Some(BufferText::BlockMarker { .. }) | Some(BufferText::Newline) => Some('\n'),
            _ => None,
        }
    }

    /// Move the cursor to the given charoffset and return the char at the cursor position.
    /// This could be empty.
    pub fn char_at(&mut self, offset: CharOffset) -> Option<char> {
        self.seek_to_offset_after_markers(offset);
        self.char()
    }

    pub fn start(&self) -> &U {
        self.cursor.start()
    }

    /// Attempts to move to the next character position. If the next item has
    /// zero character offset length, move to that item and keep the current character
    /// offset.
    ///
    /// This is different from Self::next as it will only increment the offset and not
    /// move the cursor if it is in the middle of a text fragment.
    pub fn next_char_position(&mut self) {
        let end = self.cursor.end_seek_position();

        let next_char_position = self.offset + 1;
        if next_char_position >= end {
            self.cursor.next();
            self.offset = *self.cursor.seek_position();
        } else {
            self.offset = next_char_position;
        }
    }

    /// Attempts to move to the previous character position. If the previous item has
    /// zero character offset length, move to that item and keep the current character
    /// offset.
    ///
    /// This is different from Self::prev as it will only decrement the offset and not
    /// move the cursor if it is in the middle of a text fragment.
    pub fn prev_char_position(&mut self) {
        let start = *self.cursor.seek_position();

        let prev_char_position = self.offset.saturating_sub(&CharOffset::from(1));
        if start > prev_char_position || self.offset == CharOffset::zero() {
            self.cursor.prev();
            // If we move into a text fragment, this makes sure we are not jumping
            // to the start of that fragment.
            self.offset = (*self.cursor.seek_position()).max(prev_char_position);
        } else {
            self.offset = prev_char_position;
        }
    }

    /// Moves directly to the next buffer text item and updates the active offset.
    pub fn next(&mut self) {
        self.cursor.next();
        self.offset = *self.cursor.seek_position();
    }

    pub fn prev_item(&self) -> Option<&'a BufferText> {
        self.cursor.prev_item()
    }

    /// Place the cursor at the beginning of end before all the zero-width markers.
    pub fn seek_to_offset_before_markers(&mut self, end: CharOffset) -> bool {
        debug_assert!(end >= self.offset);

        let found = self.cursor.seek(&end, SeekBias::Left);

        if found && self.cursor.end_seek_position() == end {
            self.cursor.next();
        }

        self.offset = end;
        found || end == CharOffset::zero()
    }

    /// Place the cursor at the beginning of end after all the zero-width markers.
    pub fn seek_to_offset_after_markers(&mut self, end: CharOffset) -> bool {
        if end < self.offset {
            return false;
        }
        debug_assert!(end >= self.offset);

        self.offset = end;
        self.cursor.seek(&end, SeekBias::Right)
    }

    /// Place the cursor at the beginning of end before all the zero-width markers.
    /// Return a new SumTree with items from cursor up to but not including end.
    pub fn slice_to_offset_before_markers(&mut self, end: CharOffset) -> SumTree<BufferText> {
        debug_assert!(end >= self.offset);
        let mut new_content = SumTree::new();

        // If the current offset is in the middle of a text fragment, split the fragment and push
        // the latter half to the SumTree.
        if self.offset > *self.cursor.seek_position()
            && let Some(BufferText::Text { fragment, .. }) = self.cursor.item()
        {
            let start_ix = self.offset - *self.cursor.seek_position();
            let char_to_take = end - self.offset;
            let new_fragment: String = fragment
                .chars()
                .skip(start_ix.as_usize())
                .take(char_to_take.as_usize())
                .collect();

            if !new_fragment.is_empty() {
                new_content.append_str(&new_fragment);
            }

            if end < self.cursor.end_seek_position() {
                self.offset = end;
                return new_content;
            }

            self.cursor.next();
        }

        let sliced = self.cursor.slice(&end, SeekBias::Left);
        new_content.push_tree(sliced);

        // If the end offset is in the middle of a text fragment, split the fragment and push
        // the earlier half to the SumTree.
        if self.cursor.end_seek_position() > end {
            if let Some(BufferText::Text { fragment, .. }) = self.cursor.item() {
                let ix = end - *self.cursor.seek_position();
                let new_fragment: String = fragment.chars().take(ix.as_usize()).collect();

                if !new_fragment.is_empty() {
                    new_content.append_str(&new_fragment);
                }
            }
        } else if end > *self.cursor.seek_position() && end > CharOffset::zero() {
            if let Some(item) = self.cursor.item() {
                new_content.push(item.clone());
            }

            self.cursor.next();
        }

        self.offset = end;
        new_content
    }

    /// Place the cursor at the beginning of end after all the zero-width markers.
    /// Return a new SumTree with items from cursor up to but not including end.
    pub fn slice_to_offset_after_markers(&mut self, end: CharOffset) -> SumTree<BufferText> {
        debug_assert!(end >= self.offset);
        let mut new_content = SumTree::new();

        // If the current offset is in the middle of a text fragment, split the fragment and push
        // the earlier half to the SumTree.
        if self.offset > *self.cursor.seek_position()
            && let Some(BufferText::Text { fragment, .. }) = self.cursor.item()
        {
            let ix = self.offset - *self.cursor.seek_position();
            let char_to_take = end - self.offset;
            let new_fragment: String = fragment
                .chars()
                .skip(ix.as_usize())
                .take(char_to_take.as_usize())
                .collect();

            if !new_fragment.is_empty() {
                new_content.append_str(&new_fragment);
            }

            if end < self.cursor.end_seek_position() {
                self.offset = end;
                return new_content;
            }

            self.cursor.next();
        }

        new_content.push_tree(self.cursor.slice(&end, SeekBias::Right));

        // If the current offset is in the middle of a text fragment, split the fragment and push
        // the latter half to the SumTree.
        if *self.cursor.seek_position() < end
            && let Some(BufferText::Text { fragment, .. }) = self.cursor.item()
        {
            let ix = end - *self.cursor.seek_position();
            let new_fragment: String = fragment.chars().take(ix.as_usize()).collect();

            if !new_fragment.is_empty() {
                new_content.append_str(&new_fragment);
            }
        }

        self.offset = end;
        new_content
    }

    /// Return a new SumTree with all items after the current cursor.
    pub fn suffix(&mut self) -> SumTree<BufferText> {
        let mut new_content = SumTree::new();
        if self.offset > *self.cursor.seek_position()
            && let Some(BufferText::Text { fragment, .. }) = self.cursor.item()
        {
            let start_ix = self.offset - *self.cursor.seek_position();
            let new_fragment: String = fragment.chars().skip(start_ix.as_usize()).collect();

            if !new_fragment.is_empty() {
                new_content.append_str(&new_fragment);
            }

            self.cursor.next();
        }
        new_content.push_tree(self.cursor.suffix());
        new_content
    }
}

pub trait BufferSumTree {
    /// Replace an item at the given offset with a new BufferText item.
    /// The edit happens in-palace.
    fn replace_item_at_offset(&mut self, offset: CharOffset, item: BufferText);

    /// Renders a debug representation of this SumTree.
    fn debug(&self) -> String;

    /// Returns the url with the given link count if the link exists.
    fn url_at_link_count(&self, link_count: &LinkCount) -> Option<String>;

    /// Returns the color with the given color count if there is a decorated syntax color.
    fn color_at_color_count(&self, color_count: &SyntaxColorId) -> Option<ColorU>;

    /// Iterate over items in the given character range.
    fn items_in_range<'a, U: sum_tree::Dimension<'a, BufferSummary>>(
        &'a self,
        range: Range<CharOffset>,
    ) -> BoundedCursor<'a, U>;

    /// Append a new str to the end of the SumTree. If the last item in the tree is a Text
    /// fragment and it has extra byte size left, fill that text fragment first before creating
    /// a new one.
    fn append_str(&mut self, s: &str);
}

impl BufferSumTree for SumTree<BufferText> {
    fn replace_item_at_offset(&mut self, offset: CharOffset, item: BufferText) {
        let old_tree = self.clone();
        let mut new_tree = SumTree::new();
        let cursor = old_tree.cursor::<CharOffset, CharOffset>();
        let mut buffer_cursor = BufferCursor::new(cursor);

        new_tree.push_tree(buffer_cursor.slice_to_offset_after_markers(offset));
        new_tree.push(item);

        buffer_cursor.next();
        new_tree.push_tree(buffer_cursor.suffix());
        drop(buffer_cursor);
        *self = new_tree;
    }

    fn items_in_range<'a, U: sum_tree::Dimension<'a, BufferSummary>>(
        &'a self,
        range: Range<CharOffset>,
    ) -> BoundedCursor<'a, U> {
        let inner = self.cursor::<CharOffset, U>();
        let mut buffer_cursor = BufferCursor::new(inner);
        buffer_cursor.seek_to_offset_after_markers(range.start);
        BoundedCursor {
            inner: buffer_cursor,
            end: range.end,
        }
    }

    fn url_at_link_count(&self, link_count: &LinkCount) -> Option<String> {
        let mut cursor = self.cursor::<LinkCount, ()>();
        cursor.seek(link_count, SeekBias::Left);

        match cursor.item() {
            Some(BufferText::Link(LinkMarker::Start(url))) => Some(url.clone()),
            _ => None,
        }
    }

    fn color_at_color_count(&self, color_count: &SyntaxColorId) -> Option<ColorU> {
        let mut cursor = self.cursor::<SyntaxColorId, ()>();
        cursor.seek(color_count, SeekBias::Left);

        match cursor.item() {
            Some(BufferText::Color(ColorMarker::Start(color))) => Some(*color),
            _ => None,
        }
    }

    fn debug(&self) -> String {
        use std::fmt::Write;

        let mut cursor = self.cursor::<(), ()>();
        cursor.descend_to_first_item(self, |_| true);
        let mut total_string = String::new();
        while let Some(item) = cursor.item() {
            let _ = write!(&mut total_string, "{item}");
            cursor.next();
        }

        total_string
    }

    fn append_str(&mut self, s: &str) {
        let trailing_newline = s.ends_with('\n');
        let mut is_first = true;
        let mut new_fragments = Vec::new();

        // Split str into lines first. For linebreaks, we need to push BufferText::Newline.
        let mut lines = s.lines().peekable();
        while let Some(line) = lines.next() {
            let mut text = line;
            // For the first fragment we are pushing, try to fill up the trailing text fragment if 1) it exists 2) it has extra byte space.
            if is_first && !self.is_empty() {
                self.update_last(|last_text| {
                    if let BufferText::Text {
                        fragment,
                        char_count,
                    } = last_text
                    {
                        let split_ix = if fragment.len() + text.len() <= TEXT_FRAGMENT_SIZE {
                            text.len()
                        } else {
                            let mut split_ix = TEXT_FRAGMENT_SIZE
                                .saturating_sub(fragment.len())
                                .min(text.len());
                            while !text.is_char_boundary(split_ix) {
                                split_ix -= 1;
                            }
                            split_ix
                        };

                        let (suffix, remainder) = text.split_at(split_ix);
                        fragment.push_str(suffix);
                        *char_count = fragment.chars().count() as u8;

                        text = remainder;
                    }
                });
            }
            is_first = false;

            // If there are still remaining text, push it as a new fragment.
            while !text.is_empty() {
                let mut split_ix = text.len().min(TEXT_FRAGMENT_SIZE);
                while !text.is_char_boundary(split_ix) {
                    split_ix -= 1;
                }
                let (chunk, remainder) = text.split_at(split_ix);
                new_fragments.push(BufferText::Text {
                    char_count: chunk.chars().count() as u8,
                    fragment: ArrayString::from(chunk).unwrap(),
                });
                text = remainder;
            }

            if lines.peek().is_some() || trailing_newline {
                new_fragments.push(BufferText::Newline);
            }
        }
        self.extend(new_fragments);
    }
}

/// A [`Cursor`] that limits itself to only consuming a maximum number of characters.
/// Use this as a building block for range-based iteration.
pub struct BoundedCursor<'a, U: Dimension<'a, BufferSummary>> {
    inner: BufferCursor<'a, U>,
    end: CharOffset,
}

impl<'a, U> BoundedCursor<'a, U>
where
    U: sum_tree::Dimension<'a, BufferSummary>,
{
    /// Summary at the start of the current cursor position (see [`Cursor::start`]).
    pub fn start(&self) -> &U {
        self.inner.start()
    }
}

impl<'a, U> Iterator for BoundedCursor<'a, U>
where
    U: sum_tree::Dimension<'a, BufferSummary>,
{
    type Item = &'a BufferText;

    fn next(&mut self) -> Option<Self::Item> {
        if self.inner.offset() >= self.end {
            None
        } else {
            let item = self.inner.item()?;
            self.inner.next();
            Some(item)
        }
    }
}

#[cfg(test)]
#[path = "cursor_tests.rs"]
mod tests;
