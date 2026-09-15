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

/// Regression coverage for CSAT-10272's confirmed root cause: cosmic-text's `Align::Left`
/// alignment math for a run computes `line_width - visual_line.w`, then
/// `start_x - alignment_correction` (RTL) to place glyphs. This is a geometry no-op for any
/// *finite* `line_width` (it always cancels out to the same result), but `layout_line` and
/// `layout_text` used to forward `max_width` verbatim even when it was `f32::INFINITY`--which
/// arises legitimately, e.g. a bare (non-flexible) `Flex` child always gets an infinite
/// main-axis constraint via `SizeConstraint::child_constraint_along_axis`. That turned the
/// alignment math into `INFINITY - INFINITY`, i.e. NaN, for every glyph in an RTL run, so the
/// label rendered nothing (its row icon, a separate element, still painted). LTR runs are
/// immune because their alignment correction is always zero regardless of width.
///
/// IMPORTANT SCOPE NOTE: these tests only assert *alignment geometry*--that glyph positions and
/// the line's advance width stay finite instead of going NaN. They deliberately do not assert
/// anything about `chars_with_missing_glyphs` / real glyph coverage: whether the bundled Roboto
/// font's shaping falls back to real Arabic/CJK glyphs depends on what other fonts happen to be
/// discoverable via system font-fallback (fontconfig) in the environment running the test, which
/// is not deterministic across machines/CI. This was instead verified out-of-band with a
/// guaranteed-real Arabic-covering font (Noto Sans Arabic) loaded directly: the same
/// `f32::INFINITY`-width call produced real, non-`.notdef` glyph ids with `chars_with_missing_
/// glyphs` empty, and still went NaN before this fix / finite after it--confirming this defect
/// is purely in the alignment math, independent of glyph coverage.
fn assert_line_has_finite_glyph_positions(line: &Line, text: &str) {
    let glyphs = line
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .collect_vec();
    assert!(
        !glyphs.is_empty(),
        "expected at least one (possibly missing-glyph placeholder) glyph laying out {text:?}, \
         got none"
    );
    for glyph in &glyphs {
        assert!(
            glyph.position_along_baseline.x().is_finite()
                && glyph.position_along_baseline.y().is_finite(),
            "expected finite glyph position laying out {text:?}, got {:?}",
            glyph.position_along_baseline
        );
        assert!(
            glyph.width.is_finite(),
            "expected finite glyph width laying out {text:?}, got {}",
            glyph.width
        );
    }
    assert!(
        line.width.is_finite() && line.width > 0.,
        "expected a positive, finite advance width laying out {text:?}, got {}",
        line.width
    );
}

/// A pure (no spaces) Arabic string laid out with an unbounded max width, matching how
/// `Text::new_inline` is laid out as a non-flexible child of a `Flex` (e.g. the completion menu's
/// list row, `app/src/input_suggestions.rs`). See the scope note on
/// `assert_line_has_finite_glyph_positions`: this is an alignment-geometry test, not a glyph
/// coverage/legibility test.
#[test]
fn test_layout_line_arabic_alignment_with_unbounded_width_stays_finite() -> Result<()> {
    let (font_db, roboto) = init_fonts();
    let text = "التعرف";

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
            StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
        )],
        f32::INFINITY,
        crate::text_layout::ClipConfig::default(),
    );

    assert_line_has_finite_glyph_positions(&line, text);

    Ok(())
}

/// An Arabic string containing spaces, laid out with an unbounded max width. Spaces only affect
/// shell escaping of the completion replacement elsewhere in the product; they should have no
/// bearing on whether the label's alignment geometry stays finite. See the scope note on
/// `assert_line_has_finite_glyph_positions`.
#[test]
fn test_layout_line_arabic_with_spaces_alignment_with_unbounded_width_stays_finite() -> Result<()> {
    let (font_db, roboto) = init_fonts();
    let text = "التعرف على خط اليد";

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
            StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
        )],
        f32::INFINITY,
        crate::text_layout::ClipConfig::default(),
    );

    assert_line_has_finite_glyph_positions(&line, text);

    Ok(())
}

/// A CJK (LTR) string laid out with an unbounded max width. CJK filenames have separately been
/// reported as blank specifically in the file tree (CSAT-7199, CSAT-7810, CSAT-8095 / CODE-1155).
/// Since CJK is left-to-right, its `Align::Left` alignment correction is always zero regardless
/// of width, so it can never hit the `INFINITY - INFINITY` NaN this fix addresses; this test
/// documents that the unbounded-width alignment geometry was already finite for CJK before and
/// after this fix. It does NOT establish what does cause the older file-tree CJK reports: live
/// GUI testing during this change's review (see the PR description) showed CJK file names
/// rendering as missing-glyph ('tofu') boxes rather than blank, which is a distinct, unresolved
/// issue in font/glyph fallback, not this alignment defect.
#[test]
fn test_layout_line_cjk_alignment_with_unbounded_width_stays_finite() -> Result<()> {
    let (font_db, roboto) = init_fonts();
    let text = "测试文件";

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
            StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
        )],
        f32::INFINITY,
        crate::text_layout::ClipConfig::default(),
    );

    assert_line_has_finite_glyph_positions(&line, text);

    Ok(())
}

/// Regression coverage for the analogous fix in `layout_text` (the soft-wrapped, multi-line
/// path used by `Text::new` / `Text::soft_wrap(true)`). A paragraph that is short enough to fit
/// entirely on one visual line even at an unbounded width exercises the exact same
/// `Align::Left` + RTL alignment math as `layout_line`, so it is vulnerable to the same
/// `INFINITY - INFINITY` NaN when a bare `Flex` child hands it an infinite `max_width`.
#[test]
fn test_layout_text_arabic_alignment_with_unbounded_width_stays_finite() -> Result<()> {
    let (font_db, roboto) = init_fonts();
    let text = "التعرف على خط اليد";

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
            StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
        )],
        f32::INFINITY,
        f32::INFINITY,
        TextAlignment::Left,
        None,
    );

    assert_eq!(
        frame.lines().len(),
        1,
        "a short paragraph laid out at unbounded width should never wrap"
    );
    let line = &frame.lines()[0];
    assert_line_has_finite_glyph_positions(line, text);

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
