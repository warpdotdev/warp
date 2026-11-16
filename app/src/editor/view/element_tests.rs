use super::EditorElement;

const FLOAT_TOLERANCE: f32 = 1e-4;

#[test]
fn scroll_position_y_fract_is_continuous_at_first_visible_row_boundary_without_top_section() {
    let line_height = 20.0;

    let fract_before_boundary = EditorElement::scroll_position_y_fract(0.99, line_height, 0.0);
    assert!((fract_before_boundary - 19.8).abs() < FLOAT_TOLERANCE);

    let fract_at_boundary = EditorElement::scroll_position_y_fract(1.0, line_height, 0.0);
    assert!((fract_at_boundary - 0.0).abs() < FLOAT_TOLERANCE);
}

#[test]
fn scroll_position_y_fract_is_continuous_at_first_visible_row_boundary_with_top_section() {
    let line_height = 20.0;
    let top_section_height_px = 10.0;

    let fract_before_boundary =
        EditorElement::scroll_position_y_fract(1.49, line_height, top_section_height_px);
    assert!((fract_before_boundary - 29.8).abs() < FLOAT_TOLERANCE);

    let fract_at_boundary =
        EditorElement::scroll_position_y_fract(1.5, line_height, top_section_height_px);
    assert!((fract_at_boundary - 0.0).abs() < FLOAT_TOLERANCE);
}

#[test]
fn scroll_position_y_fract_tracks_fractional_offset_after_boundary() {
    let line_height = 20.0;
    let top_section_height_px = 10.0;

    let fract = EditorElement::scroll_position_y_fract(1.75, line_height, top_section_height_px);
    assert!((fract - 5.0).abs() < FLOAT_TOLERANCE);
}
