use std::num::NonZeroU16;

use crate::model::grid::FlatStorage;

use super::*;

const ASCII_GRAPHEME_INFO: GraphemeInfo = GraphemeInfo {
    cell_width: 1,
    utf8_bytes: NonZeroU16::new(1).unwrap(),
};

const EMOJI_GRAPHEME_INFO: GraphemeInfo = GraphemeInfo {
    cell_width: 2,
    utf8_bytes: NonZeroU16::new(4).unwrap(),
};

#[test]
fn test_index_with_empty_string() {
    // 1: \n
    let storage = FlatStorage::from_content_using_rows("\n", 5, Some(1));
    assert_eq!(storage.index.rows.len(), 1);
}

#[test]
fn test_index_with_consistent_one_byte_length_and_cell_width() {
    // 1: abcde
    // 2: fgh\n
    let storage = FlatStorage::from_content_using_rows("abcdefgh\n", 5, Some(2));
    assert_eq!(storage.index.rows.len(), 2);

    assert_eq!(storage.index.rows[0].content_offset, ByteOffset::zero());
    assert_eq!(
        storage.index.rows[0].grapheme_sizing,
        GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(5).unwrap(),
            info: ASCII_GRAPHEME_INFO
        })
    );

    assert_eq!(storage.index.rows[1].content_offset, ByteOffset::from(5));
    assert_eq!(
        storage.index.rows[1].grapheme_sizing,
        GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(3).unwrap(),
            info: ASCII_GRAPHEME_INFO
        })
    );
}

#[test]
fn test_index_with_consistent_two_cell_width_and_four_byte_length() {
    // 1: 😀😃😄😁
    // 2: 😆😅😂\n
    let storage = FlatStorage::from_content_using_rows("😀😃😄😁😆😅😂\n", 8, Some(2));
    assert_eq!(storage.index.rows.len(), 2);

    assert_eq!(storage.index.rows[0].content_offset, ByteOffset::zero());
    assert_eq!(
        storage.index.rows[0].grapheme_sizing,
        GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(4).unwrap(),
            info: EMOJI_GRAPHEME_INFO
        })
    );

    assert_eq!(storage.index.rows[1].content_offset, ByteOffset::from(16));
    assert_eq!(
        storage.index.rows[1].grapheme_sizing,
        GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(3).unwrap(),
            info: EMOJI_GRAPHEME_INFO
        })
    );
}

#[test]
fn test_index_with_grapheme_overflowing_end_of_row() {
    // 1: 😀😃
    // 2: 😄\n
    let storage = FlatStorage::from_content_using_rows("😀😃😄\n", 5, Some(2));
    assert_eq!(storage.index.rows.len(), 2);

    assert_eq!(storage.index.rows[0].content_offset, ByteOffset::zero());
    assert_eq!(
        storage.index.rows[0].grapheme_sizing,
        GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(2).unwrap(),
            info: EMOJI_GRAPHEME_INFO
        })
    );

    assert_eq!(storage.index.rows[1].content_offset, ByteOffset::from(8));
    assert_eq!(
        storage.index.rows[1].grapheme_sizing,
        GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(1).unwrap(),
            info: EMOJI_GRAPHEME_INFO
        })
    );
}

#[test]
fn test_index_with_inconsistent_cell_widths() {
    // 1: 😀a😃
    // 2: 😄\n
    let storage = FlatStorage::from_content_using_rows("😀a😃😄\n", 5, Some(2));
    assert_eq!(storage.index.rows.len(), 2);

    assert_eq!(storage.index.rows[0].content_offset, ByteOffset::zero());
    assert_eq!(
        storage.index.rows[0].grapheme_sizing,
        GraphemeSizing::NonUniform
    );
    let grapheme_runs = storage
        .index
        .grapheme_sizing
        .get(&ByteOffset::zero())
        .expect("index should have grapheme run info");
    assert_eq!(grapheme_runs.len(), 3);
    assert_eq!(
        grapheme_runs[0],
        GraphemeRun {
            count: NonZeroU16::new(1).unwrap(),
            info: EMOJI_GRAPHEME_INFO,
        }
    );
    assert_eq!(
        grapheme_runs[1],
        GraphemeRun {
            count: NonZeroU16::new(1).unwrap(),
            info: ASCII_GRAPHEME_INFO,
        }
    );
    assert_eq!(
        grapheme_runs[2],
        GraphemeRun {
            count: NonZeroU16::new(1).unwrap(),
            info: EMOJI_GRAPHEME_INFO,
        }
    );

    assert_eq!(storage.index.rows[1].content_offset, ByteOffset::from(9));
    assert_eq!(
        storage.index.rows[1].grapheme_sizing,
        GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(1).unwrap(),
            info: EMOJI_GRAPHEME_INFO
        })
    );
}

#[test]
fn test_index_with_newlines() {
    // 1: abc\n
    // 2: defgh
    let storage = FlatStorage::from_content_using_rows("abc\ndefgh", 5, Some(2));
    assert_eq!(storage.index.rows.len(), 2);

    assert_eq!(storage.index.rows[0].content_offset, ByteOffset::zero());
    assert_eq!(
        storage.index.rows[0].grapheme_sizing,
        GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(3).unwrap(),
            info: ASCII_GRAPHEME_INFO
        })
    );

    assert_eq!(storage.index.rows[1].content_offset, ByteOffset::from(4));
    assert_eq!(
        storage.index.rows[1].grapheme_sizing,
        GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(5).unwrap(),
            info: ASCII_GRAPHEME_INFO
        })
    );
}

#[test]
fn test_index_with_repeated_newlines() {
    // 1: abc\n
    // 2: \n
    // 3: defgh
    let storage = FlatStorage::from_content_using_rows("abc\n\ndefgh", 5, Some(3));
    assert_eq!(storage.index.rows.len(), 3);

    assert_eq!(storage.index.rows[0].content_offset, ByteOffset::zero());
    assert_eq!(
        storage.index.rows[0].grapheme_sizing,
        GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(3).unwrap(),
            info: ASCII_GRAPHEME_INFO
        })
    );

    assert_eq!(storage.index.rows[1].content_offset, ByteOffset::from(4));
    assert_eq!(
        storage.index.rows[1].grapheme_sizing,
        GraphemeSizing::EmptyRow
    );

    assert_eq!(storage.index.rows[2].content_offset, ByteOffset::from(5));
    assert_eq!(
        storage.index.rows[2].grapheme_sizing,
        GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(5).unwrap(),
            info: ASCII_GRAPHEME_INFO
        })
    );
}

#[test]
fn test_index_with_exactly_full_row() {
    // 1: abc
    let storage = FlatStorage::from_content_using_rows("abc", 3, Some(1));
    assert_eq!(storage.index.rows.len(), 1);
    assert_eq!(storage.index.content_len, 3);
}

#[test]
fn test_index_with_full_row_and_newline() {
    // The newline shouldn't start a new row; it should only affect whether the
    // single row soft or hard wraps.
    //
    // 1: abc\n
    let storage = FlatStorage::from_content_using_rows("abc\n", 3, Some(1));
    assert_eq!(storage.index.rows.len(), 1);
    assert_eq!(storage.index.content_len, 4);

    // 1: abc
    // 2: d\n
    let storage = FlatStorage::from_content_using_rows("abcd\n", 3, Some(1));
    assert_eq!(storage.index.rows.len(), 2);
    assert_eq!(storage.index.content_len, 5);
}

#[test]
fn test_push_extra_row_onto_index() {
    // 1: abc\n
    let mut storage = FlatStorage::from_content_using_rows("abc\n", 5, Some(1));
    assert_eq!(storage.index.rows.len(), 1);

    // Adding a second hard-wrapped line of text to the index should give us a
    // total of 3 lines (not 4).
    //
    // 1: abc\n
    // 2: def\n
    storage.push_rows_from_string("def\n");
    assert_eq!(storage.index.rows.len(), 2);
}

#[test]
fn test_push_extra_row_onto_index_with_softwrapped_first_line() {
    // 1: abcde
    let mut storage = FlatStorage::from_content_using_rows("abcde", 5, Some(1));
    assert_eq!(storage.index.rows.len(), 1);

    // Adding a hard-wrapped line of text to the index should give us a
    // total of 2 lines.
    //
    // 1: abcde
    // 2: 123\n
    storage.push_rows_from_string("123\n");
    assert_eq!(storage.index.rows.len(), 2);
}

#[test]
fn test_cell_type() {
    // 1: 😀😃
    // 2: 😄\n
    // 3: a😄\n
    let storage = FlatStorage::from_content_using_rows("😀😃😄\na😄\n", 5, Some(2));
    assert_eq!(storage.index.rows.len(), 3);

    assert_eq!(storage.cell_type(0, 0), Some(CellType::WideChar));
    assert_eq!(storage.cell_type(0, 1), Some(CellType::WideCharSpacer));

    assert_eq!(
        storage.cell_type(0, 4),
        Some(CellType::LeadingWideCharSpacer)
    );

    // Empty cells at the end of a hard-wrapped line are narrow.
    // We test both the first empty cell (to check off-by-one errors) and
    // a later cell (for completeness).
    assert_eq!(storage.cell_type(1, 2), Some(CellType::RegularChar));
    assert_eq!(storage.cell_type(1, 4), Some(CellType::RegularChar));

    // Make sure we properly handle rows with non-uniform grapheme sizing.
    assert_eq!(storage.cell_type(2, 0), Some(CellType::RegularChar));
    assert_eq!(storage.cell_type(2, 1), Some(CellType::WideChar));
    assert_eq!(storage.cell_type(2, 2), Some(CellType::WideCharSpacer));
}

mod offset_point_conversion {
    use super::*;

    #[test]
    fn test_normal_cell() {
        // 1: 😀😃
        // 2: 😄\n
        // 3: a😄\n
        let storage = FlatStorage::from_content_using_rows("😀😃😄\na😄\n", 5, Some(2));

        let original_point = Point::new(2, 0);

        let offset = storage
            .content_offset_at_point(original_point)
            .expect("should be able to convert point to offset");
        assert_eq!(offset, ByteOffset::from(13));

        let point = storage
            .content_offset_to_point(offset)
            .expect("should be able to convert offset back to point");
        assert_eq!(point, original_point);
    }

    #[test]
    fn test_wide_char() {
        // 1: 😀😃
        // 2: 😄\n
        // 3: a😄\n
        let storage = FlatStorage::from_content_using_rows("😀😃😄\na😄\n", 5, Some(2));

        let original_point = Point::new(0, 2);

        let offset = storage
            .content_offset_at_point(original_point)
            .expect("should be able to convert point to offset");
        assert_eq!(offset, ByteOffset::from(4));

        let point = storage
            .content_offset_to_point(offset)
            .expect("should be able to convert offset back to point");
        assert_eq!(point, original_point);
    }

    #[test]
    #[ignore = "does not work properly; will re-enable once content offset/point conversion uses a custom type"]
    fn test_wide_char_spacer() {
        // 1: 😀😃
        // 2: 😄\n
        // 3: a😄\n
        let storage = FlatStorage::from_content_using_rows("😀😃😄\na😄\n", 5, Some(2));

        let original_point = Point::new(0, 3);

        let offset = storage
            .content_offset_at_point(original_point)
            .expect("should be able to convert point to offset");
        assert_eq!(offset, ByteOffset::from(4));

        let point = storage
            .content_offset_to_point(offset)
            .expect("should be able to convert offset back to point");
        assert_eq!(point, original_point);
    }

    #[test]
    fn test_nonuniform_row() {
        // 1: 😀😃
        // 2: 😄\n
        // 3: a😄\n
        let storage = FlatStorage::from_content_using_rows("😀😃😄\na😄\n", 5, Some(2));

        let original_point = Point::new(2, 1);

        let offset = storage
            .content_offset_at_point(original_point)
            .expect("should be able to convert point to offset");
        assert_eq!(offset, ByteOffset::from(14));

        let point = storage
            .content_offset_to_point(offset)
            .expect("should be able to convert offset back to point");
        assert_eq!(point, original_point);
    }
}
