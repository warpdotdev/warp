use super::{
    TuiTwoColumnConstraints, TuiTwoColumnLayout, format_tui_first_column, tui_two_column_layout,
};

const CONSTRAINTS: TuiTwoColumnConstraints = TuiTwoColumnConstraints {
    preferred_first_columns: 10,
    minimum_first_columns: 5,
    minimum_second_columns: 5,
    preferred_maximum_second_columns: 10,
    gap_columns: 1,
};

#[test]
fn hides_second_column_below_combined_minimum_width() {
    let layout = tui_two_column_layout(9, [("first", "second")], CONSTRAINTS);

    assert!(!layout.show_second);
    assert_eq!(layout.first_columns, 9);
}

#[test]
fn reserves_second_column_before_growing_first_column() {
    let layout = tui_two_column_layout(25, [("long first value", "second")], CONSTRAINTS);

    assert!(layout.show_second);
    assert_eq!(layout.first_columns, 17);
}

#[test]
fn ellipsizes_and_pads_first_column_once() {
    let layout = TuiTwoColumnLayout {
        available_columns: 20,
        first_columns: 10,
        show_second: true,
        gap_columns: 1,
    };

    assert_eq!(
        format_tui_first_column("long first value", layout),
        "long f... "
    );
}
#[test]
fn ellipsis_follows_wide_character_prefix_without_overflowing() {
    let layout = TuiTwoColumnLayout {
        available_columns: 29,
        first_columns: 29,
        show_second: true,
        gap_columns: 1,
    };

    assert_eq!(
        format_tui_first_column("/12345678901234567890123界suffix", layout),
        "/12345678901234567890123...  "
    );
}

#[test]
fn ellipsis_scales_to_extremely_narrow_columns() {
    let layout = TuiTwoColumnLayout {
        available_columns: 2,
        first_columns: 2,
        show_second: false,
        gap_columns: 0,
    };

    assert_eq!(format_tui_first_column("first", layout), "..");
}
