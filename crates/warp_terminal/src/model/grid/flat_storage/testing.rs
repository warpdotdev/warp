use unicode_segmentation::UnicodeSegmentation as _;

use super::FlatStorage;
use super::grapheme::Grapheme;
use crate::model::grid::cell::Flags;
use crate::model::grid::row::Row;

pub fn assert_rows_equal(actual: &[Row], expected: &[Row]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "Expected to have {} rows but got {}.  Got: {actual:?}; expected {expected:?}",
        expected.len(),
        actual.len()
    );
    actual.iter().zip(expected.iter())
        .enumerate()
        .for_each(|(row_idx, (actual, expected))| {
            assert_eq!(actual.occ, expected.occ, "Expected row {row_idx} to have {} occupied cells but got {}", expected.occ, actual.occ);
            for col_idx in 0..actual.occ {
                let actual = &actual[col_idx];
                let expected = &expected[col_idx];

                let actual_content = actual.raw_content();
                let expected_content = expected.raw_content();
                assert_eq!(actual_content, expected_content, "Expected ({row_idx}, {col_idx}) to contain {expected_content:?} but got {actual_content:?}");

                assert_eq!(actual.fg, expected.fg, "Expected ({row_idx}, {col_idx}) to have fg {:?} but got {:?}", expected.fg, actual.fg);
                assert_eq!(actual.bg, expected.bg, "Expected ({row_idx}, {col_idx}) to have bg {:?} but got {:?}", expected.bg, actual.bg);

                assert_eq!(actual.flags(), expected.flags(), "Expected ({row_idx}, {col_idx}) to have flags {:?} but got {:?}", expected.flags(), actual.flags());

                // TODO(vorporeal): Check CellExtra::end_of_prompt.
            }
        })
}

/// Extra functions on [`FlatStorage`] that are useful for testing purposes.
impl FlatStorage {
    pub fn push_rows_from_string(&mut self, string: &str) {
        let rows = string.to_rows(self.columns);
        self.push_rows(&rows);
    }

    pub fn from_content_using_rows(
        content: &str,
        columns: usize,
        initial_capacity: Option<usize>,
    ) -> Self {
        let mut storage = Self::new(columns, None, initial_capacity);
        storage.push_rows_from_string(content);
        storage
    }
}

/// Helper trait for converting something to a set of [`Row`]s.
///
/// Ultimately, we want to be testing equivalence of behavior between grid and
/// flat storage at a higher level, but this is still useful for quick-to-run
/// unit tests.
///
/// TODO(vorporeal): Eliminate this and use existing string-to-row machinery
/// (e.g.: session restoration logic) once that logic is in this crate.
pub trait ToRows {
    fn to_rows(&self, columns: usize) -> Vec<Row>;
}

impl ToRows for &str {
    #[allow(unused_assignments)]
    fn to_rows(&self, columns: usize) -> Vec<Row> {
        let mut rows = vec![Row::new(columns)];

        let mut needs_new_row = false;

        for grapheme in self.graphemes(true).map(Grapheme::new_from_str) {
            let mut cell_idx = rows.last().unwrap().occ;

            macro_rules! new_row {
                () => {
                    rows.push(Row::new(columns));
                    cell_idx = 0;
                };
            }

            if needs_new_row {
                needs_new_row = false;
                new_row!();
            }

            if grapheme.starts_new_row() {
                // Don't immediately start a new row - if this is the last
                // grapheme, we shouldn't append an extra empty row.
                needs_new_row = true;
                continue;
            }

            let cell_width = grapheme.cell_width();
            if cell_width == 0 {
                continue;
            }

            if cell_idx + cell_width as usize > columns {
                // If the row is full and we have another character to add to
                // it, set the WRAPLINE flag on the final cell and then start
                // a new row.
                let mut flags = Flags::WRAPLINE;
                if cell_width > 1 && cell_idx < columns {
                    cell_idx += cell_width as usize - 1;
                    // If this is a wide character and there isn't enough space
                    // for it, also add the appropriate flag.
                    flags |= Flags::LEADING_WIDE_CHAR_SPACER;
                }

                rows.last_mut().unwrap()[columns - 1].flags.insert(flags);
                new_row!();
            }

            let row = rows.last_mut().unwrap();
            let cell = &mut row[cell_idx];

            let mut chars = grapheme.chars();
            cell.c = chars.next().unwrap();
            // Add any remaining chars in the grapheme to the cell as zero-width
            // characters.
            chars.for_each(|c| cell.push_zerowidth(c, /* log_long_grapheme_warnings */ true));

            // If the grapheme takes up two cells, mark the following cell as
            // a spacer.
            if cell_width == 2 {
                row[cell_idx].flags.insert(Flags::WIDE_CHAR);
                row[cell_idx + 1].flags.insert(Flags::WIDE_CHAR_SPACER);
            }
        }

        // If the last row didn't end in a newline character, assert that it
        // was a full row, and add the WRAPLINE (soft wrap) flag.
        let last_row = rows.last_mut().unwrap();
        let occupied_cells = last_row.occ;
        if !needs_new_row {
            assert!(
                occupied_cells == last_row.len(),
                "All non-filled rows must explicitly end in a newline to avoid surprises and incorrect tests."
            );
            last_row
                .last_mut()
                .unwrap()
                .flags_mut()
                .insert(Flags::WRAPLINE);
        }

        rows
    }
}
