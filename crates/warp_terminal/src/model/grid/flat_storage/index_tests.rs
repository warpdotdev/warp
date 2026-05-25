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
                eb.process_grapheme_info(info, &mut index);
                cells_used += info.cell_width as usize;
            }
            if rng.next() % 4 != 0 {
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
                eb.process_grapheme_info(info, &mut index);
                cells_used += info.cell_width as usize;
            }
            if rng.next() % 4 != 0 {
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
                    eb.process_grapheme_info(ascii_info, &mut index);
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
                            eb.process_grapheme_info(ascii_info, &mut index);
                            cells += 1;
                        }
                    }
                    1 => {
                        // Partial row (half full)
                        let target = cols / 2;
                        while cells < target {
                            eb.process_grapheme_info(ascii_info, &mut index);
                            cells += 1;
                        }
                    }
                    2 => {
                        // Wide char row
                        while cells + 2 <= cols {
                            eb.process_grapheme_info(wide_info, &mut index);
                            cells += 2;
                        }
                    }
                    3 => {
                        // Mixed row
                        while cells < cols {
                            if rng.next() % 3 == 0 && cells + 2 <= cols {
                                eb.process_grapheme_info(wide_info, &mut index);
                                cells += 2;
                            } else {
                                eb.process_grapheme_info(ascii_info, &mut index);
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
}
