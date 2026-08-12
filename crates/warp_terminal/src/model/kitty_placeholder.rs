//! Decoding of kitty graphics protocol [Unicode placeholder] cells.
//!
//! A placeholder cell has U+10EEEE as its base character. The cell's foreground color encodes
//! the image id, and combining diacritics from [`ROW_COLUMN_DIACRITICS`] encode which tile of
//! the virtual placement the cell shows: the first diacritic is the tile row, the second is the
//! tile column, and an optional third is the most significant byte of the image id.
//!
//! [Unicode placeholder]: https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders

use crate::model::char_or_str::CharOrStr;

use super::ansi::Color;
use super::cell::Cell;

/// The Unicode placeholder character (`U+10EEEE`) used by the kitty graphics protocol to mark a
/// cell as showing one tile of a virtual image placement.
pub const PLACEHOLDER_CHAR: char = '\u{10EEEE}';

/// kitty's row/column diacritics table, in ascending codepoint order.
///
/// The diacritic at position `i` encodes the number `i` (so `U+0305` encodes 0). The table is
/// checked in verbatim from kitty's `gen/rowcolumn-diacritics.txt`; its order in that file is
/// both the encoding order and ascending codepoint order, which is what allows
/// [`diacritic_index`] to binary-search it.
#[rustfmt::skip]
static ROW_COLUMN_DIACRITICS: [char; 297] = [
    '\u{0305}', '\u{030D}', '\u{030E}', '\u{0310}', '\u{0312}', '\u{033D}', '\u{033E}', '\u{033F}',
    '\u{0346}', '\u{034A}', '\u{034B}', '\u{034C}', '\u{0350}', '\u{0351}', '\u{0352}', '\u{0357}',
    '\u{035B}', '\u{0363}', '\u{0364}', '\u{0365}', '\u{0366}', '\u{0367}', '\u{0368}', '\u{0369}',
    '\u{036A}', '\u{036B}', '\u{036C}', '\u{036D}', '\u{036E}', '\u{036F}', '\u{0483}', '\u{0484}',
    '\u{0485}', '\u{0486}', '\u{0487}', '\u{0592}', '\u{0593}', '\u{0594}', '\u{0595}', '\u{0597}',
    '\u{0598}', '\u{0599}', '\u{059C}', '\u{059D}', '\u{059E}', '\u{059F}', '\u{05A0}', '\u{05A1}',
    '\u{05A8}', '\u{05A9}', '\u{05AB}', '\u{05AC}', '\u{05AF}', '\u{05C4}', '\u{0610}', '\u{0611}',
    '\u{0612}', '\u{0613}', '\u{0614}', '\u{0615}', '\u{0616}', '\u{0617}', '\u{0657}', '\u{0658}',
    '\u{0659}', '\u{065A}', '\u{065B}', '\u{065D}', '\u{065E}', '\u{06D6}', '\u{06D7}', '\u{06D8}',
    '\u{06D9}', '\u{06DA}', '\u{06DB}', '\u{06DC}', '\u{06DF}', '\u{06E0}', '\u{06E1}', '\u{06E2}',
    '\u{06E4}', '\u{06E7}', '\u{06E8}', '\u{06EB}', '\u{06EC}', '\u{0730}', '\u{0732}', '\u{0733}',
    '\u{0735}', '\u{0736}', '\u{073A}', '\u{073D}', '\u{073F}', '\u{0740}', '\u{0741}', '\u{0743}',
    '\u{0745}', '\u{0747}', '\u{0749}', '\u{074A}', '\u{07EB}', '\u{07EC}', '\u{07ED}', '\u{07EE}',
    '\u{07EF}', '\u{07F0}', '\u{07F1}', '\u{07F3}', '\u{0816}', '\u{0817}', '\u{0818}', '\u{0819}',
    '\u{081B}', '\u{081C}', '\u{081D}', '\u{081E}', '\u{081F}', '\u{0820}', '\u{0821}', '\u{0822}',
    '\u{0823}', '\u{0825}', '\u{0826}', '\u{0827}', '\u{0829}', '\u{082A}', '\u{082B}', '\u{082C}',
    '\u{082D}', '\u{0951}', '\u{0953}', '\u{0954}', '\u{0F82}', '\u{0F83}', '\u{0F86}', '\u{0F87}',
    '\u{135D}', '\u{135E}', '\u{135F}', '\u{17DD}', '\u{193A}', '\u{1A17}', '\u{1A75}', '\u{1A76}',
    '\u{1A77}', '\u{1A78}', '\u{1A79}', '\u{1A7A}', '\u{1A7B}', '\u{1A7C}', '\u{1B6B}', '\u{1B6D}',
    '\u{1B6E}', '\u{1B6F}', '\u{1B70}', '\u{1B71}', '\u{1B72}', '\u{1B73}', '\u{1CD0}', '\u{1CD1}',
    '\u{1CD2}', '\u{1CDA}', '\u{1CDB}', '\u{1CE0}', '\u{1DC0}', '\u{1DC1}', '\u{1DC3}', '\u{1DC4}',
    '\u{1DC5}', '\u{1DC6}', '\u{1DC7}', '\u{1DC8}', '\u{1DC9}', '\u{1DCB}', '\u{1DCC}', '\u{1DD1}',
    '\u{1DD2}', '\u{1DD3}', '\u{1DD4}', '\u{1DD5}', '\u{1DD6}', '\u{1DD7}', '\u{1DD8}', '\u{1DD9}',
    '\u{1DDA}', '\u{1DDB}', '\u{1DDC}', '\u{1DDD}', '\u{1DDE}', '\u{1DDF}', '\u{1DE0}', '\u{1DE1}',
    '\u{1DE2}', '\u{1DE3}', '\u{1DE4}', '\u{1DE5}', '\u{1DE6}', '\u{1DFE}', '\u{20D0}', '\u{20D1}',
    '\u{20D4}', '\u{20D5}', '\u{20D6}', '\u{20D7}', '\u{20DB}', '\u{20DC}', '\u{20E1}', '\u{20E7}',
    '\u{20E9}', '\u{20F0}', '\u{2CEF}', '\u{2CF0}', '\u{2CF1}', '\u{2DE0}', '\u{2DE1}', '\u{2DE2}',
    '\u{2DE3}', '\u{2DE4}', '\u{2DE5}', '\u{2DE6}', '\u{2DE7}', '\u{2DE8}', '\u{2DE9}', '\u{2DEA}',
    '\u{2DEB}', '\u{2DEC}', '\u{2DED}', '\u{2DEE}', '\u{2DEF}', '\u{2DF0}', '\u{2DF1}', '\u{2DF2}',
    '\u{2DF3}', '\u{2DF4}', '\u{2DF5}', '\u{2DF6}', '\u{2DF7}', '\u{2DF8}', '\u{2DF9}', '\u{2DFA}',
    '\u{2DFB}', '\u{2DFC}', '\u{2DFD}', '\u{2DFE}', '\u{2DFF}', '\u{A66F}', '\u{A67C}', '\u{A67D}',
    '\u{A6F0}', '\u{A6F1}', '\u{A8E0}', '\u{A8E1}', '\u{A8E2}', '\u{A8E3}', '\u{A8E4}', '\u{A8E5}',
    '\u{A8E6}', '\u{A8E7}', '\u{A8E8}', '\u{A8E9}', '\u{A8EA}', '\u{A8EB}', '\u{A8EC}', '\u{A8ED}',
    '\u{A8EE}', '\u{A8EF}', '\u{A8F0}', '\u{A8F1}', '\u{AAB0}', '\u{AAB2}', '\u{AAB3}', '\u{AAB7}',
    '\u{AAB8}', '\u{AABE}', '\u{AABF}', '\u{AAC1}', '\u{FE20}', '\u{FE21}', '\u{FE22}', '\u{FE23}',
    '\u{FE24}', '\u{FE25}', '\u{FE26}', '\u{10A0F}', '\u{10A38}', '\u{1D185}', '\u{1D186}', '\u{1D187}',
    '\u{1D188}', '\u{1D189}', '\u{1D1AA}', '\u{1D1AB}', '\u{1D1AC}', '\u{1D1AD}', '\u{1D242}', '\u{1D243}',
    '\u{1D244}',
];

/// The number a row/column diacritic encodes, or `None` if `c` is not in kitty's table.
pub fn diacritic_index(c: char) -> Option<u16> {
    ROW_COLUMN_DIACRITICS
        .binary_search(&c)
        .ok()
        .map(|index| index as u16)
}

/// A decoded placeholder cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedPlaceholder {
    /// The referenced image id: the low 24 bits come from the cell's foreground color (all 8-bit
    /// values for an indexed color), and bits 24-31 come from the third diacritic when present.
    pub image_id: u32,
    /// Tile row, from the first diacritic. `None` means the cell carries no row diacritic and
    /// the row must be inferred from the cell to its left, per the kitty continuation rules.
    pub row: Option<u16>,
    /// Tile column, from the second diacritic. `None` means the column must be inferred.
    pub col: Option<u16>,
}

/// Decodes `cell` as a Unicode placeholder, or returns `None` when it is not one.
///
/// A cell decodes only when its base character is [`PLACEHOLDER_CHAR`] and its foreground color
/// carries an image id (a direct RGB color or an indexed color; default/named foregrounds encode
/// nothing). Diacritics stop being read at the first character that is not in kitty's table, so
/// malformed suffixes degrade to the continuation rules instead of failing the cell.
pub fn decode_placeholder(cell: &Cell) -> Option<DecodedPlaceholder> {
    if cell.c != PLACEHOLDER_CHAR {
        return None;
    }

    let image_id_low = match cell.fg {
        Color::Spec(color) => u32::from_be_bytes([0, color.r, color.g, color.b]),
        Color::Indexed(index) => u32::from(index),
        Color::Named(_) => return None,
    };

    let (row, col, image_id_high) = match cell.raw_content() {
        CharOrStr::Char(_) => (None, None, None),
        CharOrStr::Str(content) => {
            let mut indices = content.chars().skip(1).map_while(diacritic_index);
            (indices.next(), indices.next(), indices.next())
        }
    };

    let image_id = match image_id_high {
        // Only the low byte of the third diacritic's value is meaningful; kitty's table has
        // more than 256 entries, but ids only have 8 bits above the foreground color's 24.
        Some(high) => image_id_low | (u32::from(high) & 0xFF) << 24,
        None => image_id_low,
    };

    Some(DecodedPlaceholder { image_id, row, col })
}

/// One maximal horizontal run of placeholder cells that reference the same image and source
/// row, with consecutive source columns. A run shows one contiguous strip of the virtual
/// placement, so the renderer can draw it with a single clipped image draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceholderRun {
    pub image_id: u32,
    /// Tile row within the virtual placement.
    pub src_row: u16,
    /// Tile column of the run's first cell.
    pub src_col_start: u16,
    /// Number of cells (and consecutive tiles) in the run.
    pub len: usize,
    /// Grid column of the run's first cell.
    pub screen_col: usize,
}

/// Scans one grid row and groups its placeholder cells into maximal [`PlaceholderRun`]s.
///
/// Omitted diacritics resolve with kitty's continuation rules: a cell with no diacritics
/// continues one tile to the right of the placeholder cell directly to its left when that cell
/// references the same image (tile (0, 0) otherwise), and a cell with only a row diacritic
/// starts that row at column 0 unless it continues its left neighbor's row.
pub fn placeholder_runs_in_row(row: &[Cell]) -> Vec<PlaceholderRun> {
    let mut runs: Vec<PlaceholderRun> = Vec::new();
    // The resolved (image_id, src_row, src_col) of the placeholder cell directly to the left.
    let mut prev: Option<(u32, u16, u16)> = None;

    for (col, cell) in row.iter().enumerate() {
        let Some(decoded) = decode_placeholder(cell) else {
            prev = None;
            continue;
        };

        let (src_row, src_col) = match (decoded.row, decoded.col) {
            (Some(row), Some(col)) => (row, col),
            (Some(row), None) => match prev {
                Some((prev_id, prev_row, prev_col))
                    if prev_id == decoded.image_id && prev_row == row =>
                {
                    (row, prev_col.saturating_add(1))
                }
                _ => (row, 0),
            },
            // A column diacritic cannot appear without a row diacritic (diacritics decode in
            // order), so a missing row means the cell continues from its left neighbor, or
            // starts at the placement origin.
            (None, _) => match prev {
                Some((prev_id, prev_row, prev_col)) if prev_id == decoded.image_id => {
                    (prev_row, prev_col.saturating_add(1))
                }
                _ => (0, 0),
            },
        };

        match runs.last_mut() {
            Some(run)
                if run.image_id == decoded.image_id
                    && run.src_row == src_row
                    && col == run.screen_col + run.len
                    && src_col as usize == run.src_col_start as usize + run.len =>
            {
                run.len += 1;
            }
            _ => runs.push(PlaceholderRun {
                image_id: decoded.image_id,
                src_row,
                src_col_start: src_col,
                len: 1,
                screen_col: col,
            }),
        }

        prev = Some((decoded.image_id, src_row, src_col));
    }

    runs
}

#[cfg(test)]
#[path = "kitty_placeholder_tests.rs"]
mod tests;
