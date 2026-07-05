use itertools::Itertools;
use testing::{assert_rows_equal, ToRows as _};

use super::*;
use crate::model::char_or_str::CharOrStr;
use crate::model::grid::cell::{Cell, Flags};

#[test]
fn test_row_iteration() {
    let storage = FlatStorage::from_content_using_rows("hello world\n", 7, Some(2));

    let mut rows = storage.rows_from(0);

    let row1 = rows
        .next()
        .expect("should be able to get first row from storage");
    assert_eq!(row1.occ, 7);
    assert_eq!(row1[0].c, 'h');
    assert_eq!(row1[6].c, 'w');

    let row2 = rows
        .next()
        .expect("should be able to get first row from storage");
    assert_eq!(row2.occ, 4);
    assert_eq!(row2[0].c, 'o');
    assert_eq!(row2[3].c, 'd');

    assert!(rows.next().is_none());
}

#[test]
fn test_row_with_double_width_char() {
    let storage = FlatStorage::from_content_using_rows("hi 😀 hello\n", 6, Some(2));

    let mut rows = storage.rows_from(0);

    let row1 = rows
        .next()
        .expect("should be able to get first row from storage");
    assert_eq!(row1.occ, 6);
    assert_eq!(row1[0].c, 'h');
    assert_eq!(row1[3].c, '😀');
    assert!(row1[4].flags().contains(Flags::WIDE_CHAR_SPACER));
    assert_eq!(row1[5].c, ' ');

    let row2 = rows
        .next()
        .expect("should be able to get first row from storage");
    assert_eq!(row2.occ, 5);
    assert_eq!(row2[0].c, 'h');

    assert!(rows.next().is_none());
}

/// This test validates our handling of complex emoji sequences.
///
/// The three graphemes here are comprised of a number of Unicode characters.
/// Below are the individual characters that comprise the test string, with
/// "---" denoting how the string gets segmented into graphemes.
///
///  1. 🧑  1F9D1   ADULT
///  2.     1F3FF   EMOJI MODIFIER FITZPATRICK TYPE-6
///  3. ‍    200D    ZERO WIDTH JOINER
///  4. 🦰  1F9B0   EMOJI COMPONENT RED HAIR
///  ---
///  1. 👩  1F469   WOMAN
///  2. ‍    200D    ZERO WIDTH JOINER
///  3. 🦲  1F9B2   EMOJI COMPONENT BALD
///  ---
///  1. 🧔  1F9D4   BEARDED PERSON
///  2. 🏿   1F3FF   EMOJI MODIFIER FITZPATRICK TYPE-6
///  3. ‍    200D    ZERO WIDTH JOINER
///  4. ♂   2642    MALE SIGN
///  5. ️    FE0F    VARIATION SELECTOR-16
#[test]
#[ignore = "will not pass until using a version of unicode-width that includes commit afab363"]
fn test_row_with_complex_emoji() {
    let storage = FlatStorage::from_content_using_rows("🧑🏿‍🦰👩‍🦲🧔🏿‍♂️", 6, Some(1));

    let mut rows = storage.rows_from(0);
    let row1 = rows
        .next()
        .expect("should be able to get first row from storage");
    assert_eq!(row1.occ, 6);

    assert_eq!(row1[0].c, '🧑');
    assert!(matches!(
        row1[0].content_for_display(),
        CharOrStr::Str("🧑🏿‍🦰")
    ));

    assert!(row1[1].flags().contains(Flags::WIDE_CHAR_SPACER));
}

#[test]
fn test_push_rows_with_color() {
    let mut storage = FlatStorage::new(5, None, Some(2));

    let mut fg_cell = Cell::default();
    fg_cell.c = 'f';

    let mut red_cell = Cell::default();
    red_cell.c = 'r';
    red_cell.fg = ansi::Color::Named(ansi::NamedColor::Red);

    let row = Row::from_vec(
        vec![
            Cell::default(),
            Cell::default(),
            red_cell.clone(),
            red_cell,
            Cell::default(),
        ],
        5,
    );
    storage.push_rows([&row]);

    assert_eq!(storage.rows_from(0).next().unwrap().as_ref(), &row);
}

#[test]
fn test_push_rows_with_color_and_multibyte_chars() {
    let mut storage = FlatStorage::new(5, None, Some(2));

    let mut fg_cell = Cell::default();
    fg_cell.c = '❤';

    let mut red_cell = Cell::default();
    red_cell.c = 'r';
    red_cell.fg = ansi::Color::Named(ansi::NamedColor::Red);

    let row = Row::from_vec(
        vec![
            fg_cell.clone(),
            fg_cell.clone(),
            red_cell.clone(),
            red_cell,
            fg_cell,
        ],
        5,
    );
    storage.push_rows([&row]);

    assert_eq!(storage.rows_from(0).next().unwrap().as_ref(), &row);
}

#[test]
fn test_row_roundtrip_and_resize() {
    let num_cols = 5;
    let rows = "😀😃😄ag\na😁😆~!!\n😅sdf😂\n".to_rows(num_cols);

    // Build FlatStorage from the set of rows.
    let mut storage = FlatStorage::new(num_cols, None, None);
    storage.push_rows(&rows);

    // Make sure the generated rows match the original input.
    let flat_rows = storage
        .rows_from(0)
        .map(|row| row.as_ref().clone())
        .collect_vec();

    assert_rows_equal(&flat_rows, &rows);

    // "Resize" the storage, keeping the number of columns the same.
    storage.set_columns(num_cols);

    // Make sure the generated rows match the original input.
    let flat_rows = storage
        .rows_from(0)
        .map(|row| row.as_ref().clone())
        .collect_vec();

    assert_rows_equal(&flat_rows, &rows);
}

#[test]
fn test_styling_change_within_trailing_empty_cells() {
    let num_cols = 5;
    let mut rows = "a\nb\n".to_rows(num_cols);

    // Make the final cell in the first row bold.
    rows[0][num_cols - 1].flags.insert(Flags::BOLD);

    // Push the rows into storage.  This should produce a first row that is 5
    // cells long (the "a" followed by 3 empty cells followed by a bold empty
    // cell) and then clear the bold styling on the first cell of the second
    // line.
    let mut storage = FlatStorage::new(num_cols, None, None);
    storage.push_rows(&rows);

    let flat_rows = storage
        .rows_from(0)
        .map(|row| row.as_ref().clone())
        .collect_vec();

    // The first row's content should be 5 characters + a trailing newline.
    assert_eq!(flat_rows[0][0].c, 'a');
    assert_eq!(flat_rows[0][1].c, '\0');
    assert_eq!(flat_rows[0][2].c, '\0');
    assert_eq!(flat_rows[0][3].c, '\0');
    assert_eq!(flat_rows[0][4].c, '\0');
    assert!(!flat_rows[0][4].flags.contains(Flags::WRAPLINE));

    // The final cell in the first row should be bold, but the first cell in
    // the second row should not.
    assert!(flat_rows[0][num_cols - 1].flags.intersects(Flags::BOLD));
    assert!(!flat_rows[1][0].flags.intersects(Flags::BOLD));
}

/// Phase 13 (Telugu variable-width-cells plan): `GridHandler::carry_indic_word`
/// moves an in-progress Indic word to the next line instead of splitting it
/// mid-word, and marks the soft-wrap boundary with `WRAPLINE` on the cell
/// right before the word rather than the row's physical last cell (the tail
/// after that point is left blank -- the word moved away). This is a NEW
/// kind of soft-wrapped row: unlike every wrap this crate produced before,
/// it isn't fully occupied edge-to-edge. Confirms the row-commit soft-wrap
/// check (which used to hardcode `row[columns - 1]`) now correctly
/// recognizes this row as continuing rather than inserting a spurious
/// trailing newline that would split the carried word from its prefix once
/// the row scrolls into flat storage.
#[test]
fn test_row_soft_wraps_with_wrapline_before_physical_last_cell() {
    let num_cols = 8;
    let mut rows = vec![Row::new(num_cols), Row::new(num_cols)];

    // Row 0: "aaa" (0-2), WRAPLINE on col 2 (not the physical last cell,
    // col 7) -- exactly what `carry_indic_word` produces after erasing a
    // word's already-written cells from col 3 onward. `occ` is bumped to
    // the full row width, mirroring the erase touching every cell up to
    // the row's end (the same as `carry_indic_word`'s real behaviour).
    for (i, c) in "aaa".chars().enumerate() {
        rows[0][i].c = c;
    }
    rows[0][2].flags.insert(Flags::WRAPLINE);
    rows[0].occ = num_cols;

    // Row 1: the carried word's replayed content.
    for (i, c) in "bbbbbbb".chars().enumerate() {
        rows[1][i].c = c;
    }

    let mut storage = FlatStorage::new(num_cols, None, None);
    storage.push_rows(&rows);

    assert!(
        storage.row_wraps(0),
        "a row whose WRAPLINE sits before its physical last cell (because its \
         tail was erased by an Indic word carry) must still be recognized as \
         soft-wrapped -- otherwise a spurious hard newline gets inserted, \
         splitting the carried word from its prefix once this row is read \
         back from flat storage (e.g. after scrolling into history)"
    );

    let flat_rows = storage
        .rows_from(0)
        .map(|row| row.as_ref().clone())
        .collect_vec();
    assert_eq!(flat_rows[0][0].c, 'a');
    assert_eq!(flat_rows[0][1].c, 'a');
    assert_eq!(flat_rows[0][2].c, 'a');
    assert_eq!(flat_rows[1][0].c, 'b');
}

#[test]
fn test_clear_after_truncate_front() {
    let num_cols = 20;
    let rows = "abcd\n789\n1 overflow\n2 overflow\n".to_rows(num_cols);

    let mut storage = FlatStorage::new(num_cols, Some(2), None);
    storage.push_rows(&rows);

    // We pushed 4 rows, and the limit is 2, so we should have truncated 2 rows.
    assert_eq!(storage.total_rows(), 2);
    assert_eq!(storage.num_truncated_rows(), 2);

    // Make sure the truncated rows are what we expect.
    assert_eq!(
        storage.rows_from(0).next().expect("should have a row")[0].c,
        '1'
    );
    assert_eq!(
        storage.rows_from(1).next().expect("should have a row")[0].c,
        '2'
    );

    // Clear flat storage, and ensure the state is as we expect.
    storage.clear();
    assert_eq!(storage.total_rows(), 0);
    // Should still have 2 truncated rows, as clearing storage doesn't affect
    // the number of rows we've truncated in total so far.
    assert_eq!(storage.num_truncated_rows(), 2);

    // Make sure we can push new rows.
    storage.push_rows(&rows);
    assert_eq!(storage.total_rows(), 2);
    assert_eq!(storage.num_truncated_rows(), 4);

    // Make sure remaining truncated rows are what we expect.
    assert_eq!(
        storage.rows_from(0).next().expect("should have a row")[0].c,
        '1'
    );
    assert_eq!(
        storage.rows_from(1).next().expect("should have a row")[0].c,
        '2'
    );
}

#[test]
fn test_clear_after_truncate_front_then_resize_and_push_does_not_panic() {
    let old_cols = 20;
    let new_cols = 21;
    let initial_content = "abcdefghijklmnopqrst\n".repeat(100);
    let rows = initial_content.as_str().to_rows(old_cols);

    let mut storage = FlatStorage::new(old_cols, Some(1), None);
    storage.push_rows(&rows);
    assert_eq!(storage.total_rows(), 1);

    storage.clear();
    storage.set_columns(new_cols);

    let new_rows = "new output\n".to_rows(new_cols);
    storage.push_rows(&new_rows);

    let row = storage
        .rows_from(0)
        .next()
        .expect("should materialize a row after clearing and resizing storage");
    assert_eq!(row[0].c, 'n');
}
