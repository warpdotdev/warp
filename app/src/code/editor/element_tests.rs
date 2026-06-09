use super::*;
fn config(
    mode: CodeEditorLineNumberMode,
    starting_line_number: Option<usize>,
    active_line_number: Option<LineCount>,
) -> LineNumberConfig {
    LineNumberConfig {
        font_family: FamilyId(0),
        font_size: 0.,
        text_color: ColorU::transparent_black(),
        highlight_text_color: ColorU::transparent_black(),
        starting_line_number,
        mode,
        active_line_number,
        active_cursor_is_visible: true,
    }
}

#[test]
fn absolute_line_numbers_default_to_one_based_values() {
    let config = config(CodeEditorLineNumberMode::Absolute, None, None);

    assert_eq!(config.absolute_line_number(LineCount::from(0)), 1);
    assert_eq!(config.absolute_line_number(LineCount::from(4)), 5);
}

#[test]
fn absolute_line_numbers_honor_starting_line_number() {
    let config = config(CodeEditorLineNumberMode::Absolute, Some(10), None);

    assert_eq!(config.absolute_line_number(LineCount::from(0)), 10);
    assert_eq!(config.absolute_line_number(LineCount::from(4)), 14);
}

#[test]
fn relative_line_numbers_show_absolute_value_on_active_line() {
    let config = config(
        CodeEditorLineNumberMode::Relative,
        None,
        Some(LineCount::from(4)),
    );
    assert_eq!(config.display_line_number(LineCount::from(4)), 5);
}

#[test]
fn relative_line_numbers_show_distance_above_and_below_active_line() {
    let config = config(
        CodeEditorLineNumberMode::Relative,
        None,
        Some(LineCount::from(5)),
    );
    assert_eq!(config.display_line_number(LineCount::from(2)), 3);
    assert_eq!(config.display_line_number(LineCount::from(8)), 3);
}

#[test]
fn relative_line_numbers_fall_back_to_absolute_without_active_line() {
    let config = config(CodeEditorLineNumberMode::Relative, None, None);
    assert_eq!(config.display_line_number(LineCount::from(4)), 5);
}

#[test]
fn relative_line_numbers_use_starting_line_number_for_active_line_only() {
    let config = config(
        CodeEditorLineNumberMode::Relative,
        Some(10),
        Some(LineCount::from(4)),
    );
    assert_eq!(config.display_line_number(LineCount::from(4)), 14);
    assert_eq!(config.display_line_number(LineCount::from(1)), 3);
}
