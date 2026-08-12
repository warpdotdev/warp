use warpui_core::color::ColorU;

use super::super::ansi::NamedColor;
use super::*;

fn placeholder_cell(fg: Color) -> Cell {
    let mut cell = Cell::default();
    cell.c = PLACEHOLDER_CHAR;
    cell.fg = fg;
    cell
}

#[test]
fn diacritic_table_is_strictly_ascending() {
    assert!(
        ROW_COLUMN_DIACRITICS.is_sorted_by(|a, b| a < b),
        "table must be strictly ascending for binary search"
    );
}

#[test]
fn diacritic_index_maps_table_entries_to_their_position() {
    assert_eq!(diacritic_index('\u{0305}'), Some(0));
    assert_eq!(diacritic_index('\u{030D}'), Some(1));
    assert_eq!(diacritic_index('\u{1D244}'), Some(296));
}

#[test]
fn diacritic_index_rejects_characters_outside_the_table() {
    // U+0301 (combining acute) is deliberately excluded from kitty's table.
    assert_eq!(diacritic_index('\u{0301}'), None);
    assert_eq!(diacritic_index('a'), None);
}

#[test]
fn decodes_image_id_from_rgb_foreground() {
    let cell = placeholder_cell(Color::Spec(ColorU::new(0x12, 0x34, 0x56, 255)));

    let decoded = decode_placeholder(&cell).expect("placeholder cell should decode");

    assert_eq!(decoded.image_id, 0x123456);
    assert_eq!(decoded.row, None);
    assert_eq!(decoded.col, None);
}

#[test]
fn decodes_image_id_from_indexed_foreground() {
    let cell = placeholder_cell(Color::Indexed(42));

    let decoded = decode_placeholder(&cell).expect("placeholder cell should decode");

    assert_eq!(decoded.image_id, 42);
}

#[test]
fn rejects_named_foreground_color() {
    let cell = placeholder_cell(Color::Named(NamedColor::Foreground));

    assert_eq!(decode_placeholder(&cell), None);
}

#[test]
fn rejects_non_placeholder_characters() {
    let mut cell = Cell::default();
    cell.c = 'a';
    cell.fg = Color::Indexed(42);

    assert_eq!(decode_placeholder(&cell), None);
}

#[test]
fn decodes_tile_row_and_column_from_diacritics() {
    let mut cell = placeholder_cell(Color::Indexed(1));
    cell.push_zerowidth('\u{030D}', true); // row 1
    cell.push_zerowidth('\u{030E}', true); // column 2

    let decoded = decode_placeholder(&cell).expect("placeholder cell should decode");

    assert_eq!(decoded.row, Some(1));
    assert_eq!(decoded.col, Some(2));
    assert_eq!(decoded.image_id, 1);
}

#[test]
fn third_diacritic_supplies_the_image_id_high_byte() {
    let mut cell = placeholder_cell(Color::Spec(ColorU::new(0x12, 0x34, 0x56, 255)));
    cell.push_zerowidth('\u{0305}', true); // row 0
    cell.push_zerowidth('\u{0305}', true); // column 0
    cell.push_zerowidth('\u{0310}', true); // id high byte 3

    let decoded = decode_placeholder(&cell).expect("placeholder cell should decode");

    assert_eq!(decoded.image_id, 0x03123456);
    assert_eq!(decoded.row, Some(0));
    assert_eq!(decoded.col, Some(0));
}

#[test]
fn row_only_diacritic_leaves_column_unspecified() {
    let mut cell = placeholder_cell(Color::Indexed(1));
    cell.push_zerowidth('\u{030D}', true); // row 1

    let decoded = decode_placeholder(&cell).expect("placeholder cell should decode");

    assert_eq!(decoded.row, Some(1));
    assert_eq!(decoded.col, None);
}

#[test]
fn invalid_diacritic_stops_reading_and_leaves_position_unspecified() {
    let mut cell = placeholder_cell(Color::Indexed(1));
    cell.push_zerowidth('\u{0301}', true); // not in kitty's table

    let decoded = decode_placeholder(&cell).expect("placeholder cell should decode");

    assert_eq!(decoded.row, None);
    assert_eq!(decoded.col, None);
}

// Diacritics by table index: 0 = U+0305, 1 = U+030D, 2 = U+030E, 3 = U+0310.
fn tile_cell(image_id: u8, diacritics: &[char]) -> Cell {
    let mut cell = placeholder_cell(Color::Indexed(image_id));
    for &diacritic in diacritics {
        cell.push_zerowidth(diacritic, true);
    }
    cell
}

#[test]
fn explicit_consecutive_tiles_form_one_run() {
    let row = [
        tile_cell(1, &['\u{0305}', '\u{0305}']), // (0, 0)
        tile_cell(1, &['\u{0305}', '\u{030D}']), // (0, 1)
        tile_cell(1, &['\u{0305}', '\u{030E}']), // (0, 2)
    ];

    assert_eq!(
        placeholder_runs_in_row(&row),
        vec![PlaceholderRun {
            image_id: 1,
            src_row: 0,
            src_col_start: 0,
            len: 3,
            screen_col: 0,
        }]
    );
}

#[test]
fn cells_without_diacritics_continue_their_left_neighbor() {
    let row = [
        tile_cell(1, &['\u{030D}', '\u{0305}']), // (1, 0)
        tile_cell(1, &[]),                       // continues as (1, 1)
        tile_cell(1, &[]),                       // continues as (1, 2)
    ];

    assert_eq!(
        placeholder_runs_in_row(&row),
        vec![PlaceholderRun {
            image_id: 1,
            src_row: 1,
            src_col_start: 0,
            len: 3,
            screen_col: 0,
        }]
    );
}

#[test]
fn row_only_diacritics_start_at_column_zero_and_continue() {
    let row = [
        tile_cell(1, &['\u{030D}']), // (1, 0)
        tile_cell(1, &['\u{030D}']), // continues as (1, 1)
    ];

    assert_eq!(
        placeholder_runs_in_row(&row),
        vec![PlaceholderRun {
            image_id: 1,
            src_row: 1,
            src_col_start: 0,
            len: 2,
            screen_col: 0,
        }]
    );
}

#[test]
fn cells_without_any_diacritics_start_at_the_placement_origin() {
    let row = [tile_cell(7, &[])];

    assert_eq!(
        placeholder_runs_in_row(&row),
        vec![PlaceholderRun {
            image_id: 7,
            src_row: 0,
            src_col_start: 0,
            len: 1,
            screen_col: 0,
        }]
    );
}

#[test]
fn different_image_ids_split_runs() {
    let row = [
        tile_cell(1, &['\u{0305}', '\u{0305}']), // image 1, (0, 0)
        tile_cell(2, &['\u{0305}', '\u{0305}']), // image 2, (0, 0)
    ];

    assert_eq!(
        placeholder_runs_in_row(&row),
        vec![
            PlaceholderRun {
                image_id: 1,
                src_row: 0,
                src_col_start: 0,
                len: 1,
                screen_col: 0,
            },
            PlaceholderRun {
                image_id: 2,
                src_row: 0,
                src_col_start: 0,
                len: 1,
                screen_col: 1,
            },
        ]
    );
}

#[test]
fn non_placeholder_gap_resets_continuation() {
    let mut text_cell = Cell::default();
    text_cell.c = 'x';
    let row = [
        tile_cell(1, &['\u{030D}', '\u{030D}']), // (1, 1)
        text_cell,
        tile_cell(1, &[]), // no left neighbor: starts over at (0, 0)
    ];

    assert_eq!(
        placeholder_runs_in_row(&row),
        vec![
            PlaceholderRun {
                image_id: 1,
                src_row: 1,
                src_col_start: 1,
                len: 1,
                screen_col: 0,
            },
            PlaceholderRun {
                image_id: 1,
                src_row: 0,
                src_col_start: 0,
                len: 1,
                screen_col: 2,
            },
        ]
    );
}

#[test]
fn duplicated_tiles_start_a_new_run() {
    let row = [
        tile_cell(1, &['\u{0305}', '\u{030D}']), // (0, 1)
        tile_cell(1, &['\u{0305}', '\u{030D}']), // (0, 1) again
    ];

    assert_eq!(
        placeholder_runs_in_row(&row),
        vec![
            PlaceholderRun {
                image_id: 1,
                src_row: 0,
                src_col_start: 1,
                len: 1,
                screen_col: 0,
            },
            PlaceholderRun {
                image_id: 1,
                src_row: 0,
                src_col_start: 1,
                len: 1,
                screen_col: 1,
            },
        ]
    );
}
