use itertools::Itertools;
use testing::{ToRows as _, assert_rows_equal};

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

#[test]
fn test_wide_char_hyperlink_spacer_survives_roundtrip() {
    use crate::model::ansi::control_sequence_parameters::Hyperlink;
    use crate::model::grid::HyperlinkRegistry;

    let mut registry = HyperlinkRegistry::new();
    let id = registry
        .intern(Hyperlink {
            id: None,
            uri: "https://example.com".to_string(),
        })
        .expect("registry should have capacity for one hyperlink");

    let mut wide = Cell::default();
    wide.c = '😀';
    wide.flags.insert(Flags::WIDE_CHAR);
    wide.set_hyperlink_id(Some(id));

    let mut spacer = Cell::default();
    spacer.flags.insert(Flags::WIDE_CHAR_SPACER);
    spacer.set_hyperlink_id(Some(id));

    let row = Row::from_vec(vec![wide, spacer, Cell::default()], 3);

    let mut storage = FlatStorage::new(5, None, Some(2));
    storage.push_rows([&row]);

    let flat = storage
        .rows_from(0)
        .next()
        .expect("should materialize the pushed row");
    assert_eq!(flat[0].c, '😀');
    assert!(flat[0].flags().contains(Flags::WIDE_CHAR));
    assert_eq!(flat[0].hyperlink_id(), Some(id));
    // Both halves of the wide glyph must stay hoverable after
    // rematerialization, not just the leading cell.
    assert!(flat[1].flags().contains(Flags::WIDE_CHAR_SPACER));
    assert_eq!(flat[1].hyperlink_id(), Some(id));
}

#[test]
fn test_blank_cells_before_hyperlink_are_not_clickable() {
    use crate::model::ansi::control_sequence_parameters::Hyperlink;
    use crate::model::grid::HyperlinkRegistry;

    let mut registry = HyperlinkRegistry::new();
    let id = registry
        .intern(Hyperlink {
            id: None,
            uri: "https://example.com".to_string(),
        })
        .expect("registry should have capacity for one hyperlink");

    let mut linked = Cell::default();
    linked.c = 'a';
    linked.set_hyperlink_id(Some(id));

    // Two blank cells precede the linked cell. They are backfilled when the
    // row is flattened, and must not inherit the hyperlink id.
    let row = Row::from_vec(
        vec![Cell::default(), Cell::default(), linked, Cell::default()],
        4,
    );

    let mut storage = FlatStorage::new(5, None, Some(2));
    storage.push_rows([&row]);

    let flat = storage
        .rows_from(0)
        .next()
        .expect("should materialize the pushed row");
    assert_eq!(flat[0].hyperlink_id(), None);
    assert_eq!(flat[1].hyperlink_id(), None);
    assert_eq!(flat[2].c, 'a');
    assert_eq!(flat[2].hyperlink_id(), Some(id));
}
