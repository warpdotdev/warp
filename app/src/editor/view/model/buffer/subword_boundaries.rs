use super::{CharOffset, Point};
use itertools::Either;
use std::iter::Peekable;
use warpui::text::{
    word_boundaries::WordBoundariesApproach, words::is_subword_boundary_char, TextBuffer,
};

pub struct SubwordBoundaries<'a, T: TextBuffer + ?Sized> {
    offset: CharOffset,
    chars: Peekable<Either<T::Chars<'a>, T::CharsReverse<'a>>>,
    char_window: CharWindow,
    buffer: &'a T,
    approach: WordBoundariesApproach,
    in_word: bool,
    done: bool,
}

impl<'a, T: TextBuffer + ?Sized> SubwordBoundaries<'a, T> {
    /// Returns an iterator for the beginnings of the subwords in the buffer.
    pub fn forward_subword_starts(offset: CharOffset, chars: T::Chars<'a>, buffer: &'a T) -> Self {
        let mut peekable_chars = Either::Left(chars).peekable();
        let first = peekable_chars.next();
        let second = peekable_chars.next();
        let third = peekable_chars.next();
        Self {
            offset,
            buffer,
            chars: peekable_chars,
            char_window: CharWindow::new(first, second, third),
            in_word: true,
            approach: WordBoundariesApproach::ForwardWordStarts,
            done: false,
        }
    }

    /// Returns an iterator for the ends of the subwords in the buffer.
    pub fn forward_subword_ends_exclusive(
        offset: CharOffset,
        chars: T::Chars<'a>,
        buffer: &'a T,
    ) -> Self {
        let mut peekable_chars = Either::Left(chars).peekable();
        let first = peekable_chars.next();
        let second = peekable_chars.next();
        let third = peekable_chars.next();
        Self {
            offset,
            buffer,
            chars: peekable_chars,
            char_window: CharWindow::new(first, second, third),
            in_word: false,
            approach: WordBoundariesApproach::ForwardWordEnds,
            done: false,
        }
    }

    /// Returns a backwards iterator for the beginning of subwords in the buffer.
    pub fn backward_subword_starts_exclusive(
        offset: CharOffset,
        chars: T::CharsReverse<'a>,
        buffer: &'a T,
    ) -> Self {
        let mut peekable_chars = Either::Right(chars).peekable();
        let first = peekable_chars.next();
        let second = peekable_chars.next();
        let third = peekable_chars.next();
        Self {
            offset,
            buffer,
            chars: peekable_chars,
            char_window: CharWindow::new(first, second, third),
            in_word: false,
            approach: WordBoundariesApproach::BackwardWordStarts,
            done: false,
        }
    }

    /// Move to the next character and update the offset.
    fn step(&mut self) {
        let new_char = self.chars.next();
        self.char_window.forward(new_char.to_owned());

        match self.approach {
            WordBoundariesApproach::ForwardWordStarts | WordBoundariesApproach::ForwardWordEnds => {
                self.offset += 1;
            }
            WordBoundariesApproach::BackwardWordStarts => {
                self.offset -= 1;
            }
        }
    }

    /// Helper for the `Iterator::next()` implementation.
    /// Moves forward through the string and returns the start of the next
    /// subword.
    fn next_forward_starts(&mut self) -> Option<<Self as Iterator>::Item> {
        while let Some(c) = self.char_window.first() {
            if self.in_word {
                // We're in a word, but are we in a subword?
                if is_subword_boundary_char(c) {
                    // c isn't part of a subword.
                    self.in_word = false;
                    self.step();
                } else if c.is_uppercase() {
                    if let Some(c_next) = self.char_window.second() {
                        // If c is upper and c_next is lower, then
                        // c marks the start of a Capitalized word.
                        if c_next.is_lowercase() {
                            let point = self.buffer.to_point(self.offset).ok();
                            self.step(); // to avoid getting stuck
                            return point;
                        } else if c_next.is_uppercase() {
                            // peek once more, to see if c is the "X" in "XYz"
                            if let Some(_c_next_next) = self.char_window.third() {
                                self.step();
                            } else {
                                // we've reached the last character in the string,
                                // and since it's uppercase it should be treated
                                // as the start of a one-letter word.
                                self.step();
                                return self.buffer.to_point(self.offset).ok();
                            }
                        } else {
                            self.step();
                        }
                    } else {
                        // c_next wasn't available.
                        self.step();
                    }
                } else if c.is_lowercase() {
                    self.step();

                    if let Some(c_next) = self.char_window.first() {
                        if c_next.is_uppercase() {
                            let point = self.buffer.to_point(self.offset).ok();
                            self.step();
                            return point;
                        }
                    }
                } else {
                    self.step();
                }

            // Still haven't entered a word.
            } else if is_subword_boundary_char(c) {
                self.step();

            // Just entered a word.
            } else {
                self.in_word = true;
                let point = self.buffer.to_point(self.offset).ok();
                self.step();
                return point;
            }
        }
        None
    }

    /// Helper for the `Iterator::next()` implementation.
    /// Moves forward through the string and returns the end of the next
    /// subword.
    fn next_forward_ends(&mut self) -> Option<<Self as Iterator>::Item> {
        while let Some(c) = self.char_window.first() {
            if self.in_word && is_subword_boundary_char(c) {
                // We are in a word, but the next character is _not_ in a word,
                // so we have found the boundary.
                self.in_word = false;
                return self.buffer.to_point(self.offset).ok();
            }

            if !self.in_word && !is_subword_boundary_char(c) {
                self.in_word = true;
            }

            if self.in_word {
                if let Some(c_next) = self.char_window.second() {
                    if c.is_lowercase() && c_next.is_uppercase() {
                        // c is the end of a word, and c_next begins the next word.
                        self.step();
                        return self.buffer.to_point(self.offset).ok();
                    } else if c.is_uppercase() && c_next.is_uppercase() {
                        // c_next could be part of an all-caps word
                        // or the start of a new Capitalized word
                        if let Some(c_next_next) = self.char_window.third() {
                            if c_next_next.is_lowercase() {
                                self.step();
                                return self.buffer.to_point(self.offset).ok();
                            }
                        }
                    }
                }
            }

            self.step();
        }
        None
    }

    /// Helper for the `Iterator::next()` implementation.
    /// Moves backward through the string and returns the start of the closest
    /// subword.
    fn next_backward_starts(&mut self) -> Option<<Self as Iterator>::Item> {
        while let Some(c) = self.char_window.first() {
            if self.in_word && is_subword_boundary_char(c) {
                self.in_word = false;
                let point = self.buffer.to_point(self.offset).ok();
                self.step();
                return point;
            }

            if !self.in_word && !is_subword_boundary_char(c) {
                self.in_word = true;
            }

            if self.in_word {
                if let Some(c_next) = self.char_window.second() {
                    if c.is_lowercase() && c_next.is_uppercase() {
                        // c_next is the start of the c's subword.
                        self.step();
                        self.step();
                        let point = self.buffer.to_point(self.offset).ok();
                        self.step(); // to avoid re-designating this character as a start
                        return point;
                    } else if c.is_uppercase() && c_next.is_lowercase() {
                        // c is the start Capitalized or Uppercase subword,
                        // and c_next is the end of a Capitalized or lowercase
                        // subword to the right of it.
                        self.step();
                        let point = self.buffer.to_point(self.offset).ok();
                        return point;
                    }
                }
            }

            self.step();
        }
        None
    }
}

impl<T: TextBuffer + ?Sized> Iterator for SubwordBoundaries<'_, T> {
    type Item = Point;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(point) = match self.approach {
            WordBoundariesApproach::ForwardWordStarts => self.next_forward_starts(),
            WordBoundariesApproach::ForwardWordEnds => self.next_forward_ends(),
            WordBoundariesApproach::BackwardWordStarts => self.next_backward_starts(),
        } {
            return Some(point);
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

/// Storage for characters from the buffer, used by the `SubwordBoundaries`
/// iterator to find the start and end of subwords.
struct CharWindow {
    /// A store of characters retrieved from the `chars` iterator.
    ///
    /// `char_window[0]`: character at the current offset.
    ///
    /// `char_window[1]`: character after the current offset.
    ///
    /// `char_window[2]`: two characters after the current offset.
    window: Vec<Option<char>>,
}

impl CharWindow {
    fn new(first: Option<char>, second: Option<char>, third: Option<char>) -> Self {
        Self {
            window: vec![first, second, third],
        }
    }

    fn forward(&mut self, new_char: Option<char>) {
        self.window.remove(0);
        self.window.push(new_char);
    }

    fn first(&self) -> Option<char> {
        self.window.first().unwrap_or(&None).to_owned()
    }

    fn second(&self) -> Option<char> {
        self.window.get(1).unwrap_or(&None).to_owned()
    }

    fn third(&self) -> Option<char> {
        self.window.get(2).unwrap_or(&None).to_owned()
    }
}

#[cfg(test)]
#[path = "subword_boundaries_tests.rs"]
mod tests;
