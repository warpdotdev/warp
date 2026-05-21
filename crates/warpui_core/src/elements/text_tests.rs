use float_cmp::assert_approx_eq;

use crate::scene::ZIndex;
use crate::App;

use super::*;

#[test]
fn test_laid_out_text_height() {
    App::test((), |mut app| async move {
        app.update(|_ctx| {
            let text_frame = TextFrame::mock("foo\nbar\nbaz");
            let line_count = text_frame.lines().len();
            let laid_out_text = LaidOutText::Frame(Arc::new(text_frame));
            let height = laid_out_text.height();
            let expected = 13. * 1.2 * line_count as f32;
            assert_approx_eq!(f32, height, expected);
        });
    });
}

/// We calculate height of a line by multiplying the line's font size by line
/// height ratio. This test ensures that the height of a laid out line respects
/// this calculation.
#[test]
fn test_laid_out_line_height() {
    App::test((), |mut app| async move {
        app.update(|_ctx| {
            let line = Line::mock_from_str("foo");
            let laid_out_line = LaidOutText::Line(Arc::new(line));
            let height = laid_out_line.height();

            // 13 and 1.2 are the default font size and line height ratios, respectively.
            let expected = 13. * 1.2;
            assert_approx_eq!(f32, height, expected);
        });
    });
}

#[test]
fn test_single_line_char_hit_testing_respects_y_bounds() {
    App::test((), |mut app| async move {
        app.update(|_ctx| {
            let mut line = Line::mock_from_str("foo");
            line.width = 30.;
            line.runs[0].width = 30.;
            let line = Arc::new(line);
            let line_height = line.height();
            let mut text = Text::new_inline("foo", crate::fonts::FamilyId(0), 13.);
            text.laid_out_text = LaidOutText::Line(Arc::clone(&line));
            text.origin = Some(Point::from_vec2f(vec2f(10., 20.), ZIndex::new(0)));

            assert!(text.get_char_index(&vec2f(10., 19.9)).is_none());
            assert!(text
                .get_char_index(&vec2f(10., 20. + line_height + 0.1))
                .is_none());
            assert!(text
                .get_char_index(&vec2f(10., 20. + line_height / 2.))
                .is_some());
        });
    });
}

#[test]
fn test_merge_non_overlapping_ranges() {
    let highlight = Highlight::new();

    let range1 = HighlightedRange {
        highlight,
        highlight_indices: vec![1, 2, 3],
    };
    let range2 = HighlightedRange {
        highlight,
        highlight_indices: vec![5, 6, 7],
    };

    let result = HighlightedRange::merge_overlapping_ranges(vec![range1.clone(), range2.clone()]);

    assert_eq!(result, vec![range1, range2]);
}

#[test]
fn test_merge_contiguous_ranges() {
    let highlight = Highlight::new();

    let range1 = HighlightedRange {
        highlight,
        highlight_indices: vec![1, 2, 3],
    };
    let range2 = HighlightedRange {
        highlight,
        highlight_indices: vec![4, 5, 6],
    };

    let result = HighlightedRange::merge_overlapping_ranges(vec![range1.clone(), range2.clone()]);

    assert_eq!(
        result,
        vec![HighlightedRange {
            highlight,
            highlight_indices: vec![1, 2, 3, 4, 5, 6],
        }]
    );
}

#[test]
fn test_merge_overlapping_ranges() {
    let highlight = Highlight::new();

    let range1 = HighlightedRange {
        highlight,
        highlight_indices: vec![1, 2, 3],
    };
    let range2 = HighlightedRange {
        highlight,
        highlight_indices: vec![3, 4, 5],
    };

    let result = HighlightedRange::merge_overlapping_ranges(vec![range1.clone(), range2.clone()]);

    assert_eq!(
        result,
        vec![HighlightedRange {
            highlight,
            highlight_indices: vec![1, 2, 3, 4, 5],
        }]
    );
}

#[test]
fn test_merge_single_range() {
    let highlight = Highlight::new();

    let range = HighlightedRange {
        highlight,
        highlight_indices: vec![1, 2, 3],
    };

    let result = HighlightedRange::merge_overlapping_ranges(vec![range.clone()]);

    assert_eq!(result, vec![range]);
}

#[test]
fn test_merge_empty_ranges() {
    let result = HighlightedRange::merge_overlapping_ranges(vec![]);
    assert!(result.is_empty());
}

#[test]
fn test_merge_adjacent_non_contiguous_ranges() {
    let highlight1 = Highlight::new();
    let highlight2 = Highlight::new();

    let range1 = HighlightedRange {
        highlight: highlight1,
        highlight_indices: vec![1, 2],
    };
    let range2 = HighlightedRange {
        highlight: highlight2,
        highlight_indices: vec![4, 5],
    };

    let result = HighlightedRange::merge_overlapping_ranges(vec![range1.clone(), range2.clone()]);

    assert_eq!(result, vec![range1, range2]);
}
