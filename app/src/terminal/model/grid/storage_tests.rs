use crate::terminal::model::cell::Cell;
use crate::terminal::model::grid::row::Row;
use crate::terminal::model::grid::storage::Storage;

// Use a large value for `MAX_CACHE_SIZE` for testing.
const MAX_CACHE_SIZE: usize = 1_000;

#[test]
fn with_capacity() {
    let storage = Storage::with_capacity(3, 1, false);

    assert_eq!(storage.inner.len(), 3);
    assert_eq!(storage.len, 3);
    assert_eq!(storage.bottom_row, 0);
    assert_eq!(storage.visible_lines, 3);
}

#[test]
fn testing_grid_to_raw_storage_indexing() {
    // Macro testing converstion of grid indices to raw-storage indices
    macro_rules! assert_index_mapping {
            ($storage:ident, $(($grid_idx:literal, $expected_storage_idx:literal)),+) => {
                $(
                    let actual = $storage.from_grid_index($grid_idx);
                    assert_eq!(actual, $expected_storage_idx, "Expected grid index {} to map to storage index {} but got {actual}", $grid_idx, $expected_storage_idx);
                )+
            }
        }

    let mut storage = Storage::with_capacity(10, 1, false);
    assert_index_mapping!(
        storage,
        (0, 9),
        (1, 8),
        (2, 7),
        (3, 6),
        (4, 5),
        (5, 4),
        (6, 3),
        (7, 2),
        (8, 1),
        (9, 0)
    );

    // Simulate a shift (going over grid limits)
    // Changing bottom_row to 8 should mean the oldest row (0) is at raw index 7
    storage.bottom_row = 8;
    assert_index_mapping!(
        storage,
        (0, 7),
        (1, 6),
        (2, 5),
        (3, 4),
        (4, 3),
        (5, 2),
        (6, 1),
        (7, 0),
        (8, 9),
        (9, 8)
    );

    // Simulate a size increase.
    // This causes a rezero-ing and we're back to regular reversed order.
    storage.initialize(5, 1);
    assert_index_mapping!(
        storage,
        (0, 14),
        (1, 13),
        (2, 12),
        (3, 11),
        (4, 10),
        (5, 9),
        (6, 8),
        (7, 7),
        (8, 6),
        (9, 5)
    );
}

#[test]
fn testing_visible_row_to_grid_indexing() {
    // visible rows are always the bottom (for grids)
    // Here, we have 10 visible rows and an overall grid of 15 rows.
    let mut storage = Storage::with_capacity(10, 1, false);
    storage.initialize(5, 1);
    assert_eq!(
        storage.to_grid_index(crate::terminal::model::index::VisibleRow(9)),
        14
    );
    assert_eq!(
        storage.to_grid_index(crate::terminal::model::index::VisibleRow(0)),
        5
    );
}

#[test]
fn indexing() {
    let mut storage = Storage::with_capacity(3, 1, false);

    storage[0] = filled_row('0');
    storage[1] = filled_row('1');
    storage[2] = filled_row('2');

    assert_eq!(storage[0], filled_row('0'));
    assert_eq!(storage[1], filled_row('1'));
    assert_eq!(storage[2], filled_row('2'));

    storage.bottom_row = 1;

    // now it's 2, 0, 1
    // indexing into storage takes a grid index and does its own conversion to a raw index
    assert_eq!(storage[0], filled_row('2'));
    assert_eq!(storage[1], filled_row('0'));
    assert_eq!(storage[2], filled_row('1'));
}

#[test]
#[should_panic]
fn indexing_above_inner_len() {
    let storage = Storage::with_capacity(1, 1, false);
    let _ = &storage[2];
}

#[test]
fn rotate() {
    let mut storage = Storage::with_capacity(3, 1, false);
    storage.rotate(2);
    assert_eq!(storage.bottom_row, 2);
    storage.shrink_lines(2);
    assert_eq!(storage.len, 1);
    assert_eq!(storage.inner.len(), 1);
    assert_eq!(storage.bottom_row, 0);
}

/// Grow the buffer one line at the end of the buffer.
///
/// Before:
///   0: 0 <- Zero
///   1: 1
///   2: -
/// After:
///   0: 0 <- Zero
///   1: 1
///   2: -
///   3: Cell::default
///   ...
///   MAX_CACHE_SIZE: 0
#[test]
fn grow_after_zero() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![filled_row('0'), filled_row('1'), filled_row('-')],
        bottom_row: 0,
        visible_lines: 3,
        len: 3,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Grow buffer.
    storage.grow_visible_lines(4);

    // Make sure the result is correct.
    let mut expected = Storage {
        inner: vec![filled_row('0'), filled_row('1'), filled_row('-')],
        bottom_row: 0,
        visible_lines: 4,
        len: 4,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };
    expected
        .inner
        .append(&mut vec![filled_row('\0'); MAX_CACHE_SIZE]);

    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// Grow the buffer one line at the start of the buffer.
///
/// Before:
///   0: -
///   1: 0 <- Zero
///   2: 1
/// After:
///   0: 0 <- Zero
///   1: 1
///   2: -
///   3: Cell::default
///   ...
///   MAX_CACHE_SIZE: 0
#[test]
fn grow_before_zero() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![filled_row('-'), filled_row('0'), filled_row('1')],
        bottom_row: 1,
        visible_lines: 3,
        len: 3,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Grow buffer.
    storage.grow_visible_lines(4);

    // Make sure the result is correct.
    let mut expected = Storage {
        inner: vec![filled_row('0'), filled_row('1'), filled_row('-')],
        bottom_row: 0,
        visible_lines: 4,
        len: 4,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };
    expected
        .inner
        .append(&mut vec![filled_row('\0'); MAX_CACHE_SIZE]);

    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// Shrink the buffer one line at the start of the buffer.
///
/// Before:
///   0: 2
///   1: 0 <- Zero
///   2: 1
/// After:
///   0: 2 <- Hidden
///   0: 0 <- Zero
///   1: 1
#[test]
fn shrink_before_zero() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![filled_row('2'), filled_row('0'), filled_row('1')],
        bottom_row: 1,
        visible_lines: 3,
        len: 3,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Shrink buffer.
    storage.shrink_visible_lines(2);

    // Make sure the result is correct.
    let expected = Storage {
        inner: vec![filled_row('2'), filled_row('0'), filled_row('1')],
        bottom_row: 1,
        visible_lines: 2,
        len: 2,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };
    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// Shrink the buffer one line at the end of the buffer.
///
/// Before:
///   0: 0 <- Zero
///   1: 1
///   2: 2
/// After:
///   0: 0 <- Zero
///   1: 1
///   2: 2 <- Hidden
#[test]
fn shrink_after_zero() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![filled_row('0'), filled_row('1'), filled_row('2')],
        bottom_row: 0,
        visible_lines: 3,
        len: 3,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Shrink buffer.
    storage.shrink_visible_lines(2);

    // Make sure the result is correct.
    let expected = Storage {
        inner: vec![filled_row('0'), filled_row('1'), filled_row('2')],
        bottom_row: 0,
        visible_lines: 2,
        len: 2,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };
    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// Shrink the buffer at the start and end of the buffer.
///
/// Before:
///   0: 4
///   1: 5
///   2: 0 <- Zero
///   3: 1
///   4: 2
///   5: 3
/// After:
///   0: 4 <- Hidden
///   1: 5 <- Hidden
///   2: 0 <- Zero
///   3: 1
///   4: 2 <- Hidden
///   5: 3 <- Hidden
#[test]
fn shrink_before_and_after_zero() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![
            filled_row('4'),
            filled_row('5'),
            filled_row('0'),
            filled_row('1'),
            filled_row('2'),
            filled_row('3'),
        ],
        bottom_row: 2,
        visible_lines: 6,
        len: 6,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Shrink buffer.
    storage.shrink_visible_lines(2);

    // Make sure the result is correct.
    let expected = Storage {
        inner: vec![
            filled_row('4'),
            filled_row('5'),
            filled_row('0'),
            filled_row('1'),
            filled_row('2'),
            filled_row('3'),
        ],
        bottom_row: 2,
        visible_lines: 2,
        len: 2,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };
    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// Check that truncating columns works as expected.
///
/// Before (e indicates empty):
///   0: 1 1 1 e e e <- Zero
///   1: 2 2 e e e e
///   2: 3 3 3 3 e e
///   3: 4 e e e e e
///   4: 5 5 e e e e
/// After:
///   0: 1 1 1 e <- Zero
///   1: 2 2 e e
///   2: 3 3 3 3
///   3: 4 e e e
///   4: 5 5 e e
#[test]
fn truncate_columns() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![
            partially_filled_row('1', 3, 6),
            partially_filled_row('2', 2, 6),
            partially_filled_row('3', 4, 6),
            partially_filled_row('4', 1, 6),
            partially_filled_row('5', 2, 6),
        ],
        bottom_row: 0,
        visible_lines: 5,
        len: 5,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Truncate buffer (columns).
    storage.truncate_columns(4);

    // Make sure the result is correct.
    let expected = Storage {
        inner: vec![
            partially_filled_row('1', 3, 4),
            partially_filled_row('2', 2, 4),
            partially_filled_row('3', 4, 4),
            partially_filled_row('4', 1, 4),
            partially_filled_row('5', 2, 4),
        ],
        bottom_row: 0,
        visible_lines: 5,
        len: 5,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };
    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// Check that truncating columns works as expected.
///
/// Before (e indicates empty):
///   0: 1 1 1 e e e <- Zero
///   1: 2 2 e e e e
///   2: 3 3 3 3 e e
///   3: 4 e e e e e
///   4: 5 5 5 5 5 e <- Hidden
/// After:
///   0: 1 1 1 e <- Zero
///   1: 2 2 e e
///   2: 3 3 3 3
///   3: 4 e e e
///   4: 5 5 5 5 <- Hidden (truncated as well)
#[test]
fn truncate_columns_ignore_hidden_rows() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![
            partially_filled_row('1', 3, 6),
            partially_filled_row('2', 2, 6),
            partially_filled_row('3', 4, 6),
            partially_filled_row('4', 1, 6),
            partially_filled_row('5', 5, 6),
        ],
        bottom_row: 0,
        visible_lines: 4,
        len: 4,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Truncate buffer (columns).
    storage.truncate_columns(4);

    // Make sure the result is correct.
    let expected = Storage {
        inner: vec![
            partially_filled_row('1', 3, 4),
            partially_filled_row('2', 2, 4),
            partially_filled_row('3', 4, 4),
            partially_filled_row('4', 1, 4),
            partially_filled_row('5', 4, 4),
        ],
        bottom_row: 0,
        visible_lines: 4,
        len: 4,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };
    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// Check that when truncating all hidden lines are removed from the raw buffer.
///
/// Before:
///   0: 4 <- Hidden
///   1: 5 <- Hidden
///   2: 0 <- Zero
///   3: 1
///   4: 2 <- Hidden
///   5: 3 <- Hidden
/// After:
///   0: 0 <- Zero
///   1: 1
#[test]
fn truncate_invisible_lines() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![
            filled_row('4'),
            filled_row('5'),
            filled_row('0'),
            filled_row('1'),
            filled_row('2'),
            filled_row('3'),
        ],
        bottom_row: 2,
        visible_lines: 1,
        len: 2,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Truncate buffer.
    storage.truncate_unused_rows();

    // Make sure the result is correct.
    let expected = Storage {
        inner: vec![filled_row('0'), filled_row('1')],
        bottom_row: 0,
        visible_lines: 1,
        len: 2,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };
    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// Truncate buffer only at the beginning.
///
/// Before:
///   0: 1
///   1: 2 <- Hidden
///   2: 0 <- Zero
/// After:
///   0: 1
///   0: 0 <- Zero
#[test]
fn truncate_invisible_lines_beginning() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![filled_row('1'), filled_row('2'), filled_row('0')],
        bottom_row: 2,
        visible_lines: 1,
        len: 2,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Truncate buffer.
    storage.truncate_unused_rows();

    // Make sure the result is correct.
    let expected = Storage {
        inner: vec![filled_row('0'), filled_row('1')],
        bottom_row: 0,
        visible_lines: 1,
        len: 2,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };
    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// Before:
///   0: 1 <- Zero
///   1: 2
///   2: 0
/// After:
///   0: 0 <- Zero
#[test]
fn truncate_to_no_invisible_lines_unrotated_buffer() {
    let mut storage = Storage {
        inner: vec![filled_row('1'), filled_row('2'), filled_row('0')],
        bottom_row: 0,
        visible_lines: 3,
        len: 3,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    storage.truncate_to(1);

    let expected = Storage {
        inner: vec![filled_row('0')],
        bottom_row: 0,
        visible_lines: 1,
        len: 1,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// Before:
///   0: 1
///   1: 2 <- Zero
///   2: 0
/// After:
///   0: 1 <- Zero
#[test]
fn truncate_to_no_invisible_lines_rotated_buffer() {
    let mut storage = Storage {
        inner: vec![filled_row('1'), filled_row('2'), filled_row('0')],
        bottom_row: 1,
        visible_lines: 3,
        len: 3,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    storage.truncate_to(1);

    let expected = Storage {
        inner: vec![filled_row('1')],
        bottom_row: 0,
        visible_lines: 1,
        len: 1,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// Before:
///   0: 3 <- Hidden
///   1: 4 <- Zero
///   2: 0
///   3: 1
///   4: 2
/// After:
///   0: 2 <- Zero
///   1: 3
#[test]
fn truncate_to_with_invisible_lines_rotated_buffer() {
    let mut storage = Storage {
        inner: vec![
            filled_row('3'),
            filled_row('4'),
            filled_row('0'),
            filled_row('1'),
            filled_row('2'),
        ],
        bottom_row: 1,
        visible_lines: 2,
        // The grid has 1 hidden line (`inner` has 5 lines).
        len: 4,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    storage.truncate_to(2);

    let expected = Storage {
        inner: vec![filled_row('1'), filled_row('2')],
        bottom_row: 0,
        visible_lines: 2,
        len: 2,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

#[test]
fn truncate_to_with_larger_target_len() {
    let mut storage = Storage {
        inner: vec![filled_row('1')],
        bottom_row: 0,
        visible_lines: 1,
        len: 1,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    storage.truncate_to(2);

    let expected = Storage {
        inner: vec![filled_row('1')],
        bottom_row: 0,
        visible_lines: 1,
        len: 1,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };
    assert_eq!(storage.visible_lines, expected.visible_lines);
    assert_eq!(storage.inner, expected.inner);
    assert_eq!(storage.bottom_row, expected.bottom_row);
    assert_eq!(storage.len, expected.len);
}

/// First shrink the buffer and then grow it again.
///
/// Before:
///   0: 4
///   1: 5
///   2: 0 <- Zero
///   3: 1
///   4: 2
///   5: 3
/// After Shrinking:
///   0: 4 <- Hidden
///   1: 5 <- Hidden
///   2: 0 <- Zero
///   3: 1
///   4: 2
///   5: 3 <- Hidden
/// After Growing:
///   0: 4
///   1: 5
///   2: -
///   3: 0 <- Zero
///   4: 1
///   5: 2
///   6: 3
#[test]
fn shrink_then_grow() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![
            filled_row('4'),
            filled_row('5'),
            filled_row('0'),
            filled_row('1'),
            filled_row('2'),
            filled_row('3'),
        ],
        bottom_row: 2,
        visible_lines: 0,
        len: 6,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Shrink buffer.
    storage.shrink_lines(3);

    // Make sure the result after shrinking is correct.
    let shrinking_expected = Storage {
        inner: vec![
            filled_row('4'),
            filled_row('5'),
            filled_row('0'),
            filled_row('1'),
            filled_row('2'),
            filled_row('3'),
        ],
        bottom_row: 2,
        visible_lines: 0,
        len: 3,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };
    assert_eq!(storage.inner, shrinking_expected.inner);
    assert_eq!(storage.bottom_row, shrinking_expected.bottom_row);
    assert_eq!(storage.len, shrinking_expected.len);

    // Grow buffer.
    storage.initialize(1, 1);

    // Make sure the previously freed elements are reused.
    let growing_expected = Storage {
        inner: vec![
            filled_row('4'),
            filled_row('5'),
            filled_row('0'),
            filled_row('1'),
            filled_row('2'),
            filled_row('3'),
        ],
        bottom_row: 2,
        visible_lines: 0,
        len: 4,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    assert_eq!(storage.inner, growing_expected.inner);
    assert_eq!(storage.bottom_row, growing_expected.bottom_row);
    assert_eq!(storage.len, growing_expected.len);
}

#[test]
fn initialize() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![
            filled_row('4'),
            filled_row('5'),
            filled_row('0'),
            filled_row('1'),
            filled_row('2'),
            filled_row('3'),
        ],
        bottom_row: 2,
        visible_lines: 0,
        len: 6,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Initialize additional lines.
    let init_size = 3;
    storage.initialize(init_size, 1);

    // Generate expected grid.
    let mut expected_inner = vec![
        filled_row('0'),
        filled_row('1'),
        filled_row('2'),
        filled_row('3'),
        filled_row('4'),
        filled_row('5'),
    ];
    let expected_init_size = std::cmp::max(init_size, MAX_CACHE_SIZE);
    expected_inner.append(&mut vec![filled_row('\0'); expected_init_size]);
    let expected_storage = Storage {
        inner: expected_inner,
        bottom_row: 0,
        visible_lines: 0,
        len: 9,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    assert_eq!(storage.len, expected_storage.len);
    assert_eq!(storage.bottom_row, expected_storage.bottom_row);
    assert_eq!(storage.inner, expected_storage.inner);
}

#[test]
fn rotate_wrap_zero() {
    let mut storage = Storage {
        inner: vec![filled_row('-'), filled_row('-'), filled_row('-')],
        bottom_row: 2,
        visible_lines: 0,
        len: 3,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    storage.rotate(2);

    assert!(storage.bottom_row < storage.inner.len());
}

fn filled_row(content: char) -> Row {
    let mut row = Row::new(1);
    let mut cell = Cell::default();
    cell.c = content;
    row[0] = cell;
    row
}

fn partially_filled_row(content: char, columns_to_fill: usize, total_columns: usize) -> Row {
    let mut row = Row::new(total_columns);
    for col in 0..total_columns {
        let mut cell = Cell::default();
        if col <= columns_to_fill {
            cell.c = content;
        }
        row[col] = cell;
    }
    row
}
