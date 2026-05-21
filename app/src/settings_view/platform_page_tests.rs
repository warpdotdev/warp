use super::{
    api_key_table_min_non_resizable_columns_width, compute_api_key_name_column_max_width,
    API_KEY_KEY_COLUMN_WIDTH, API_KEY_NAME_COLUMN_MIN_WIDTH, API_KEY_TABLE_LAYOUT_SAFETY_PADDING,
    API_KEY_TABLE_MIN_SCOPE_COLUMN_WIDTH, SETTINGS_PAGE_HORIZONTAL_PADDING,
    SETTINGS_PAGE_MAX_CONTENT_WIDTH, SETTINGS_SECTION_BORDER_WIDTH, SETTINGS_SIDEBAR_WIDTH_DEFAULT,
};

fn table_width_chrome() -> f32 {
    SETTINGS_SIDEBAR_WIDTH_DEFAULT
        + SETTINGS_SECTION_BORDER_WIDTH
        + SETTINGS_PAGE_HORIZONTAL_PADDING
        + API_KEY_TABLE_LAYOUT_SAFETY_PADDING
}

fn assert_f32_eq(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < f32::EPSILON,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn key_column_width_is_fixed_and_narrow() {
    assert_f32_eq(API_KEY_KEY_COLUMN_WIDTH, 120.);
}

#[test]
fn name_column_max_width_reserves_non_resizable_columns_without_scope() {
    let min_non_resizable_columns_width = api_key_table_min_non_resizable_columns_width(false);
    let max_width = compute_api_key_name_column_max_width(
        2000.,
        API_KEY_NAME_COLUMN_MIN_WIDTH,
        min_non_resizable_columns_width,
        table_width_chrome(),
    );

    let expected = SETTINGS_PAGE_MAX_CONTENT_WIDTH - min_non_resizable_columns_width;
    assert_f32_eq(max_width, expected);
}

#[test]
fn name_column_max_width_reserves_extra_scope_budget_when_scope_enabled() {
    let min_without_scope = api_key_table_min_non_resizable_columns_width(false);
    let min_with_scope = api_key_table_min_non_resizable_columns_width(true);
    assert_f32_eq(
        min_with_scope - min_without_scope,
        API_KEY_TABLE_MIN_SCOPE_COLUMN_WIDTH,
    );

    let max_without_scope = compute_api_key_name_column_max_width(
        2000.,
        API_KEY_NAME_COLUMN_MIN_WIDTH,
        min_without_scope,
        table_width_chrome(),
    );
    let max_with_scope = compute_api_key_name_column_max_width(
        2000.,
        API_KEY_NAME_COLUMN_MIN_WIDTH,
        min_with_scope,
        table_width_chrome(),
    );
    assert_f32_eq(
        max_without_scope - max_with_scope,
        API_KEY_TABLE_MIN_SCOPE_COLUMN_WIDTH,
    );
}

#[test]
fn name_column_max_width_never_drops_below_min_width() {
    let max_width = compute_api_key_name_column_max_width(
        200.,
        API_KEY_NAME_COLUMN_MIN_WIDTH,
        api_key_table_min_non_resizable_columns_width(false),
        table_width_chrome(),
    );
    assert_f32_eq(max_width, API_KEY_NAME_COLUMN_MIN_WIDTH);
}
