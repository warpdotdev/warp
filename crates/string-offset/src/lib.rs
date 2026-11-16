//! Base types for representing offset-based text locations.

use std::ops::{Add, AddAssign, Range, Sub, SubAssign};

use get_size::GetSize;
use serde::{Deserialize, Serialize};

/// An offset within a piece of text, in terms of characters (for Rust's definition
/// of `char`).
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    GetSize,
)]
pub struct CharOffset(usize);

/// An offset within a piece of text, in terms of bytes.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Hash,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    GetSize,
    Serialize,
    Deserialize,
)]
pub struct ByteOffset(usize);

#[macro_export]
macro_rules! impl_offset {
    ($typ:ty) => {
        impl $typ {
            pub const fn zero() -> Self {
                Self(0)
            }

            pub fn as_usize(self) -> usize {
                self.0
            }

            /// Returns a zero-length range at this offset.
            pub fn empty_range(self) -> Range<Self> {
                self..self
            }

            /// Add a signed integer amount to this offset. In debug builds, this will panic on
            /// overflow.
            pub fn add_signed(self, rhs: isize) -> Self {
                let (inner, overflow) = self.0.overflowing_add_signed(rhs);
                debug_assert!(!overflow, "arithmetic overflow");
                Self(inner)
            }

            pub fn range(range: Range<usize>) -> Range<Self> {
                range.start.into()..range.end.into()
            }
        }

        impl From<usize> for $typ {
            fn from(value: usize) -> Self {
                Self(value)
            }
        }

        impl AddAssign for $typ {
            fn add_assign(&mut self, rhs: Self) {
                *self += rhs.0
            }
        }

        impl AddAssign<usize> for $typ {
            fn add_assign(&mut self, rhs: usize) {
                self.0 += rhs
            }
        }

        impl SubAssign for $typ {
            fn sub_assign(&mut self, rhs: Self) {
                *self -= rhs.0
            }
        }

        impl SubAssign<usize> for $typ {
            fn sub_assign(&mut self, rhs: usize) {
                self.0 -= rhs
            }
        }

        impl Add for $typ {
            type Output = Self;

            fn add(self, rhs: Self) -> Self::Output {
                Self(self.0 + rhs.0)
            }
        }

        impl Add<usize> for $typ {
            type Output = Self;

            fn add(self, rhs: usize) -> Self::Output {
                Self(self.0 + rhs)
            }
        }

        impl Sub for $typ {
            type Output = Self;

            fn sub(self, rhs: Self) -> Self::Output {
                Self(self.0 - rhs.0)
            }
        }

        impl Sub<usize> for $typ {
            type Output = Self;

            fn sub(self, rhs: usize) -> Self::Output {
                Self(self.0 - rhs)
            }
        }

        impl num_traits::SaturatingSub for $typ {
            #[inline]
            fn saturating_sub(&self, rhs: &Self) -> Self {
                Self(self.0.saturating_sub(rhs.0))
            }
        }

        impl std::fmt::Display for $typ {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

impl_offset!(CharOffset);
impl_offset!(ByteOffset);

impl AddAssign<i32> for CharOffset {
    fn add_assign(&mut self, rhs: i32) {
        if rhs.is_positive() {
            self.0 += rhs as usize;
        } else {
            self.0 -= rhs.unsigned_abs() as usize;
        }
    }
}

/// Stores both the char range and the byte range of the same span of text. Some APIs, e.g. the
/// hover/click APIs, use char ranges, while text-related APIs, e.g. `Regex` and `replace_range`,
/// use byte ranges.
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct StringRange {
    pub char_range: Range<usize>,
    pub byte_range: Range<usize>,
}

impl StringRange {
    /// Extends the current range to include the provided range.
    pub fn extend_range_end(&mut self, other: &StringRange) {
        self.char_range.end = self.char_range.end.max(other.char_range.end);
        self.byte_range.end = self.byte_range.end.max(other.byte_range.end);
    }
}

/// A utility for counting characters while iterating through a string from left to right.
///
/// # Example
///
/// ```
/// # use string_offset::CharCounter;
/// let text = "abc🔥abc☄️abc😬";
/// let mut counter = CharCounter::new(text);
/// let matches: Vec<_> = text.match_indices("abc")
///   .map(|(byte_start, _)| (byte_start, counter.char_offset(byte_start).unwrap().as_usize()))
///   .collect();
/// assert_eq!(matches, vec![(0, 0), (7, 4), (16, 9)]);
/// ```
pub struct CharCounter<'a> {
    current_offset: CharOffset,
    char_indices: std::str::CharIndices<'a>,
}

impl<'a> CharCounter<'a> {
    pub fn new(str: &'a str) -> Self {
        Self {
            char_indices: str.char_indices(),
            current_offset: CharOffset::zero(),
        }
    }

    /// Get the character offset that corresponds to `byte_offset`. That is, if the `nth` item of
    /// [`str::char_indices`] had `byte_offset` as its position, this would return `n`.
    ///
    /// This returns `None` if:
    /// * `byte_offset` is past the length of the backing string
    /// * `byte_offset` is not the start of a character
    /// * The counter has already advanced past `byte_offset`
    pub fn char_offset(&mut self, byte_offset: impl Into<ByteOffset>) -> Option<CharOffset> {
        let byte_offset = byte_offset.into();
        if self.char_indices.offset() > byte_offset.as_usize() {
            None
        } else {
            for (next_byte_offset, _) in self.char_indices.by_ref() {
                self.current_offset += 1;

                if next_byte_offset == byte_offset.as_usize() {
                    return Some(self.current_offset - 1);
                }
            }
            None
        }
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
