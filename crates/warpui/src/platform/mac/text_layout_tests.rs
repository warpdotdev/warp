use super::*;
use crate::fonts::Properties;

use crate::fonts::{collect_glyph_indices, collect_line_caret_position_starts, init_fonts};
use crate::platform::FontDB as _;
use crate::text_layout::DEFAULT_TOP_BOTTOM_RATIO;

use anyhow::Result;
use rand::random;

pub(crate) fn collect_line_caret_position_pairs(line: &Line) -> Vec<(usize, usize)> {
    line.caret_positions
        .iter()
        .map(|pos| (pos.start_offset, pos.last_offset))
        .collect_vec()
}

#[test]
fn test_char_indices_ligatures() -> Result<()> {
    let mut font_db = FontDB::new();
    let zapfino = font_db.load_from_system("Zapfino")?;
    let menlo = font_db.load_from_system("Menlo")?;

    let text = "This is, m𐍈re 𐍈r less, Zapfino!𐍈";
    let line = layout_line(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[
            (
                0..9,
                StyleAndFont::new(zapfino, Properties::default(), TextStyle::new()),
            ),
            (
                9..22,
                StyleAndFont::new(menlo, Properties::default(), TextStyle::new()),
            ),
            (
                22..text.encode_utf16().count(),
                StyleAndFont::new(zapfino, Properties::default(), TextStyle::new()),
            ),
        ],
        &font_db,
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
        vec![0, 2, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 30, 31]
    );
    Ok(())
}

#[test]
fn test_caret_positions_ligatures() -> Result<()> {
    // There's some overlap between caret positions and the character indices we
    // store in glyphs. However, a single glyph may have multiple caret positions
    // because characters/graphemes may get combined into a single glyph.

    let mut font_db = FontDB::new();
    let zapfino = font_db.load_from_system("Zapfino")?;
    let menlo = font_db.load_from_system("Menlo")?;

    // This string has 32 characters, but 35 UTF-16 code points and 41 UTF-8 code points.
    // Each '𐍈' character encodes as 2 UTF-16 code points or 4 UTF-8 code points.
    let text = "This is, m𐍈re 𐍈r less, Zapfino!𐍈";

    let line = layout_line(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[
            (
                0..9,
                StyleAndFont::new(zapfino, Properties::default(), TextStyle::new()),
            ),
            (
                9..22,
                StyleAndFont::new(menlo, Properties::default(), TextStyle::new()),
            ),
            (
                22..35,
                StyleAndFont::new(zapfino, Properties::default(), TextStyle::new()),
            ),
        ],
        &font_db,
        ClipConfig::default(),
    );

    // There are only 23 glyphs because 'Zapfino', 'Th', and 'is' each have ligatures.
    assert_eq!(
        line.runs.iter().map(|run| run.glyphs.len()).sum::<usize>(),
        23
    );

    // There should be a caret position for each character.
    assert_eq!(
        line.caret_positions
            .iter()
            .map(|pos| pos.start_offset)
            .collect::<Vec<_>>(),
        (0..32).collect::<Vec<usize>>()
    );

    // There is a caret for the 3rd character at the 3rd position, even though
    // the first 2 characters are represented with 1 glyph.
    assert_eq!(
        line.caret_position_for_index(3),
        line.caret_positions[3].position_in_line
    );

    // Likewise for the second 𐍈, even though it (and the previous one) take
    // multiple code points.
    assert_eq!(
        line.caret_position_for_index(15),
        line.caret_positions[15].position_in_line
    );

    // This tests hit-testing on a regular character.
    assert_eq!(line.caret_index_for_x(0.), Some(0));
    assert_eq!(line.caret_index_for_x(20.), Some(1));

    // This tests hit-testing within a ligature.
    assert_eq!(line.caret_index_for_x(260.), Some(25));

    // This tests rounding up to the next character.
    assert_eq!(line.caret_index_for_x(268.), Some(26));

    // This tests a few random positions within the bound and before the last character.
    let last_caret_pos = line
        .caret_positions
        .last()
        .map_or(0., |p| p.position_in_line);
    // The bounded and unbounded method should return the same result.
    for _ in 0..5 {
        let pos: f32 = random();
        let index = line.caret_index_for_x(pos * last_caret_pos);
        assert_eq!(
            index,
            Some(line.caret_index_for_x_unbounded(pos * last_caret_pos))
        );
    }

    // This tests that the unbounded method returns the first index for out-of-bound position to the left
    assert_eq!(line.caret_index_for_x_unbounded(-1.), line.first_index());
    // The bounded method should return `None`
    assert_eq!(line.caret_index_for_x(-1.), None);

    // This tests that the unbounded method returns the end index for out-of-bound position to the right
    assert_eq!(
        line.caret_index_for_x_unbounded(line.width + 0.1),
        line.end_index()
    );
    assert_eq!(line.caret_index_for_x(line.width + 0.1), None);

    // This tests that the unbounded method returns the correct index either before or after the last glyph
    assert_eq!(
        line.caret_index_for_x_unbounded(0.9 * last_caret_pos + 0.1 * line.width),
        line.last_index()
    );
    assert_eq!(
        line.caret_index_for_x_unbounded(0.1 * last_caret_pos + 0.9 * line.width),
        line.end_index()
    );
    // The bounded method should always just return the last index
    assert_eq!(
        line.caret_index_for_x(0.9 * last_caret_pos + 0.1 * line.width),
        Some(line.last_index())
    );
    assert_eq!(
        line.caret_index_for_x(0.1 * last_caret_pos + 0.9 * line.width),
        Some(line.last_index())
    );

    Ok(())
}

/// The emojis in this test use font fallback, which means it won't behave
/// consistently across platforms.
#[test]
fn test_emoji_caret_positions() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    // We're using these emoji specifically because they're represented as multiple
    // combined characters.
    let text = "👨‍👧‍👧🇨🇦";

    let line = font_db.text_layout_system().layout_line(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..12,
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        10000.0,
        ClipConfig::default(),
    );

    // We want the leading edge for caret positions, so the first one is at the
    // start of the line.
    assert_eq!(line.caret_positions[0].position_in_line, 0.0);

    assert_eq!(
        collect_line_caret_position_starts(&line),
        // CoreText gives us one caret position per visible character.
        // Each emoji is multiple characters, but one grapheme and therefore one
        // caret position.
        vec![0, 5]
    );

    // The first character is within the first emoji, so its caret position is
    // at the start of the line.
    assert_eq!(line.caret_position_for_index(0), 0.0);
    // Likewise, the start of the next emoji returns its start position.
    assert_eq!(
        line.caret_position_for_index(5),
        line.caret_positions[1].position_in_line
    );
    // Subsequent positions within the emoji also resolve to its starting offset.
    assert_eq!(
        line.caret_position_for_index(6),
        line.caret_positions[1].position_in_line
    );
    // Past the end of the last emoji, we clamp to the end of the line.
    assert_eq!(line.caret_position_for_index(7), line.width);

    Ok(())
}

/// The RTL text and emoji in this test use font fallback, which means
/// this test won't behave consistently across platforms.
#[test]
fn test_bidi_caret_positions() -> Result<()> {
    let (font_db, font_family) = init_fonts();

    let text = "a שָׁלוֹם 🇨🇦 test";
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
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )],
        10000.0,
        ClipConfig::default(),
    );

    // Caret positions should account for diacritics in the Hebrew text, as well
    // as the 🇨🇦 emoji consisting of multiple characters. In addition, they should
    // be sorted by display order.
    assert_eq!(
        collect_line_caret_position_pairs(&line),
        vec![
            // "a "
            (0, 0),
            (1, 1),
            // שָׁלוֹם
            (8, 8),
            (6, 7),
            (5, 5),
            (2, 4),
            // " "
            (9, 9),
            // 🇨🇦
            (10, 11),
            // " test"
            (12, 12),
            (13, 13),
            (14, 14),
            (15, 15),
            (16, 16)
        ]
    );

    Ok(())
}

#[test]
fn test_layout_text_ligatures() -> Result<()> {
    let mut font_db = FontDB::new();
    let zapfino = font_db.load_from_system("Zapfino")?;
    let menlo = font_db.load_from_system("Menlo")?;

    let text = "This is, m𐍈re 𐍈r less, Zapfino!𐍈";
    let frame = layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[
            (
                0..9,
                StyleAndFont::new(zapfino, Properties::default(), TextStyle::new()),
            ),
            (
                9..22,
                StyleAndFont::new(menlo, Properties::default(), TextStyle::new()),
            ),
            (
                22..text.encode_utf16().count(),
                StyleAndFont::new(zapfino, Properties::default(), TextStyle::new()),
            ),
        ],
        &font_db,
        125.,     /* max_width */
        f32::MAX, /* max_height */
        Default::default(),
        None,
    );

    // The text should contain multiple lines since it can't fit in 125 pixels on the first
    // line.
    assert_eq!(frame.lines().len(), 4);

    // The text should be wrapped over 4 lines and look like this:
    // "This is
    // m𐍈re or
    // less,
    // Zapfino!𐍈"
    assert_eq!(
        collect_glyph_indices(&frame),
        vec![
            vec![0, 2, 4, 5, 7, 8],
            vec![9, 10, 11, 12, 13, 14, 15, 16],
            vec![17, 18, 19, 20, 21, 22,],
            vec![23, 30, 31]
        ]
    );

    Ok(())
}

#[test]
fn test_layout_text_first_line_head_indent_ligatures() -> Result<()> {
    // Similar test to above, except we add in a left head indent (with reduced max width)!
    let mut font_db = FontDB::new();
    let zapfino = font_db.load_from_system("Zapfino")?;
    let menlo = font_db.load_from_system("Menlo")?;

    let text = "This is, m𐍈re 𐍈r less, Zapfino!𐍈";
    let frame = layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[
            (
                0..9,
                StyleAndFont::new(zapfino, Properties::default(), TextStyle::new()),
            ),
            (
                9..22,
                StyleAndFont::new(menlo, Properties::default(), TextStyle::new()),
            ),
            (
                22..text.encode_utf16().count(),
                StyleAndFont::new(zapfino, Properties::default(), TextStyle::new()),
            ),
        ],
        &font_db,
        80.,      /* max_width */
        f32::MAX, /* max_height */
        Default::default(),
        Some(50.), /* first_line_head_indent */
    );

    // The text should contain multiple lines since we have a 50px left head indent on the first
    // line and then each line only has 80px.
    assert_eq!(frame.lines().len(), 6);

    assert_eq!(
        collect_glyph_indices(&frame),
        vec![
            vec![0], // left head indent means we don't have much content laid out on this line.
            vec![1, 2, 4, 5, 7, 8],
            vec![9, 10, 11, 12, 13, 14, 15, 16],
            vec![17, 18, 19, 20, 21, 22],
            vec![23, 24, 25, 27, 28],
            vec![29, 30, 31],
        ]
    );

    Ok(())
}

#[test]
fn test_tab_stops_affect_line_width() -> Result<()> {
    let mut font_db = FontDB::new();
    let menlo = font_db.load_from_system("Menlo")?;

    let font_size = 13.0;
    let tab_size = 4;

    let font_id = font_db.select_font(menlo, Properties::default());
    let tab_interval = (font_db
        .space_advance_width(font_id, font_size)
        .expect("space width should be measurable")
        * tab_size as f64) as f32;

    let style = StyleAndFont::new(menlo, Properties::default(), TextStyle::new());
    let strings = "strings";
    let tabbed = "\t\t\tstrings";

    let strings_line = layout_line(
        strings,
        LineStyle {
            font_size,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(0..strings.chars().count(), style)],
        &font_db,
        ClipConfig::default(),
    );

    let tabbed_line = layout_line(
        tabbed,
        LineStyle {
            font_size,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: Some(tab_size),
        },
        &[(0..tabbed.chars().count(), style)],
        &font_db,
        ClipConfig::default(),
    );

    // Each tab advances to the next stop.
    let expected_width = (tab_interval * 3.0) + strings_line.width;
    let error = (tabbed_line.width - expected_width).abs();
    assert!(
        error < 1.0,
        "expected tabbed width ~{}, got {} (error {})",
        expected_width,
        tabbed_line.width,
        error
    );

    Ok(())
}

#[test]
fn test_tab_stops_do_not_drift_over_long_runs() -> Result<()> {
    let mut font_db = FontDB::new();
    let menlo = font_db.load_from_system("Menlo")?;

    let font_size = 13.0;
    let tab_size = 4;

    let font_id = font_db.select_font(menlo, Properties::default());
    let tab_interval = (font_db
        .space_advance_width(font_id, font_size)
        .expect("space width should be measurable")
        * tab_size as f64) as f32;

    let style = StyleAndFont::new(menlo, Properties::default(), TextStyle::new());
    let strings = "strings";

    let strings_line = layout_line(
        strings,
        LineStyle {
            font_size,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(0..strings.chars().count(), style)],
        &font_db,
        ClipConfig::default(),
    );

    let tab_count = 100;
    let tabbed = format!("{}{}", "\t".repeat(tab_count), strings);

    let tabbed_line = layout_line(
        &tabbed,
        LineStyle {
            font_size,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: Some(tab_size),
        },
        &[(0..tabbed.chars().count(), style)],
        &font_db,
        ClipConfig::default(),
    );

    let expected_width = (tab_interval * tab_count as f32) + strings_line.width;
    let error = (tabbed_line.width - expected_width).abs();
    assert!(
        error < 1.0,
        "expected tabbed width ~{}, got {} (error {})",
        expected_width,
        tabbed_line.width,
        error
    );

    Ok(())
}

#[test]
fn test_layout_text_large_first_line_head_indent_ligatures() -> Result<()> {
    // Similar test to above, except we have a large first line head indent which goes beyond the
    // max_width of the first line! We expect an empty line at the start to account for this (post-layout).
    let mut font_db = FontDB::new();
    let zapfino = font_db.load_from_system("Zapfino")?;
    let menlo = font_db.load_from_system("Menlo")?;

    let text = "This is, some text, in Zapfino being laid out!";
    let frame = layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[
            (
                0..9,
                StyleAndFont::new(zapfino, Properties::default(), TextStyle::new()),
            ),
            (
                9..22,
                StyleAndFont::new(menlo, Properties::default(), TextStyle::new()),
            ),
            (
                22..text.encode_utf16().count(),
                StyleAndFont::new(zapfino, Properties::default(), TextStyle::new()),
            ),
        ],
        &font_db,
        80.,      /* max_width */
        f32::MAX, /* max_height */
        Default::default(),
        Some(80.), /* first_line_head_indent */
    );

    // We expect 1 empty line at the start and then 7 lines of content.
    assert_eq!(frame.lines().len(), 8);

    assert_eq!(
        collect_glyph_indices(&frame),
        vec![
            vec![], // first line head indent takes up entire line!
            vec![0, 2, 4],
            vec![5, 7, 8, 9, 10, 11, 12, 13],
            vec![14, 15, 16, 17, 18, 19, 20, 21, 22],
            vec![23, 24, 25, 27, 28],
            vec![29, 30, 31, 32, 33, 34, 35, 36],
            vec![37, 38, 39, 40, 41],
            vec![42, 44, 45],
        ]
    );

    Ok(())
}

#[test]
fn test_layout_text_last_line_clipped_ligatures() -> Result<()> {
    let mut font_db = FontDB::new();
    let zapfino = font_db.load_from_system("Zapfino")?;
    let menlo = font_db.load_from_system("Menlo")?;

    let text = "m𐍈re, Zapfino!ll𐍈, qqqq";
    let max_width = 180.;

    let frame = layout_text(
        text,
        LineStyle {
            font_size: 16.0,
            line_height_ratio: 1.2,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[
            (
                0..5,
                StyleAndFont::new(menlo, Properties::default(), TextStyle::new()),
            ),
            (
                5..13,
                StyleAndFont::new(zapfino, Properties::default(), TextStyle::new()),
            ),
            (
                13..16,
                StyleAndFont::new(menlo, Properties::default(), TextStyle::new()),
            ),
            (
                16..text.encode_utf16().count(),
                StyleAndFont::new(menlo, Properties::default(), TextStyle::new()),
            ),
        ],
        &font_db,
        max_width,
        70., /* max_height */
        Default::default(),
        None,
    );

    // The text should only fit one line.
    assert_eq!(frame.lines().len(), 1);

    // The text is one line long and should be clipped like so: "m𐍈re, Zapfin𐍈!l...".
    // Note that the contents are not clipped, but the width being greater than the max width
    // indicates that when we paint, this is clipped.
    assert_eq!(
        collect_glyph_indices(&frame),
        vec![[0, 1, 2, 3, 4, 5, 6, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22],]
    );
    let first_line = frame.lines().first().unwrap();
    assert!(first_line.width > max_width);

    Ok(())
}
