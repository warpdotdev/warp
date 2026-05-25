use std::num::NonZeroU16;

use super::*;
use crate::model::grid::FlatStorage;

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
fn test_truncate_front_clamps_count() {
    let mut index = Index::new(5, Some(2));

    let mut eb = index.start_row();
    eb.process_grapheme_info_unchecked(ASCII_GRAPHEME_INFO);
    eb.add_trailing_newline();
    eb.append_to_index(&mut index);

    let mut eb = index.start_row();
    eb.process_grapheme_info_unchecked(ASCII_GRAPHEME_INFO);
    eb.add_trailing_newline();
    eb.append_to_index(&mut index);

    let new_start_offset = index.truncate_front(99);

    assert_eq!(new_start_offset, ByteOffset::from(4));
    assert_eq!(index.len(), 0);
}

#[test]
#[should_panic(expected = "should not have more than 2^16 graphemes in a single row")]
fn test_rebuild_panics_on_run_count_overflow() {
    let mut index = Index::new(1, Some(u16::MAX as usize + 1));

    for _ in 0..=(u16::MAX as usize) {
        let mut eb = index.start_row();
        eb.process_grapheme_info_unchecked(ASCII_GRAPHEME_INFO);
        eb.append_to_index(&mut index);
    }

    let _ = Index::rebuild(&index, u16::MAX as usize + 1);
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

#[cfg(test)]
mod property_tests {
    use super::*;
    use std::num::NonZeroU16;

    // Inline xorshift PRNG - no external dependencies
    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed)
        }
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn range(&mut self, lo: usize, hi: usize) -> usize {
            lo + (self.next() as usize % (hi - lo))
        }
    }

    fn random_index(rng: &mut Rng, rows: usize, cols: usize) -> Index {
        let mut index = Index::new(cols, Some(rows));
        let ascii_info = GraphemeInfo {
            cell_width: 1,
            utf8_bytes: NonZeroU16::new(1).unwrap(),
        };
        let wide_info = GraphemeInfo {
            cell_width: 2,
            utf8_bytes: NonZeroU16::new(3).unwrap(),
        };
        let multi_info = GraphemeInfo {
            cell_width: 1,
            utf8_bytes: NonZeroU16::new(3).unwrap(),
        };

        for _ in 0..rows {
            let mut eb = index.start_row();
            let mut cells_used = 0;
            while cells_used < cols {
                let kind = rng.next() % 10;
                let info = if kind < 6 {
                    ascii_info
                } else if kind < 8 {
                    wide_info
                } else {
                    multi_info
                };
                if cells_used + info.cell_width as usize > cols {
                    break;
                }
                eb.process_grapheme_info_unchecked(info);
                cells_used += info.cell_width as usize;
            }
            if !rng.next().is_multiple_of(4) {
                eb.add_trailing_newline();
            }
            eb.append_to_index(&mut index);
        }
        index
    }

    #[test]
    fn prop_content_len_preserved() {
        let mut rng = Rng::new(12345);
        for _ in 0..100 {
            let cols = rng.range(10, 200);
            let rows = rng.range(10, 500);
            let new_cols = rng.range(10, 200);
            let index = random_index(&mut rng, rows, cols);
            let original_content_len = index.content_len;
            let rebuilt = Index::rebuild(&index, new_cols);
            assert_eq!(
                rebuilt.content_len, original_content_len,
                "content_len mismatch: cols={cols}->{new_cols}, rows={rows}"
            );
        }
    }

    #[test]
    fn prop_no_row_exceeds_columns() {
        let mut rng = Rng::new(67890);
        for _ in 0..100 {
            let cols = rng.range(10, 200);
            let rows = rng.range(10, 500);
            let new_cols = rng.range(10, 200);
            let index = random_index(&mut rng, rows, cols);
            let rebuilt = Index::rebuild(&index, new_cols);
            for row_idx in 0..rebuilt.len() {
                if let Some(infos) = rebuilt.grapheme_infos_for_row(row_idx) {
                    let total_cells: usize = infos.map(|i| i.cell_width as usize).sum();
                    assert!(
                        total_cells <= new_cols,
                        "row {row_idx} has {total_cells} cells but max is {new_cols}"
                    );
                }
            }
        }
    }

    #[test]
    fn prop_idempotent_rebuild() {
        let mut rng = Rng::new(11111);
        for _ in 0..50 {
            let cols = rng.range(10, 200);
            let rows = rng.range(10, 200);
            let new_cols = rng.range(10, 200);
            let index = random_index(&mut rng, rows, cols);
            let rebuilt_once = Index::rebuild(&index, new_cols);
            let rebuilt_twice = Index::rebuild(&rebuilt_once, new_cols);
            assert_eq!(rebuilt_once.len(), rebuilt_twice.len());
            assert_eq!(rebuilt_once.content_len, rebuilt_twice.content_len);
        }
    }

    #[test]
    fn prop_total_cells_preserved() {
        let mut rng = Rng::new(99999);
        for _ in 0..100 {
            let cols = rng.range(10, 200);
            let rows = rng.range(10, 500);
            let new_cols = rng.range(10, 200);
            let index = random_index(&mut rng, rows, cols);

            let original_cells: usize = (0..index.len())
                .filter_map(|i| index.grapheme_infos_for_row(i))
                .flatten()
                .map(|info| info.cell_width as usize)
                .sum();

            let rebuilt = Index::rebuild(&index, new_cols);
            let rebuilt_cells: usize = (0..rebuilt.len())
                .filter_map(|i| rebuilt.grapheme_infos_for_row(i))
                .flatten()
                .map(|info| info.cell_width as usize)
                .sum();

            assert_eq!(
                original_cells, rebuilt_cells,
                "total cells mismatch: cols={cols}->{new_cols}"
            );
        }
    }
}

#[cfg(test)]
mod differential_tests {
    use super::*;
    use std::num::NonZeroU16;

    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed)
        }
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn range(&mut self, lo: usize, hi: usize) -> usize {
            lo + (self.next() as usize % (hi - lo))
        }
    }

    fn random_index(rng: &mut Rng, rows: usize, cols: usize) -> Index {
        let mut index = Index::new(cols, Some(rows));
        let ascii_info = GraphemeInfo {
            cell_width: 1,
            utf8_bytes: NonZeroU16::new(1).unwrap(),
        };
        let wide_info = GraphemeInfo {
            cell_width: 2,
            utf8_bytes: NonZeroU16::new(3).unwrap(),
        };
        let multi_info = GraphemeInfo {
            cell_width: 1,
            utf8_bytes: NonZeroU16::new(3).unwrap(),
        };

        for _ in 0..rows {
            let mut eb = index.start_row();
            let mut cells_used = 0;
            while cells_used < cols {
                let kind = rng.next() % 10;
                let info = if kind < 6 {
                    ascii_info
                } else if kind < 8 {
                    wide_info
                } else {
                    multi_info
                };
                if cells_used + info.cell_width as usize > cols {
                    break;
                }
                eb.process_grapheme_info_unchecked(info);
                cells_used += info.cell_width as usize;
            }
            if !rng.next().is_multiple_of(4) {
                eb.add_trailing_newline();
            }
            eb.append_to_index(&mut index);
        }
        index
    }

    fn assert_indexes_equal(optimized: &Index, baseline: &Index, context: &str) {
        assert_eq!(
            optimized.len(),
            baseline.len(),
            "{context}: row count mismatch: optimized={}, baseline={}",
            optimized.len(),
            baseline.len()
        );
        assert_eq!(
            optimized.content_len, baseline.content_len,
            "{context}: content_len mismatch: optimized={}, baseline={}",
            optimized.content_len, baseline.content_len
        );
        for row_idx in 0..optimized.len() {
            let opt_entry = optimized.get_entry(row_idx).unwrap();
            let base_entry = baseline.get_entry(row_idx).unwrap();
            assert_eq!(
                opt_entry.content_offset, base_entry.content_offset,
                "{context}: row {row_idx} content_offset mismatch"
            );
            assert_eq!(
                opt_entry.has_trailing_newline, base_entry.has_trailing_newline,
                "{context}: row {row_idx} has_trailing_newline mismatch"
            );
            assert_eq!(
                opt_entry.ends_with_leading_wide_char_spacer,
                base_entry.ends_with_leading_wide_char_spacer,
                "{context}: row {row_idx} ends_with_leading_wide_char_spacer mismatch"
            );
            // Compare grapheme content
            let opt_infos: Vec<_> = optimized
                .grapheme_infos_for_row(row_idx)
                .map(|it| it.collect())
                .unwrap_or_default();
            let base_infos: Vec<_> = baseline
                .grapheme_infos_for_row(row_idx)
                .map(|it| it.collect())
                .unwrap_or_default();
            assert_eq!(
                opt_infos, base_infos,
                "{context}: row {row_idx} grapheme infos differ"
            );
        }
    }

    #[test]
    fn differential_random_inputs() {
        let mut rng = Rng::new(0xDEADBEEF);
        for trial in 0..200 {
            let cols = rng.range(5, 300);
            let rows = rng.range(1, 500);
            let new_cols = rng.range(5, 300);
            let index = random_index(&mut rng, rows, cols);

            let optimized = Index::rebuild(&index, new_cols);
            let baseline = Index::rebuild_baseline(&index, new_cols);

            assert_indexes_equal(
                &optimized,
                &baseline,
                &format!("trial {trial}: {cols}cols x {rows}rows -> {new_cols}cols"),
            );
        }
    }

    #[test]
    fn differential_widening_all_row_types() {
        let mut rng = Rng::new(0xCAFE);
        for trial in 0..100 {
            let cols = rng.range(10, 80);
            let rows = rng.range(10, 200);
            let new_cols = rng.range(cols + 1, cols + 100); // always wider
            let index = random_index(&mut rng, rows, cols);

            let optimized = Index::rebuild(&index, new_cols);
            let baseline = Index::rebuild_baseline(&index, new_cols);

            assert_indexes_equal(
                &optimized,
                &baseline,
                &format!("widen trial {trial}: {cols}->{new_cols}, {rows} rows"),
            );
        }
    }

    #[test]
    fn differential_narrowing_all_row_types() {
        let mut rng = Rng::new(0xBEEF);
        for trial in 0..100 {
            let cols = rng.range(40, 200);
            let rows = rng.range(10, 200);
            let new_cols = rng.range(5, cols); // always narrower
            let index = random_index(&mut rng, rows, cols);

            let optimized = Index::rebuild(&index, new_cols);
            let baseline = Index::rebuild_baseline(&index, new_cols);

            assert_indexes_equal(
                &optimized,
                &baseline,
                &format!("narrow trial {trial}: {cols}->{new_cols}, {rows} rows"),
            );
        }
    }

    #[test]
    fn differential_extreme_dimensions() {
        let cases: &[(usize, usize, usize)] = &[
            (1, 100, 200),  // single row widening
            (1, 200, 1),    // single row narrowing to 1 col
            (1000, 1, 80),  // many single-cell rows
            (10, 1000, 50), // very wide rows narrowing
            (100, 80, 80),  // same width (should be identity)
            (500, 80, 79),  // off-by-one narrowing
            (500, 80, 81),  // off-by-one widening
        ];

        for &(rows, old_cols, new_cols) in cases {
            let mut rng = Rng::new((rows * old_cols * new_cols) as u64);
            let index = random_index(&mut rng, rows, old_cols);

            let optimized = Index::rebuild(&index, new_cols);
            let baseline = Index::rebuild_baseline(&index, new_cols);

            assert_indexes_equal(
                &optimized,
                &baseline,
                &format!("{rows}rows x {old_cols}cols -> {new_cols}cols"),
            );
        }
    }

    #[test]
    fn differential_all_softwrapped() {
        // Worst case for our optimization: no newlines at all
        let mut rng = Rng::new(0x5F5F);
        let ascii_info = GraphemeInfo {
            cell_width: 1,
            utf8_bytes: NonZeroU16::new(1).unwrap(),
        };
        for trial in 0..50 {
            let cols = rng.range(20, 120);
            let rows = rng.range(50, 300);
            let new_cols = rng.range(5, 200);

            let mut index = Index::new(cols, Some(rows));
            for _ in 0..rows {
                let mut eb = index.start_row();
                for _ in 0..cols {
                    eb.process_grapheme_info_unchecked(ascii_info);
                }
                // No trailing newline — fully softwrapped
                eb.append_to_index(&mut index);
            }

            let optimized = Index::rebuild(&index, new_cols);
            let baseline = Index::rebuild_baseline(&index, new_cols);

            assert_indexes_equal(
                &optimized,
                &baseline,
                &format!("softwrap trial {trial}: {cols}->{new_cols}, {rows} rows"),
            );
        }
    }

    #[test]
    fn differential_mixed_newline_patterns() {
        // Various patterns of newline placement
        let mut rng = Rng::new(0xAAAA);
        let ascii_info = GraphemeInfo {
            cell_width: 1,
            utf8_bytes: NonZeroU16::new(1).unwrap(),
        };
        let wide_info = GraphemeInfo {
            cell_width: 2,
            utf8_bytes: NonZeroU16::new(3).unwrap(),
        };

        for trial in 0..50 {
            let cols = rng.range(10, 100);
            let rows = rng.range(20, 200);
            let new_cols = rng.range(5, 150);

            let mut index = Index::new(cols, Some(rows));
            for i in 0..rows {
                let mut eb = index.start_row();
                let mut cells = 0;
                // Alternate between full ASCII rows, partial rows, and wide char rows
                match i % 5 {
                    0 => {
                        // Full ASCII row
                        while cells < cols {
                            eb.process_grapheme_info_unchecked(ascii_info);
                            cells += 1;
                        }
                    }
                    1 => {
                        // Partial row (half full)
                        let target = cols / 2;
                        while cells < target {
                            eb.process_grapheme_info_unchecked(ascii_info);
                            cells += 1;
                        }
                    }
                    2 => {
                        // Wide char row
                        while cells + 2 <= cols {
                            eb.process_grapheme_info_unchecked(wide_info);
                            cells += 2;
                        }
                    }
                    3 => {
                        // Mixed row
                        while cells < cols {
                            if rng.next().is_multiple_of(3) && cells + 2 <= cols {
                                eb.process_grapheme_info_unchecked(wide_info);
                                cells += 2;
                            } else {
                                eb.process_grapheme_info_unchecked(ascii_info);
                                cells += 1;
                            }
                        }
                    }
                    _ => {
                        // Empty row (just newline)
                    }
                }
                // Newline on odd rows only (creates groups of softwrapped lines)
                if i % 2 == 1 {
                    eb.add_trailing_newline();
                }
                eb.append_to_index(&mut index);
            }

            let optimized = Index::rebuild(&index, new_cols);
            let baseline = Index::rebuild_baseline(&index, new_cols);

            assert_indexes_equal(
                &optimized,
                &baseline,
                &format!("mixed trial {trial}: {cols}->{new_cols}, {rows} rows"),
            );
        }
    }

    /// Regression test: `try_emit_carryover_uniform` Sub-case A must use
    /// strict `>` in its inner loop, not `>=`.
    ///
    /// When `count - graphemes_per_row` is an exact multiple of
    /// `graphemes_per_row` the `>=` variant reduces `rem` to 0, causing the
    /// trailing-newline path to emit an empty row instead of annotating the
    /// last content row.
    ///
    /// Trigger: soft-wrapped row fills the builder to exactly `columns`, then
    /// the next row is a uniform run of `2 * columns` graphemes with a
    /// trailing newline.
    #[test]
    fn carryover_uniform_sub_a_exact_multiple_newline() {
        // Source: 40-cell soft-wrap + 80-cell newline row, rebuild to 40 cols.
        // entry_builder accumulates 40 cells from row 1 → Sub-case A triggers
        // with count=80, graphemes_per_row=40, rem=40.
        let mut index = Index::new(80, Some(2));

        let mut eb = index.start_row();
        for _ in 0..40 {
            eb.process_grapheme_info_unchecked(ASCII_GRAPHEME_INFO);
        }
        eb.append_to_index(&mut index); // no newline — soft-wrap

        let mut eb = index.start_row();
        for _ in 0..80 {
            eb.process_grapheme_info_unchecked(ASCII_GRAPHEME_INFO);
        }
        eb.add_trailing_newline();
        eb.append_to_index(&mut index);

        let optimized = Index::rebuild(&index, 40);
        let baseline = Index::rebuild_baseline(&index, 40);
        assert_indexes_equal(&optimized, &baseline, "carryover sub-A exact multiple");
        assert_eq!(
            optimized.content_len, index.content_len,
            "content_len not preserved"
        );
    }

    /// Regression test: `try_emit_carryover_uniform` Sub-case B must also use
    /// strict `>` in its inner loop.
    ///
    /// Trigger: partial soft-wrapped row leaves `remaining_graphemes` cells in
    /// the builder such that `count - remaining_graphemes` is an exact multiple
    /// of `graphemes_per_row`.
    #[test]
    fn carryover_uniform_sub_b_exact_multiple_newline() {
        // Source: 20-cell soft-wrap + 60-cell newline row, rebuild to 40 cols.
        // entry_builder has 20 cells → remaining_graphemes=20, rem=40=GPR.
        let mut index = Index::new(80, Some(2));

        let mut eb = index.start_row();
        for _ in 0..20 {
            eb.process_grapheme_info_unchecked(ASCII_GRAPHEME_INFO);
        }
        eb.append_to_index(&mut index); // no newline — soft-wrap

        let mut eb = index.start_row();
        for _ in 0..60 {
            eb.process_grapheme_info_unchecked(ASCII_GRAPHEME_INFO);
        }
        eb.add_trailing_newline();
        eb.append_to_index(&mut index);

        let optimized = Index::rebuild(&index, 40);
        let baseline = Index::rebuild_baseline(&index, 40);
        assert_indexes_equal(&optimized, &baseline, "carryover sub-B exact multiple");
        assert_eq!(
            optimized.content_len, index.content_len,
            "content_len not preserved"
        );
    }

    /// Regression test: `try_emit_carryover_uniform` now handles `cell_width == 2`
    /// (wide chars) and correctly sets `ends_with_leading_wide_char_spacer` on
    /// rows whose wide-char content doesn't reach an even column boundary.
    ///
    /// Uses an odd column count (7) so that every full wide-char row leaves a
    /// 1-cell gap and must set the spacer flag — exercises both `full_row_spacer`
    /// (inner direct-emit rows) and `partial_row_spacer` (the initial builder
    /// flush in Sub-case B).
    #[test]
    fn carryover_uniform_wide_char_odd_columns() {
        // Source: 4 ASCII (no nl) + 5 wide chars (with nl) in a 10-col index.
        // Rebuild to 7 cols (odd).
        //
        // Expected output (verified against baseline):
        //   row 0: [4 ascii + 1 wide],  ends_with_leading_wide_char_spacer=true
        //   row 1: [3 wide],             ends_with_leading_wide_char_spacer=true
        //   row 2: [1 wide + nl],        ends_with_leading_wide_char_spacer=false
        let mut index = Index::new(10, Some(2));

        let mut eb = index.start_row();
        for _ in 0..4 {
            eb.process_grapheme_info_unchecked(ASCII_GRAPHEME_INFO);
        }
        eb.append_to_index(&mut index); // no newline

        let mut eb = index.start_row();
        for _ in 0..5 {
            eb.process_grapheme_info_unchecked(EMOJI_GRAPHEME_INFO);
        }
        eb.add_trailing_newline();
        eb.append_to_index(&mut index);

        let optimized = Index::rebuild(&index, 7);
        let baseline = Index::rebuild_baseline(&index, 7);
        assert_indexes_equal(&optimized, &baseline, "carryover wide char odd cols");
        assert_eq!(
            optimized.content_len, index.content_len,
            "content_len not preserved"
        );
    }

    /// Broader sweep: wide-char carry-over across various column widths including
    /// odd counts, large runs, and multiple full-row direct-emits.
    #[test]
    fn carryover_uniform_wide_char_sweep() {
        // (old_cols, new_cols) pairs that exercise wide-char carry-over paths
        let cases: &[(usize, usize, usize, usize)] = &[
            // (old_cols, new_cols, ascii_prefix_len, wide_count)
            (10, 7, 4, 5),  // odd new_cols, Sub-case B + full_row_spacer
            (10, 6, 4, 5),  // even new_cols, Sub-case B no spacer
            (20, 7, 3, 8),  // longer run, multiple direct-emit rows
            (20, 9, 5, 7),  // odd new_cols, Sub-case B with partial_row_spacer
            (20, 10, 6, 7), // even new_cols, no spacers needed
            (30, 7, 2, 12), // many full rows through direct-emit loop
            (14, 7, 0, 7),  // no ASCII prefix — Sub-case A (builder full from softwrap)
        ];

        for &(old_cols, new_cols, ascii_len, wide_count) in cases {
            let mut index = Index::new(old_cols, Some(2));

            // Row 1: ASCII prefix, soft-wrapped.
            if ascii_len > 0 {
                let mut eb = index.start_row();
                for _ in 0..ascii_len {
                    eb.process_grapheme_info_unchecked(ASCII_GRAPHEME_INFO);
                }
                eb.append_to_index(&mut index);
            }

            // Row 2 (or row 1 if no prefix): wide chars with newline.
            // If no prefix we use a source row of wide chars that fills
            // old_cols exactly so it appears as a soft-wrapped row followed
            // by a second wide-char row with newline.
            if ascii_len == 0 {
                // First source row: enough wide chars to fill old_cols (soft-wrap)
                let mut eb = index.start_row();
                let fill = old_cols / 2;
                for _ in 0..fill {
                    eb.process_grapheme_info_unchecked(EMOJI_GRAPHEME_INFO);
                }
                eb.append_to_index(&mut index);
            }

            let mut eb = index.start_row();
            for _ in 0..wide_count {
                eb.process_grapheme_info_unchecked(EMOJI_GRAPHEME_INFO);
            }
            eb.add_trailing_newline();
            eb.append_to_index(&mut index);

            let optimized = Index::rebuild(&index, new_cols);
            let baseline = Index::rebuild_baseline(&index, new_cols);
            assert_indexes_equal(
                &optimized,
                &baseline,
                &format!(
                    "wide-char sweep: old={old_cols}, new={new_cols}, ascii={ascii_len}, wide={wide_count}"
                ),
            );
        }
    }

    /// Regression test: `emit_narrowed_uniform` (fast path A) must set
    /// `ends_with_leading_wide_char_spacer` on inner loop rows when the column
    /// count is odd and cell_width == 2.
    ///
    /// A full row of wide chars fills `(columns / 2) * 2 = columns - 1` cells,
    /// leaving a 1-cell gap that acts as a leading-wide-char-spacer.
    /// Previously the flag was hardcoded to `false`, causing a mismatch with
    /// the baseline (`process_graphemes_batch`) which correctly sets it.
    #[test]
    fn narrowed_uniform_wide_char_spacer_odd_columns() {
        // (old_cols, new_cols, wide_count): source has wide_count wide chars
        // with a trailing newline, rebuild to an odd new_cols that forces
        // multiple full-row emits through emit_narrowed_uniform.
        let cases: &[(usize, usize, usize)] = &[
            (20, 7, 10), // 10 wide → 3+3+4 rows at new=7 (each full row needs spacer)
            (20, 5, 10), // 10 wide → 5+5 rows at new=5 (even cols, no spacer)
            (30, 7, 15), // 15 wide → 3+3+3+3+3 at new=7, each full row needs spacer
            (30, 9, 15), // 15 wide → 4+4+4+3 at new=9 (odd cols, spacer on full rows)
            (20, 3, 10), // extreme narrowing, odd cols
            (10, 7, 5),  // minimal: 5 wide → 3+2, first full row needs spacer
        ];

        for &(old_cols, new_cols, wide_count) in cases {
            let mut index = Index::new(old_cols, Some(1));
            let mut eb = index.start_row();
            for _ in 0..wide_count {
                eb.process_grapheme_info_unchecked(EMOJI_GRAPHEME_INFO);
            }
            eb.add_trailing_newline();
            eb.append_to_index(&mut index);

            let optimized = Index::rebuild(&index, new_cols);
            let baseline = Index::rebuild_baseline(&index, new_cols);
            assert_indexes_equal(
                &optimized,
                &baseline,
                &format!(
                    "narrowed wide-char spacer: old={old_cols}, new={new_cols}, count={wide_count}"
                ),
            );
        }
    }

    /// Regression test: narrowing fast path must preserve `content_len` when
    /// `count` is an exact multiple of `graphemes_per_row`.
    ///
    /// Because the loop uses `while rem > graphemes_per_row` (strict), `rem`
    /// exits in `[1, graphemes_per_row]` and the `else` branch is technically
    /// unreachable today.  The fix restructures the code so `content_len` is
    /// only incremented inside the branch that actually pushes a row, making
    /// the intent clear and preventing a real double-count if the loop
    /// condition is ever relaxed to `>=`.
    #[test]
    fn narrowing_uniform_exactly_divisible_newline() {
        // Build several source indexes whose rows divide evenly at the new
        // column width, so the narrowing fast path always lands in the
        // `rem == 0` branch.
        let ascii_info = GraphemeInfo {
            cell_width: 1,
            utf8_bytes: NonZeroU16::new(1).unwrap(),
        };

        // (old_cols, new_cols) pairs where old_cols % new_cols == 0
        let cases: &[(usize, usize)] = &[
            (80, 40),
            (80, 20),
            (80, 10),
            (80, 8),
            (80, 5),
            (80, 4),
            (100, 50),
            (100, 25),
            (120, 60),
            (120, 40),
            (120, 30),
            (120, 24),
        ];

        for &(old_cols, new_cols) in cases {
            // Single row, fully packed, with a trailing newline.
            let mut index = Index::new(old_cols, Some(1));
            let mut eb = index.start_row();
            for _ in 0..old_cols {
                eb.process_grapheme_info_unchecked(ascii_info);
            }
            eb.add_trailing_newline();
            eb.append_to_index(&mut index);

            let optimized = Index::rebuild(&index, new_cols);
            let baseline = Index::rebuild_baseline(&index, new_cols);

            assert_indexes_equal(
                &optimized,
                &baseline,
                &format!("divisible narrowing: {old_cols}->{new_cols}"),
            );

            // Also verify the total byte count is preserved.
            assert_eq!(
                optimized.content_len, index.content_len,
                "content_len not preserved for {old_cols}->{new_cols}: \
                 optimized={}, original={}",
                optimized.content_len, index.content_len
            );
        }
    }
}
