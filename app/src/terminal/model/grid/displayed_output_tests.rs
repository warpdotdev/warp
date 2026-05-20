use super::*;
use bimap::BiMap;
use itertools::Itertools;

/// Converts a vector of ranges into DisplayedRows objects with source FilterMatch.
fn make_displayed_rows_from_ranges(ranges: Vec<RangeInclusive<usize>>) -> Vec<DisplayedRows> {
    ranges
        .into_iter()
        .map(|range| DisplayedRows {
            range,
            source: DisplaySource::FilterMatch,
        })
        .collect_vec()
}

#[test]
pub fn test_displayed_output_rows_iterator() {
    let displayed_output = DisplayedOutput::new_for_test(vec![0..=2, 4..=4, 7..=10]);
    let iterator = displayed_output.rows();

    let res = iterator.collect_vec();
    assert_eq!(res, vec![0, 1, 2, 4, 7, 8, 9, 10]);
}

#[test]
pub fn test_displayed_output_rows_iterator_no_rows() {
    let displayed_output = DisplayedOutput::default();
    let mut iterator = displayed_output.rows();

    assert!(iterator.next().is_none());
}

#[test]
pub fn test_truncate_to_row() {
    let mut displayed_output =
        DisplayedOutput::new_for_test(vec![6..=11, 15..=17, 20..=25, 27..=30]);

    displayed_output.truncate_to_row(19);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 15..=17])
    );
    assert_eq!(displayed_output.height, 9);
}

#[test]
pub fn test_truncate_to_row_truncate_before_start() {
    let mut displayed_output = DisplayedOutput::new_for_test(vec![6..=11, 12..=14]);

    displayed_output.truncate_to_row(5);
    assert_eq!(displayed_output.displayed_rows, vec![]);
    assert_eq!(displayed_output.height, 0);
}

#[test]
pub fn test_truncate_to_row_truncate_after_end() {
    let mut displayed_output = DisplayedOutput::new_for_test(vec![6..=11, 12..=14]);

    displayed_output.truncate_to_row(15);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 12..=14])
    );
    assert_eq!(displayed_output.height, 9);
}

#[test]
pub fn test_truncate_to_row_in_range() {
    let mut displayed_output =
        DisplayedOutput::new_for_test(vec![6..=11, 15..=17, 20..=25, 27..=30]);

    displayed_output.truncate_to_row(16);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 15..=16])
    );
    assert_eq!(displayed_output.height, 8);

    let mut displayed_output =
        DisplayedOutput::new_for_test(vec![6..=11, 15..=17, 20..=25, 27..=30]);

    displayed_output.truncate_to_row(15);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 15..=15])
    );
    assert_eq!(displayed_output.height, 7);

    let mut displayed_output =
        DisplayedOutput::new_for_test(vec![6..=11, 15..=17, 20..=25, 27..=30]);

    displayed_output.truncate_to_row(17);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 15..=17])
    );
    assert_eq!(displayed_output.height, 9);
}

#[test]
pub fn test_truncate_to_row_in_last_range() {
    let mut displayed_output =
        DisplayedOutput::new_for_test(vec![6..=11, 15..=17, 20..=25, 27..=30]);

    displayed_output.truncate_to_row(28);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 15..=17, 20..=25, 27..=28])
    );
    assert_eq!(displayed_output.height, 17);
}

fn create_bimap_with_insertions(insertion_list: &[(usize, usize)]) -> BiMap<usize, usize> {
    let mut bimap = BiMap::new();
    for (row, offset_row) in insertion_list {
        bimap.insert(*row, *offset_row);
    }
    bimap
}

#[test]
pub fn test_append_to_row_translation_empty_displayed_rows() {
    // Start with empty displayed rows.
    let mut displayed_output = DisplayedOutput {
        displayed_rows: vec![],
        height: 5,
        row_translation_map: Default::default(),
    };

    // Append displayed rows 0..=1.
    displayed_output.append_to_row_translation(make_displayed_rows_from_ranges(vec![0..=1]).iter());

    let expected_bimap = create_bimap_with_insertions(&[(0, 0), (1, 1)]);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
pub fn test_append_to_row_translation() {
    // Start with displayed rows 0..=1, 4..=5.
    let mut bimap = create_bimap_with_insertions(&[(0, 0), (1, 1), (4, 2), (5, 3)]);
    let mut displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![0..=1, 4..=5]),
        height: 5,
        row_translation_map: bimap.clone().into(),
    };

    // Append displayed rows 6..7.
    displayed_output.append_to_row_translation(make_displayed_rows_from_ranges(vec![6..=7]).iter());

    bimap.insert(6, 4);
    bimap.insert(7, 5);
    assert_eq!(displayed_output.row_translation_map, bimap.into());
}

#[test]
pub fn test_prepend_to_row_translation_empty_displayed_rows() {
    // Start with empty displayed rows.
    let mut displayed_output = DisplayedOutput {
        displayed_rows: vec![],
        height: 5,
        row_translation_map: Default::default(),
    };

    // Prepend displayed rows 0..=1.
    let new_line_ranges = make_displayed_rows_from_ranges(vec![0..=1]);
    displayed_output
        .displayed_rows
        .splice(0..0, new_line_ranges);
    displayed_output.prepend_to_row_translation();

    let mut expected_bimap = BiMap::new();
    expected_bimap.insert(0, 0);
    expected_bimap.insert(1, 1);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
pub fn test_prepend_to_row_translation() {
    // Start with displayed rows 2..=3, 4..=5.
    let bimap = create_bimap_with_insertions(&[(2, 0), (3, 1), (4, 2), (5, 3)]);
    let mut displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![2..=3, 4..=5]),
        height: 5,
        row_translation_map: bimap.into(),
    };

    // Prepend displayed rows 0..=1.
    let new_line_ranges = make_displayed_rows_from_ranges(vec![0..=1]);
    displayed_output
        .displayed_rows
        .splice(0..0, new_line_ranges);
    displayed_output.prepend_to_row_translation();

    let expected_bimap =
        create_bimap_with_insertions(&[(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
pub fn test_replace_rows_from_row_translation() {
    // Start with displayed rows 0, 2, 4.
    let bimap = create_bimap_with_insertions(&[(0, 0), (2, 1), (4, 2)]);
    let mut displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![0..=0, 2..=2, 4..=4]),
        height: 3,
        row_translation_map: bimap.into(),
    };

    // Replace displayed row 2 with displayed row 3.
    let replace_range = 1..2;
    let new_line_ranges = make_displayed_rows_from_ranges(vec![3..=3]);
    displayed_output
        .displayed_rows
        .splice(replace_range.clone(), new_line_ranges);
    displayed_output.replace_rows_from_row_translation(replace_range);

    let expected_bimap = create_bimap_with_insertions(&[(0, 0), (3, 1), (4, 2)]);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
pub fn test_replace_rows_from_row_translation_from_middle_to_end() {
    // Start with displayed rows 0, 2, 4.
    let bimap = create_bimap_with_insertions(&[(0, 0), (2, 1), (4, 2)]);
    let mut displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![0..=0, 2..=2, 4..=4]),
        height: 3,
        row_translation_map: bimap.into(),
    };

    // Replace displayed rows 2, 4 with displayed row 3..=5.
    let replace_range = 1..3;
    let new_line_ranges = make_displayed_rows_from_ranges(vec![3..=5]);
    displayed_output
        .displayed_rows
        .splice(replace_range.clone(), new_line_ranges);
    displayed_output.replace_rows_from_row_translation(replace_range);

    let expected_bimap = create_bimap_with_insertions(&[(0, 0), (3, 1), (4, 2), (5, 3)]);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
pub fn test_replace_rows_from_row_translation_from_beginning_to_middle() {
    // Start with displayed rows 0, 2, 4, 6.
    let bimap = create_bimap_with_insertions(&[(0, 0), (2, 1), (4, 2), (6, 3)]);
    let mut displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![0..=0, 2..=2, 4..=4, 6..=6]),
        height: 3,
        row_translation_map: bimap.into(),
    };

    // Replace displayed rows 0, 2 with displayed row 0..=1.
    let replace_range = 0..2;
    let new_line_ranges = make_displayed_rows_from_ranges(vec![0..=1]);
    displayed_output
        .displayed_rows
        .splice(replace_range.clone(), new_line_ranges);
    displayed_output.replace_rows_from_row_translation(replace_range);

    let expected_bimap = create_bimap_with_insertions(&[(0, 0), (1, 1), (4, 2), (6, 3)]);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
pub fn test_replace_rows_from_row_translation_replace_beginning_row() {
    // Start with displayed rows 0.
    let bimap = create_bimap_with_insertions(&[(0, 0)]);
    let mut displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![0..=0]),
        height: 1,
        row_translation_map: bimap.into(),
    };

    // Replace displayed rows 0 with displayed row 0.
    // This is intended to simulate what happens if we process dirty bytes multiple times.
    let replace_range = 0..1;
    let new_line_ranges = make_displayed_rows_from_ranges(vec![0..=0]);
    displayed_output
        .displayed_rows
        .splice(replace_range.clone(), new_line_ranges);
    displayed_output.replace_rows_from_row_translation(replace_range);

    let expected_bimap = create_bimap_with_insertions(&[(0, 0)]);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
pub fn test_replace_rows_from_row_translation_break_range() {
    // Start with displayed_rows 0..=8.
    let bimap = create_bimap_with_insertions((0..=8).map(|x| (x, x)).collect_vec().as_slice());
    let mut displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![0..=1, 2..=3, 4..=8]),
        height: 3,
        row_translation_map: bimap.into(),
    };

    // Replace displayed rows 2..=3 with displayed row 2.
    let replace_range = 1..2;
    let new_line_ranges = make_displayed_rows_from_ranges(vec![2..=2]);
    displayed_output
        .displayed_rows
        .splice(replace_range.clone(), new_line_ranges);
    displayed_output.replace_rows_from_row_translation(replace_range);

    let expected_bimap = create_bimap_with_insertions(&[
        (0, 0),
        (1, 1),
        (2, 2),
        (4, 3),
        (5, 4),
        (6, 5),
        (7, 6),
        (8, 7),
    ]);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
fn test_replace_rows_from_row_translation_no_removed_entries() {
    // Start with displayed_rows 0..=1, 6..=7.
    let bimap = create_bimap_with_insertions(&[(0, 0), (1, 1), (6, 2), (7, 3)]);
    let mut displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![0..=1, 6..=7]),
        height: 3,
        row_translation_map: bimap.into(),
    };

    // Insert displayed rows 3..=4.
    let replace_range = 1..1;
    let new_line_ranges = make_displayed_rows_from_ranges(vec![3..=4]);
    displayed_output
        .displayed_rows
        .splice(replace_range.clone(), new_line_ranges);
    displayed_output.replace_rows_from_row_translation(replace_range);

    let expected_bimap =
        create_bimap_with_insertions(&[(0, 0), (1, 1), (3, 2), (4, 3), (6, 4), (7, 5)]);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
fn test_replace_rows_from_row_translation_dirty_range_overlaps_existing_ranges() {
    // Start with displayed_rows: [2..=4, 6..=11, 12..=14, 17..=20].
    let bimap = create_bimap_with_insertions(&[
        (2, 0),
        (3, 1),
        (4, 2),
        (6, 3),
        (7, 4),
        (8, 5),
        (9, 6),
        (10, 7),
        (11, 8),
        (12, 9),
        (13, 10),
        (14, 11),
        (17, 12),
        (18, 13),
        (19, 14),
        (20, 15),
    ]);
    let mut displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![2..=4, 6..=11, 12..=14, 17..=20]),
        height: 0,
        row_translation_map: bimap.into(),
    };

    // Pretend that the dirty byte range is 8..=13.
    // The new displayed rows for the dirty range are 8..=10, 12..=12.
    let replace_range = 1..3;
    let new_line_ranges = make_displayed_rows_from_ranges(vec![8..=10, 12..=12]);
    displayed_output
        .displayed_rows
        .splice(replace_range.clone(), new_line_ranges);
    displayed_output.replace_rows_from_row_translation(replace_range);

    let expected_bimap = create_bimap_with_insertions(&[
        (2, 0),
        (3, 1),
        (4, 2),
        (8, 3),
        (9, 4),
        (10, 5),
        (12, 6),
        (17, 7),
        (18, 8),
        (19, 9),
        (20, 10),
    ]);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
fn test_replace_rows_from_row_translation_dirty_range_lies_within_existing_range() {
    // Start with displayed_rows [2..=4, 6..=15, 17..=20].
    let bimap = create_bimap_with_insertions(&[
        (2, 0),
        (3, 1),
        (4, 2),
        (6, 3),
        (7, 4),
        (8, 5),
        (9, 6),
        (10, 7),
        (11, 8),
        (12, 9),
        (13, 10),
        (14, 11),
        (15, 12),
        (17, 13),
        (18, 14),
        (19, 15),
        (20, 16),
    ]);
    let mut displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![2..=4, 6..=15, 17..=20]),
        height: 0,
        row_translation_map: bimap.into(),
    };

    // Pretend that the dirty byte range is 8..=13.
    // The new displayed rows for the dirty range are 8..=10, 12..=12.
    let replace_range = 1..2;
    let new_line_ranges = make_displayed_rows_from_ranges(vec![6..=10, 12..=12, 13..=15]);
    displayed_output
        .displayed_rows
        .splice(replace_range.clone(), new_line_ranges);
    displayed_output.replace_rows_from_row_translation(replace_range);

    let expected_bimap = create_bimap_with_insertions(&[
        (2, 0),
        (3, 1),
        (4, 2),
        (6, 3),
        (7, 4),
        (8, 5),
        (9, 6),
        (10, 7),
        (12, 8),
        (13, 9),
        (14, 10),
        (15, 11),
        (17, 12),
        (18, 13),
        (19, 14),
        (20, 15),
    ]);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
fn test_get_exact_or_next_displayed_row() {
    // Start with displayed_rows [2..=3, 6..=8, 14..=15].
    let bimap =
        create_bimap_with_insertions(&[(2, 0), (3, 1), (6, 2), (7, 3), (8, 4), (14, 5), (15, 6)]);
    let displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![2..=3, 6..=8, 14..=15]),
        height: 0,
        row_translation_map: bimap.into(),
    };

    let expected_input_output_pairs = vec![
        (0, Some(0)),
        (1, Some(0)),
        (2, Some(0)),
        (3, Some(1)),
        (4, Some(2)),
        (5, Some(2)),
        (6, Some(2)),
        (7, Some(3)),
        (8, Some(4)),
        (9, Some(5)),
        (10, Some(5)),
        (11, Some(5)),
        (12, Some(5)),
        (13, Some(5)),
        (14, Some(5)),
        (15, Some(6)),
    ];
    for (input, expected_output) in expected_input_output_pairs {
        assert_eq!(
            displayed_output.get_exact_or_next_displayed_row(input),
            expected_output
        );
    }
}

#[test]
fn test_get_exact_or_next_displayed_row_no_next_closest() {
    // Start with displayed_rows [2..=3, 6..=8, 14..=15].
    let bimap =
        create_bimap_with_insertions(&[(2, 0), (3, 1), (6, 2), (7, 3), (8, 4), (14, 5), (15, 6)]);
    let displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![2..=3, 6..=8, 14..=15]),
        height: 0,
        row_translation_map: bimap.into(),
    };

    assert_eq!(displayed_output.get_exact_or_next_displayed_row(16), None);
}

#[test]
fn test_get_exact_or_next_displayed_row_empty() {
    let displayed_output = DisplayedOutput {
        displayed_rows: Vec::new(),
        height: 0,
        row_translation_map: BiMap::new().into(),
    };

    assert!(displayed_output
        .get_exact_or_next_displayed_row(5)
        .is_none());
}

#[test]
fn test_new_from_displayed_lines() {
    let displayed_rows = make_displayed_rows_from_ranges(vec![1..=3, 7..=7, 14..=18]);
    let displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_rows.clone());

    let expected_bimap = create_bimap_with_insertions(&[
        (1, 0),
        (2, 1),
        (3, 2),
        (7, 3),
        (14, 4),
        (15, 5),
        (16, 6),
        (17, 7),
        (18, 8),
    ]);
    assert_eq!(displayed_output.displayed_rows, displayed_rows);
    assert_eq!(displayed_output.height, 9);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
fn test_extend_displayed_lines() {
    let displayed_rows = make_displayed_rows_from_ranges(vec![1..=3]);
    let mut displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_rows.clone());

    displayed_output.extend_displayed_lines(make_displayed_rows_from_ranges(vec![7..=7, 14..=18]));

    let expected_displayed_rows = make_displayed_rows_from_ranges(vec![1..=3, 7..=7, 14..=18]);
    let expected_bimap = create_bimap_with_insertions(&[
        (1, 0),
        (2, 1),
        (3, 2),
        (7, 3),
        (14, 4),
        (15, 5),
        (16, 6),
        (17, 7),
        (18, 8),
    ]);
    assert_eq!(displayed_output.displayed_rows, expected_displayed_rows);
    assert_eq!(displayed_output.height, 9);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
fn test_prepend_displayed_lines() {
    let displayed_rows = make_displayed_rows_from_ranges(vec![14..=18]);
    let mut displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_rows.clone());

    displayed_output.prepend_displayed_lines(make_displayed_rows_from_ranges(vec![1..=3, 7..=7]));

    let expected_displayed_rows = make_displayed_rows_from_ranges(vec![1..=3, 7..=7, 14..=18]);
    let expected_bimap = create_bimap_with_insertions(&[
        (1, 0),
        (2, 1),
        (3, 2),
        (7, 3),
        (14, 4),
        (15, 5),
        (16, 6),
        (17, 7),
        (18, 8),
    ]);
    assert_eq!(displayed_output.displayed_rows, expected_displayed_rows);
    assert_eq!(displayed_output.height, 9);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
fn test_splice_displayed_lines() {
    let displayed_rows = make_displayed_rows_from_ranges(vec![0..=1, 4..=5, 14..=16, 19..=19]);
    let mut displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_rows.clone());

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![6..=6, 8..=9]);
    displayed_output.splice_displayed_lines(1..3, new_displayed_rows);

    let expected_displayed_rows =
        make_displayed_rows_from_ranges(vec![0..=1, 6..=6, 8..=9, 19..=19]);
    let expected_bimap =
        create_bimap_with_insertions(&[(0, 0), (1, 1), (6, 2), (8, 3), (9, 4), (19, 5)]);
    assert_eq!(displayed_output.displayed_rows, expected_displayed_rows);
    assert_eq!(displayed_output.height, 6);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![10..=12, 14..=15]);
    displayed_output.splice_displayed_lines(3..4, new_displayed_rows);

    let expected_displayed_rows =
        make_displayed_rows_from_ranges(vec![0..=1, 6..=6, 8..=9, 10..=12, 14..=15]);
    let expected_bimap = create_bimap_with_insertions(&[
        (0, 0),
        (1, 1),
        (6, 2),
        (8, 3),
        (9, 4),
        (10, 5),
        (11, 6),
        (12, 7),
        (14, 8),
        (15, 9),
    ]);
    assert_eq!(displayed_output.displayed_rows, expected_displayed_rows);
    assert_eq!(displayed_output.height, 10);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
fn test_splice_displayed_lines_nothing_replaced() {
    let displayed_rows = make_displayed_rows_from_ranges(vec![0..=1, 4..=5, 14..=16, 19..=19]);
    let mut displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_rows.clone());

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![8..=9]);
    displayed_output.splice_displayed_lines(2..2, new_displayed_rows);

    let expected_displayed_rows =
        make_displayed_rows_from_ranges(vec![0..=1, 4..=5, 8..=9, 14..=16, 19..=19]);
    let expected_bimap = create_bimap_with_insertions(&[
        (0, 0),
        (1, 1),
        (4, 2),
        (5, 3),
        (8, 4),
        (9, 5),
        (14, 6),
        (15, 7),
        (16, 8),
        (19, 9),
    ]);
    assert_eq!(displayed_output.displayed_rows, expected_displayed_rows);
    assert_eq!(displayed_output.height, 10);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
fn test_splice_displayed_lines_no_new_lines() {
    let displayed_rows = make_displayed_rows_from_ranges(vec![0..=1, 4..=5, 14..=16, 19..=19]);
    let mut displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_rows.clone());

    displayed_output.splice_displayed_lines(1..2, Vec::new());

    let expected_displayed_rows = make_displayed_rows_from_ranges(vec![0..=1, 14..=16, 19..=19]);
    let expected_bimap =
        create_bimap_with_insertions(&[(0, 0), (1, 1), (14, 2), (15, 3), (16, 4), (19, 5)]);
    assert_eq!(displayed_output.displayed_rows, expected_displayed_rows);
    assert_eq!(displayed_output.height, 6);
    assert_eq!(displayed_output.row_translation_map, expected_bimap.into());
}

#[test]
fn test_first_rows_greater_than_or_contained_in() {
    let displayed_rows = make_displayed_rows_from_ranges(vec![2..=5, 7..=9, 14..=16, 19..=19]);
    let displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_rows.clone());

    let res = displayed_output.first_rows_greater_than_or_contained_in(19);
    assert_eq!(res.unwrap().0, 3);
    assert_eq!(res.unwrap().1.range.clone(), 19..=19);

    let res = displayed_output.first_rows_greater_than_or_contained_in(15);
    assert_eq!(res.unwrap().0, 2);
    assert_eq!(res.unwrap().1.range.clone(), 14..=16);

    let res = displayed_output.first_rows_greater_than_or_contained_in(1);
    assert_eq!(res.unwrap().0, 0);
    assert_eq!(res.unwrap().1.range.clone(), 2..=5);
}

#[test]
fn test_first_rows_greater_than_or_contained_in_no_result() {
    let displayed_rows = make_displayed_rows_from_ranges(vec![2..=5, 7..=9, 14..=16, 19..=19]);
    let displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_rows.clone());

    let res = displayed_output.first_rows_greater_than_or_contained_in(20);
    assert_eq!(res, None);
}

#[test]
fn test_last_rows_less_than_or_contained_in() {
    let displayed_rows = make_displayed_rows_from_ranges(vec![2..=5, 7..=9, 14..=16, 19..=19]);
    let displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_rows.clone());

    let res = displayed_output.last_rows_less_than_or_contained_in(2);
    assert_eq!(res.unwrap().0, 0);
    assert_eq!(res.unwrap().1.range.clone(), 2..=5);

    let res = displayed_output.last_rows_less_than_or_contained_in(15);
    assert_eq!(res.unwrap().0, 2);
    assert_eq!(res.unwrap().1.range.clone(), 14..=16);

    let res = displayed_output.last_rows_less_than_or_contained_in(20);
    assert_eq!(res.unwrap().0, 3);
    assert_eq!(res.unwrap().1.range.clone(), 19..=19);
}

#[test]
fn test_last_rows_less_than_or_contained_in_no_result() {
    let displayed_rows = make_displayed_rows_from_ranges(vec![2..=5, 7..=9, 14..=16, 19..=19]);
    let displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_rows.clone());

    let res = displayed_output.last_rows_less_than_or_contained_in(1);
    assert_eq!(res, None);
}
