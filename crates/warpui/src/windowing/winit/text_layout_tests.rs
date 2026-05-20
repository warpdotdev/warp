use super::*;
use crate::fonts::{collect_glyph_indices, init_fonts, Properties};
use crate::platform::FontDB as _;
use crate::{
    elements::DEFAULT_UI_LINE_HEIGHT_RATIO,
    text_layout::{TextStyle, DEFAULT_TOP_BOTTOM_RATIO},
};
use anyhow::Result;

const FONT_SIZE: f32 = 16.;
const FRAME_WIDTH: f32 = 80.;
const FRAME_HEIGHT: f32 = f32::MAX;

#[test]
fn test_fixed_width_tab_size_affects_tab_width() -> Result<()> {
    let (font_db, roboto) = init_fonts();

    let tabbed = "\tX";
    let spaced = "        X";

    let line_style = LineStyle {
        font_size: FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: Some(8),
    };

    let tabbed_line = font_db.text_layout_system().layout_line(
        tabbed,
        line_style,
        &[(
            0..tabbed.chars().count(),
            StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
        )],
        f32::MAX,
        crate::text_layout::ClipConfig::default(),
    );
    let spaced_line = font_db.text_layout_system().layout_line(
        spaced,
        line_style,
        &[(
            0..spaced.chars().count(),
            StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
        )],
        f32::MAX,
        crate::text_layout::ClipConfig::default(),
    );

    let error = (tabbed_line.width - spaced_line.width).abs();
    assert!(
        error < 1.0,
        "expected tab width ~= 8 spaces; got tabbed {}, spaced {} (error {})",
        tabbed_line.width,
        spaced_line.width,
        error
    );

    Ok(())
}

#[test]
fn test_layout_text_first_line_indent_small() -> Result<()> {
    let (font_db, roboto) = init_fonts();

    let text = "Let's lay out s𐍈me Roboto text.";
    //          0123456789012345678901234567890
    let line_style = LineStyle {
        font_size: FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: None,
    };
    let style_runs = [(
        0..text.encode_utf16().count(),
        StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
    )];

    // First, lay out the text with no head indent.
    let no_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        None,
    );

    // The text should contain multiple lines.
    // The first line has about the same amount of content as the others,
    // since there's no head indent.
    assert_eq!(no_indent_frame.lines().len(), 4);
    assert_eq!(
        collect_glyph_indices(&no_indent_frame),
        vec![
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],      // 9 is whitespace.
            vec![10, 11, 12, 13, 14, 15, 16, 17], // 18 is whitespace.
            vec![19, 20, 21, 22, 23, 24],         // 25 is whitespace.
            vec![26, 27, 28, 29, 30],
        ]
    );
    assert!(first_line_bounded(&no_indent_frame, 0., FRAME_WIDTH));
    assert!(all_lines_bounded(&no_indent_frame, FRAME_WIDTH));

    // Lay out the text with a 5px head indent.
    let small_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(5.),
    );

    // The first line has about the same amount of content as the others,
    // since the head indent is small.
    assert_eq!(small_indent_frame.lines().len(), 4);
    assert_eq!(
        collect_glyph_indices(&small_indent_frame),
        vec![
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            vec![10, 11, 12, 13, 14, 15, 16, 17],
            vec![19, 20, 21, 22, 23, 24],
            vec![26, 27, 28, 29, 30],
        ]
    );
    assert!(first_line_bounded(&small_indent_frame, 5., FRAME_WIDTH));
    assert!(all_lines_bounded(&small_indent_frame, FRAME_WIDTH));

    // Lay out the text with a 40px head indent,
    // which is half the width of the frame.
    let half_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(FRAME_WIDTH / 2.),
    );

    // The text contains an additional line to accommodate the indent.
    assert_eq!(half_indent_frame.lines().len(), 5);
    assert_eq!(
        collect_glyph_indices(&half_indent_frame),
        vec![
            vec![0, 1, 2, 3, 4],          // Fewer glyphs fit on this line. 5 is whitespace.
            vec![6, 7, 8, 9, 10, 11, 12], // 13 is whitespace.
            vec![14, 15, 16, 17],
            vec![19, 20, 21, 22, 23, 24],
            vec![26, 27, 28, 29, 30],
        ]
    );
    assert!(first_line_bounded(
        &half_indent_frame,
        FRAME_WIDTH / 2.,
        FRAME_WIDTH,
    ));
    assert!(all_lines_bounded(&half_indent_frame, FRAME_WIDTH));

    Ok(())
}

#[test]
fn test_layout_text_first_line_indent_medium() -> Result<()> {
    let (font_db, roboto) = init_fonts();

    let text = "Let's lay out s𐍈me Roboto text.";
    //          0123456789012345678901234567890
    let line_style = LineStyle {
        font_size: FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: None,
    };
    let style_runs = [(
        0..text.encode_utf16().count(),
        StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
    )];

    // First, lay out the text with no head indent.
    let no_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(0.),
    );

    // The text should contain multiple lines.
    // The first line has about the same amount of content as the others,
    // since there's no head indent.
    assert_eq!(no_indent_frame.lines().len(), 4);
    assert_eq!(
        collect_glyph_indices(&no_indent_frame),
        vec![
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            vec![10, 11, 12, 13, 14, 15, 16, 17],
            vec![19, 20, 21, 22, 23, 24],
            vec![26, 27, 28, 29, 30],
        ]
    );
    assert!(first_line_bounded(&no_indent_frame, 0., FRAME_WIDTH));
    assert!(all_lines_bounded(&no_indent_frame, FRAME_WIDTH));

    // Lay out the text with a head indent that's 15px smaller than
    // the width of the frame.
    let overflow_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(FRAME_WIDTH - 20.),
    );

    // The first line should have some glyphs on it, but not the whole
    // first word.
    assert_eq!(overflow_indent_frame.lines().len(), 5);
    assert_eq!(
        collect_glyph_indices(&overflow_indent_frame),
        vec![
            vec![0, 1], // Only a few glyphs fit.
            vec![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
            vec![14, 15, 16, 17],
            vec![19, 20, 21, 22, 23, 24],
            vec![26, 27, 28, 29, 30],
        ]
    );
    assert!(first_line_bounded(
        &overflow_indent_frame,
        FRAME_WIDTH - 20.,
        FRAME_WIDTH,
    ));
    assert!(all_lines_bounded(&overflow_indent_frame, FRAME_WIDTH));

    Ok(())
}

#[test]
fn test_layout_text_first_line_indent_large() -> Result<()> {
    let (font_db, roboto) = init_fonts();

    let text = "Let's lay out s𐍈me Roboto text.";
    //          0123456789012345678901234567890
    let line_style = LineStyle {
        font_size: FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: None,
    };
    let style_runs = [(
        0..text.encode_utf16().count(),
        StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
    )];

    // First, lay out the text with no head indent.
    let no_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(0.),
    );

    // The text should contain multiple lines.
    // The first line has about the same amount of content as the others,
    // since there's no head indent.
    assert_eq!(no_indent_frame.lines().len(), 4);
    assert_eq!(
        collect_glyph_indices(&no_indent_frame),
        vec![
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            vec![10, 11, 12, 13, 14, 15, 16, 17],
            vec![19, 20, 21, 22, 23, 24],
            vec![26, 27, 28, 29, 30],
        ]
    );
    assert!(first_line_bounded(&no_indent_frame, 0., FRAME_WIDTH));
    assert!(all_lines_bounded(&no_indent_frame, FRAME_WIDTH));

    // Lay out the text with a head indent that's 5px bigger than the width of the frame.
    let overflow_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(FRAME_WIDTH + 5.),
    );

    // The first line is left entirely blank since no glyphs fit on it.
    assert_eq!(
        collect_glyph_indices(&overflow_indent_frame),
        vec![
            vec![], // No glyphs fit on this line.
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            vec![10, 11, 12, 13, 14, 15, 16, 17],
            vec![19, 20, 21, 22, 23, 24],
            vec![26, 27, 28, 29, 30],
        ]
    );
    assert!(first_line_bounded(
        &overflow_indent_frame,
        FRAME_WIDTH + 5.,
        FRAME_WIDTH,
    ));
    assert!(all_lines_bounded(&overflow_indent_frame, FRAME_WIDTH));

    // Lay out the text with a 79px head indent,
    // which spans almost the entire width of the frame.
    let big_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(FRAME_WIDTH - 0.1),
    );

    // The first line is left entirely blank since no glyphs fit on it.
    assert_eq!(big_indent_frame.lines().len(), 5);
    assert_eq!(
        collect_glyph_indices(&big_indent_frame),
        vec![
            vec![], // No glyphs fit on this line.
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            vec![10, 11, 12, 13, 14, 15, 16, 17],
            vec![19, 20, 21, 22, 23, 24],
            vec![26, 27, 28, 29, 30],
        ]
    );
    assert!(first_line_bounded(
        &big_indent_frame,
        FRAME_WIDTH - 0.1,
        FRAME_WIDTH,
    ));
    assert!(all_lines_bounded(&big_indent_frame, FRAME_WIDTH));

    Ok(())
}

// TODO(PLAT-779): check all line bounds once bidirectional wrapping is fixed in cosmic-text.
// See https://github.com/pop-os/cosmic-text/issues/252.
#[test]
fn test_layout_text_first_line_indent_small_bidirectional() -> Result<()> {
    let (font_db, roboto) = init_fonts();

    let text = "brekkie, إفطار, lunch (غداء) and dinner - عشاء";
    //          0123456783210945678901265437890123456789015432
    // RTL spans:       |-----|       |----|             |----|
    let line_style = LineStyle {
        font_size: FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: None,
    };
    let style_runs = [(
        0..text.encode_utf16().count(),
        StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
    )];

    // First, lay out the text with no head indent.
    let no_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        None,
    );

    // The text should contain multiple lines.
    // The first line has about the same amount of content as the others,
    // since there's no head indent.
    assert_eq!(no_indent_frame.lines().len(), 4);
    assert!(first_line_bounded(&no_indent_frame, 0., FRAME_WIDTH));
    // assert!(all_lines_bounded(&no_indent_frame, FRAME_WIDTH));

    // Lay out the text with a 5px head indent.
    let small_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(5.),
    );

    // The first line has about the same amount of content as the others,
    // since the head indent is small.
    assert_eq!(small_indent_frame.lines().len(), 4);
    assert!(first_line_bounded(&small_indent_frame, 5., FRAME_WIDTH));
    // assert!(all_lines_bounded(&small_indent_frame, FRAME_WIDTH));

    // Lay out the text with a 40px head indent,
    // which is half the width of the frame.
    let half_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(FRAME_WIDTH / 2.),
    );

    // The text contains an additional line to accommodate the indent.
    assert_eq!(half_indent_frame.lines().len(), 5);
    assert!(first_line_bounded(
        &half_indent_frame,
        FRAME_WIDTH / 2.,
        FRAME_WIDTH,
    ));
    // assert!(all_lines_bounded(&half_indent_frame, FRAME_WIDTH));

    Ok(())
}

// TODO(PLAT-779): check all line bounds once bidirectional wrapping is fixed in cosmic-text.
// See https://github.com/pop-os/cosmic-text/issues/252.
#[test]
fn test_layout_text_first_line_indent_medium_bidirectional() -> Result<()> {
    let (font_db, roboto) = init_fonts();

    let text = "brekkie, إفطار, lunch (غداء) and dinner - عشاء";
    //          0123456783210945678901265437890123456789015432
    // RTL spans:       |-----|       |----|             |----|
    let line_style = LineStyle {
        font_size: FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: None,
    };
    let style_runs = [(
        0..text.encode_utf16().count(),
        StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
    )];

    // First, lay out the text with no head indent.
    let no_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        None,
    );

    // The text should contain multiple lines.
    // The first line has about the same amount of content as the others,
    // since there's no head indent.
    assert_eq!(no_indent_frame.lines().len(), 4);
    assert!(first_line_bounded(&no_indent_frame, 0., FRAME_WIDTH));
    // assert!(all_lines_bounded(&no_indent_frame, FRAME_WIDTH));

    // Lay out the text with a head indent that's 15px smaller than
    // the width of the frame.
    let overflow_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(FRAME_WIDTH - 20.),
    );

    // The first line should have some glyphs on it, but not the whole
    // first word.
    assert_eq!(overflow_indent_frame.lines().len(), 5);
    assert!(first_line_bounded(
        &overflow_indent_frame,
        FRAME_WIDTH - 20.,
        FRAME_WIDTH,
    ));
    // assert!(all_lines_bounded(&overflow_indent_frame, FRAME_WIDTH));

    Ok(())
}

// TODO(PLAT-779): check all line bounds once bidirectional wrapping is fixed in cosmic-text.
// See https://github.com/pop-os/cosmic-text/issues/252.
#[test]
fn test_layout_text_first_line_indent_large_bidirectional() -> Result<()> {
    let (font_db, roboto) = init_fonts();

    let text = "brekkie, إفطار, lunch (غداء) and dinner - عشاء";
    //          0123456783210945678901265437890123456789015432
    // RTL spans:       |-----|       |----|             |----|
    let line_style = LineStyle {
        font_size: FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: None,
    };
    let style_runs = [(
        0..text.encode_utf16().count(),
        StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
    )];

    // First, lay out the text with no head indent.
    let no_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(0.),
    );

    // The text should contain multiple lines.
    // The first line has about the same amount of content as the others,
    // since there's no head indent.
    assert_eq!(no_indent_frame.lines().len(), 4);
    assert!(first_line_bounded(&no_indent_frame, 0., FRAME_WIDTH));
    // assert!(all_lines_bounded(&no_indent_frame, FRAME_WIDTH));

    // Lay out the text with a head indent that's 5px bigger than the width of the frame.
    let overflow_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(FRAME_WIDTH + 5.),
    );

    // The first line is left entirely blank since no glyphs fit on it.
    assert_eq!(overflow_indent_frame.lines().len(), 5);
    assert!(collect_glyph_indices(&overflow_indent_frame)
        .first()
        .unwrap()
        .is_empty(),);
    assert!(first_line_bounded(
        &overflow_indent_frame,
        FRAME_WIDTH + 5.,
        FRAME_WIDTH,
    ));
    // assert!(all_lines_bounded(&overflow_indent_frame, FRAME_WIDTH));

    // Lay out the text with a 79px head indent,
    // which spans almost the entire width of the frame.
    let big_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(FRAME_WIDTH - 0.1),
    );

    // The first line is left entirely blank since no glyphs fit on it.
    assert_eq!(big_indent_frame.lines().len(), 5);
    assert!(collect_glyph_indices(&big_indent_frame)
        .first()
        .unwrap()
        .is_empty(),);
    assert!(first_line_bounded(
        &big_indent_frame,
        FRAME_WIDTH - 0.1,
        FRAME_WIDTH,
    ));
    // assert!(all_lines_bounded(&big_indent_frame, FRAME_WIDTH));

    Ok(())
}

/// Checks that the head indent and first line's width don't exceed the frame's width.
fn first_line_bounded(frame: &TextFrame, first_line_indent: f32, frame_width: f32) -> bool {
    let first_line_width = frame.lines().first().unwrap().width;
    first_line_width + first_line_indent.min(frame_width) <= frame_width
}

fn all_lines_bounded(frame: &TextFrame, frame_width: f32) -> bool {
    frame.lines().iter().fold(true, |all_bounded, line| {
        let current_bounded = line.width <= frame_width;
        all_bounded && current_bounded
    })
}

#[test]
fn test_softwrap_caret_positions_are_contiguous() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    // A single paragraph (no newlines) long enough to soft-wrap at 200px.
    let text = "The quick brown fox jumps over the lazy dog and then keeps running onward";
    let frame = font_db.text_layout_system().layout_text(
        text,
        LineStyle {
            font_size: FONT_SIZE,
            line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..text.chars().count(),
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        200.,
        f32::MAX,
        TextAlignment::Left,
        None,
    );

    // Should wrap onto multiple lines.
    assert!(
        frame.lines().len() >= 2,
        "Expected at least 2 lines but got {}",
        frame.lines().len()
    );

    // Collect all caret position start_offsets across all lines.
    let all_caret_starts: Vec<usize> = frame
        .lines()
        .iter()
        .flat_map(|line| line.caret_positions.iter().map(|c| c.start_offset))
        .collect();

    // The caret positions should be monotonically non-decreasing across all lines.
    // Before the fix, the second/third wrapped line's carets would reset to 0.
    for window in all_caret_starts.windows(2) {
        assert!(
            window[0] <= window[1],
            "Caret positions are not monotonically non-decreasing: {} > {} (all: {:?})",
            window[0],
            window[1],
            all_caret_starts
        );
    }

    // The first caret should start at 0 and the last should correspond to near the end of the text.
    assert_eq!(
        *all_caret_starts.first().unwrap(),
        0,
        "First caret should start at 0"
    );
    let last_caret = frame
        .lines()
        .last()
        .unwrap()
        .caret_positions
        .last()
        .unwrap();
    assert!(
        last_caret.last_offset > 0,
        "Last caret offset should be > 0"
    );

    // Each wrapped line's first caret should pick up where the previous line left off.
    for i in 1..frame.lines().len() {
        let prev_line = &frame.lines()[i - 1];
        let curr_line = &frame.lines()[i];
        if let (Some(prev_last), Some(curr_first)) = (
            prev_line.caret_positions.last(),
            curr_line.caret_positions.first(),
        ) {
            assert!(
                curr_first.start_offset > prev_last.start_offset,
                "Line {}'s first caret ({}) should be after line {}'s last caret ({})",
                i,
                curr_first.start_offset,
                i - 1,
                prev_last.start_offset
            );
        }
    }

    Ok(())
}

#[test]
fn test_softwrap_caret_positions_multi_paragraph() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    // Two paragraphs, each long enough to soft-wrap.
    let text = "The quick brown fox jumps over the lazy dog repeatedly\nAnother paragraph that \
        also wraps around when narrow";
    let frame = font_db.text_layout_system().layout_text(
        text,
        LineStyle {
            font_size: FONT_SIZE,
            line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..text.chars().count(),
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        200.,
        f32::MAX,
        TextAlignment::Left,
        None,
    );

    // Should have multiple lines from wrapping.
    assert!(
        frame.lines().len() >= 3,
        "Expected at least 3 lines but got {}",
        frame.lines().len()
    );

    // Caret positions should be monotonically non-decreasing across ALL lines (including across
    // the paragraph boundary).
    let all_caret_starts: Vec<usize> = frame
        .lines()
        .iter()
        .flat_map(|line| line.caret_positions.iter().map(|c| c.start_offset))
        .collect();

    for window in all_caret_starts.windows(2) {
        assert!(
            window[0] <= window[1],
            "Caret positions are not monotonically non-decreasing across paragraphs: {} > {} (all: {:?})",
            window[0],
            window[1],
            all_caret_starts
        );
    }

    Ok(())
}
