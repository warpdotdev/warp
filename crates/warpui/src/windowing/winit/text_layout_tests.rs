use anyhow::Result;

use super::*;
use crate::elements::DEFAULT_UI_LINE_HEIGHT_RATIO;
use crate::fonts::{Properties, collect_glyph_indices, init_fonts};
use crate::platform::FontDB as _;
use crate::text_layout::{DEFAULT_TOP_BOTTOM_RATIO, TextStyle};

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
    assert!(
        collect_glyph_indices(&overflow_indent_frame)
            .first()
            .unwrap()
            .is_empty(),
    );
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
    assert!(
        collect_glyph_indices(&big_indent_frame)
            .first()
            .unwrap()
            .is_empty(),
    );
    assert!(first_line_bounded(
        &big_indent_frame,
        FRAME_WIDTH - 0.1,
        FRAME_WIDTH,
    ));
    // assert!(all_lines_bounded(&big_indent_frame, FRAME_WIDTH));

    Ok(())
}

/// Combining marks must be placed with the shaper's GPOS offsets, not with the pen position.
///
/// [`cosmic_text::LayoutGlyph`] reports the shaped placement of a glyph in `x_offset` / `y_offset`
/// (em units, to be scaled by `font_size`), separately from `x` / `y`, which are the pen position
/// after advance accumulation. Ignoring the offsets draws every zero-advance combining mark at the
/// pen instead of on its base.
///
/// This is invisible with a single mark — most fonts draw mark glyphs extending left of their
/// origin, so a mark placed at the pen happens to land over its base — and only becomes visible
/// once two marks stack, since both then get identical coordinates.
#[test]
fn test_combining_marks_are_placed_with_gpos_offsets() -> Result<()> {
    use std::path::PathBuf;

    let (mut font_db, _roboto) = init_fonts();

    // The bundled Roboto-Regular doesn't cover COMBINING DIAERESIS, and carries no mark anchors
    // for COMBINING ACUTE ACCENT either, so it can't exercise GPOS mark positioning. RobotoFlex,
    // also bundled, has both marks plus mark-to-base and mark-to-mark anchors.
    let font_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "..",
        "app",
        "assets",
        "bundled",
        "fonts",
        "roboto",
        "RobotoFlex-Semibold.ttf",
    ]
    .iter()
    .collect();
    let roboto_flex = font_db
        .load_from_bytes(
            "RobotoFlex",
            vec![std::fs::read(font_path).expect("should be able to read the bundled RobotoFlex")],
        )
        .expect("should be able to load RobotoFlex for test");

    // `b` has no precomposed form with either mark, so the shaper cannot compose them away and
    // emits three glyphs: the base, then two stacked zero-advance marks.
    let text = "b\u{0301}\u{0308}"; // b + COMBINING ACUTE ACCENT + COMBINING DIAERESIS

    let line = font_db.text_layout_system().layout_line(
        text,
        LineStyle {
            font_size: FONT_SIZE,
            line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..text.chars().count(),
            StyleAndFont::new(roboto_flex, Properties::default(), TextStyle::new()),
        )],
        f32::MAX,
        crate::text_layout::ClipConfig::default(),
    );

    let glyphs = line
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .collect_vec();
    assert_eq!(
        glyphs.len(),
        3,
        "expected a base glyph followed by two combining marks, got {glyphs:?}"
    );

    let base = glyphs[0];
    let (acute, diaeresis) = (glyphs[1], glyphs[2]);

    assert!(base.width > 0., "the base glyph should advance the pen");
    assert_eq!(
        (
            base.position_along_baseline.x(),
            base.position_along_baseline.y()
        ),
        (0., 0.),
        "the base glyph starts at the origin"
    );

    for mark in [acute, diaeresis] {
        assert_eq!(mark.width, 0., "combining marks have no advance");
        // Without the GPOS offsets a mark keeps the pen position it inherited from the base's
        // advance, which places it after the base instead of over it.
        assert!(
            mark.position_along_baseline.x() < base.width,
            "mark should be pulled back over its base by its GPOS x offset, got x = {} with a \
             base advance of {}",
            mark.position_along_baseline.x(),
            base.width
        );
        // Screen coordinates grow downwards, so a mark rendered above the baseline has a negative
        // y. Without the GPOS offsets every mark stays on the baseline.
        assert!(
            mark.position_along_baseline.y() < 0.,
            "mark should be raised above the baseline by its GPOS y offset, got y = {}",
            mark.position_along_baseline.y()
        );
    }

    // The point of the fix: stacked marks must not collapse onto one another. The diaeresis is
    // attached to the acute by the font's mark-to-mark anchors, so it sits strictly higher.
    assert!(
        diaeresis.position_along_baseline.y() < acute.position_along_baseline.y(),
        "the second mark should stack above the first, but they are at y = {} and y = {}",
        diaeresis.position_along_baseline.y(),
        acute.position_along_baseline.y()
    );

    Ok(())
}

/// Regression test for a right-to-left (e.g. Arabic) filename rendering blank in
/// the Project Explorer and completion rows, even though the row's icon and
/// background paint normally.
///
/// A `Text::new_inline`/`Text::new` element placed as a non-flexible child of a
/// `Flex` row (as the completion row's filename label is; see
/// `input_suggestions.rs`) is laid out with an unbounded main-axis constraint --
/// `f32::INFINITY` -- per `SizeConstraint::child_constraint_along_axis`, and that
/// constraint flows straight into `layout_line`'s `max_width`. cosmic-text's
/// right-to-left alignment math computes `line_width - (line_width -
/// content_width)`, where `line_width` is `max_width`; for an infinite `max_width`
/// that subtraction is `INFINITY - INFINITY`, which is `NaN` in IEEE 754, so every
/// glyph in the line ends up at a NaN position and silently fails to paint.
/// Left-to-right text is unaffected, since its equivalent term is always zero,
/// independent of `line_width`.
///
/// This does not require a font with actual Arabic glyph coverage: the broken
/// alignment math runs purely from the text's Unicode bidi properties, before
/// glyph shaping is consulted, so the bundled Roboto font (which lacks Arabic
/// glyphs) still reproduces it.
#[test]
fn test_layout_line_rtl_with_unbounded_width_does_not_produce_nan_positions() -> Result<()> {
    let (font_db, roboto) = init_fonts();

    let text = "التعرف على خط اليد في المخطوطات العربية التاريخية.pdf";
    let line_style = LineStyle {
        font_size: FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: None,
    };
    let style_runs = [(
        0..text.chars().count(),
        StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
    )];

    let line = font_db.text_layout_system().layout_line(
        text,
        line_style,
        &style_runs,
        f32::INFINITY,
        crate::text_layout::ClipConfig::default(),
    );

    assert!(
        line.width.is_finite() && line.width > 0.,
        "line width should be a positive finite value, got {}",
        line.width
    );

    let glyphs = line
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .collect_vec();
    assert!(!glyphs.is_empty(), "expected at least one glyph");

    // Allow a small tolerance around [0, line.width] for ordinary floating-point
    // rounding; the bug this guards against places every glyph at NaN or at a
    // wildly out-of-bounds position, not merely a fraction of a pixel off.
    const TOLERANCE: f32 = 1.0;
    for glyph in glyphs {
        let x = glyph.position_along_baseline.x();
        assert!(
            x.is_finite(),
            "glyph {} at char index {} should have a finite x position, got {x}",
            glyph.id,
            glyph.index
        );
        assert!(
            x >= -TOLERANCE && x <= line.width + TOLERANCE,
            "glyph {} at char index {} should be within the line's bounds \
             [0, {}], got x = {x}",
            glyph.id,
            glyph.index,
            line.width
        );
    }

    Ok(())
}

/// Regression probe for the Project Explorer row half of REV-26: does an overflowing
/// right-to-left filename under a *finite*, realistic row width (as opposed to the
/// unbounded-width completion row fixed above) ever get a visible glyph painted at all?
///
/// The Explorer row's label is `Shrinkable`-wrapped inside a `Flex::row`, so under
/// normal (non-drag) layout it always receives a finite main-axis width -- this does
/// NOT reproduce the `NaN` bug fixed by `finite_line_layout_width`. This test instead
/// checks the *painting* side: `Line::paint`'s glyph-truncation loop walks
/// `run.glyphs` in storage order and stops once a `remaining_width` budget (seeded from
/// the available width) is exhausted. For left-to-right text that walk is in
/// increasing-x order, so the prefix that fits the budget is exactly the prefix that's
/// visually inside `[0, available_width)`. For right-to-left text, storage order is
/// *not* sorted by x (this line's first character sits at the highest x, near
/// `line.width`), so budget-based truncation can spend the whole budget on glyphs whose
/// real positions are already beyond `available_width`, and stop before ever reaching
/// the glyphs that would actually fall inside the visible window.
#[test]
fn test_line_paint_rtl_overflow_under_finite_width() -> Result<()> {
    use pathfinder_color::ColorU;
    use pathfinder_geometry::rect::RectF;

    use crate::Scene;
    use crate::fonts::Cache as FontCache;
    use crate::rendering::Config;
    use crate::text_layout::PaintStyleOverride;

    let (font_db, roboto) = init_fonts();
    let font_cache = FontCache::new(Box::new(font_db));

    let text = "التعرف على خط اليد في المخطوطات العربية التاريخية.pdf";
    let line_style = LineStyle {
        font_size: FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: None,
    };
    let style_runs = [(
        0..text.chars().count(),
        StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
    )];

    // A realistic, *finite* width for the Explorer row's filename label -- narrower
    // than the text's natural content width, so it must be clipped/faded, exactly as
    // it would be in the real Project Explorer panel for a long filename.
    let available_width = 180.0_f32;

    let line = font_cache.text_layout_system().layout_line(
        text,
        line_style,
        &style_runs,
        available_width,
        crate::text_layout::ClipConfig::default(),
    );

    assert!(
        line.width > available_width,
        "expected the text to overflow the available width in this test (line.width = {}, \
         available_width = {available_width})",
        line.width
    );

    let mut scene = Scene::new(1.0, Config::default());
    let bounds = RectF::new(vec2f(0., 0.), vec2f(available_width, line.height()));
    line.paint(
        bounds,
        &PaintStyleOverride::default(),
        ColorU::white(),
        &font_cache,
        &mut scene,
    );

    let glyph_positions: Vec<f32> = scene
        .layers()
        .flat_map(|layer| layer.glyphs.iter())
        .map(|glyph| glyph.position.x())
        .collect();

    assert!(
        !glyph_positions.is_empty(),
        "expected at least one glyph to be painted"
    );
    assert!(
        glyph_positions
            .iter()
            .any(|&x| x >= 0.0 && x < available_width),
        "expected at least one glyph to be painted within the visible row bounds \
         [0, {available_width}), but none were -- every attempted glyph draw landed \
         outside the row, which is indistinguishable from a blank label: {glyph_positions:?}"
    );

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
