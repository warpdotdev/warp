use itertools::Itertools;
use string_offset::CharOffset;
use warpui_core::text::TextBuffer;

use crate::vim::{BracketChar, BracketEnd};

/// This method looks for the bracket that complements/pairs with the passed in BracketChar,
/// starting the search at the passed in offset.
pub fn vim_find_matching_bracket<'a, T, C>(
    buffer: &'a T,
    bracket_char: &BracketChar,
    offset: C,
) -> Option<CharOffset>
where
    T: TextBuffer + ?Sized + 'a,
    C: Into<CharOffset>,
{
    let offset = offset.into();

    // If we're matching for an opening bracket, search forward, and vice versa.
    let mut iter: Box<dyn Iterator<Item = char>> = match bracket_char.end {
        BracketEnd::Opening => Box::new(buffer.chars_at(offset + 1).ok()?),
        BracketEnd::Closing => Box::new(buffer.chars_rev_at(offset).ok()?),
    };

    // If we encounter more instances of the same bracket, i.e. we're matching "(" and we pass by
    // another "(", we must first match each additional "(" we encounter. Keep a count of those in
    // this "depth" variable.
    let mut depth: u32 = 0;
    let (i, _) = iter.find_position(|c| {
        if bracket_char.is_char(*c) {
            depth += 1;
        } else if bracket_char.complements(*c) {
            if depth == 0 {
                return true;
            } else {
                depth -= 1;
            }
        }
        false
    })?;
    match bracket_char.end {
        BracketEnd::Opening => Some(offset + i + 1),
        BracketEnd::Closing => Some(offset - i - 1),
    }
}

#[cfg(test)]
#[path = "matching_brackets_tests.rs"]
mod tests;
