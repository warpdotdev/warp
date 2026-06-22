use warpui::units::IntoLines as _;

use super::*;
use crate::terminal::SizeUpdateReason;

fn default_size() -> SizeInfo {
    SizeInfo::new_without_font_metrics(10, 7)
}

fn new_alt_screen(size: SizeInfo) -> AltScreen {
    AltScreen::new(
        size,
        1_000, /* max_scroll_limit */
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
    )
}

#[test]
fn test_alt_screen_resize_clears_selection() {
    let original_size = default_size();
    let mut screen = new_alt_screen(original_size);

    screen.start_selection(Point::new(2, 6), SelectionType::Simple, Side::Left);
    screen.update_selection(Point::new(4, 4), Side::Right);
    let semantic_selection = SemanticSelection::mock(false, "");

    assert_eq!(
        screen.selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Regular {
            start: Point::new(2, 6),
            end: Point::new(4, 4),
            reversed: false,
        })
    );

    // Resizing with same number of rows / cols should not clear selections.
    let size_update = SizeUpdate {
        update_reason: SizeUpdateReason::Refresh,
        last_size: original_size,
        new_size: original_size,
        new_gap_height: None,
        natural_rows: original_size.rows(),
        natural_cols: original_size.columns(),
    };
    screen.resize(&size_update);
    assert_eq!(
        screen.selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Regular {
            start: Point::new(2, 6),
            end: Point::new(4, 4),
            reversed: false,
        })
    );

    // Resizing with diff size should clear selections.
    let new_size =
        default_size().with_rows_and_columns(original_size.rows() + 1, original_size.columns() + 1);
    let size_update = SizeUpdate {
        update_reason: SizeUpdateReason::Refresh,
        last_size: original_size,
        new_size,
        new_gap_height: None,
        natural_rows: new_size.rows(),
        natural_cols: new_size.columns(),
    };
    screen.resize(&size_update);
    assert!(screen.selection_range(&semantic_selection).is_none());
}

#[test]
fn test_alt_screen_single_cell_selection() {
    let mut screen = new_alt_screen(default_size());

    screen.start_selection(Point::new(2, 6), SelectionType::Simple, Side::Left);
    screen.update_selection(Point::new(2, 7), Side::Left);
    let semantic_selection = SemanticSelection::mock(false, "");

    assert_eq!(
        screen.selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Regular {
            start: Point::new(2, 6),
            end: Point::new(2, 6),
            reversed: false,
        })
    );
}

#[test]
fn test_accumulate_lines_to_scroll() {
    let mut screen = new_alt_screen(default_size());

    assert_eq!(screen.accumulate_lines_to_scroll(-0.9.into_lines()), 0);
    assert_lines_approx_eq!(screen.pending_lines_to_scroll(), -0.9);

    assert_eq!(screen.accumulate_lines_to_scroll(-3.2.into_lines()), -4);
    assert_lines_approx_eq!(screen.pending_lines_to_scroll(), -0.1);

    assert_eq!(screen.accumulate_lines_to_scroll(5.0.into_lines()), 4);
    assert_lines_approx_eq!(screen.pending_lines_to_scroll(), 0.9);
}
