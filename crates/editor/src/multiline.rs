//! Common types for dealing with multi-line strings safely.
//!
//! There are two pairs of string types, similar to `String` / `str` and `PathBuf` / `Path`:
//! * [`MultilineString`] and [`MultilineStr`] track the line ending at compile time. These are
//!   zero-cost wrappers over [`String`] and [`str`].
//! * [`AnyMultilineString`] and [`AnyMultilineStr`] track the line ending at runtime. This is more
//!   flexible, but incurs a space and ergonomic cost.
//!
//! Generally, prefer [`MultilineString`] and [`MultilineStr`] when working consistently with a set
//! line format (for example, the editor buffer only uses `\n` for line endings). When reading
//! external text, use [`AnyMultilineString`] and [`AnyMultilineStr`] unless normalizing to a set
//! ending.
//!
//! ## Invariants
//! All multiline string types guarantee that their backing string uses the specified line ending.
//! They do not support mixed line endings. All constructors will either normalize to a single line
//! ending or return an error.
//!
//! ## Creating Multiline Strings
//!
//! If you are managing line endings at runtime, use [`AnyMultilineString::infer`]. This will infer
//! a line ending from the input text, normalizing it if necessary.
//!
//! To create a string with a guaranteed, statically-known ending, use either [`MultilineString::apply`]
//! or [`MultilineStr::apply`]. The latter avoids allocating if the input string's line ending is already
//! correct, but may borrow its input.
//!
//! If you already know the ending of a string (e.g. it's a literal), use [`MultilineStr::try_new`].
//!
//! ## Converting Line Endings
//!
//! If you already have a multiline string type, and need to convert to a different line ending, use
//! one of the following methods:
//!
//! * `to_format` for converting to a statically-known ending ([`MultilineStr::to_format`],
//!   [`AnyMultilineString::to_format`])
//! * [`AnyMultilineString::to_line_ending`] or [`AnyMultilineString::into_line_ending`] to convert
//!   to a line ending known at runtime

use std::borrow::{Borrow, Cow};
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;

use itertools::Itertools as _;
use line_ending::LineEnding;
use warp_core::platform::SessionPlatform;

/// A line ending format. This is the compile-time equivalent to [`LineEnding`].
pub trait LineFormat {
    /// The `LineEnding` corresponding to this format.
    fn ending() -> LineEnding;

    /// Apply this line ending format to `s`, replacing any other line endings.
    fn apply_to(s: &str) -> String {
        // TODO: LineEnding::apply is pretty inefficient, it may be worth optimizing.
        Self::ending().apply(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CRLF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CR;

/// An owned multiline string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultilineString<F: LineFormat> {
    _format: PhantomData<F>,
    inner: String,
}

/// A borrowed multiline string.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq)]
pub struct MultilineStr<F: LineFormat> {
    _format: PhantomData<F>,
    inner: str,
}

/// A string of multiline text, with an associated line ending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnyMultilineString {
    inner: String,
    line_ending: LineEnding,
}

/// A borrowed multiline string, with an associated line ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnyMultilineStr<'a> {
    inner: &'a str,
    line_ending: LineEnding,
}

/// The error returned when a string did not have the expected line ending.
#[derive(Debug, thiserror::Error)]
#[error("Expected line ending {expected:?}, but was {actual:?}")]
pub struct IncorrectLineEndingError {
    pub expected: LineEnding,
    pub actual: LineEnding,
}

impl LineFormat for LF {
    fn ending() -> LineEnding {
        LineEnding::LF
    }

    fn apply_to(s: &str) -> String {
        // As opposed to `apply`, `normalize` avoids an extra conversion pass.
        LineEnding::normalize(s)
    }
}

impl LineFormat for CRLF {
    fn ending() -> LineEnding {
        LineEnding::CRLF
    }
}

impl LineFormat for CR {
    fn ending() -> LineEnding {
        LineEnding::CR
    }

    fn apply_to(s: &str) -> String {
        // This avoids a normalization pass - we can use the same implementation as `normalize`,
        // but with a different replacement string.
        s.replace("\r\n", "\r").replace('\n', "\r")
    }
}

impl<F: LineFormat> MultilineString<F> {
    /// Create a new `MultilineString` that assumes the given line ending. The caller must
    /// guarantee that `s` already uses the given line ending - prefer [`Self::apply`] instead.
    #[inline]
    fn new_unchecked(s: impl Into<String>) -> Self {
        Self {
            _format: PhantomData,
            inner: s.into(),
        }
    }

    /// Create a new `MultilineString` by unconditionally applying `F` to `s`.
    pub fn apply(s: impl AsRef<str>) -> Self {
        Self::new_unchecked(F::apply_to(s.as_ref()))
    }

    /// Extract the underlying string, which is guaranteed to use `F` as its line ending.
    pub fn into_string(self) -> String {
        self.inner
    }
}

impl<F: LineFormat> fmt::Display for MultilineString<F> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<F: LineFormat> fmt::Display for MultilineStr<F> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<F: LineFormat> MultilineStr<F> {
    /// Create a new `MultilineStr` that assumes the given line ending. The caller must
    /// guarantee that `s` already uses the given line ending - prefer [`MultilineString::apply`]
    /// if the format is not known.
    #[inline]
    fn new_unchecked<S: AsRef<str> + ?Sized>(s: &S) -> &Self {
        // Safety: Because `MultilineStr` is `#repr(transparent)`, it has an identical in-memory
        // representation to `str`. Therefore, we can cast from `*str` to `*MultilineStr<F>`, and
        // then to `&MultilineStr<F>`, with the same lifetime as the original `&str`.
        // See https://users.rust-lang.org/t/creating-types-analoguous-to-path-and-pathbuf/117310.
        unsafe { &*(s.as_ref() as *const str as *const Self) }
    }

    /// Create a new `MultilineStr` from a string slice. This returns an error if the slice does
    /// not already use `F` for line endings, or if it contains mixed line endings.
    pub fn try_new<S: AsRef<str> + ?Sized>(s: &S) -> Result<&Self, IncorrectLineEndingError> {
        let s = s.as_ref();
        let endings = evaluate_line_endings(s);
        match endings {
            // If there are no line endings, then they trivially match.
            TextLineEndings::SingleLine => Ok(MultilineStr::new_unchecked(s)),
            TextLineEndings::MultiLine {
                primary_ending,
                mixed_endings,
            } => {
                if primary_ending == F::ending() && !mixed_endings {
                    Ok(MultilineStr::new_unchecked(s))
                } else {
                    Err(IncorrectLineEndingError {
                        expected: F::ending(),
                        actual: primary_ending,
                    })
                }
            }
        }
    }

    /// Create a `MultilineStr` or [`MultilineString`] from `s`, guaranteed to use `F` for line endings.
    ///
    /// If `s` already uses `F` for line endings, then no new allocation is needed and a [`Cow::Borrowed`] is returned.
    /// Otherwise, a new [`MultilineString`] is allocated with the converted text.
    pub fn apply<S: AsRef<str> + ?Sized>(s: &S) -> Cow<'_, Self> {
        let s = s.as_ref();
        let endings = evaluate_line_endings(s);
        match endings {
            TextLineEndings::SingleLine => Cow::Borrowed(MultilineStr::new_unchecked(s)),
            TextLineEndings::MultiLine {
                primary_ending,
                mixed_endings,
            } => {
                if primary_ending == F::ending() && !mixed_endings {
                    Cow::Borrowed(MultilineStr::new_unchecked(s))
                } else {
                    Cow::Owned(MultilineString::<F>::apply(s))
                }
            }
        }
    }

    /// Convert to an owned [`MultilineString`] with a new line ending, `G`. This must allocate in
    /// order to create a string with the new line ending.
    pub fn to_format<G: LineFormat>(&self) -> MultilineString<G> {
        let converted = G::apply_to(&self.inner);
        MultilineString::new_unchecked(converted)
    }

    /// Get the underlying string, which is guaranteed to use `F` as its line ending.
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Iterate over the lines of this string, split by `F`.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.inner.split(F::ending().as_str())
    }
}

impl<F: LineFormat> ToOwned for MultilineStr<F> {
    type Owned = MultilineString<F>;

    fn to_owned(&self) -> Self::Owned {
        MultilineString {
            _format: PhantomData,
            inner: self.inner.to_owned(),
        }
    }
}

impl<F: LineFormat> Deref for MultilineString<F> {
    type Target = MultilineStr<F>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        MultilineStr::new_unchecked(self.inner.as_str())
    }
}

impl<F: LineFormat> Borrow<MultilineStr<F>> for MultilineString<F> {
    #[inline]
    fn borrow(&self) -> &MultilineStr<F> {
        self
    }
}

impl<F: LineFormat> AsRef<MultilineStr<F>> for MultilineString<F> {
    #[inline]
    fn as_ref(&self) -> &MultilineStr<F> {
        self
    }
}

impl<F: LineFormat> TryFrom<AnyMultilineString> for MultilineString<F> {
    type Error = IncorrectLineEndingError;

    /// Attempt to convert an [`AnyMultilineString`] to a [`MultilineString`]. This fails if the
    /// line ending is not `F`. Use [`AnyMultilineString::to_format`] to infallibly convert line
    /// endings.
    fn try_from(value: AnyMultilineString) -> Result<Self, Self::Error> {
        if value.line_ending == F::ending() {
            Ok(MultilineString::new_unchecked(value.inner))
        } else {
            Err(IncorrectLineEndingError {
                expected: F::ending(),
                actual: value.line_ending,
            })
        }
    }
}

impl<F: LineFormat> TryFrom<String> for MultilineString<F> {
    type Error = IncorrectLineEndingError;

    /// Attempt to convert a [`String`] to a [`MultilineString`]. This fails if the inferred line
    /// ending is not `F`.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        AnyMultilineString::infer(value).try_into()
    }
}

impl<'a, F: LineFormat> TryFrom<AnyMultilineStr<'a>> for &'a MultilineStr<F> {
    type Error = IncorrectLineEndingError;

    /// Attempt to convert an [`AnyMultilineStr`] to a [`MultilineStr`]. This fails if the
    /// line ending is not `F`. Use [`AnyMultilineString::to_format`] to infallibly convert line
    /// endings.
    fn try_from(value: AnyMultilineStr<'a>) -> Result<Self, Self::Error> {
        if value.line_ending == F::ending() {
            Ok(MultilineStr::new_unchecked(value.inner))
        } else {
            Err(IncorrectLineEndingError {
                expected: F::ending(),
                actual: value.line_ending,
            })
        }
    }
}

impl AnyMultilineString {
    /// Create a new `AnyMultilineString` with the given line ending. The caller must guarantee that `line_ending`
    /// is the line ending used in `text`.
    pub fn new_unchecked(text: impl Into<String>, line_ending: LineEnding) -> Self {
        Self {
            inner: text.into(),
            line_ending,
        }
    }

    /// Create a new `AnyMultilineString` with the given text normalized via [`LineEnding::normalize`].
    ///
    /// This converts all line endings to `\n`.
    pub fn normalize_to_linefeed(text: impl AsRef<str>) -> Self {
        MultilineString::<LF>::apply(text).into()
    }

    /// Create a new `AnyMultilineString` from individual lines and the desired line ending.
    pub fn from_lines<I: IntoIterator<Item = String>>(lines: I, line_ending: LineEnding) -> Self {
        let text = lines.into_iter().join(line_ending.as_str());
        Self::new_unchecked(text, line_ending)
    }

    /// Create a new `AnyMultilineString` with the line ending inferred from the input text.
    pub fn infer(text: impl Into<String>) -> Self {
        let mut text = text.into();
        let endings = evaluate_line_endings(&text);
        // TODO: Figure out how to get a SessionPlatform here.
        let primary_ending = endings.primary_ending(None);
        if endings.is_mixed() {
            // If the text contains mixed line endings (uncommon), they must be normalized to a
            // single ending to uphold the `AnyMultilineString` contract.
            text = primary_ending.apply(&text);
        }
        Self::new_unchecked(text, primary_ending)
    }

    /// The underlying string. This is guaranteed to use [`Self::line_ending`] as its line ending.
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Extract the underlying string. This is guaranteed to use [`Self::line_ending`] as its line ending.
    pub fn into_string(self) -> String {
        self.inner
    }

    /// The configured line ending of this string.
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// Converts this text to use `ending`. This is similar to [`to_line_ending`], but consumes
    /// `self` for convenience if the original value is no longer needed. It will reuse the
    /// existing allocation if possible.
    pub fn into_line_ending(self, ending: LineEnding) -> Self {
        if self.line_ending == ending {
            self
        } else {
            let new_text = ending.apply(&self.inner);
            Self::new_unchecked(new_text, ending)
        }
    }

    /// Converts this text to use `ending`.
    ///
    /// This method returns a [`Cow<'_, Self>`]. If our line ending is already `ending`, it
    /// returns a reference to `self`. Otherwise, it allocates a new `AnyMultilineString` with the
    /// new line ending applied.
    pub fn to_line_ending(&self, ending: LineEnding) -> Cow<'_, Self> {
        if self.line_ending == ending {
            Cow::Borrowed(self)
        } else {
            let new_text = ending.apply(&self.inner);
            Cow::Owned(Self::new_unchecked(new_text, ending))
        }
    }

    /// Converts this text to use `F` as its line ending.
    ///
    /// This method returns a [`Cow<'_, MultilineStr<F>>`]. If our line ending is already `F`, it
    /// returns a reference to the string backing `self`. Otherwise, it allocates a new
    /// [`MultilineString`] with the new line ending applied.
    pub fn to_format<F: LineFormat>(&self) -> Cow<'_, MultilineStr<F>> {
        if self.line_ending == F::ending() {
            Cow::Borrowed(MultilineStr::new_unchecked(self.inner.as_str()))
        } else {
            Cow::Owned(MultilineString::<F>::apply(self.inner.as_str()))
        }
    }

    /// Iterate over the lines of this string, split by [`Self::line_ending`].
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.inner.split(self.line_ending.as_str())
    }
}

impl<F: LineFormat> From<MultilineString<F>> for AnyMultilineString {
    fn from(value: MultilineString<F>) -> Self {
        Self {
            inner: value.into_string(),
            line_ending: F::ending(),
        }
    }
}

impl<'a> AnyMultilineStr<'a> {
    pub fn new_unchecked<S: AsRef<str> + ?Sized>(text: &'a S, line_ending: LineEnding) -> Self {
        Self {
            inner: text.as_ref(),
            line_ending,
        }
    }

    /// Get the underlying string. This is guaranteed to use [`Self::line_ending`] as its line ending.
    pub fn as_str(&self) -> &str {
        self.inner
    }

    /// The configured line ending of this string.
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }
}

impl<'a, F: LineFormat> From<&'a MultilineStr<F>> for AnyMultilineStr<'a> {
    fn from(value: &'a MultilineStr<F>) -> Self {
        Self {
            inner: value.as_str(),
            line_ending: F::ending(),
        }
    }
}

/// Returns the line ending style to use for the input string.
///
/// This is the most common line ending in the string. If there are no line endings, the platform
/// default is used.
pub fn infer_line_ending(text: &str, platform: Option<&SessionPlatform>) -> LineEnding {
    evaluate_line_endings(text).primary_ending(platform)
}

/// Results of analyzing the line endings in a string.
enum TextLineEndings {
    /// The text has no line endings.
    SingleLine,
    /// The text contains at least one line ending.
    MultiLine {
        /// The most common line ending in the string.
        primary_ending: LineEnding,
        /// Whether the string contains a mix of line endings.
        mixed_endings: bool,
    },
}

impl TextLineEndings {
    /// Whether or not the string contained a mix of line endings.
    fn is_mixed(&self) -> bool {
        match self {
            TextLineEndings::SingleLine => false,
            TextLineEndings::MultiLine { mixed_endings, .. } => *mixed_endings,
        }
    }

    /// The primary line ending of the string. For single-line strings, this is the platform default.
    #[allow(clippy::disallowed_methods)]
    fn primary_ending(&self, platform: Option<&SessionPlatform>) -> LineEnding {
        match self {
            TextLineEndings::SingleLine => platform
                .map_or_else(LineEnding::from_current_platform, |platform| {
                    platform.default_line_ending()
                }),
            TextLineEndings::MultiLine { primary_ending, .. } => *primary_ending,
        }
    }
}

/// Evaluates the line endings present in a string.
///
/// The primary line ending is the line ending type that occurred most often in a string.
fn evaluate_line_endings(text: &str) -> TextLineEndings {
    // This is similar to LineEnding::from, but with platform-aware tie-breaking.
    let scores = LineEnding::score_mixed_types(text);

    // The score_mixed_types implementation guarantees a score for each line ending.
    let crlf_score = scores[&LineEnding::CRLF];
    let lf_score = scores[&LineEnding::LF];
    let cr_score = scores[&LineEnding::CR];

    // Use the most-prevalent line ending, or the platform ending if there are no lines.
    // // In case of a tie, prefer LF over CRLF as Unix-style endings are overall more common.
    let max_score = crlf_score.max(lf_score).max(cr_score);
    if max_score == 0 {
        TextLineEndings::SingleLine
    } else if max_score == lf_score {
        TextLineEndings::MultiLine {
            primary_ending: LineEnding::LF,
            mixed_endings: crlf_score > 0 || cr_score > 0,
        }
    } else if max_score == crlf_score {
        TextLineEndings::MultiLine {
            primary_ending: LineEnding::CRLF,
            mixed_endings: lf_score > 0 || cr_score > 0,
        }
    } else {
        TextLineEndings::MultiLine {
            primary_ending: LineEnding::CR,
            mixed_endings: crlf_score > 0 || lf_score > 0,
        }
    }
}

#[cfg(test)]
#[path = "multiline_tests.rs"]
mod tests;
