use string_offset::CharOffset;
use warpui_core::text::TextBuffer;

/// Returns the offset of the first newline above a paragraph start before the current position.
pub fn find_previous_paragraph_start<'a, T, C>(buffer: &'a T, offset: C) -> Option<CharOffset>
where
    T: TextBuffer + ?Sized + 'a,
    C: Into<CharOffset>,
{
    let offset = offset.into();

    // Skip newlines between the current position and prior paragraph end
    let iter = buffer
        .chars_rev_at(offset + 1) // chars_rev_at doesn't include the current offset
        .ok()?
        .enumerate()
        .skip_while(|(_, c)| *c == '\n');

    // Scan from current position backward until we find two newlines in a row
    let mut prev_was_newline = false;
    for (curr, c) in iter {
        if c == '\n' {
            if prev_was_newline {
                return Some(offset + 1 - curr); // + 1 because the enumerate is shifted above
            }
            prev_was_newline = true;
        } else {
            prev_was_newline = false;
        }
    }

    None
}
/// Returns the offset of the first newline below a paragraph end after the current position.
pub fn find_next_paragraph_end<'a, T, C>(buffer: &'a T, offset: C) -> Option<CharOffset>
where
    T: TextBuffer + ?Sized + 'a,
    C: Into<CharOffset>,
{
    let offset = offset.into();

    // Skip newlines between the current position and next paragraph start
    let iter = buffer
        .chars_at(offset)
        .ok()?
        .enumerate()
        .skip_while(|(_, c)| *c == '\n');

    // Scan from current position forward until we find two newlines in a row
    let mut prev_was_newline = false;
    for (curr, c) in iter {
        if c == '\n' {
            if prev_was_newline {
                return Some(offset + curr);
            }
            prev_was_newline = true;
        } else {
            prev_was_newline = false;
        }
    }

    None
}
#[cfg(test)]
#[path = "paragraph_iterator_tests.rs"]
mod tests;
