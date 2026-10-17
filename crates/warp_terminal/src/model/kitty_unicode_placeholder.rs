//! Unicode placeholders for kitty images: cells holding U+10EEEE, each showing one tile of an
//! image given a virtual placement (`U=1`). See
//! <https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders>.
//!
//! The image id is the cell's foreground color (a 256-color index, or 24-bit RGB), and up to three
//! diacritics give the tile's row, its column, and the id's most significant byte. A diacritic
//! left out is inferred from the placeholder cell to the left.

use std::collections::HashMap;

use super::ansi::Color;
use super::cell::Cell;
use super::char_or_str::CharOrStr;

const PLACEHOLDER: char = '\u{10EEEE}';

/// kitty's `rowcolumn-diacritics.txt`: a diacritic stands for its index. Sorted by code point.
#[rustfmt::skip]
const ROW_COLUMN_DIACRITICS: [char; 297] = [
    '\u{305}', '\u{30D}', '\u{30E}', '\u{310}', '\u{312}', '\u{33D}', '\u{33E}',
    '\u{33F}', '\u{346}', '\u{34A}', '\u{34B}', '\u{34C}', '\u{350}', '\u{351}',
    '\u{352}', '\u{357}', '\u{35B}', '\u{363}', '\u{364}', '\u{365}', '\u{366}',
    '\u{367}', '\u{368}', '\u{369}', '\u{36A}', '\u{36B}', '\u{36C}', '\u{36D}',
    '\u{36E}', '\u{36F}', '\u{483}', '\u{484}', '\u{485}', '\u{486}', '\u{487}',
    '\u{592}', '\u{593}', '\u{594}', '\u{595}', '\u{597}', '\u{598}', '\u{599}',
    '\u{59C}', '\u{59D}', '\u{59E}', '\u{59F}', '\u{5A0}', '\u{5A1}', '\u{5A8}',
    '\u{5A9}', '\u{5AB}', '\u{5AC}', '\u{5AF}', '\u{5C4}', '\u{610}', '\u{611}',
    '\u{612}', '\u{613}', '\u{614}', '\u{615}', '\u{616}', '\u{617}', '\u{657}',
    '\u{658}', '\u{659}', '\u{65A}', '\u{65B}', '\u{65D}', '\u{65E}', '\u{6D6}',
    '\u{6D7}', '\u{6D8}', '\u{6D9}', '\u{6DA}', '\u{6DB}', '\u{6DC}', '\u{6DF}',
    '\u{6E0}', '\u{6E1}', '\u{6E2}', '\u{6E4}', '\u{6E7}', '\u{6E8}', '\u{6EB}',
    '\u{6EC}', '\u{730}', '\u{732}', '\u{733}', '\u{735}', '\u{736}', '\u{73A}',
    '\u{73D}', '\u{73F}', '\u{740}', '\u{741}', '\u{743}', '\u{745}', '\u{747}',
    '\u{749}', '\u{74A}', '\u{7EB}', '\u{7EC}', '\u{7ED}', '\u{7EE}', '\u{7EF}',
    '\u{7F0}', '\u{7F1}', '\u{7F3}', '\u{816}', '\u{817}', '\u{818}', '\u{819}',
    '\u{81B}', '\u{81C}', '\u{81D}', '\u{81E}', '\u{81F}', '\u{820}', '\u{821}',
    '\u{822}', '\u{823}', '\u{825}', '\u{826}', '\u{827}', '\u{829}', '\u{82A}',
    '\u{82B}', '\u{82C}', '\u{82D}', '\u{951}', '\u{953}', '\u{954}', '\u{F82}',
    '\u{F83}', '\u{F86}', '\u{F87}', '\u{135D}', '\u{135E}', '\u{135F}', '\u{17DD}',
    '\u{193A}', '\u{1A17}', '\u{1A75}', '\u{1A76}', '\u{1A77}', '\u{1A78}', '\u{1A79}',
    '\u{1A7A}', '\u{1A7B}', '\u{1A7C}', '\u{1B6B}', '\u{1B6D}', '\u{1B6E}', '\u{1B6F}',
    '\u{1B70}', '\u{1B71}', '\u{1B72}', '\u{1B73}', '\u{1CD0}', '\u{1CD1}', '\u{1CD2}',
    '\u{1CDA}', '\u{1CDB}', '\u{1CE0}', '\u{1DC0}', '\u{1DC1}', '\u{1DC3}', '\u{1DC4}',
    '\u{1DC5}', '\u{1DC6}', '\u{1DC7}', '\u{1DC8}', '\u{1DC9}', '\u{1DCB}', '\u{1DCC}',
    '\u{1DD1}', '\u{1DD2}', '\u{1DD3}', '\u{1DD4}', '\u{1DD5}', '\u{1DD6}', '\u{1DD7}',
    '\u{1DD8}', '\u{1DD9}', '\u{1DDA}', '\u{1DDB}', '\u{1DDC}', '\u{1DDD}', '\u{1DDE}',
    '\u{1DDF}', '\u{1DE0}', '\u{1DE1}', '\u{1DE2}', '\u{1DE3}', '\u{1DE4}', '\u{1DE5}',
    '\u{1DE6}', '\u{1DFE}', '\u{20D0}', '\u{20D1}', '\u{20D4}', '\u{20D5}', '\u{20D6}',
    '\u{20D7}', '\u{20DB}', '\u{20DC}', '\u{20E1}', '\u{20E7}', '\u{20E9}', '\u{20F0}',
    '\u{2CEF}', '\u{2CF0}', '\u{2CF1}', '\u{2DE0}', '\u{2DE1}', '\u{2DE2}', '\u{2DE3}',
    '\u{2DE4}', '\u{2DE5}', '\u{2DE6}', '\u{2DE7}', '\u{2DE8}', '\u{2DE9}', '\u{2DEA}',
    '\u{2DEB}', '\u{2DEC}', '\u{2DED}', '\u{2DEE}', '\u{2DEF}', '\u{2DF0}', '\u{2DF1}',
    '\u{2DF2}', '\u{2DF3}', '\u{2DF4}', '\u{2DF5}', '\u{2DF6}', '\u{2DF7}', '\u{2DF8}',
    '\u{2DF9}', '\u{2DFA}', '\u{2DFB}', '\u{2DFC}', '\u{2DFD}', '\u{2DFE}', '\u{2DFF}',
    '\u{A66F}', '\u{A67C}', '\u{A67D}', '\u{A6F0}', '\u{A6F1}', '\u{A8E0}', '\u{A8E1}',
    '\u{A8E2}', '\u{A8E3}', '\u{A8E4}', '\u{A8E5}', '\u{A8E6}', '\u{A8E7}', '\u{A8E8}',
    '\u{A8E9}', '\u{A8EA}', '\u{A8EB}', '\u{A8EC}', '\u{A8ED}', '\u{A8EE}', '\u{A8EF}',
    '\u{A8F0}', '\u{A8F1}', '\u{AAB0}', '\u{AAB2}', '\u{AAB3}', '\u{AAB7}', '\u{AAB8}',
    '\u{AABE}', '\u{AABF}', '\u{AAC1}', '\u{FE20}', '\u{FE21}', '\u{FE22}', '\u{FE23}',
    '\u{FE24}', '\u{FE25}', '\u{FE26}', '\u{10A0F}', '\u{10A38}', '\u{1D185}', '\u{1D186}',
    '\u{1D187}', '\u{1D188}', '\u{1D189}', '\u{1D1AA}', '\u{1D1AB}', '\u{1D1AC}', '\u{1D1AD}',
    '\u{1D242}', '\u{1D243}', '\u{1D244}',
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlaceholderCell {
    image_id: u32,
    row: u32,
    col: u32,
}

/// Decodes `cell` if it is a placeholder; `left` is the decoded placeholder just left of it.
fn decode(cell: &Cell, left: Option<PlaceholderCell>) -> Option<PlaceholderCell> {
    if cell.c != PLACEHOLDER {
        return None;
    }
    let low_id = match cell.fg {
        Color::Spec(rgb) => ((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32,
        Color::Indexed(index) => index as u32,
        Color::Named(_) => return None,
    };

    let mut values = [None; 3];
    if let CharOrStr::Str(content) = cell.raw_content() {
        for (value, c) in values.iter_mut().zip(content.chars().skip(1)) {
            *value = ROW_COLUMN_DIACRITICS
                .binary_search(&c)
                .ok()
                .map(|index| index as u32);
        }
    }
    let [row, col, msb] = values;

    // The left cell continues into this one if it shows the same image and, where given, the same
    // row and the next column.
    let left = left.filter(|left| {
        left.image_id & 0x00FF_FFFF == low_id
            && row.is_none_or(|row| row == left.row)
            && col.is_none_or(|col| col == left.col + 1)
    });
    let row = row.or(left.map(|left| left.row)).unwrap_or(0);
    let col = col.or(left.map(|left| left.col + 1)).unwrap_or(0);
    let msb = msb.or(left.map(|left| left.image_id >> 24)).unwrap_or(0);

    let image_id = (msb << 24) | low_id;
    (image_id != 0).then_some(PlaceholderCell { image_id, row, col })
}

/// A rectangle of placeholder cells, `len` columns by `rows` screen rows, that show the
/// corresponding rectangle of tiles of one image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlaceholderRun {
    pub screen_row: usize,
    pub screen_col: usize,
    pub len: usize,
    pub rows: usize,
    pub image_id: u32,
    /// The tile the top left cell shows.
    pub tile_row: u32,
    pub tile_col: u32,
}

/// Collects a drawn grid's placeholder cells into runs, fed one cell at a time in row-major order.
#[derive(Debug)]
pub struct PlaceholderRunBuilder {
    enabled: bool,
    runs: Vec<PlaceholderRun>,
    last: Option<(usize, usize, PlaceholderCell)>,
}

impl PlaceholderRunBuilder {
    /// A builder for one drawn grid. Unless `enabled` (kitty images are on), it finds no
    /// placeholders and leaves every cell as it is.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            runs: Vec::new(),
            last: None,
        }
    }

    /// Records the cell at (`screen_row`, `screen_col`). For a placeholder, returns the cell to
    /// draw in its place: an empty cell with its background, since the image covers it.
    ///
    /// Inlined into the renderer's loop over every cell, where a cell that is not a placeholder
    /// should cost one comparison rather than a call into this crate.
    #[inline]
    pub fn push(&mut self, screen_row: usize, screen_col: usize, cell: &Cell) -> Option<Cell> {
        (cell.c == PLACEHOLDER && self.enabled)
            .then(|| self.push_placeholder(screen_row, screen_col, cell))
    }

    fn push_placeholder(&mut self, screen_row: usize, screen_col: usize, cell: &Cell) -> Cell {
        let mut blank = Cell::default();
        blank.bg = cell.bg;

        let left = self.last.and_then(|(row, col, left)| {
            (row == screen_row && col + 1 == screen_col).then_some(left)
        });
        self.last = decode(cell, left).map(|decoded| (screen_row, screen_col, decoded));
        let Some((_, _, decoded)) = self.last else {
            return blank;
        };

        let continues_run = left.is_some_and(|left| {
            left.image_id == decoded.image_id
                && left.row == decoded.row
                && left.col + 1 == decoded.col
        });
        match self.runs.last_mut() {
            Some(run) if continues_run => run.len += 1,
            _ => self.runs.push(PlaceholderRun {
                screen_row,
                screen_col,
                len: 1,
                rows: 1,
                image_id: decoded.image_id,
                tile_row: decoded.row,
                tile_col: decoded.col,
            }),
        }
        blank
    }

    /// The runs found, a run merged with the one below it when that continues its tiles, so
    /// that an image shown whole is one run.
    pub fn finish(self) -> Vec<PlaceholderRun> {
        let mut merged: Vec<PlaceholderRun> = Vec::with_capacity(self.runs.len());
        // The index of each merged run, by the one-row run that would continue it.
        let mut continued_by: HashMap<PlaceholderRun, usize> = HashMap::new();
        for run in self.runs {
            let index = match continued_by.remove(&run) {
                Some(index) => {
                    merged[index].rows += 1;
                    index
                }
                None => {
                    merged.push(run);
                    merged.len() - 1
                }
            };
            let below = PlaceholderRun {
                screen_row: run.screen_row + 1,
                tile_row: run.tile_row + 1,
                ..run
            };
            continued_by.insert(below, index);
        }
        merged
    }
}

#[cfg(test)]
#[path = "kitty_unicode_placeholder_tests.rs"]
mod tests;
