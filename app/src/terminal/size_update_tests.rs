use super::{SizeInfo, SizeUpdate};

#[test]
fn headless_layout_update_uses_cell_dimensions() {
    let last_size = SizeInfo::new_without_font_metrics(24, 120);
    let update = SizeUpdate::after_headless_layout(last_size, 8, 42);

    assert!(update.rows_or_columns_changed());
    assert_eq!(update.new_size().rows(), 8);
    assert_eq!(update.new_size().columns(), 42);
    assert_eq!(update.natural_rows(), 8);
    assert_eq!(update.natural_cols(), 42);
}

#[test]
fn unchanged_headless_layout_has_no_dimension_change() {
    let last_size = SizeInfo::new_without_font_metrics(8, 42);
    let update = SizeUpdate::after_headless_layout(last_size, 8, 42);

    assert!(!update.rows_or_columns_changed());
}
