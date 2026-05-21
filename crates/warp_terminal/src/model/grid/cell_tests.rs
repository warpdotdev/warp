// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

use super::{Cell, LineLength};

use crate::model::{
    char_or_str::CharOrStr,
    grid::{
        cell::{Flags, MAX_GRAPHEME_BYTES},
        row::Row,
    },
};

#[test]
fn verify_cell_size() {
    // If this test fails, then something has changed about Cell that alters its memory layout and
    // causes it to be a different size than expected. Verify carefully if that is expected before
    // updating the constant value.
    const EXPECTED_CELL_SIZE_IN_BYTES: usize = 24;

    assert_eq!(std::mem::size_of::<Cell>(), EXPECTED_CELL_SIZE_IN_BYTES);
}

#[test]
fn line_length_works() {
    let mut row = Row::new(10);
    row[5].c = 'a';

    assert_eq!(row.line_length(), 6);
}

#[test]
fn line_length_works_with_wrapline() {
    let mut row = Row::new(10);
    row[9].flags.insert(super::Flags::WRAPLINE);

    assert_eq!(row.line_length(), 10);
}

#[test]
fn line_length_works_with_empty_line() {
    let mut row = Row::new(1);
    row.shrink(0);
    assert_eq!(row.line_length(), 0);
}

#[test]
fn test_contains_cell_decorations() {
    assert!(Flags::UNDERLINE.intersects(Flags::CELL_DECORATIONS));
    assert!(Flags::STRIKEOUT.intersects(Flags::CELL_DECORATIONS));
    assert!(Flags::DOUBLE_UNDERLINE.intersects(Flags::CELL_DECORATIONS));
}

#[test]
fn push_zerowidth_caps_accumulated_grapheme() {
    // A ZWJ (U+200D) is three bytes in UTF-8.  Push enough of them to go
    // well past `MAX_GRAPHEME_BYTES`, and verify that the accumulated
    // content stops growing at the cap.
    let mut cell = Cell {
        c: 'e',
        ..Cell::default()
    };
    let zwj = '\u{200D}';
    let zwj_bytes = zwj.len_utf8();
    let pushes = (MAX_GRAPHEME_BYTES * 10) / zwj_bytes;
    for _ in 0..pushes {
        cell.push_zerowidth(zwj, /* log_long_grapheme_warnings */ true);
    }

    let CharOrStr::Str(content) = cell.raw_content() else {
        panic!("cell should have accumulated zero-width content as a string");
    };
    // The stored content is "base char + N zero-width chars".  The total
    // length in bytes must fit within the cap.
    assert!(
        content.len() <= MAX_GRAPHEME_BYTES,
        "expected stored content length {} to be <= cap {}",
        content.len(),
        MAX_GRAPHEME_BYTES,
    );
    // We also want the cap to actually be approached: the stored content
    // should contain many zero-width characters and not have been truncated
    // early.
    let zero_width_bytes = content.len() - 'e'.len_utf8();
    let zero_width_count = zero_width_bytes / zwj_bytes;
    assert!(
        zero_width_count >= 80,
        "expected at least 80 zero-width chars to fit, got {zero_width_count}",
    );
    assert!(content.starts_with('e'));
    assert!(content[1..].chars().all(|c| c == zwj));
}

#[test]
fn push_zerowidth_seeds_base_char_on_first_push() {
    // Before any zero-width char is pushed, the cell's raw content is just
    // the base char.  After the first push, the content becomes a string
    // consisting of the base char plus the pushed zero-width char.
    let mut cell = Cell {
        c: 'x',
        ..Cell::default()
    };
    assert_eq!(cell.raw_content(), CharOrStr::Char('x'));

    cell.push_zerowidth('\u{0301}', /* log_long_grapheme_warnings */ true);
    assert_eq!(cell.raw_content(), CharOrStr::Str("x\u{0301}"));
}
