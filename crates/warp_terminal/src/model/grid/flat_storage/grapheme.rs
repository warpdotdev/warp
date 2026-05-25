use std::num::NonZeroU16;

use string_offset::ByteOffset;

use super::index::GraphemeInfo;
use crate::model::char_or_str::CharOrStr;
use crate::model::grid::cell::{self, Cell};

/// A grapheme is a collection of [`char`]s that, together, represent a single
/// "user-perceived character".
///
/// This wrapper exposes a number of helper methods for working with graphemes,
/// such as exposing the number of grid cells that the grapheme will take up in
/// a terminal grid.
///
/// # Panics
///
/// Will panic if constructed for a grapheme with a length of zero bytes.
#[derive(Debug)]
pub struct Grapheme<'a> {
    info: GraphemeInfo,
    content: CharOrStr<'a>,
}

impl<'a> Grapheme<'a> {
    pub const EMPTY_CELL: Grapheme<'static> = Grapheme {
        info: GraphemeInfo {
            cell_width: 1,
            utf8_bytes: NonZeroU16::new(1).expect("1 != 0"),
        },
        content: CharOrStr::Char(cell::DEFAULT_CHAR),
    };

    pub const NEWLINE: Grapheme<'static> = Grapheme {
        info: GraphemeInfo {
            cell_width: 0,
            utf8_bytes: NonZeroU16::new(1).expect("1 != 0"),
        },
        content: CharOrStr::Char('\n'),
    };

    /// Constructs a new [`Grapheme`] from a [`Cell`].
    pub fn new_from_cell(cell: &'a Cell) -> Self {
        let cell_width = 1 + cell.flags().contains(cell::Flags::WIDE_CHAR) as u8;

        let content = cell.raw_content();
        let utf8_bytes = match content {
            CharOrStr::Char(c) => c.len_utf8(),
            CharOrStr::Str(s) => s.len(),
        };
        let utf8_bytes = u16::try_from(utf8_bytes).expect("grapheme length should fit in a u16");
        let utf8_bytes = NonZeroU16::new(utf8_bytes).expect("grapheme string should be non-empty");

        let info = GraphemeInfo {
            cell_width,
            utf8_bytes,
        };
        Self { info, content }
    }

    /// Constructs a new [`Grapheme`] from a string slice and already-computed
    /// [`GraphemeInfo`].
    ///
    /// This is useful when assembling [`Row`](crate::model::grid::row::Row)s
    /// a flat content string and an [`Index`](super::Index).
    pub fn new_from_str_and_info(grapheme: &'a str, info: GraphemeInfo) -> Self {
        Self {
            info,
            content: CharOrStr::Str(grapheme),
        }
    }

    /// Constructs a new [`Grapheme`] from a string slice.
    #[cfg(test)]
    pub fn new_from_str(grapheme: &'a str) -> Self {
        let cell_width = str_to_cell_width(grapheme);
        let utf8_bytes =
            NonZeroU16::new(grapheme.len() as u16).expect("grapheme string should be non-empty");
        let info = GraphemeInfo {
            cell_width,
            utf8_bytes,
        };
        Self {
            info,
            content: CharOrStr::Str(grapheme),
        }
    }

    /// Returns information about the grapheme cell width and byte length.
    pub fn sizing_info(&self) -> GraphemeInfo {
        self.info
    }

    /// Returns the number of cells that this grapheme will take up in a
    /// terminal grid.
    ///
    /// This can return 0 if the grapheme is not user-visible.
    pub fn cell_width(&self) -> u8 {
        self.info.cell_width
    }

    /// Returns the length of this grapheme, in bytes.
    pub fn len(&self) -> ByteOffset {
        ByteOffset::from(self.info.utf8_bytes.get() as usize)
    }

    /// Returns the grapheme content.
    pub fn content(&self) -> CharOrStr<'_> {
        self.content
    }

    /// Returns an iterator over the characters in this grapheme.
    pub fn chars(&self) -> impl Iterator<Item = char> + 'a {
        match self.content {
            CharOrStr::Char(c) => itertools::Either::Left(std::iter::once(c)),
            CharOrStr::Str(s) => itertools::Either::Right(s.chars()),
        }
    }

    /// Returns true if this grapheme triggers the start of a new grid row.
    pub fn starts_new_row(&self) -> bool {
        match self.content {
            CharOrStr::Char(c) => c == '\n',
            CharOrStr::Str(s) => s == "\n",
        }
    }
}

/// Returns the cell width for a grapheme.
#[cfg(test)]
fn str_to_cell_width(grapheme: &str) -> u8 {
    use unicode_width::UnicodeWidthStr as _;

    let first_byte = grapheme.as_bytes()[0];
    if grapheme.len() == 1
        && ((32..127).contains(&first_byte)
            || first_byte == cell::DEFAULT_CHAR_BYTE
            || first_byte == b'\t')
    {
        1
    } else {
        grapheme
            .width()
            .try_into()
            .expect("cell width of a grapheme should never be larger than 2^8")
    }
}
