//! This module is for "block objects" i.e. text objects delimited by brackets/parentheses.

use std::ops::Range;

use itertools::Itertools;
use string_offset::CharOffset;
use warpui::text::TextBuffer;

use crate::vim::{BracketChar, BracketEnd, BracketType};
use crate::vim_find_matching_bracket;

/// Vim's block-based text objects, e.g. `di{`. This includes a string of text enclosed by any pair
/// of [`BracketType`], not including the brackets themselves.
/// See https://vimdoc.sourceforge.net/htmldoc/motion.html#ib
/// or enter ":help i{" in Vim.
///
/// There may be whitespace in within the brackets which is also not included. If the closing
/// bracket is on a different line then the opening bracket, and there is only whitespace between
/// the closing bracket and the newline preceding it, all that space plus the newline are _not_
/// included in the text object. (AFAIK this is not documented.) We can refer to that space as
/// "trailing padding". Furthermore, the `c` and `d` commands differ in how they respect "leading
/// padding", i.e. the whitespace plus newline after the opening bracket, in that `d` will delete
/// the leading padding but `c` will not. The `preserve_leading_padding` parameter controls this.
///
/// Returns None when the buffer is empty, the offset is out of bounds, or there is no matching
/// pair of brackets of the type we're looking for around the cursor.
pub fn vim_inner_block<T, C>(
    buffer: &T,
    offset: C,
    bracket_type: BracketType,
    preserve_leading_padding: bool,
) -> Option<Range<CharOffset>>
where
    T: TextBuffer + ?Sized,
    C: Into<CharOffset>,
{
    let block_range = vim_a_block(buffer, offset, bracket_type)?;
    let (mut block_start, mut block_end) = (block_range.start, block_range.end);

    // Move bounds by 1 to remove the brackets, though we may need to trim any trailing or leading
    // padding.
    block_start += 1;
    block_end -= 1;

    // First, check if we need to remove trailing padding from the range.
    if let Some((i, _)) = buffer
        .chars_rev_at(block_end)
        .ok()?
        .take_while(|c| c.is_whitespace())
        .find_position(|c| *c == '\n')
    {
        block_end -= i + 1;
    }

    // Finally, if we're doing a command that preserves leading padding, e.g. `c`, check if we need
    // to remove that from the range.
    if preserve_leading_padding {
        if let Some((i, _)) = buffer
            .chars_at(block_start)
            .ok()?
            .take_while(|c| c.is_whitespace())
            .find_position(|c| *c == '\n')
        {
            block_start += i + 1;
        }
    }

    Some(block_start..block_end)
}

/// Vim's block-based text objects, e.g. `da{`. This includes a string of text enclosed by any pair
/// of [`BracketType`], including the brackets themselves.
/// See https://vimdoc.sourceforge.net/htmldoc/motion.html#ab
/// or enter ":help a{" in Vim.
///
/// Returns None when the buffer is empty, the offset is out of bounds, or there is no matching
/// pair of brackets of the type we're looking for around the cursor.
pub fn vim_a_block<T, C>(
    buffer: &T,
    offset: C,
    bracket_type: BracketType,
) -> Option<Range<CharOffset>>
where
    T: TextBuffer + ?Sized,
    C: Into<CharOffset>,
{
    let offset = offset.into();
    // If the buffer is empty, return early.
    let c = buffer.chars_at(offset).ok()?.next()?;

    // Check if the cursor is already on a bracket of the type we're looking for.
    match BracketChar::try_from(c) {
        // If it is, just call [`vim_find_matching_bracket`] for the other bracket.
        Ok(bracket) if bracket.kind == bracket_type => {
            let other_offset = vim_find_matching_bracket(buffer, &bracket, offset)?;
            // We may have just moved backwards or forwards. Make sure the smaller offset comes
            // first in the range bounds.
            let (start_offset, end_offset) = if other_offset > offset {
                (offset, other_offset)
            } else {
                (other_offset, offset)
            };
            Some(start_offset..end_offset + 1)
        }
        // If not, perform the search in both directions.
        _ => {
            let end_offset = vim_find_matching_bracket(
                buffer,
                &BracketChar {
                    end: BracketEnd::Opening,
                    kind: bracket_type,
                },
                offset,
            )?;
            let start_offset = vim_find_matching_bracket(
                buffer,
                &BracketChar {
                    end: BracketEnd::Closing,
                    kind: bracket_type,
                },
                end_offset,
            )?;
            Some(start_offset..end_offset + 1)
        }
    }
}

#[cfg(test)]
#[path = "block_tests.rs"]
mod tests;
