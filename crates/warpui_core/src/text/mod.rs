use anyhow::{Result, anyhow};
use itertools::Itertools;
use string_offset::{ByteOffset, CharCounter, CharOffset};

use self::point::Point;
use self::word_boundaries::{WordBoundaries, WordBoundariesPolicy};
use crate::event::ModifiersState;

pub mod header;
pub mod point;
pub mod word_boundaries;
pub mod words;

pub use header::BlockHeaderSize;
/// A row/column position in rendered character-cell content.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TuiGridPoint {
    pub row: usize,
    pub col: u16,
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub enum SelectionType {
    #[default]
    Simple,
    Semantic,
    Lines,
    Rect,
}

impl SelectionType {
    pub fn from_click_count(click_count: u32) -> Self {
        match click_count {
            0 => SelectionType::Simple,
            1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            3 => SelectionType::Lines,
            _ => SelectionType::Lines,
        }
    }

    pub fn from_mouse_event(modifiers: ModifiersState, click_count: u32) -> Self {
        let is_rect = if cfg!(target_os = "macos") {
            modifiers.cmd && modifiers.alt
        } else {
            modifiers.ctrl && modifiers.alt
        };

        if is_rect {
            return SelectionType::Rect;
        }

        SelectionType::from_click_count(click_count)
    }
}

impl From<SelectionType> for IsRect {
    fn from(selection_type: SelectionType) -> Self {
        match selection_type {
            SelectionType::Rect => IsRect::True,
            _ => IsRect::False,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum IsRect {
    True,
    #[default]
    False,
}

#[derive(Copy, Clone, Debug, Default)]
pub enum SelectionDirection {
    #[default]
    Forward,
    Backward,
}

/// A buffer of text characters. This trait acts as a base layer to implement text segmentation
/// on top of. Currently, it supports word navigation.
pub trait TextBuffer {
    type Chars<'a>: Iterator<Item = char> + 'a
    where
        Self: 'a;

    type CharsReverse<'a>: Iterator<Item = char> + 'a
    where
        Self: 'a;

    /// Begin iterating over the characters at `offset`, continuing to the end of the buffer.
    ///
    /// The maximum valid `offset` is the length of the buffer (not 1 less than the length). This
    /// allows starting just after the last character.
    fn chars_at(&self, offset: CharOffset) -> Result<Self::Chars<'_>>;

    /// Begin iterating backwards over the characters from `offset` to the start of the buffer.
    ///
    /// Note that this is _different_ from the semantics of `Iterator::rev`, which would instead
    /// start at the very end of the buffer.
    ///
    /// The maximum valid `offset` is the length of the buffer (not 1 less than the length). This
    /// allows starting just after the last character.
    fn chars_rev_at(&self, offset: CharOffset) -> Result<Self::CharsReverse<'_>>;

    /// Converts a character offset to a buffer [`Point`], if it is in bounds.
    fn to_point(&self, offset: CharOffset) -> Result<Point>;

    /// Convert a point to its offset within the buffer.
    fn to_offset(&self, point: Point) -> Result<CharOffset>;

    /// Get an iterator of word starting points forward from the given offset
    fn word_starts_from_offset<T: BufferIndex>(
        &self,
        position: T,
    ) -> Result<WordBoundaries<'_, Self>> {
        let offset = position.to_char_offset(self)?;
        Ok(WordBoundaries::forward_starts(
            offset,
            self.chars_at(offset)?,
            self,
        ))
    }

    /// Get an iterator of word ending points forward from the given offset, excluding the current
    /// location if it is a word boundary.
    ///
    /// Example: For a buffer of "word one two three", with an offset of `4` (immediately after
    /// the 'word'), this will yield columns [8, 12, 18], the ends of `one`, `two`, and `three`,
    /// but _excluding_ the initial position at the end of `word`.
    fn word_ends_from_offset_exclusive<T: BufferIndex>(
        &self,
        position: T,
    ) -> Result<WordBoundaries<'_, Self>> {
        let offset = position.to_char_offset(self)?;
        Ok(WordBoundaries::forward_ends_exclusive(
            offset,
            self.chars_at(offset)?,
            self,
        ))
    }

    /// Get an iterator of word ending points forward from the given offset, including the current
    /// location if appropriate.
    ///
    /// Example: For a buffer of "word one two three", with an offset of `4` (immediately after
    /// the 'word'), this will yield columns [4, 8, 12, 18], the ends of all four words,
    /// _including_ the initial position at the end of `word`.
    fn word_ends_from_offset_inclusive<T: BufferIndex>(
        &self,
        position: T,
    ) -> Result<WordBoundaries<'_, Self>> {
        let offset = position.to_char_offset(self)?;
        Ok(WordBoundaries::forward_ends_inclusive(
            offset,
            self.chars_at(offset)?,
            self,
        ))
    }

    /// Get an iterator of word starting points backwards from the given offset, excluding the
    /// current location if it is a word boundary.
    ///
    /// Example: For a buffer of "word one two three", with an offset of `13` (immediately before
    /// the 'three'), this will yield columns [9, 5, 0], the starts of `two`, `one`, and `word`,
    /// but _excluding_ the initial position at the start of `three`.
    fn word_starts_backward_from_offset_exclusive<T: BufferIndex>(
        &self,
        position: T,
    ) -> Result<WordBoundaries<'_, Self>> {
        let offset = position.to_char_offset(self)?;
        Ok(WordBoundaries::backward_starts_exclusive(
            offset,
            self.chars_rev_at(offset)?,
            self,
        ))
    }

    /// Get an iterator of word starting points backwards from the given offset, including the
    /// current location if appropriate.
    ///
    /// Example: For a buffer of "word one two three", with an offset of `13` (immediately before
    /// the 'three'), this will yield columns [13, 9, 5, 0], the starts of all four words,
    /// _including_ the initial position at the start of `three`.
    fn word_starts_backward_from_offset_inclusive<T: BufferIndex>(
        &self,
        position: T,
    ) -> Result<WordBoundaries<'_, Self>> {
        let offset = position.to_char_offset(self)?;
        Ok(WordBoundaries::backward_starts_inclusive(
            offset,
            self.chars_rev_at(offset)?,
            self,
        ))
    }

    /// Returns the target [`Point`] for expanding a semantic (double-click) selection from
    /// `position` in the given `direction`, respecting the word-boundary `policy`.
    fn semantic_expansion_target<T: BufferIndex>(
        &self,
        position: T,
        direction: SelectionDirection,
        policy: &WordBoundariesPolicy,
    ) -> Result<Point> {
        let offset = position.to_char_offset(self)?;

        let target = match direction {
            SelectionDirection::Forward => {
                let mut chars = self.chars_at(offset)?;
                let mut end = offset;
                // Include the character under the cursor (if any).
                if chars.next().is_some() {
                    end += 1;
                    // Extend through the following word only if the very next character is part
                    // of a word; a boundary char here stops us (no trailing whitespace, no
                    // crossing into the next word past a separator run).
                    if chars.next().is_some_and(|c| !policy.is_word_boundary(c)) {
                        end += 1;
                        for c in chars {
                            if policy.is_word_boundary(c) {
                                break;
                            }
                            end += 1;
                        }
                    }
                }
                end
            }
            SelectionDirection::Backward => {
                let mut chars = self.chars_rev_at(offset)?;
                let mut start = offset;
                // Extend left through a preceding word only if the character immediately before
                // the cursor is part of a word.
                if chars.next().is_some_and(|c| !policy.is_word_boundary(c)) {
                    start -= 1;
                    for c in chars {
                        if policy.is_word_boundary(c) {
                            break;
                        }
                        start -= 1;
                    }
                }
                start
            }
        };

        self.to_point(target)
    }
}

/// A type which can index into a text buffer.
pub trait BufferIndex {
    fn to_char_offset<B: TextBuffer + ?Sized>(&self, buffer: &B) -> Result<CharOffset>;
}

impl BufferIndex for CharOffset {
    fn to_char_offset<B: TextBuffer + ?Sized>(&self, _: &B) -> Result<CharOffset> {
        Ok(*self)
    }
}

impl BufferIndex for Point {
    fn to_char_offset<B: TextBuffer + ?Sized>(&self, buffer: &B) -> Result<CharOffset> {
        buffer.to_offset(*self)
    }
}

impl TextBuffer for str {
    type Chars<'a> = std::str::Chars<'a>;
    type CharsReverse<'a> = std::iter::Rev<std::str::Chars<'a>>;

    fn chars_at(&self, offset: CharOffset) -> Result<Self::Chars<'_>> {
        let chars = self.chars().count();
        if offset.as_usize() <= chars {
            Ok(self.chars().dropping(offset.as_usize()))
        } else {
            Err(anyhow!(
                "Offset {offset} out of bounds; char length is {chars}"
            ))
        }
    }

    fn chars_rev_at(&self, offset: CharOffset) -> Result<Self::CharsReverse<'_>> {
        let chars = self.chars().count();
        if offset.as_usize() <= chars {
            Ok(self.chars().rev().dropping(chars - offset.as_usize()))
        } else {
            Err(anyhow!(
                "Offset {offset} out of bounds; char length is {chars}"
            ))
        }
    }

    fn to_point(&self, offset: CharOffset) -> Result<Point> {
        let chars = self.chars().count();
        if offset.as_usize() <= chars {
            Ok(Point::new(0, offset.as_usize() as u32))
        } else {
            Err(anyhow!(
                "Offset {offset} out of bounds; char length is {chars}"
            ))
        }
    }

    fn to_offset(&self, point: Point) -> Result<CharOffset> {
        if point.row == 0 {
            let chars = self.chars().count();
            if (point.column as usize) <= chars {
                Ok(CharOffset::from(point.column as usize))
            } else {
                Err(anyhow!(
                    "Column {} out of bounds; char length is {chars}",
                    point.column
                ))
            }
        } else {
            Err(anyhow!(
                "Row {} out of bounds; str only has 1 row",
                point.row
            ))
        }
    }
}

/// Convert a slice of text into a `Vec` of UTF-8 bytes.
pub fn str_to_byte_vec(text: &str) -> Vec<u8> {
    text.as_bytes().iter().cloned().collect_vec()
}

/// Slice a string by [`char`] offsets, rather than byte offsets.
///
/// The starting index is inclusive, while the ending index is exclusive.
pub fn char_slice(s: &str, start: usize, end: usize) -> Option<&str> {
    if end < start {
        return None;
    }

    if start == end {
        return Some("");
    }

    let mut indices = s.char_indices();
    let (start_index, _) = indices.nth(start)?;
    // Why not just use `nth()` again? We need to distinguish between a `None` because `end`
    // is out of bounds and a `None` because `end` is the end of the string.
    // If/when Iterator::advance_by (https://github.com/rust-lang/rust/issues/77404) stabilizes,
    // we should use that. In the meantime, this doesn't hurt performance because `nth()`
    // also has to advance character-by-character.
    for _ in start + 1..end {
        indices.next()?;
    }

    let end_index = match indices.next() {
        Some((index, _)) => index,
        None => s.len(),
    };

    s.get(start_index..end_index)
}

pub fn count_chars_up_to_byte(text: &str, byte_offset: ByteOffset) -> Option<CharOffset> {
    if byte_offset.as_usize() == text.len() {
        return Some(CharOffset::from(text.chars().count()));
    }
    let mut counter = CharCounter::new(text);
    counter.char_offset(byte_offset)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
