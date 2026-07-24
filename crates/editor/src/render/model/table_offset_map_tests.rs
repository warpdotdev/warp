use markdown_parser::parse_inline_markdown_with_source_map;

use super::*;

#[test]
fn test_simple_table() {
    let map = TableOffsetMap::new(vec![vec![1, 2], vec![3, 1]]);

    assert_eq!(map.total_length(), CharOffset::from(11));
    assert_eq!(map.num_rows(), 2);
    assert_eq!(map.num_cols(), 2);

    assert!(matches!(
        map.position_at_offset(CharOffset::from(0)),
        Some(TablePosition::InCell { row: 0, col: 0, .. })
    ));

    assert!(matches!(
        map.position_at_offset(CharOffset::from(1)),
        Some(TablePosition::OnTab {
            row: 0,
            after_col: 0
        })
    ));

    assert!(matches!(
        map.position_at_offset(CharOffset::from(2)),
        Some(TablePosition::InCell { row: 0, col: 1, .. })
    ));

    assert!(matches!(
        map.position_at_offset(CharOffset::from(4)),
        Some(TablePosition::OnNewline { row: 0 })
    ));

    assert!(matches!(
        map.position_at_offset(CharOffset::from(5)),
        Some(TablePosition::InCell { row: 1, col: 0, .. })
    ));
}

#[test]
fn test_cell_at_offset() {
    let map = TableOffsetMap::new(vec![vec![3, 3]]);

    assert_eq!(
        map.cell_at_offset(CharOffset::from(0)),
        Some(CellAtOffset {
            row: 0,
            col: 0,
            offset_in_cell: CharOffset::from(0)
        })
    );

    assert_eq!(
        map.cell_at_offset(CharOffset::from(2)),
        Some(CellAtOffset {
            row: 0,
            col: 0,
            offset_in_cell: CharOffset::from(2)
        })
    );

    assert_eq!(
        map.cell_at_offset(CharOffset::from(4)),
        Some(CellAtOffset {
            row: 0,
            col: 1,
            offset_in_cell: CharOffset::from(0)
        })
    );
}

#[test]
fn test_out_of_bounds_offset() {
    let map = TableOffsetMap::new(vec![vec![2, 2]]);
    assert!(map.position_at_offset(map.total_length()).is_none());
    assert!(map.position_at_offset(CharOffset::from(100)).is_none());
    assert!(map.cell_at_offset(map.total_length()).is_none());
}

#[test]
fn test_is_separator() {
    let map = TableOffsetMap::new(vec![vec![1, 1], vec![1, 1]]);
    assert!(!map.is_separator(CharOffset::from(0)));
    assert!(map.is_separator(CharOffset::from(1)));
    assert!(map.is_separator(CharOffset::from(3)));
    assert!(!map.is_separator(CharOffset::from(4)));
}

#[test]
fn test_empty_cells() {
    let map = TableOffsetMap::new(vec![vec![0, 3], vec![2, 0]]);
    assert_eq!(map.num_rows(), 2);
    assert_eq!(map.num_cols(), 2);

    assert!(matches!(
        map.position_at_offset(CharOffset::from(0)),
        Some(TablePosition::OnTab {
            row: 0,
            after_col: 0
        })
    ));

    assert!(matches!(
        map.position_at_offset(CharOffset::from(1)),
        Some(TablePosition::InCell { row: 0, col: 1, .. })
    ));
}

#[test]
fn test_cells_in_range() {
    let map = TableOffsetMap::new(vec![vec![2, 2], vec![2, 2]]);
    let cells = map.cells_in_range(CharOffset::from(0), map.total_length());
    assert_eq!(cells.len(), 4);

    let first_row = map.cells_in_range(CharOffset::from(0), CharOffset::from(5));
    assert_eq!(first_row.len(), 2);
    assert_eq!(first_row[0].row, 0);
    assert_eq!(first_row[0].col, 0);
    assert_eq!(first_row[1].row, 0);
    assert_eq!(first_row[1].col, 1);
}

#[test]
fn test_cell_range() {
    let map = TableOffsetMap::new(vec![vec![3, 2]]);
    assert_eq!(
        map.cell_range(0, 0),
        Some(CellOffsetRange {
            start: CharOffset::from(0),
            end: CharOffset::from(3)
        })
    );
    assert_eq!(
        map.cell_range(0, 1),
        Some(CellOffsetRange {
            start: CharOffset::from(4),
            end: CharOffset::from(6)
        })
    );
    assert!(map.cell_range(0, 2).is_none());
    assert!(map.cell_range(1, 0).is_none());
}

#[test]
fn test_table_cell_offset_map_delegates_generic_newline_selection_mapping() {
    let parsed = parse_inline_markdown_with_source_map("a\nb");
    let map = TableCellOffsetMap::from_source_map(parsed.source_map);

    assert_eq!(map.rendered_length(), CharOffset::from(3));
    assert_eq!(map.source_length(), CharOffset::from(3));
    assert_eq!(
        map.rendered_to_source(CharOffset::from(1)),
        CharOffset::from(1)
    );
    assert_eq!(
        map.source_to_rendered(CharOffset::from(2)),
        CharOffset::from(2)
    );
}
