use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use warpui::fonts::{Cache as FontCache, FamilyId};
use warpui::units::{IntoLines, Lines, Pixels};

use super::{AttributedStringBuilder, CachedBackgroundColor, active_or_next_match};
use crate::terminal::grid_size_util::calculate_grid_baseline_position;
use crate::terminal::model::char_or_str::CharOrStr;
use crate::terminal::model::index::Point;
use crate::terminal::model::selection::SelectionPoint;
use crate::terminal::{SizeInfo, grid_renderer};

fn rect_from_points(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> RectF {
    RectF::from_points(vec2f(min_x, min_y), vec2f(max_x, max_y))
}

// TODO(CORE-2002): Make test non-Mac specific by switching to using bundled Roboto font.
#[test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "Assumes existence of Arial font, which is only guaranteed on macOS"
)]
fn test_calculate_grid_baseline_position() {
    let font_db = warpui::platform::test::FontDB::new();
    let mut font_cache = FontCache::new(Box::new(font_db));
    // Note we've restricted this unit test to Mac, so we expect Arial to exist.
    let arial = font_cache
        .load_system_font("Arial")
        .expect("Arial must exist");
    let baseline_position = calculate_grid_baseline_position(
        &font_cache,
        arial,
        16., /* font_size */
        1.2, /* line_height_ratio */
        19., /* cell_size_y */
    );
    assert_eq!(baseline_position, vec2f(0., 15.));
}

#[test]
fn test_next_match_same_row_matches() {
    let match_1 = Point::new(0, 0)..=Point::new(0, 4);
    let match_2 = Point::new(1, 0)..=Point::new(1, 4);
    let matches = [match_1.clone(), match_2.clone()];
    let mut filter_match_iter = matches.iter();

    let mut current_match = None;

    // The first match should return for points (0,0) through (0,4).
    for i in 0..=4 {
        current_match =
            active_or_next_match(&mut filter_match_iter, current_match, &Point::new(0, i));
        assert_eq!(current_match, Some(&match_1));
    }

    // The second match should return for points (1,0) through (1,4).
    for i in 0..=4 {
        current_match =
            active_or_next_match(&mut filter_match_iter, current_match, &Point::new(1, i));
        assert_eq!(current_match, Some(&match_2));
    }

    // There should be no more matches left after we advance to point (2,0).
    current_match = active_or_next_match(&mut filter_match_iter, current_match, &Point::new(2, 0));
    assert_eq!(current_match, None);
}

#[test]
fn test_next_match_multi_row_matches() {
    let match_1 = Point::new(0, 0)..=Point::new(1, 2);
    let match_2 = Point::new(2, 0)..=Point::new(3, 2);
    let matches = [match_1.clone(), match_2.clone()];
    let mut match_iter = matches.iter();

    let mut current_match = None;

    // The first match should be returned for all points from (0,0) to (1,2).
    let points_1 = [
        Point::new(0, 0),
        Point::new(0, 1),
        Point::new(0, 2),
        Point::new(1, 0),
        Point::new(1, 1),
        Point::new(1, 2),
    ];
    for point in points_1.iter() {
        current_match = active_or_next_match(&mut match_iter, current_match, point);
        assert_eq!(current_match, Some(&match_1));
    }

    // The second match should be returned for all points from (2,0) to (3,2).
    let points_2 = [
        Point::new(2, 0),
        Point::new(2, 1),
        Point::new(2, 2),
        Point::new(3, 0),
        Point::new(3, 1),
        Point::new(3, 2),
    ];
    for point in points_2.iter() {
        current_match = active_or_next_match(&mut match_iter, current_match, point);
        assert_eq!(current_match, Some(&match_2));
    }

    // There should be no more matches left after we advance to point (4,0).
    current_match = active_or_next_match(&mut match_iter, current_match, &Point::new(4, 0));
    assert_eq!(current_match, None);
}

#[test]
fn test_active_or_next_match_point_before_next_match() {
    let match_1 = Point::new(1, 0)..=Point::new(1, 4);
    let match_2 = Point::new(3, 0)..=Point::new(3, 4);
    let matches = [match_1.clone(), match_2.clone()];
    let mut match_iter = matches.iter();

    // The match for (0,0) should be the first match.
    let mut current_match = active_or_next_match(&mut match_iter, None, &Point::new(0, 0));
    assert_eq!(current_match, Some(&match_1));

    // The match for (2,0) should be the second match.
    current_match = active_or_next_match(&mut match_iter, current_match, &Point::new(2, 0));
    assert_eq!(current_match, Some(&match_2));
}

#[test]
fn test_calculate_background_bounds() {
    let origin = vec2f(100., 100.);
    let cell_size = vec2f(2., 4.);
    let max_columns = 150;
    let create_cached = |start_row: usize, start_col: usize, end_row: usize, end_col: usize| {
        CachedBackgroundColor {
            start: SelectionPoint {
                row: start_row.into_lines(),
                col: start_col,
            },
            end: SelectionPoint {
                row: end_row.into_lines(),
                col: end_col,
            },
            background_color: Default::default(),
        }
    };

    // Background with 1 row
    let (start_row, start_col, end_row, end_col) = (10, 20, 10, 130);
    let cached = create_cached(start_row, start_col, end_row, end_col);
    assert_eq!(
        grid_renderer::calculate_background_bounds(origin, cached, cell_size, max_columns),
        vec![rect_from_points(
            origin.x() + (start_col as f32) * cell_size.x(),
            origin.y() + (start_row as f32) * cell_size.y(),
            origin.x() + (end_col as f32 + 1.) * cell_size.x(),
            origin.y() + (end_row as f32 + 1.) * cell_size.y()
        )]
    );

    // Background with 2 rows
    let (start_row, start_col, end_row, end_col) = (20, 30, 21, 100);
    let cached = create_cached(start_row, start_col, end_row, end_col);
    assert_eq!(
        grid_renderer::calculate_background_bounds(origin, cached, cell_size, max_columns),
        vec![
            rect_from_points(
                origin.x() + (start_col as f32) * cell_size.x(),
                origin.y() + (start_row as f32) * cell_size.y(),
                origin.x() + (max_columns as f32 + 1.) * cell_size.x(),
                origin.y() + (start_row as f32 + 1.) * cell_size.y()
            ),
            rect_from_points(
                origin.x(),
                origin.y() + (start_row as f32 + 1.) * cell_size.y(),
                origin.x() + (end_col as f32 + 1.) * cell_size.x(),
                origin.y() + (end_row as f32 + 1.) * cell_size.y()
            ),
        ]
    );

    // Background with 3+ rows
    let assert_multi_row_selection_bounds =
        |start_row: usize, start_col: usize, end_row: usize, end_col: usize| {
            let cached = create_cached(start_row, start_col, end_row, end_col);
            assert_eq!(
                grid_renderer::calculate_background_bounds(origin, cached, cell_size, max_columns),
                vec![
                    rect_from_points(
                        origin.x() + (start_col as f32) * cell_size.x(),
                        origin.y() + (start_row as f32) * cell_size.y(),
                        origin.x() + (max_columns as f32 + 1.) * cell_size.x(),
                        origin.y() + (start_row as f32 + 1.) * cell_size.y()
                    ),
                    rect_from_points(
                        origin.x(),
                        origin.y() + (start_row as f32 + 1.) * cell_size.y(),
                        origin.x() + (max_columns as f32 + 1.) * cell_size.x(),
                        origin.y() + (end_row as f32) * cell_size.y()
                    ),
                    rect_from_points(
                        origin.x(),
                        origin.y() + (end_row as f32) * cell_size.y(),
                        origin.x() + (end_col as f32 + 1.) * cell_size.x(),
                        origin.y() + (end_row as f32 + 1.) * cell_size.y()
                    ),
                ]
            );
        };
    assert_multi_row_selection_bounds(30, 80, 32, 40); // 3 lines
    assert_multi_row_selection_bounds(40, 60, 43, 10); // 4 lines
    assert_multi_row_selection_bounds(50, 140, 59, 20); // 10 lines
}

/// Verifies that `AttributedStringBuilder::character_index_to_cell_map` is **char-indexed**, not
/// byte-indexed. `paint_line` looks up cells via `glyph.index`, which BOTH layout backends emit as
/// a *character* index — CoreText as `char_offset + char_index`, and cosmic-text after
/// `StrIndexMap::char_index` converts its raw byte offset. A byte-indexed map lines up with ASCII
/// by coincidence, but for multi-byte scripts (Thai, CJK, emoji, …) a char-index `glyph.index`
/// would land mid-codepoint and draw the glyph at the wrong column (or read out of bounds).
#[test]
fn test_attributed_string_builder_char_indexed_cell_map() {
    let dummy_family = FamilyId(0);
    let mut builder = AttributedStringBuilder::new(dummy_family, dummy_family, 10);

    // "สวัสดี" — every codepoint is 3 bytes in UTF-8. The cell layout that the terminal grid
    // produces for this string is:
    //   col 0: ส (Char)         col 1: วั (Str — ว + zerowidth ั)
    //   col 2: ส (Char)         col 3: ดี (Str — ด + zerowidth ี)
    builder.append_content(CharOrStr::Char('ส'), 0);
    builder.append_content(CharOrStr::Str("วั"), 1);
    builder.append_content(CharOrStr::Char('ส'), 2);
    builder.append_content(CharOrStr::Str("ดี"), 3);

    let data = builder.build();

    assert_eq!(
        data.line, "สวัสดี",
        "appended chars must round-trip into the line"
    );
    assert_eq!(
        data.line.len(),
        18,
        "six 3-byte Thai codepoints = 18 UTF-8 bytes"
    );
    assert_eq!(
        data.character_index_to_cell_map.len(),
        6,
        "the cell map must have one entry per CHARACTER (NOT per byte) — paint_line indexes it with \
         glyph.index, which both layout backends emit as a character index; a per-byte map would \
         put every non-ASCII glyph in the wrong cell"
    );

    // One entry per character, each pointing at its grid column.
    let expected = [
        0, // ส @ col 0
        1, // ว @ col 1
        1, // ั @ col 1 (combining mark stays in same cell as base)
        2, // ส @ col 2
        3, // ด @ col 3
        3, // ี @ col 3 (combining mark stays in same cell as base)
    ];
    assert_eq!(
        data.character_index_to_cell_map, expected,
        "each character (base or combining mark) maps to exactly one cell column"
    );
}

/// ASCII regression check — a build that wrongly switched back to byte-indexing would still pass
/// for pure-ASCII input (byte index == char index there), so we assert the char map for ASCII.
#[test]
fn test_attributed_string_builder_char_indexed_cell_map_ascii() {
    let dummy_family = FamilyId(0);
    let mut builder = AttributedStringBuilder::new(dummy_family, dummy_family, 10);

    builder.append_content(CharOrStr::Char('h'), 0);
    builder.append_content(CharOrStr::Char('i'), 1);

    let data = builder.build();
    assert_eq!(data.line, "hi");
    assert_eq!(data.character_index_to_cell_map, vec![0, 1]);
}

#[test]
fn test_calculate_selection_bounds() {
    let origin = vec2f(100., 100.);
    let size_info = SizeInfo::new(
        Vector2F::zero(),
        Pixels::new(2.),
        Pixels::new(4.),
        Pixels::new(8.),
        Pixels::new(16.),
    )
    .with_rows_and_columns(151, 151);

    let cell_width = size_info.cell_width_px.as_f32();
    let cell_height = size_info.cell_height_px.as_f32();
    let horizontal_padding = size_info.padding_x_px.as_f32();
    let max_columns = size_info.columns - 1;

    let make_selection_point = |row: usize, col: usize| SelectionPoint {
        row: row.into_lines(),
        col,
    };

    let start = make_selection_point(10, 10);
    let end = make_selection_point(20, 50);

    let assert_selection_bounds = |scroll_top: Lines| {
        assert_eq!(
            grid_renderer::calculate_selection_bounds(&start, &end, &size_info, scroll_top, origin),
            vec![
                rect_from_points(
                    origin.x() + horizontal_padding + (start.col as f32) * cell_width,
                    origin.y() + ((start.row - scroll_top).as_f64() as f32) * cell_height,
                    origin.x() + horizontal_padding + (max_columns as f32 + 1.) * cell_width,
                    origin.y() + ((start.row - scroll_top).as_f64() as f32 + 1.) * cell_height
                ),
                rect_from_points(
                    origin.x() + horizontal_padding,
                    origin.y() + ((start.row - scroll_top).as_f64() as f32 + 1.) * cell_height,
                    origin.x() + horizontal_padding + (max_columns as f32 + 1.) * cell_width,
                    origin.y() + ((end.row - scroll_top).as_f64() as f32) * cell_height
                ),
                rect_from_points(
                    origin.x() + horizontal_padding,
                    origin.y() + ((end.row - scroll_top).as_f64() as f32) * cell_height,
                    origin.x() + horizontal_padding + (end.col as f32 + 1.) * cell_width,
                    origin.y() + ((end.row - scroll_top).as_f64() as f32 + 1.) * cell_height
                ),
            ]
        );
    };
    assert_selection_bounds(5.into_lines()); // Without scroll clipping
    assert_selection_bounds(10.into_lines()); // Without scroll clipping (but on the cusp of clipping)
    assert_selection_bounds(80.into_lines()); // With scroll clipping
}

#[test]
fn test_decompose_sara_am() {
    // THAI CHARACTER SARA AM -> NIKHAHIT (dot, drawn over the consonant) + SARA AA (spacing tail).
    assert_eq!(
        super::decompose_sara_am('\u{0E33}'),
        Some(('\u{0E4D}', '\u{0E32}'))
    );
    // LAO VOWEL SIGN AM -> NIGGAHITA + LAO SARA AA.
    assert_eq!(
        super::decompose_sara_am('\u{0EB3}'),
        Some(('\u{0ECD}', '\u{0EB2}'))
    );
    // Plain Thai consonants and the bare sara-am parts are left untouched.
    assert_eq!(super::decompose_sara_am('\u{0E04}'), None); // ค
    assert_eq!(super::decompose_sara_am('\u{0E4D}'), None); // nikhahit
    assert_eq!(super::decompose_sara_am('\u{0E32}'), None); // sara aa
    assert_eq!(super::decompose_sara_am('a'), None);
}

#[test]
fn test_is_sara_am_base() {
    // Thai consonants (ก..ฮ) and Lao consonants (ກ..ຮ) are attachable bases.
    assert!(super::is_sara_am_base('\u{0E01}')); // ก
    assert!(super::is_sara_am_base('\u{0E19}')); // น
    assert!(super::is_sara_am_base('\u{0E2E}')); // ฮ
    assert!(super::is_sara_am_base('\u{0E81}')); // ກ (Lao)
    // Non-attachable cells a SARA AM must NOT be folded onto.
    assert!(!super::is_sara_am_base(' '));
    assert!(!super::is_sara_am_base('a'));
    assert!(!super::is_sara_am_base('.'));
    assert!(!super::is_sara_am_base('\u{0E33}')); // sara am itself
    assert!(!super::is_sara_am_base('\u{0E32}')); // sara aa (a vowel, not a base)
    assert!(!super::is_sara_am_base('\u{0E4D}')); // nikhahit
}
