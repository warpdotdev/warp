use pathfinder_color::ColorU;

use super::*;
use crate::model::ansi::NamedColor;

/// A placeholder cell in `fg` with the diacritics for `values` (row, column, id byte).
fn placeholder(fg: Color, values: &[u32]) -> Cell {
    let mut cell = Cell::default();
    cell.c = PLACEHOLDER;
    cell.fg = fg;
    for &value in values {
        cell.push_zerowidth(ROW_COLUMN_DIACRITICS[value as usize], false);
    }
    cell
}

fn decoded(image_id: u32, row: u32, col: u32) -> Option<PlaceholderCell> {
    Some(PlaceholderCell { image_id, row, col })
}

#[test]
fn test_diacritics_are_sorted() {
    assert!(
        ROW_COLUMN_DIACRITICS
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    );
}

#[test]
fn test_decode_reads_id_and_tile() {
    assert_eq!(
        decode(&placeholder(Color::Indexed(42), &[3, 7]), None),
        decoded(42, 3, 7)
    );
    let rgb = Color::Spec(ColorU::new(0x12, 0x34, 0x56, 0xff));
    assert_eq!(
        decode(&placeholder(rgb, &[0, 1, 2]), None),
        decoded(0x0212_3456, 0, 1)
    );
}

#[test]
fn test_decode_rejects_cells_without_an_image() {
    let default_fg = Color::Named(NamedColor::Foreground);
    assert_eq!(decode(&placeholder(default_fg, &[0, 0]), None), None);
    assert_eq!(decode(&placeholder(Color::Indexed(0), &[0, 0]), None), None);
    assert_eq!(decode(&Cell::default(), None), None);
}

#[test]
fn test_decode_infers_omitted_diacritics_from_left_cell() {
    let left = decoded(0x0300_0005, 4, 9);
    assert_eq!(
        decode(&placeholder(Color::Indexed(5), &[]), left),
        decoded(0x0300_0005, 4, 10)
    );
    assert_eq!(
        decode(&placeholder(Color::Indexed(5), &[4]), left),
        decoded(0x0300_0005, 4, 10)
    );
    // Another row or another image does not continue the left cell.
    assert_eq!(
        decode(&placeholder(Color::Indexed(5), &[5]), left),
        decoded(5, 5, 0)
    );
    assert_eq!(
        decode(&placeholder(Color::Indexed(6), &[]), left),
        decoded(6, 0, 0)
    );
}

#[test]
fn test_decode_inherits_id_byte_only_from_the_previous_tile() {
    let left = decoded(0x0300_0005, 4, 9);
    assert_eq!(
        decode(&placeholder(Color::Indexed(5), &[4, 10]), left),
        decoded(0x0300_0005, 4, 10)
    );
    // A row and column that don't follow the left cell's start afresh, with no id byte.
    assert_eq!(
        decode(&placeholder(Color::Indexed(5), &[4, 12]), left),
        decoded(5, 4, 12)
    );
    assert_eq!(
        decode(&placeholder(Color::Indexed(5), &[4, 10, 1]), left),
        decoded(0x0100_0005, 4, 10)
    );
}

#[test]
fn test_run_builder_merges_consecutive_tiles() {
    let mut builder = PlaceholderRunBuilder::new(true);
    assert!(builder.push(2, 0, &Cell::default()).is_none());
    let blank = builder.push(2, 1, &placeholder(Color::Indexed(9), &[1, 0]));
    assert!(blank.is_some_and(|cell| cell.is_empty()));
    builder.push(2, 2, &placeholder(Color::Indexed(9), &[]));
    builder.push(3, 1, &placeholder(Color::Indexed(9), &[2, 0]));
    // A tile that does not follow on starts a new run.
    builder.push(3, 2, &placeholder(Color::Indexed(9), &[2, 5]));

    let run = |screen_row, screen_col, len, tile_row, tile_col| PlaceholderRun {
        screen_row,
        screen_col,
        len,
        rows: 1,
        image_id: 9,
        tile_row,
        tile_col,
    };
    assert_eq!(
        builder.finish(),
        vec![run(2, 1, 2, 1, 0), run(3, 1, 1, 2, 0), run(3, 2, 1, 2, 5)]
    );
}

#[test]
fn test_run_builder_splits_runs_at_gaps_and_repeated_tiles() {
    let mut builder = PlaceholderRunBuilder::new(true);
    let mut text = Cell::default();
    text.c = 'x';
    builder.push(0, 0, &placeholder(Color::Indexed(9), &[0, 0]));
    builder.push(0, 1, &placeholder(Color::Indexed(9), &[]));
    builder.push(0, 2, &text);
    // After a gap, a cell without diacritics starts again at the first tile.
    builder.push(0, 3, &placeholder(Color::Indexed(9), &[]));
    // The same tile again is a run of its own.
    builder.push(0, 4, &placeholder(Color::Indexed(9), &[0, 0]));
    builder.push(0, 5, &placeholder(Color::Indexed(9), &[]));

    let run = |screen_col, len| PlaceholderRun {
        screen_row: 0,
        screen_col,
        len,
        rows: 1,
        image_id: 9,
        tile_row: 0,
        tile_col: 0,
    };
    assert_eq!(builder.finish(), vec![run(0, 2), run(3, 1), run(4, 2)]);
}

#[test]
fn test_run_builder_merges_rows_that_continue_the_tiles_above() {
    /// Three cells showing tiles 0 to 2 of `tile_row`.
    fn image_row(
        builder: &mut PlaceholderRunBuilder,
        screen_row: usize,
        screen_col: usize,
        tile_row: u32,
    ) {
        for col in 0..3 {
            let cell = placeholder(Color::Indexed(9), &[tile_row, col]);
            builder.push(screen_row, screen_col + col as usize, &cell);
        }
    }
    let mut builder = PlaceholderRunBuilder::new(true);
    image_row(&mut builder, 5, 2, 0);
    image_row(&mut builder, 6, 2, 1);
    image_row(&mut builder, 7, 2, 2);
    // Shifted by a column, or after a screen row without the image: not continuations.
    image_row(&mut builder, 8, 3, 3);
    image_row(&mut builder, 10, 3, 4);

    let run = |screen_row, screen_col, rows, tile_row| PlaceholderRun {
        screen_row,
        screen_col,
        len: 3,
        rows,
        image_id: 9,
        tile_row,
        tile_col: 0,
    };
    assert_eq!(
        builder.finish(),
        vec![run(5, 2, 3, 0), run(8, 3, 1, 3), run(10, 3, 1, 4)]
    );
}

#[test]
fn test_run_builder_ignores_placeholders_when_disabled() {
    let mut builder = PlaceholderRunBuilder::new(false);
    assert!(
        builder
            .push(0, 0, &placeholder(Color::Indexed(9), &[0, 0]))
            .is_none()
    );
    assert_eq!(builder.finish(), vec![]);
}
