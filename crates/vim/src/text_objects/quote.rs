use std::ops::Range;

use itertools::Itertools;
use string_offset::CharOffset;
use warpui::text::TextBuffer;

use crate::vim::QuoteType;

/// Vim's "inner quote" text object, e.g. `di"`. This includes characters enclosed by quotes.
/// See https://vimdoc.sourceforge.net/htmldoc/motion.html#iquote
/// or enter ":help iquote" in Vim.
///
/// Returns None when the line is empty, doesn't contain a pair of the quotes we're looking for, or
/// the offset is out of bounds.
pub fn vim_inner_quote<T, C>(
    buffer: &T,
    offset: C,
    quote_type: QuoteType,
) -> Option<Range<CharOffset>>
where
    T: TextBuffer + ?Sized,
    C: Into<CharOffset>,
{
    vim_a_quote(buffer, offset, quote_type).map(|range| (range.start + 1)..(range.end - 1))
}

/// Vim's "a quote" text object, e.g. `da"`. This includes characters enclosed by quotes along with
/// the quotes themselves.
/// See https://vimdoc.sourceforge.net/htmldoc/motion.html#aquote
/// or enter ":help aquote" in Vim.
///
/// Returns None when the line is empty, doesn't contain a pair of the quotes we're looking for, or
/// the offset is out of bounds.
pub fn vim_a_quote<T, C>(buffer: &T, offset: C, quote_type: QuoteType) -> Option<Range<CharOffset>>
where
    T: TextBuffer + ?Sized,
    C: Into<CharOffset>,
{
    let offset = offset.into();
    let mut forward_iter = buffer
        .chars_at(offset)
        .ok()?
        .take_while(|c| *c != '\n') // cannot traverse newline boundaries
        .enumerate()
        .peekable();
    let backward_iter = buffer
        .chars_rev_at(offset)
        .ok()?
        .take_while(|c| *c != '\n')
        .enumerate()
        .peekable();

    // This will be None if the line is empty
    let (_, c) = forward_iter.next()?;

    // First, check if the cursor is currently on top of a quote.
    if !quote_type.is_char(c) {
        // If not, see if there are quotes surrounding the cursor, one before one after.
        let mut quote_behind_found_at = None;
        for (i, c) in backward_iter {
            if quote_type.is_char(c) {
                quote_behind_found_at = Some(offset - i - 1);
                break;
            }
        }
        let mut quote_ahead_found_at = None;
        for (i, c) in &mut forward_iter {
            if quote_type.is_char(c) {
                quote_ahead_found_at = Some(offset + i);
                break;
            }
        }

        match (quote_behind_found_at, quote_ahead_found_at) {
            // If there are quotes surrounding the cursor, take that range.
            (Some(start), Some(end)) => Some(start..end + 1),
            // If there is no quote before, but there _is_ one after, treat the one after as the
            // opening quote and look for a closing quote after that.
            (None, Some(start)) => {
                for (i, c) in forward_iter {
                    if quote_type.is_char(c) {
                        let end = offset + i + 1;
                        return Some(start..end);
                    }
                }
                None
            }
            // Otherwise, no valid range. Vim doesn't attempt to look for a pair of quotes behind.
            (_, None) => None,
        }
    } else {
        // If the cursor is on a quote, we need to count all quotes on this line to figure out if
        // this should be treated as an opening or closing quote.
        let quotes_behind = backward_iter
            .filter(|(_, c)| quote_type.is_char(*c))
            .collect_vec();
        // If there is an odd number of quotes before this one, then treat this one as closing.
        if quotes_behind.len() % 2 == 1 {
            let (i, _) = quotes_behind[0];
            let start = offset - i - 1;
            let end = offset + 1;
            Some(start..end)
        } else {
            // If there is an even number of quotes before this one, treat this one as opening.
            for (i, c) in forward_iter {
                if quote_type.is_char(c) {
                    let end = offset + i + 1;
                    return Some(offset..end);
                }
            }
            None
        }
    }
}

#[cfg(test)]
#[path = "quote_tests.rs"]
mod tests;
