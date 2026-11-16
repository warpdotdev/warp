use std::ops::Range;

use string_offset::CharOffset;
use warpui_core::text::TextBuffer;

/// Neovim's "inner line" text object, excluding leading and trailing spaces and tabs.
pub fn vim_inner_line<T, C>(buffer: &T, offset: C) -> Option<Range<CharOffset>>
where
    T: TextBuffer + ?Sized,
    C: Into<CharOffset>,
{
    let offset = offset.into();
    let line_start = offset
        - buffer
            .chars_rev_at(offset)
            .ok()?
            .take_while(|c| *c != '\n')
            .count();
    let line_end = offset
        + buffer
            .chars_at(offset)
            .ok()?
            .take_while(|c| *c != '\n')
            .count();

    let leading_whitespace = buffer
        .chars_at(line_start)
        .ok()?
        .take_while(|c| matches!(c, ' ' | '\t'))
        .count();
    let trailing_whitespace = buffer
        .chars_rev_at(line_end)
        .ok()?
        .take_while(|c| matches!(c, ' ' | '\t'))
        .count();
    let start = line_start + leading_whitespace;
    let end = line_end - trailing_whitespace;

    (start < end).then_some(start..end)
}

/// Neovim's "all lines" text object, covering the entire buffer.
pub fn vim_all_lines<T>(buffer: &T) -> Option<Range<CharOffset>>
where
    T: TextBuffer + ?Sized,
{
    let end = CharOffset::from(buffer.chars_at(CharOffset::default()).ok()?.count());
    Some(CharOffset::default()..end)
}

#[cfg(test)]
#[path = "line_tests.rs"]
mod tests;
