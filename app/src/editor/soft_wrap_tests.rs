use vec1::vec1;

use super::*;

#[test]
fn test_singleton_frames_displayed_lines() {
    let frame_layouts = FrameLayouts {
        frames: vec![
            Arc::new(text_layout::TextFrame::empty(12., 1.)),
            Arc::new(text_layout::TextFrame::empty(13., 1.)),
            Arc::new(text_layout::TextFrame::empty(14., 1.)),
            Arc::new(text_layout::TextFrame::empty(15., 1.)),
            Arc::new(text_layout::TextFrame::empty(16., 1.)),
        ],
        start_line: 2,
        end_line: 3,
    };

    assert_eq!(frame_layouts.displayed_lines().count(), 1);

    let mut iter = frame_layouts.displayed_lines();
    assert_eq!(iter.next().expect("Should have line").font_size, 14.);
}

#[test]
fn test_displayed_lines_end_line_greater_than_iterator_size() {
    let frame_layouts = FrameLayouts {
        frames: vec![
            Arc::new(text_layout::TextFrame::empty(12., 1.)),
            Arc::new(text_layout::TextFrame::empty(13., 1.)),
            Arc::new(text_layout::TextFrame::empty(14., 1.)),
            Arc::new(text_layout::TextFrame::empty(15., 1.)),
            Arc::new(text_layout::TextFrame::empty(16., 1.)),
        ],
        start_line: 3,
        end_line: 6,
    };

    assert_eq!(frame_layouts.displayed_lines().count(), 2);

    let mut iter = frame_layouts.displayed_lines();
    assert_eq!(iter.next().expect("Should have line").font_size, 15.);
    assert_eq!(iter.next().expect("Should have line").font_size, 16.);
    assert!(iter.next().is_none());
}

#[test]
fn test_soft_wrapped_row_bounds() {
    // A single logical line that soft-wraps onto three visual rows of four
    // characters each. `TextFrame::mock` produces one frame whose lines carry
    // continuous glyph indices: row 0 -> 0..=3, row 1 -> 4..=7, row 2 -> 8..=11.
    let frame_layouts = FrameLayouts {
        frames: vec![Arc::new(text_layout::TextFrame::mock("aaaa\nbbbb\ncccc"))],
        start_line: 0,
        end_line: 3,
    };

    // A point in the middle of the first visual row -> [0, 4).
    assert_eq!(
        frame_layouts.soft_wrapped_row_bounds(DisplayPoint::new(0, 2), ClampDirection::Down),
        Some(0..4)
    );
    // A point in the middle of the second visual row -> [4, 8).
    assert_eq!(
        frame_layouts.soft_wrapped_row_bounds(DisplayPoint::new(0, 5), ClampDirection::Down),
        Some(4..8)
    );
    // A point in the middle of the third (last) visual row -> [8, 12).
    assert_eq!(
        frame_layouts.soft_wrapped_row_bounds(DisplayPoint::new(0, 10), ClampDirection::Down),
        Some(8..12)
    );
    // A point outside the laid-out text -> None.
    assert_eq!(
        frame_layouts.soft_wrapped_row_bounds(DisplayPoint::new(5, 0), ClampDirection::Down),
        None
    );
}

#[test]
fn test_soft_wrapped_row_bounds_multi_codepoint_cluster() {
    // Model a single visual row whose final character is a multi-codepoint
    // cluster (e.g. an emoji): it renders as one glyph, but its caret position
    // spans two buffer characters. The row end must land *after* the whole
    // cluster (one past the last caret position's `last_offset`), not at
    // `last_glyph().index + 1`, which would stop *inside* the cluster.
    //
    // Start from a mock line (which gives us real glyphs at indices 0, 1, 2) and
    // attach caret positions where the final position covers two characters.
    let mut line = text_layout::TextFrame::mock("abc").lines()[0].clone();
    line.caret_positions = vec![
        text_layout::CaretPosition {
            position_in_line: 0.0,
            start_offset: 0,
            last_offset: 0,
        },
        text_layout::CaretPosition {
            position_in_line: 0.0,
            start_offset: 1,
            last_offset: 1,
        },
        // The trailing cluster occupies two buffer characters (indices 2 and 3).
        text_layout::CaretPosition {
            position_in_line: 0.0,
            start_offset: 2,
            last_offset: 3,
        },
    ];
    let frame_layouts = FrameLayouts {
        frames: vec![Arc::new(text_layout::TextFrame::new(
            vec1![line],
            0.,
            Default::default(),
        ))],
        start_line: 0,
        end_line: 1,
    };

    // End is one-past the final cluster (4); a glyph-only derivation would
    // incorrectly yield 3 (the last glyph's index + 1), landing inside it.
    assert_eq!(
        frame_layouts.soft_wrapped_row_bounds(DisplayPoint::new(0, 0), ClampDirection::Down),
        Some(0..4)
    );
}

#[test]
fn test_soft_wrapped_frame_displayed_lines() {
    let frame_layouts = FrameLayouts {
        frames: vec![
            Arc::new(text_layout::TextFrame::new(
                vec1![
                    text_layout::Line::empty(10., 1., 0),
                    text_layout::Line::empty(11., 1., 1),
                    text_layout::Line::empty(12., 1., 2),
                ],
                0.,
                Default::default(),
            )),
            Arc::new(text_layout::TextFrame::empty(13., 1.)),
            Arc::new(text_layout::TextFrame::empty(14., 1.)),
            Arc::new(text_layout::TextFrame::empty(15., 1.)),
            Arc::new(text_layout::TextFrame::empty(16., 1.)),
        ],
        start_line: 2,
        end_line: 5,
    };

    assert_eq!(frame_layouts.displayed_lines().count(), 3);

    let mut iter = frame_layouts.displayed_lines();
    assert_eq!(iter.next().expect("Should have line").font_size, 12.);
    assert_eq!(iter.next().expect("Should have line").font_size, 13.);
    assert_eq!(iter.next().expect("Should have line").font_size, 14.);
    assert!(iter.next().is_none());
}
