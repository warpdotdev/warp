use super::*;
use vec1::vec1;

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
