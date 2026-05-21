use std::borrow::Cow;
use std::collections::HashSet;
use std::iter::Peekable;

use itertools::Either;
use string_offset::CharOffset;

use super::point::Point;
use super::words::is_default_word_boundary;
use super::TextBuffer;

/// This enum configures how the WordBoundaries iterator defines a "word"
#[derive(Clone, Debug)]
pub enum WordBoundariesPolicy {
    /// Break words on spaces and the characters specified in words::is_default_word_boundary
    Default,
    /// Break words on spaces plus a specific set of provided characters
    Custom(HashSet<char>),
    /// Break words only on ASCII whitespace
    OnlyWhitespace,
}

#[derive(Clone, Copy)]
pub enum WordBoundariesApproach {
    ForwardWordStarts,
    ForwardWordEnds,
    BackwardWordStarts,
}

/// Iterator that returns the edges of words from a given offset, based on the selected approach
pub struct WordBoundaries<'a, T: TextBuffer + ?Sized> {
    offset: CharOffset,
    chars: Peekable<Either<T::Chars<'a>, T::CharsReverse<'a>>>,
    buffer: &'a T,
    in_word: bool,
    approach: WordBoundariesApproach,
    policy: Cow<'a, WordBoundariesPolicy>,
    done: bool,
}

impl<'a, T: TextBuffer + ?Sized> WordBoundaries<'a, T> {
    pub fn with_policy(mut self, policy: impl Into<Cow<'a, WordBoundariesPolicy>>) -> Self {
        self.policy = policy.into();
        self
    }

    /// Create an iterator that will return the starts of words moving forwards
    pub fn forward_starts(offset: CharOffset, chars: T::Chars<'a>, buffer: &'a T) -> Self {
        Self {
            offset,
            buffer,
            chars: Either::Left(chars).peekable(),
            in_word: true,
            approach: WordBoundariesApproach::ForwardWordStarts,
            policy: Cow::Owned(WordBoundariesPolicy::Default),
            done: false,
        }
    }

    /// Create an iterator that will return the ends of words moving forwards, exclusive of the
    /// offset position.
    ///
    /// Example: For a buffer of "word one two three", with an offset of `4` (immediately after
    /// the 'word'), this will yield columns [8, 12, 18], the ends of `one`, `two`, and `three`,
    /// but _excluding_ the initial position at the end of `word`.
    pub fn forward_ends_exclusive(offset: CharOffset, chars: T::Chars<'a>, buffer: &'a T) -> Self {
        Self {
            offset,
            buffer,
            chars: Either::Left(chars).peekable(),
            in_word: false,
            approach: WordBoundariesApproach::ForwardWordEnds,
            policy: Cow::Owned(WordBoundariesPolicy::Default),
            done: false,
        }
    }

    /// Create an iterator that will return the ends of words moving forwards, inclusive of the
    /// offset position.
    ///
    /// Example: For a buffer of "word one two three", with an offset of `4` (immediately after
    /// the 'word'), this will yield columns [4, 8, 12, 18], the ends of all four words,
    /// _including_ the initial position at the end of `word`.
    pub fn forward_ends_inclusive(offset: CharOffset, chars: T::Chars<'a>, buffer: &'a T) -> Self {
        Self {
            offset,
            buffer,
            chars: Either::Left(chars).peekable(),
            in_word: true,
            approach: WordBoundariesApproach::ForwardWordEnds,
            policy: Cow::Owned(WordBoundariesPolicy::Default),
            done: false,
        }
    }

    /// Create an iterator that will return the starts of words moving _backwards_, exclusive of
    /// the offset position
    ///
    /// Example: For a buffer of "word one two three", with an offset of `13` (immediately before
    /// the 'three'), this will yield columns [9, 5, 0], the starts of `two`, `one`, and `word`,
    /// but _excluding_ the initial position at the start of `three`.
    pub fn backward_starts_exclusive(
        offset: CharOffset,
        chars: T::CharsReverse<'a>,
        buffer: &'a T,
    ) -> Self {
        Self {
            offset,
            buffer,
            chars: Either::Right(chars).peekable(),
            in_word: false,
            approach: WordBoundariesApproach::BackwardWordStarts,
            policy: Cow::Owned(WordBoundariesPolicy::Default),
            done: false,
        }
    }

    /// Create an iterator that will return the starts of words moving _backwards_, inclusive of
    /// the offset position
    ///
    /// Example: For a buffer of "word one two three", with an offset of `13` (immediately before
    /// the 'three'), this will yield columns [13, 9, 5, 0], the starts of all four words,
    /// _including_ the initial position at the start of `three`.
    pub fn backward_starts_inclusive(
        offset: CharOffset,
        chars: T::CharsReverse<'a>,
        buffer: &'a T,
    ) -> Self {
        Self {
            offset,
            buffer,
            chars: Either::Right(chars).peekable(),
            in_word: true,
            approach: WordBoundariesApproach::BackwardWordStarts,
            policy: Cow::Owned(WordBoundariesPolicy::Default),
            done: false,
        }
    }

    fn step(&mut self) {
        self.chars.next();
        match self.approach {
            WordBoundariesApproach::ForwardWordStarts | WordBoundariesApproach::ForwardWordEnds => {
                self.offset += 1;
            }
            WordBoundariesApproach::BackwardWordStarts => {
                self.offset -= 1;
            }
        }
    }

    fn is_word_boundary(&self, c: char) -> bool {
        match self.policy.as_ref() {
            WordBoundariesPolicy::Default => is_default_word_boundary(c),
            WordBoundariesPolicy::Custom(boundary_chars) => {
                c.is_whitespace() || boundary_chars.contains(&c)
            }
            WordBoundariesPolicy::OnlyWhitespace => c.is_whitespace(),
        }
    }
}

impl<T: TextBuffer + ?Sized> Iterator for WordBoundaries<'_, T> {
    type Item = Point;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(&c) = self.chars.peek() {
            match self.approach {
                // For forward word starts, we look for the transition from not in a word (i.e. in
                // a separator) to in a word. That boundary is the start of a new word
                WordBoundariesApproach::ForwardWordStarts => {
                    if self.in_word {
                        self.step();

                        if self.is_word_boundary(c) {
                            self.in_word = false;
                        }
                    } else if self.is_word_boundary(c) {
                        self.step();
                    } else {
                        // We are not in a word, but the next character _is_ in a word, so
                        // we've found the start of the next word. We mark ourselves as being
                        // in a word (for the next iteration), then return the point.
                        self.in_word = true;
                        return self.buffer.to_point(self.offset).ok();
                    }
                }
                // For forward word ends, we look for the transition from in a word to not in a
                // word. That boundary is the end of the current word. We also look for the same
                // boundary for backward starts, since going backwards the transition from in a
                // word to not in a word represents the _beginning_ of the current word
                WordBoundariesApproach::ForwardWordEnds
                | WordBoundariesApproach::BackwardWordStarts => {
                    if self.in_word {
                        if self.is_word_boundary(c) {
                            // We are in a word, but the next character is _not_ in a word, so we
                            // have found the boundary. We mark ourselves as not being in a word,
                            // then return the point.
                            self.in_word = false;
                            return self.buffer.to_point(self.offset).ok();
                        } else {
                            self.step();
                        }
                    } else {
                        self.step();

                        if !self.is_word_boundary(c) {
                            self.in_word = true;
                        }
                    }
                }
            }
        }

        // We have consumed all of the characters in the given direction. However, we should also
        // treat the end (or beginning if backward) of the buffer as a word boundary. We only want
        // to return that once, however, so we mark ourselves as done afterwards.
        if self.done {
            None
        } else {
            self.done = true;

            self.buffer.to_point(self.offset).ok()
        }
    }
}

impl From<WordBoundariesPolicy> for Cow<'_, WordBoundariesPolicy> {
    fn from(policy: WordBoundariesPolicy) -> Self {
        Cow::Owned(policy)
    }
}

impl<'a> From<&'a WordBoundariesPolicy> for Cow<'a, WordBoundariesPolicy> {
    fn from(policy: &'a WordBoundariesPolicy) -> Self {
        Cow::Borrowed(policy)
    }
}

#[cfg(test)]
#[path = "word_boundaries_tests.rs"]
mod tests;
