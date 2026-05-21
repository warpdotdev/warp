//! Platform-independent text layout tests.
use crate::elements::DEFAULT_UI_LINE_HEIGHT_RATIO;
use crate::fonts::{FamilyId, Properties, Style, Weight};
use crate::platform::FontDB as _;
use crate::platform::LineStyle;
use crate::text_layout::{
    ClipConfig, Line, StyleAndFont, TextAlignment, TextFrame, TextStyle, DEFAULT_TOP_BOTTOM_RATIO,
};
use anyhow::Result;
use itertools::Itertools;
use pathfinder_color::ColorU;

#[cfg(target_os = "macos")]
use crate::platform::mac::fonts::FontDB;

#[cfg(not(target_os = "macos"))]
use crate::windowing::winit::fonts::FontDB;

const FONT_SIZE: f32 = 16.;
const FRAME_WIDTH: f32 = 80.;
const FRAME_HEIGHT: f32 = f32::MAX;

#[test]
fn test_fixed_width_tab_size_matches_spaces_width() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    let tabbed = "\tX";
    let spaced = "    X";

    let line_style = LineStyle {
        font_size: FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: Some(4),
    };

    let tabbed_line = font_db.text_layout_system().layout_line(
        tabbed,
        line_style,
        &[(
            0..tabbed.chars().count(),
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        f32::MAX,
        ClipConfig::default(),
    );

    let spaced_line = font_db.text_layout_system().layout_line(
        spaced,
        line_style,
        &[(
            0..spaced.chars().count(),
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        f32::MAX,
        ClipConfig::default(),
    );

    let error = (tabbed_line.width - spaced_line.width).abs();
    assert!(
        error < 1.0,
        "expected tab width ~= 4 spaces; got tabbed {}, spaced {} (error {})",
        tabbed_line.width,
        spaced_line.width,
        error
    );

    Ok(())
}

/// Read the bundled Roboto font's bytes from the filesystem.
fn load_roboto_bytes() -> Vec<Vec<u8>> {
    use std::{fs::read, path::PathBuf};
    let root = env!("CARGO_MANIFEST_DIR");
    let typeface_files = ["Roboto-Italic.ttf", "Roboto-Bold.ttf", "Roboto-Regular.ttf"];
    typeface_files
        .iter()
        .map(|font_file| {
            let path = [
                root, "..", "..", "app", "assets", "bundled", "fonts", "roboto", font_file,
            ]
            .iter()
            .collect::<PathBuf>();
            Ok(read(path)?)
        })
        .collect::<Result<Vec<_>>>()
        .expect("should be able to read roboto font bytes from filesystem")
}

pub(crate) fn init_fonts() -> (FontDB, FamilyId) {
    let mut font_db = FontDB::new();
    let font_bytes = load_roboto_bytes();
    let roboto = font_db
        .load_from_bytes("Roboto", font_bytes)
        .expect("should be able to load Roboto font for test");
    (font_db, roboto)
}

pub(crate) fn collect_glyph_indices(frame: &TextFrame) -> Vec<Vec<usize>> {
    frame
        .lines()
        .iter()
        .map(|line| {
            line.runs
                .iter()
                .flat_map(|run| run.glyphs.iter())
                .map(|glyph| glyph.index)
                .collect_vec()
        })
        .collect_vec()
}

fn collect_caret_position_start_offsets(frame: &TextFrame) -> Vec<Vec<usize>> {
    frame
        .lines()
        .iter()
        .map(collect_line_caret_position_starts)
        .collect_vec()
}

fn collect_caret_position_last_offsets(frame: &TextFrame) -> Vec<Vec<usize>> {
    frame
        .lines()
        .iter()
        .map(|line| {
            line.caret_positions
                .iter()
                .map(|caret_position| caret_position.last_offset)
                .collect_vec()
        })
        .collect_vec()
}

pub(crate) fn collect_line_caret_position_starts(line: &Line) -> Vec<usize> {
    line.caret_positions
        .iter()
        .map(|pos| pos.start_offset)
        .collect_vec()
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
fn test_leading_newline_caret_positions() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    let text = "\nstart on the\nsecond line";

    let frame = font_db.text_layout_system().layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..26,
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        10000.0,
        10000.0,
        TextAlignment::Left,
        None,
    );

    let caret_position_starts = collect_caret_position_start_offsets(&frame);

    assert_eq!(
        caret_position_starts,
        vec![
            (0..1).collect_vec(),
            (1..14).collect_vec(),
            (14..25).collect_vec(),
        ],
    );

    let caret_position_lasts = collect_caret_position_last_offsets(&frame);

    assert_eq!(
        caret_position_lasts,
        vec![
            (0..1).collect_vec(),
            (1..14).collect_vec(),
            (14..25).collect_vec(),
        ],
    );

    Ok(())
}

#[test]
fn test_multiline_caret_positions() -> Result<()> {
    let (font_db, font_family) = init_fonts();
    let text = "eel\n\nivy\nthesaurus\n\nclingy";

    let frame = font_db.text_layout_system().layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..26,
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        10000.0,
        10000.0,
        TextAlignment::Left,
        None,
    );

    let caret_position_starts = collect_caret_position_start_offsets(&frame);

    assert_eq!(
        caret_position_starts,
        vec![
            (0..4).collect_vec(),
            (4..5).collect_vec(),
            (5..9).collect_vec(),
            (9..19).collect_vec(),
            (19..20).collect_vec(),
            (20..26).collect_vec(),
        ],
    );

    let caret_position_lasts = collect_caret_position_last_offsets(&frame);

    assert_eq!(
        caret_position_lasts,
        vec![
            (0..4).collect_vec(),
            (4..5).collect_vec(),
            (5..9).collect_vec(),
            (9..19).collect_vec(),
            (19..20).collect_vec(),
            (20..26).collect_vec(),
        ],
    );

    Ok(())
}

#[cfg_attr(
    not(macos),
    ignore = "discrepancy in winit vs. MacOS text layout implementation: glyph indices do not match"
)]
#[test]
fn test_layout_str_infinite_height() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    // Ensure layout_text terminates even if there's an unbounded `max_height`.
    let frame = font_db.text_layout_system().layout_text(
        "hello world",
        LineStyle {
            font_size: 14.,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..12,
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        0.,
        f32::INFINITY,
        Default::default(),
        None,
    );

    assert_eq!(frame.lines().len(), 10);

    Ok(())
}

#[test]
fn test_layout_str() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    font_db.text_layout_system().layout_line(
        "hello world 😃",
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[
            (
                0..2,
                StyleAndFont::new(
                    font_family,
                    Properties::default().weight(Weight::Bold),
                    TextStyle::new(),
                ),
            ),
            (
                2..6,
                StyleAndFont::new(
                    font_family,
                    Properties::default().style(Style::Italic),
                    TextStyle::new(),
                ),
            ),
            (
                6..13,
                StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
            ),
        ],
        100000.0,
        ClipConfig::default(),
    );

    Ok(())
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
fn test_layout_str_with_style() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    let line = font_db.text_layout_system().layout_line(
        "hello world 😃",
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[
            (
                0..2,
                StyleAndFont::new(
                    font_family,
                    Properties::default().weight(Weight::Bold),
                    TextStyle::new().with_foreground_color(ColorU::from_u32(0xFF0000FF)),
                ),
            ),
            (
                2..6,
                StyleAndFont::new(
                    font_family,
                    Properties::default().style(Style::Italic),
                    TextStyle::new(),
                ),
            ),
            (
                6..13,
                StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
            ),
        ],
        100000.0,
        ClipConfig::default(),
    );

    // The first run should have a red foreground color, while the rest of the runs should not
    // have a foreground color set.
    assert_eq!(line.runs.len(), 4);

    let foreground_colors: Vec<_> = line
        .runs
        .into_iter()
        .map(|run| run.styles.foreground_color)
        .collect();

    assert_eq!(
        foreground_colors,
        vec![Some(ColorU::from_u32(0xFF0000FF)), None, None, None]
    );

    Ok(())
}

#[cfg_attr(
    not(macos),
    ignore = "discrepancy in winit vs. MacOS text layout implementation: glyph indices do not match"
)]
#[test]
fn test_multiline_glyph_indices() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    let text = "eel\n\nivy\nthesaurus\n\nclingy";

    let frame = font_db.text_layout_system().layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..26,
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        10000.0,
        10000.0,
        TextAlignment::Left,
        None,
    );

    let glyph_indices = collect_glyph_indices(&frame);
    assert_eq!(
        glyph_indices,
        vec![
            (0..4).collect_vec(),
            (4..5).collect_vec(),
            (5..9).collect_vec(),
            (9..19).collect_vec(),
            (19..20).collect_vec(),
            (20..26).collect_vec(),
        ],
    );

    Ok(())
}

#[test]
fn test_layout_mixed_ltr_rtl_text() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    let text = "Help! Is this عربي?";
    let frame = font_db.text_layout_system().layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..text.encode_utf16().count(),
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        f32::MAX, /* max_width */
        f32::MAX, /* max_height */
        Default::default(),
        None,
    );

    // Assert that we successfully laid everything out in a single line
    // without panicking.
    assert_eq!(frame.lines().len(), 1);

    Ok(())
}

#[test]
fn test_char_indices() -> Result<()> {
    let (font_db, ligatured_font) = init_fonts();

    let text = "fluffing pillows";
    let line = font_db.text_layout_system().layout_line(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..text.encode_utf16().count(),
            StyleAndFont::new(ligatured_font, Properties::default(), TextStyle::new()),
        )],
        10000.0,
        ClipConfig::default(),
    );

    // It's easiest to understand what's happening here by visualizing the text and seeing which
    // characters get combined to become a single glyph. At a high level, what this is testing
    // is that after laying out the string, we see some characters get combined into a single
    // glyph. For example, the text "Zapfino" gets combined into a single glyph, which is why
    // there is a jump from 23 to 30 in the list of glyph indices below.
    // See https://docs.google.com/drawings/d/18qOKhzA5rWaMuxKVeWFDXh7ebrDjxongarAckkm0qnE/edit
    // for a full diagram of what's happening here.
    assert_eq!(
        line.runs
            .iter()
            .flat_map(|r| r.glyphs.iter())
            .map(|g| g.index)
            .collect::<Vec<_>>(),
        vec![0, 2, 3, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    );
    Ok(())
}

#[test]
fn test_caret_positions() -> Result<()> {
    let (font_db, ligatured_font) = init_fonts();

    // This string has 16 characters, but 14 UTF-16 code points.
    // Each 'fl' or 'fi' character encodes as 2 UTF-16 code points.
    let text: &str = "fluffing pillows";
    //                0123456789012345
    //                0 12  3456789012

    let line = font_db.text_layout_system().layout_line(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..text.encode_utf16().count(),
            StyleAndFont::new(ligatured_font, Properties::default(), TextStyle::new()),
        )],
        10000.0,
        ClipConfig::default(),
    );

    // There are only 13 glyphs because 'fl' and 'ffi' each have ligatures.
    assert_eq!(
        line.runs.iter().map(|run| run.glyphs.len()).sum::<usize>(),
        13
    );

    // On MacOS, there should be a caret position for each character.
    #[cfg(target_os = "macos")]
    assert_eq!(
        collect_line_caret_position_starts(&line),
        (0..16).collect::<Vec<usize>>()
    );

    // With cosmic-text, we only get one caret position per visual glyph.
    #[cfg(not(target_os = "macos"))]
    assert_eq!(
        collect_line_caret_position_starts(&line),
        [0, 2, 3, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    );

    // On MacOS, there is a caret for the 3rd character at the 3rd position, even though
    // the first 2 characters ("fl") are represented with 1 glyph.
    #[cfg(target_os = "macos")]
    assert_eq!(
        line.caret_position_for_index(3),
        line.caret_positions[3].position_in_line
    );

    Ok(())
}

#[cfg_attr(
    not(macos),
    ignore = "discrepancy in winit vs. MacOS text layout implementation: glyph indices do not match"
)]
#[test]
fn test_layout_text() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    let text = "This is a sample text layout!";
    let frame = font_db.text_layout_system().layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..text.encode_utf16().count(),
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        100.,     /* max_width */
        f32::MAX, /* max_height */
        Default::default(),
        None,
    );

    // The text should contain multiple lines since it can't fit in 100 pixels on the first
    // line.
    assert_eq!(frame.lines().len(), 3);

    // The text should be wrapped over 4 lines and look like this:
    // "This is a"
    // "sample text"
    // "layout!"
    assert_eq!(
        collect_glyph_indices(&frame),
        vec![
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            vec![10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21],
            vec![22, 23, 24, 25, 26, 27, 28],
        ]
    );

    Ok(())
}

#[cfg_attr(
    not(macos),
    ignore = "discrepancy in winit vs. MacOS text layout implementation: glyph indices do not match"
)]
#[test]
fn test_layout_text_first_line_head_indent() -> Result<()> {
    // Similar test to above, except we add in a left head indent (with reduced max width)!
    let (font_db, font_family) = init_fonts();

    let text = "This is a sample text layout!";
    let frame = font_db.text_layout_system().layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..text.encode_utf16().count(),
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        80.,      /* max_width */
        f32::MAX, /* max_height */
        Default::default(),
        Some(50.), /* first_line_head_indent */
    );

    // The text should contain multiple lines since we have a 50px left head indent on the first
    // line and then each line only has 80px.
    assert_eq!(frame.lines().len(), 4);

    assert_eq!(
        collect_glyph_indices(&frame),
        vec![
            vec![0, 1, 2],
            vec![3, 4, 5, 6, 7, 8, 9],
            vec![10, 11, 12, 13, 14, 15, 16],
            vec![17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28],
        ]
    );

    Ok(())
}

#[cfg_attr(
    not(macos),
    ignore = "discrepancy in winit vs. MacOS text layout implementation: glyph indices do not match"
)]
#[test]
fn test_layout_text_large_first_line_head_indent() -> Result<()> {
    // Similar test to above, except we have a large first line head indent which goes beyond the
    // max_width of the first line! We expect an empty line at the start to account for this (post-layout).
    let (font_db, font_family) = init_fonts();

    let text = "This is a sample text layout!";
    let frame = font_db.text_layout_system().layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..text.encode_utf16().count(),
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        80.,      /* max_width */
        f32::MAX, /* max_height */
        Default::default(),
        Some(80.), /* first_line_head_indent */
    );

    // We expect 1 empty line at the start and then 7 lines of content.
    assert_eq!(frame.lines().len(), 4);

    // CoreText leaves newline glyphs in the laid-out lines.
    #[cfg(target_os = "macos")]
    assert_eq!(
        collect_glyph_indices(&frame),
        vec![
            vec![], // first line head indent takes up entire line!
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            vec![10, 11, 12, 13, 14, 15, 16],
            vec![17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28],
        ]
    );

    // cosmic-text strips newline glyphs from the laid-out lines.
    #[cfg(not(target_os = "macos"))]
    assert_eq!(
        collect_glyph_indices(&frame),
        vec![
            vec![], // first line head indent takes up entire line!
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            vec![10, 11, 12, 13, 14, 15],
            vec![17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28],
        ]
    );

    Ok(())
}

#[cfg_attr(
    not(macos),
    ignore = "discrepancy in winit vs. MacOS text layout implementation: glyph indices do not match"
)]
#[test]
fn test_layout_text_last_line_clipped() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    let text = "This text doesn't fit in one line";

    let max_width = 100.;
    let frame = font_db.text_layout_system().layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..text.encode_utf16().count(),
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        max_width,
        30., /* max_height */
        Default::default(),
        None,
    );

    // The text should only fit one line.
    assert_eq!(frame.lines().len(), 1);

    // The text is one line long and should be clipped like so: "this text doesn't...".
    // Note that the contents are not clipped, but the width exceeding the max width
    // and the non-none clip direction indicate that when we paint, this is clipped.
    assert_eq!(
        collect_glyph_indices(&frame),
        vec![[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32
        ],]
    );
    let first_line = frame.lines().first().unwrap();
    assert!(first_line.width > max_width);
    assert!(first_line.clip_config.is_some());

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

    // CoreText leaves newline glyphs in the laid-out lines.
    #[cfg(target_os = "macos")]
    assert_eq!(
        collect_glyph_indices(&no_indent_frame),
        vec![
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],       // 9 is whitespace.
            vec![10, 11, 12, 13, 14, 15, 16, 17, 18], // 18 is whitespace.
            vec![19, 20, 21, 22, 23, 24, 25],         // 25 is whitespace.
            vec![26, 27, 28, 29, 30],
        ]
    );

    // cosmic-text strips newline glyphs from the laid-out lines.
    #[cfg(not(target_os = "macos"))]
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

    // CoreText leaves newline glyphs in the laid-out lines.
    #[cfg(target_os = "macos")]
    assert_eq!(
        collect_glyph_indices(&small_indent_frame),
        vec![
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            vec![10, 11, 12, 13, 14, 15, 16, 17, 18],
            vec![19, 20, 21, 22, 23, 24, 25],
            vec![26, 27, 28, 29, 30],
        ]
    );

    // cosmic-text strips newline glyphs from the laid-out lines.
    #[cfg(not(target_os = "macos"))]
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

    // CoreText leaves newline glyphs in the laid-out lines.
    #[cfg(target_os = "macos")]
    assert_eq!(
        collect_glyph_indices(&half_indent_frame),
        vec![
            vec![0, 1, 2, 3, 4, 5], // Fewer glyphs fit on this line. 5 is whitespace.
            vec![6, 7, 8, 9, 10, 11, 12, 13], // 13 is whitespace.
            vec![14, 15, 16, 17, 18],
            vec![19, 20, 21, 22, 23, 24, 25],
            vec![26, 27, 28, 29, 30],
        ]
    );

    // cosmic-text strips newline glyphs from the laid-out lines.
    #[cfg(not(target_os = "macos"))]
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

    // CoreText leaves newline glyphs in the laid-out lines.
    #[cfg(target_os = "macos")]
    assert_eq!(
        collect_glyph_indices(&no_indent_frame),
        vec![
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            vec![10, 11, 12, 13, 14, 15, 16, 17, 18],
            vec![19, 20, 21, 22, 23, 24, 25],
            vec![26, 27, 28, 29, 30],
        ]
    );

    // cosmic-text strips newline glyphs from the laid-out lines.
    #[cfg(not(target_os = "macos"))]
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

    // CoreText leaves newline glyphs in the laid-out lines.
    #[cfg(target_os = "macos")]
    assert_eq!(
        collect_glyph_indices(&overflow_indent_frame),
        vec![
            vec![0, 1], // Only a few glyphs fit.
            vec![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
            vec![14, 15, 16, 17, 18],
            vec![19, 20, 21, 22, 23, 24, 25],
            vec![26, 27, 28, 29, 30],
        ]
    );

    // cosmic-text strips newline glyphs from the laid-out lines.
    #[cfg(not(target_os = "macos"))]
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

    // CoreText leaves newline glyphs in the laid-out lines.
    #[cfg(target_os = "macos")]
    assert_eq!(
        collect_glyph_indices(&no_indent_frame),
        vec![
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            vec![10, 11, 12, 13, 14, 15, 16, 17, 18],
            vec![19, 20, 21, 22, 23, 24, 25],
            vec![26, 27, 28, 29, 30],
        ]
    );

    // cosmic-text strips newline glyphs from the laid-out lines.
    #[cfg(not(target_os = "macos"))]
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

    // CoreText leaves newline glyphs in the laid-out lines.
    #[cfg(target_os = "macos")]
    assert_eq!(
        collect_glyph_indices(&overflow_indent_frame),
        vec![
            vec![], // No glyphs fit on this line.
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            vec![10, 11, 12, 13, 14, 15, 16, 17, 18],
            vec![19, 20, 21, 22, 23, 24, 25],
            vec![26, 27, 28, 29, 30],
        ]
    );

    // cosmic-text strips newline glyphs from the laid-out lines.
    #[cfg(not(target_os = "macos"))]
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

    // CoreText leaves newline glyphs in the laid-out lines.
    #[cfg(target_os = "macos")]
    assert_eq!(
        collect_glyph_indices(&big_indent_frame),
        vec![
            vec![], // No glyphs fit on this line.
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            vec![10, 11, 12, 13, 14, 15, 16, 17, 18],
            vec![19, 20, 21, 22, 23, 24, 25],
            vec![26, 27, 28, 29, 30],
        ]
    );

    // cosmic-text strips newline glyphs from the laid-out lines.
    #[cfg(not(target_os = "macos"))]
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
#[cfg_attr(
    not(macos),
    ignore = "discrepancy in winit vs. MacOS text layout implementation: glyph indices do not match"
)]
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
    assert_eq!(no_indent_frame.lines().len(), 5);
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
    assert_eq!(small_indent_frame.lines().len(), 5);
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
#[cfg_attr(
    not(macos),
    ignore = "discrepancy in winit vs. MacOS text layout implementation: glyph indices do not match"
)]
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
    assert_eq!(no_indent_frame.lines().len(), 5);
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
#[cfg_attr(
    not(macos),
    ignore = "discrepancy in winit vs. MacOS text layout implementation: glyph indices do not match"
)]
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
    assert_eq!(no_indent_frame.lines().len(), 5);
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
    assert_eq!(overflow_indent_frame.lines().len(), 6);
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
    assert_eq!(big_indent_frame.lines().len(), 6);
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
