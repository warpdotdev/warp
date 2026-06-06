// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

use std::boxed::Box;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::model::ansi::{Color, NamedColor};
use crate::model::char_or_str::CharOrStr;
use crate::model::grid::row::Row;

/// The character set as the content in new, not-yet-set cells.  This can be
/// used to disambiguate between a cell which has never had any content and one
/// which has had a space character written into it.
pub const DEFAULT_CHAR: char = '\0';
pub const DEFAULT_CHAR_BYTE: u8 = b'\0';
pub const DEFAULT_CHAR_STR: &str = "\0";

/// Maximum byte length of a single cell's accumulated grapheme cluster
/// (the base character plus any zero-width characters attached to it).
///
/// This is chosen to be:
///
/// 1. Well above the size of any legitimate grapheme cluster.  Unicode's
///    Stream-Safe Text Format (UAX #15) restricts runs of non-starters to
///    at most 30 codepoints, which caps out around 120 bytes in UTF-8; the
///    longest standardized emoji ZWJ sequences (e.g. multi-person family
///    emoji with skin-tone modifiers) fit well below 100 bytes.
/// 2. Well below the per-chunk size used by flat scrollback storage, so
///    that pushing a cell's grapheme into scrollback can never produce an
///    oversized grapheme that would violate the chunk-size invariant.
pub const MAX_GRAPHEME_BYTES: usize = 256;

/// Soft threshold for warning about an unusually large accumulated
/// grapheme cluster on a single cell.  See [`Cell::push_zerowidth`].
const WARN_GRAPHEME_BYTES: usize = 128;

bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub struct Flags: u16 {
        const INVERSE                   = 0b0000_0000_0000_0001;
        const BOLD                      = 0b0000_0000_0000_0010;
        const ITALIC                    = 0b0000_0000_0000_0100;
        const BOLD_ITALIC               = 0b0000_0000_0000_0110;
        const UNDERLINE                 = 0b0000_0000_0000_1000;
        const WRAPLINE                  = 0b0000_0000_0001_0000;
        const WIDE_CHAR                 = 0b0000_0000_0010_0000;
        const WIDE_CHAR_SPACER          = 0b0000_0000_0100_0000;
        const DIM                       = 0b0000_0000_1000_0000;
        const DIM_BOLD                  = 0b0000_0000_1000_0010;
        const HIDDEN                    = 0b0000_0001_0000_0000;
        const STRIKEOUT                 = 0b0000_0010_0000_0000;
        const LEADING_WIDE_CHAR_SPACER  = 0b0000_0100_0000_0000;
        const DOUBLE_UNDERLINE          = 0b0000_1000_0000_0000;
        /// Set on cells which are the locations of cursor points and should be
        /// tracked through grid resizes.
        const HAS_CURSOR                = 0b0001_0000_0000_0000;
        /// Equivalent to the union of all of the following: Flags::UNDERLINE,
        /// Flags::STRIKEOUT, Flags::DOUBLE_UNDERLINE.
        const CELL_DECORATIONS          = 0b0000_1010_0000_1000;
    }
}

// Use the legacy serialization strategy for bitflags. The 2.XX version of bitflags has a different
// serialization strategy, which would require us to update our ref tests.
impl serde::Serialize for Flags {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        bitflags_serde_legacy::serialize(self, "Flags", serializer)
    }
}

// Use the legacy serialization strategy for bitflags. The 2.XX version of bitflags has a different
// serialization strategy, which would require us to update our ref tests.
impl<'de> serde::Deserialize<'de> for Flags {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        bitflags_serde_legacy::deserialize("Flags", deserializer)
    }
}

/// Trait for determining if a reset should be performed.
pub trait ResetDiscriminant<T> {
    /// Value based on which equality for the reset will be determined.
    fn discriminant(&self) -> T;
}

impl<T: Copy> ResetDiscriminant<T> for T {
    fn discriminant(&self) -> T {
        *self
    }
}

impl ResetDiscriminant<Color> for Cell {
    fn discriminant(&self) -> Color {
        self.bg
    }
}

/// Struct used simply as a "marker" for indicating whether a cell is at the end of the prompt.
#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq)]
pub(super) struct EndOfPromptMarker {
    /// Defined in the case of EndOfPromptMarker being at the end of a line. Indicates whether
    /// the prompt has a trailing newline (that isn't covered in the marker, which is inclusive of
    /// printable characters only).
    pub has_extra_trailing_newline: bool,
}

/// Dynamically allocated cell content.
///
/// This storage is reserved for cell attributes which are rarely set. This allows reducing the
/// allocation required ahead of time for every cell, with some additional overhead when the extra
/// storage is actually required.
#[derive(Serialize, Deserialize, Default, Debug, Clone, Eq, PartialEq)]
struct CellExtra {
    /// Zerowidth characters stored in this cell WITH the base character at the start. This helps
    /// optimize reads on this data structure (we don't need to allocate a new string to join the
    /// base character and zerowidth characters).
    cell_with_zero_width: Option<String>,
    end_of_prompt: Option<EndOfPromptMarker>,
}

/// Content and attributes of a single cell in the terminal grid.
/// NOTE: Many cells are allocated per grid, so this should be as memory compact as possible. Fields
/// that may be optional, or set for only a few cells, should go into the `CellExtra` instead.
///
/// Additional memory usage note: Due to holding a pointer (Box), this struct has an alignment of
/// 8 bytes. This means that the total size taken up by the struct in memory will be an even
/// multiple of 8; if the data is not an even multiple then padding will be added to reach an even
/// value. Currently, this holds exactly 24 bytes, so it is tightly packed and does not need any
/// padding:
///
/// * c: 4 bytes (equivalent to a u32)
/// * fg: 5 bytes (the data contains 4 bytes plus a discriminator for the enum variant. Since it
///   has an alignment of 1, the extra space required by the discriminator is only 1
///   byte. Altering the data could change the alignment, which could then result in
///   more padding and the total size of `Color` increasing)
/// * bg: 5 bytes (Same as fg)
/// * flags: 2 bytes (stored as a u16)
/// * extra: 8 bytes (pointer with null representing None)
///
/// Increasing any of these values by even 1 byte will cause `Cell` to ultimately take up 32 bytes
/// instead of 24, an increase of 33%.
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct Cell {
    pub c: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
    extra: Option<Box<CellExtra>>,
}

impl Default for Cell {
    #[inline]
    fn default() -> Cell {
        Cell {
            c: DEFAULT_CHAR,
            bg: Color::Named(NamedColor::Background),
            fg: Color::Named(NamedColor::Foreground),
            flags: Flags::empty(),
            extra: None,
        }
    }
}

impl Cell {
    /// Cell's character followed by all zerowidth characters stored in this cell. This
    /// is only Some if the cell has zerowidth characters.
    #[inline]
    fn content_with_zerowidth(&self) -> Option<&str> {
        self.extra
            .as_ref()
            .and_then(|extra| extra.cell_with_zero_width.as_deref())
    }

    /// Returns the content of the cell that should be used for display
    /// purposes, e.g.: rendering or stringification.
    #[inline]
    pub fn content_for_display(&self) -> CharOrStr<'_> {
        match self.raw_content() {
            CharOrStr::Char(DEFAULT_CHAR) => CharOrStr::Char(' '),
            content => content,
        }
    }

    /// Returns the raw cell content.
    ///
    /// This may include non-printable marker characters.
    ///
    /// TODO(visibility): This should be changed to `pub(super)` when possible.
    pub fn raw_content(&self) -> CharOrStr<'_> {
        match self.content_with_zerowidth() {
            Some(content_with_zerowidth) => CharOrStr::Str(content_with_zerowidth),
            None => CharOrStr::Char(self.c),
        }
    }

    /// Write a new zerowidth character to this cell.
    ///
    /// Accumulated zero-width content is capped at [`MAX_GRAPHEME_BYTES`]
    /// so that adversarial or buggy input streams cannot produce a single
    /// grapheme cluster larger than the scrollback chunk size.  See
    /// [`MAX_GRAPHEME_BYTES`] for details.
    ///
    /// If `log_long_grapheme_warnings` is true, a [`log::warn!`] is
    /// emitted on the push that first takes this cell's accumulated
    /// grapheme across [`WARN_GRAPHEME_BYTES`].  Callers that are
    /// replaying already-validated grapheme content (e.g. materializing
    /// a row from flat scrollback storage, where the stored content was
    /// already capped on the way in) should pass `false` to suppress
    /// that redundant warning.
    #[inline]
    pub fn push_zerowidth(&mut self, c: char, log_long_grapheme_warnings: bool) {
        // If we're adding a zero-width character to this cell, but it has not
        // had any content set yet, set the content to a space.  This preserves
        // its visual appearance, but clearly marks the cell as having been
        // modified from its default "empty" state.
        if self.c == DEFAULT_CHAR {
            self.c = ' ';
        }

        let extra = self.extra.get_or_insert_with(Box::default);
        match &mut extra.cell_with_zero_width {
            Some(zerowidth) => {
                let old_len = zerowidth.len();
                let new_len = old_len + c.len_utf8();
                if new_len > MAX_GRAPHEME_BYTES {
                    // The accumulated grapheme cluster would exceed our
                    // per-cell cap, which is in turn well below the
                    // scrollback chunk size.  Silently drop additional
                    // zero-width characters: logging every dropped
                    // character would produce a flood of spam for
                    // pathological streams.
                    return;
                }
                zerowidth.push(c);
                // Log exactly once, on the push that first takes this cell
                // across the soft threshold.  This surfaces unusually-large
                // graphemes in logs without producing per-character spam.
                if log_long_grapheme_warnings
                    && old_len < WARN_GRAPHEME_BYTES
                    && new_len >= WARN_GRAPHEME_BYTES
                {
                    log::warn!(
                        "cell grapheme has accumulated {new_len} bytes of zero-width content (base char {:?}); further zero-width pushes beyond {MAX_GRAPHEME_BYTES} bytes will be dropped",
                        self.c,
                    );
                }
            }
            None => {
                // First zero-width push seeds the string with the base
                // character.  The base character is always a single `char`,
                // so it cannot by itself exceed the cap.
                extra.cell_with_zero_width = Some(format!("{}{}", self.c, c));
            }
        }
    }

    /// Returns whether cell is the end of prompt content (contains `EndOfPromptMarker`).
    #[inline]
    pub fn is_end_of_prompt(&self) -> bool {
        self.end_of_prompt_marker().is_some()
    }

    /// Returns information about the end-of-prompt marker in this cell, if any.
    pub(super) fn end_of_prompt_marker(&self) -> Option<EndOfPromptMarker> {
        self.extra.as_ref()?.end_of_prompt
    }

    /// Mark cell as the end of prompt content.
    #[inline]
    pub fn mark_end_of_prompt(&mut self, has_extra_trailing_newline: bool) {
        self.extra
            .get_or_insert_with(Default::default)
            .end_of_prompt = Some(EndOfPromptMarker {
            has_extra_trailing_newline,
        });
    }

    /// Free all dynamically allocated cell storage. Preserves EndOfPromptMarker if present.
    #[inline]
    pub fn drop_extra(&mut self) {
        if let Some(extra) = self.extra.take() {
            if let Some(end_of_prompt_marker) = extra.end_of_prompt {
                // If we had a end of prompt marker, we preserve it (re-insert into extras).
                self.mark_end_of_prompt(end_of_prompt_marker.has_extra_trailing_newline);
            }
            // If `end_of_prompt` is None, `extra` is dropped here and not put back.
        }
    }
}

impl Cell {
    #[inline]
    // TODO(visibility): This should be changed to `pub(crate)` when possible.
    pub fn is_empty(&self) -> bool {
        // TODO(vorporeal): can this be a simple equality check vs. Cell::default()?
        self.c == DEFAULT_CHAR
            && self.bg == Color::Named(NamedColor::Background)
            && self.fg == Color::Named(NamedColor::Foreground)
            && !self.flags.intersects(
                Flags::INVERSE
                    | Flags::UNDERLINE
                    | Flags::DOUBLE_UNDERLINE
                    | Flags::STRIKEOUT
                    | Flags::WRAPLINE
                    | Flags::WIDE_CHAR_SPACER
                    | Flags::LEADING_WIDE_CHAR_SPACER
                    | Flags::HAS_CURSOR,
            )
    }

    /// Returns whether or not rendering the cell would produce anything visible.
    pub fn is_visible(&self) -> bool {
        !self.is_empty() && !self.c.is_ascii_whitespace()
    }

    #[inline]
    #[allow(dead_code)]
    // TODO(visibility): This should be changed to `pub(crate)` when possible.
    pub fn flags(&self) -> &Flags {
        &self.flags
    }

    #[inline]
    #[allow(dead_code)]
    // TODO(visibility): This should be changed to `pub(crate)` when possible.
    pub fn flags_mut(&mut self) -> &mut Flags {
        &mut self.flags
    }

    #[inline]
    pub(crate) fn reset(&mut self, template: &Self) {
        *self = Cell {
            bg: template.bg,
            ..Cell::default()
        };
    }
}

impl From<Color> for Cell {
    #[inline]
    fn from(color: Color) -> Self {
        Self {
            bg: color,
            ..Cell::default()
        }
    }
}

/// Get the length of occupied cells in a line.
pub trait LineLength {
    /// Calculate the occupied line length.
    fn line_length(&self) -> usize;
}

impl LineLength for Row {
    fn line_length(&self) -> usize {
        // If the row has no cells, then the line length is 0, by definition.
        if self.len() == 0 {
            return 0;
        }
        let mut length = 0;

        if self[self.len() - 1].flags.contains(Flags::WRAPLINE) {
            return self.len();
        }

        for (index, cell) in self[..].iter().rev().enumerate() {
            if cell.c != DEFAULT_CHAR {
                length = self.len() - index;
                break;
            }
        }

        length
    }
}

#[cfg(any(test, feature = "test-util"))]
impl From<char> for Cell {
    fn from(c: char) -> Self {
        Cell {
            c,
            ..Default::default()
        }
    }
}

#[cfg(test)]
#[path = "cell_tests.rs"]
mod tests;
