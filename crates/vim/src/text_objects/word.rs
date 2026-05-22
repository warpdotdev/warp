use std::ops::Range;

use string_offset::CharOffset;
use warpui::text::TextBuffer;

use crate::vim::WordType;
use crate::word_iterator::CharacterKind;

/// Vim's "inner word" text object, e.g. `diw`. This includes the series of either word chars,
/// symbols, or whitespace around the cursor.
/// See https://vimdoc.sourceforge.net/htmldoc/motion.html#iw
/// or enter ":help iw" in Vim.
///
/// Returns None when the buffer is empty or the offset is out of bounds.
pub fn vim_inner_word<T, C>(buffer: &T, offset: C, word_type: WordType) -> Option<Range<CharOffset>>
where
    T: TextBuffer + ?Sized,
    C: Into<CharOffset>,
{
    let offset = offset.into();
    let mut forward_iter = buffer.chars_at(offset).ok()?.peekable();
    // Empty buffer will be None here.
    let cursor_context = CharacterKind::from(*forward_iter.peek()?);

    let mut word_start = offset;
    let backward_iter = buffer.chars_rev_at(offset).ok()?;
    for c in backward_iter {
        if !CharacterKind::from(c).equivalent_char_kind(&cursor_context, word_type) {
            break;
        }
        word_start -= 1;
    }

    let mut word_end = offset;
    for c in forward_iter {
        if !CharacterKind::from(c).equivalent_char_kind(&cursor_context, word_type) {
            break;
        }
        word_end += 1;
    }

    Some(word_start..word_end)
}

/// Vim's "a word" text object, e.g. `daw`. This includes the series of either word chars or
/// symbols around the cursor as well as the section of whitespace after it (or if there is no
/// whitespace after it, then before it).
/// See https://vimdoc.sourceforge.net/htmldoc/motion.html#aw
/// or enter ":help aw" in Vim.
///
/// Returns None when the buffer is empty or the offset is out of bounds.
pub fn vim_a_word<T, C>(buffer: &T, offset: C, word_type: WordType) -> Option<Range<CharOffset>>
where
    T: TextBuffer + ?Sized,
    C: Into<CharOffset>,
{
    let offset = offset.into();
    let mut forward_iter = buffer.chars_at(offset).ok()?.peekable();
    // Empty buffer will be None here.
    let cursor_context = CharacterKind::from(*forward_iter.peek()?);

    let mut word_start = offset;
    let mut word_end = offset;
    let mut backward_iter = buffer.chars_rev_at(offset).ok()?.peekable();

    // Go back to the beginning of this context.
    while let Some(&c) = backward_iter.peek() {
        if !CharacterKind::from(c).equivalent_char_kind(&cursor_context, word_type) {
            break;
        }
        word_start -= 1;
        backward_iter.next();
    }

    // Proceed to the end of this context.
    while let Some(&c) = forward_iter.peek() {
        if !CharacterKind::from(c).equivalent_char_kind(&cursor_context, word_type) {
            break;
        }
        word_end += 1;
        forward_iter.next();
    }

    // If the cursor is in whitespace it needs to get the non-whitespace word ahead.
    if cursor_context == CharacterKind::Whitespace {
        // Check if there as non-whitespace after this.
        let Some(&c) = forward_iter.peek() else {
            // If this is the end of the buffer, we're done.
            return Some(word_start..word_end);
        };
        let next_cursor_context = CharacterKind::from(c);

        // Proceed to the end of this non-whitespace word.
        for c in forward_iter {
            if !CharacterKind::from(c).equivalent_char_kind(&next_cursor_context, word_type) {
                break;
            }
            word_end += 1;
        }
        return Some(word_start..word_end);
    }

    // If we've made it this far, then the cursor did not start off in whitespace. This means we
    // need to include some whitespace around this word. If there is whitespace ahead, include
    // that. Otherwise, if there is whitespace behind, include that.

    // If there is no content in this buffer ahead of this context.
    let Some(&c) = forward_iter.peek() else {
        // Check if there is any content behind.
        let Some(&c) = backward_iter.peek() else {
            return Some(word_start..word_end);
        };
        // Go backwards to include the whitespace.
        if CharacterKind::from(c) == CharacterKind::Whitespace {
            for c in backward_iter {
                if CharacterKind::from(c) != CharacterKind::Whitespace {
                    break;
                }
                word_start -= 1;
            }
        }
        return Some(word_start..word_end);
    };

    // If we made it this far, there is content ahead of this context in the buffer.
    if CharacterKind::from(c) == CharacterKind::Whitespace {
        // If the content ahead as whitespace, include that.
        for c in forward_iter {
            if CharacterKind::from(c) != CharacterKind::Whitespace {
                break;
            }
            word_end += 1;
        }
    } else {
        // If the content ahead is not whitespace, try to include any whitespace behind.
        let Some(&c) = backward_iter.peek() else {
            return Some(word_start..word_end);
        };
        if CharacterKind::from(c) == CharacterKind::Whitespace {
            for c in backward_iter {
                if CharacterKind::from(c) != CharacterKind::Whitespace {
                    break;
                }
                word_start -= 1;
            }
        }
    }
    Some(word_start..word_end)
}

#[cfg(test)]
#[path = "word_tests.rs"]
mod tests;
