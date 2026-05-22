use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};

/// Parameters for creating a visual tick mark that indicates a newline in text selection.
///
/// Used to render small rectangular indicators at line endings when text selections
/// extend beyond the end of a line.
pub struct NewlineTickParams {
    /// The top-left corner position where the tick should be drawn
    pub tick_origin: Vector2F,
    /// The width of the tick mark in pixels
    pub tick_width: f32,
    /// The height of the tick mark in pixels
    pub tick_height: f32,
}

/// Creates a rectangle for rendering a newline tick mark in text selection.
///
/// The tick mark provides a visual indicator when a text selection extends beyond
/// the end of a line, helping users understand that the selection continues to the
/// next line.
pub fn create_newline_tick_rect(params: NewlineTickParams) -> RectF {
    RectF::new(
        params.tick_origin,
        vec2f(params.tick_width, params.tick_height),
    )
}

/// Calculates the appropriate width for a newline tick mark based on font size.
///
/// Returns a width that scales proportionally with the font size to ensure the
/// tick mark remains visible and appropriately sized across different text scales.
pub fn calculate_tick_width(font_size: f32) -> f32 {
    font_size / 2.0
}

/// Determines if a text selection crosses a newline using row-based coordinates.
///
/// This function checks whether a selection that intersects with the current row
/// extends beyond the end of that row, indicating that a newline tick should be
/// displayed. Returns `false` for the last line since there's no newline after it.
pub fn selection_crosses_newline_row_based(
    current_row: usize,
    is_last_line: bool,
    selection_start_row: usize,
    selection_end_row: usize,
    selection_end_column: usize,
    line_end_index: usize,
) -> bool {
    if is_last_line {
        return false;
    }

    let selection_starts_before_or_at_line = selection_start_row <= current_row;

    let selection_ends_beyond_line = current_row < selection_end_row
        || (current_row == selection_end_row && selection_end_column >= line_end_index);

    selection_starts_before_or_at_line && selection_ends_beyond_line
}

/// Determines if a text selection crosses a newline using character offset coordinates.
///
/// This function checks whether a selection intersects with a line and extends beyond
/// the end of that line, indicating that a newline tick should be displayed. Uses
/// absolute character positions rather than row/column coordinates. Returns `false`
/// for the last line since there's no newline after it.
pub fn selection_crosses_newline_offset_based(
    is_last_line: bool,
    selection_start: usize,
    selection_end: usize,
    line_start_offset: usize,
    line_end_offset: usize,
) -> bool {
    if is_last_line {
        return false;
    }

    let selection_intersects_line =
        selection_end > line_start_offset && selection_start <= line_end_offset;

    let selection_extends_beyond_line = selection_end > line_end_offset;

    selection_intersects_line && selection_extends_beyond_line
}
