use std::iter::Peekable;

use anyhow::Result;
use itertools::{peek_nth, Either, PeekNth};
use string_offset::CharOffset;
use warpui_core::text::words::is_default_word_boundary;
use warpui_core::text::TextBuffer;

use crate::vim::{Direction, WordBound, WordType};

/// The "kind" of character that a char is. "Symbols" here refer to non-whitespace characters that
/// are traditionally word-breaking characters, e.g. punctuation and brackets. Vim treats contiguous
/// symbols as their own "words" and hence they need to be tracked as a distinct context from
/// word-characters and whitespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CharacterKind {
    WordChars,
    Symbols,
    Whitespace,
}

impl CharacterKind {
    /// This is an alternative to `==` which will treat Self::WordChars and Self::Symbols as equal
    /// to one another for WordType::BigWord. This is useful for motions or text objects
    /// involving bigwords.
    pub(super) fn equivalent_char_kind(&self, other: &Self, word_type: WordType) -> bool {
        match word_type {
            WordType::Default => self == other,
            WordType::BigWord => {
                self == other
                    || (*self == Self::WordChars && *other == Self::Symbols
                        || *self == Self::Symbols && *other == Self::WordChars)
            }
        }
    }
}

impl From<char> for CharacterKind {
    fn from(c: char) -> Self {
        if c.is_whitespace() {
            CharacterKind::Whitespace
        } else if is_default_word_boundary(c) {
            CharacterKind::Symbols
        } else {
            CharacterKind::WordChars
        }
    }
}

/// This function abstracts over the different specific types of word iterators. This is meant to
/// be used by Views rather than any of these iterator structs.
pub fn vim_word_iterator_from_offset<'a, T, C>(
    offset: C,
    buffer: &'a T,
    direction: Direction,
    bound: WordBound,
    word_type: WordType,
) -> Result<Box<dyn Iterator<Item = CharOffset> + 'a>>
where
    T: TextBuffer + ?Sized + 'a,
    C: Into<CharOffset>,
{
    let offset = offset.into();
    match (direction, bound) {
        (Direction::Forward, WordBound::Start) | (Direction::Backward, WordBound::End) => Ok(
            Box::new(WordHeadsVim::new(offset, buffer, direction, word_type)?),
        ),
        (Direction::Forward, WordBound::End) | (Direction::Backward, WordBound::Start) => Ok(
            Box::new(WordTailsVim::new(offset, buffer, direction, word_type)?),
        ),
    }
}

/// This struct represents the logic for pressing either w, W, ge, or gE in Vim.
pub struct WordHeadsVim<'a, T: TextBuffer + ?Sized + 'a> {
    offset: CharOffset,
    chars: Peekable<Either<T::Chars<'a>, T::CharsReverse<'a>>>,
    cursor_context: CharacterKind,
    direction: Direction,
    word_type: WordType,
    done: bool,
}

impl<'a, T: TextBuffer + ?Sized + 'a> WordHeadsVim<'a, T> {
    pub fn new(
        offset: CharOffset,
        buffer: &'a T,
        direction: Direction,
        word_type: WordType,
    ) -> Result<Self> {
        let (offset, mut chars) = match direction {
            // TextBuffer::chars_rev_at does not include the character _at_ the originating offset.
            // However, Vim's word motion semantics require that we see the character at the
            // originating offset, so we need to +1 the offset before creating the reverse char
            // iterator.
            Direction::Backward => {
                let offset = offset + 1;
                (
                    offset,
                    Either::Right(buffer.chars_rev_at(offset)?).peekable(),
                )
            }
            Direction::Forward => (offset, Either::Left(buffer.chars_at(offset)?).peekable()),
        };
        Ok(Self {
            offset,
            cursor_context: chars
                .peek()
                .map_or(CharacterKind::WordChars, |c| (*c).into()),
            chars,
            direction,
            word_type,
            done: false,
        })
    }

    fn step(&mut self) {
        self.chars.next();
        match self.direction {
            Direction::Backward => self.offset -= 1,
            Direction::Forward => self.offset += 1,
        }
    }
}

impl<'a, T: TextBuffer + ?Sized + 'a> Iterator for WordHeadsVim<'a, T> {
    type Item = CharOffset;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            self.step();
            let Some(&c) = self.chars.peek() else {
                break;
            };
            let prev_cursor_context = self.cursor_context;
            self.cursor_context = CharacterKind::from(c);

            if !self
                .cursor_context
                .equivalent_char_kind(&prev_cursor_context, self.word_type)
                && self.cursor_context != CharacterKind::Whitespace
            {
                return Some(match self.direction {
                    Direction::Backward => self.offset - 1,
                    Direction::Forward => self.offset,
                });
            }
        }

        self.done = true;
        Some(self.offset)
    }
}

/// This struct represents the logic for pressing either b, B, e, or E in Vim.
pub struct WordTailsVim<'a, T: TextBuffer + ?Sized + 'a> {
    offset: CharOffset,
    chars: PeekNth<Either<T::Chars<'a>, T::CharsReverse<'a>>>,
    direction: Direction,
    word_type: WordType,
    done: bool,
}

impl<'a, T: TextBuffer + ?Sized + 'a> WordTailsVim<'a, T> {
    pub fn new(
        offset: CharOffset,
        buffer: &'a T,
        direction: Direction,
        word_type: WordType,
    ) -> Result<Self> {
        let (offset, chars) = match direction {
            // TextBuffer::chars_rev_at does not include the character _at_ the originating offset.
            // However, Vim's word motion semantics require that we see the character at the
            // originating offset, so we need to +1 the offset before creating the reverse char
            // iterator.
            Direction::Backward => {
                let offset = offset + 1;
                (
                    offset,
                    peek_nth(Either::Right(buffer.chars_rev_at(offset)?)),
                )
            }
            Direction::Forward => (offset, peek_nth(Either::Left(buffer.chars_at(offset)?))),
        };
        Ok(Self {
            offset,
            chars,
            direction,
            word_type,
            done: false,
        })
    }

    fn step(&mut self) {
        self.chars.next();
        match self.direction {
            Direction::Backward => self.offset -= 1,
            Direction::Forward => self.offset += 1,
        }
    }
}

impl<'a, T: TextBuffer + ?Sized + 'a> Iterator for WordTailsVim<'a, T> {
    type Item = CharOffset;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            if self.chars.peek_nth(1).is_none() {
                break;
            }
            self.step();

            let Some(&c) = self.chars.peek() else {
                break;
            };
            let Some(&c_next) = self.chars.peek_nth(1) else {
                break;
            };

            let cursor_context = CharacterKind::from(c);
            let cursor_context_next = CharacterKind::from(c_next);

            if !cursor_context.equivalent_char_kind(&cursor_context_next, self.word_type)
                && cursor_context != CharacterKind::Whitespace
            {
                return Some(match self.direction {
                    Direction::Backward => self.offset - 1,
                    Direction::Forward => self.offset,
                });
            }
        }

        self.done = true;
        Some(match self.direction {
            Direction::Backward => self.offset - 1,
            Direction::Forward => self.offset,
        })
    }
}

#[cfg(test)]
#[path = "word_iterator_tests.rs"]
mod tests;
