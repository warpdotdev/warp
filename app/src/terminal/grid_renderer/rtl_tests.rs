use unicode_width::UnicodeWidthChar;

use super::*;

/// Builds one cell per character, attaching zero-width characters (e.g. Arabic diacritics) to the
/// preceding cell as the grid does.
fn cells(text: &str) -> Vec<Cell> {
    let mut cells: Vec<Cell> = Vec::new();
    for c in text.chars() {
        match (c.width(), cells.last_mut()) {
            (Some(0), Some(last)) => {
                last.push_zerowidth(c, false);
            }
            _ => {
                let mut cell = Cell::default();
                cell.c = c;
                cells.push(cell);
            }
        }
    }
    cells
}

#[test]
fn test_left_to_right_text_has_no_rtl_ranges() {
    assert!(rtl_column_ranges(&cells("ls -la /tmp")).is_empty());
    assert!(rtl_column_ranges(&cells("Привет 中文 12345")).is_empty());
    assert!(rtl_column_ranges(&[]).is_empty());
}

#[test]
fn test_arabic_text_is_a_single_range() {
    // Spaces between Arabic words belong to the run.
    assert_eq!(rtl_column_ranges(&cells("مرحبا بالعالم")), vec![0..13]);
}

#[test]
fn test_hebrew_text_is_a_single_range() {
    assert_eq!(rtl_column_ranges(&cells("שלום עולם")), vec![0..9]);
}

#[test]
fn test_range_keeps_the_columns_of_mixed_text() {
    // "file: " occupies columns 0..6 and ".txt" follows the Arabic run.
    assert_eq!(rtl_column_ranges(&cells("file: مرحبا.txt")), vec![6..11]);
}

#[test]
fn test_left_to_right_word_splits_rtl_ranges() {
    assert_eq!(
        rtl_column_ranges(&cells("مرحبا hello عالم")),
        vec![0..5, 12..16]
    );
}

#[test]
fn test_numbers_within_rtl_text_belong_to_the_range() {
    assert_eq!(rtl_column_ranges(&cells("عدد 42 ملفات")), vec![0..12]);
}

#[test]
fn test_trailing_whitespace_is_excluded() {
    assert_eq!(rtl_column_ranges(&cells("مرحبا   ")), vec![0..5]);
}

#[test]
fn test_diacritics_do_not_shift_columns() {
    // "مَرحبا" has a fatha attached to its first letter, so it still spans five columns.
    assert_eq!(rtl_column_ranges(&cells("ok مَرحبا ok")), vec![3..8]);
}
