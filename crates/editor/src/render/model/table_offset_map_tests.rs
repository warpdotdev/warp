use markdown_parser::weight::CustomWeight;
use markdown_parser::{Hyperlink, parse_inline_markdown};

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
fn test_table_cell_offset_map_handles_bold_and_links() {
    let source = "**Bold** [Link](https://warp.dev)";
    let inline = parse_inline_markdown(source);
    assert!(
        inline.iter().any(|fragment| fragment
            .styles
            .weight
            .is_some_and(|weight| matches!(weight, CustomWeight::Bold))),
        "parsed inline should have a bold fragment"
    );
    assert!(
        inline
            .iter()
            .any(|fragment| matches!(&fragment.styles.hyperlink, Some(Hyperlink::Url(url)) if url == "https://warp.dev")),
        "parsed inline should have a hyperlink fragment"
    );
    let map = TableCellOffsetMap::from_inline_and_source(source, &inline);

    assert_eq!(map.rendered_length(), CharOffset::from(9));
    assert_eq!(
        map.source_length(),
        CharOffset::from(source.chars().count())
    );
    assert_eq!(
        map.rendered_to_source(CharOffset::from(0)),
        CharOffset::from(2)
    );
    assert_eq!(
        map.rendered_to_source(CharOffset::from(4)),
        CharOffset::from(8)
    );
    assert_eq!(
        map.rendered_to_source(CharOffset::from(5)),
        CharOffset::from(10)
    );
    assert_eq!(
        map.rendered_to_source(CharOffset::from(9)),
        CharOffset::from(14)
    );
    assert_eq!(
        map.source_to_rendered(CharOffset::from(0)),
        CharOffset::from(0)
    );
    assert_eq!(
        map.source_to_rendered(CharOffset::from(2)),
        CharOffset::from(0)
    );
    assert_eq!(
        map.source_to_rendered(CharOffset::from(11)),
        CharOffset::from(6)
    );
    assert_eq!(
        map.source_to_rendered(CharOffset::from(14)),
        CharOffset::from(9)
    );
    assert_eq!(
        map.source_to_rendered(CharOffset::from(32)),
        CharOffset::from(9)
    );
}

#[test]
fn test_table_cell_offset_map_handles_backslash_escaped_punctuation() {
    let source = "a \\*star\\* b";
    let inline = parse_inline_markdown(source);
    let rendered_text: String = inline
        .iter()
        .map(|fragment| fragment.text.as_str())
        .collect();
    assert_eq!(rendered_text, "a *star* b");

    let map = TableCellOffsetMap::from_inline_and_source(source, &inline);
    assert_eq!(
        map.rendered_length(),
        CharOffset::from(rendered_text.chars().count())
    );
    assert_eq!(
        map.source_length(),
        CharOffset::from(source.chars().count())
    );
    assert_eq!(
        map.rendered_to_source(CharOffset::from(rendered_text.chars().count())),
        CharOffset::from(source.chars().count())
    );
}

#[test]
fn test_table_cell_offset_map_handles_nested_styles() {
    let source = "**a *b* c**";
    let inline = parse_inline_markdown(source);
    let rendered_text: String = inline
        .iter()
        .map(|fragment| fragment.text.as_str())
        .collect();
    assert_eq!(rendered_text, "a b c");

    let map = TableCellOffsetMap::from_inline_and_source(source, &inline);
    assert_eq!(
        map.source_length(),
        CharOffset::from(source.chars().count())
    );
    assert_eq!(
        map.rendered_length(),
        CharOffset::from(rendered_text.chars().count())
    );
    for (rendered_idx, rendered_char) in rendered_text.chars().enumerate() {
        let source_pos = map.rendered_to_source(CharOffset::from(rendered_idx));
        assert_eq!(
            source.chars().nth(source_pos.as_usize()),
            Some(rendered_char),
            "rendered {rendered_idx} ({rendered_char:?}) should map to same char in source",
        );
    }
}
