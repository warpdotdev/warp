use super::format_gfm_table;

#[test]
fn format_gfm_table_normalizes_column_widths() {
    let lines = vec![
        "| Short | Medium Length | This is much longer |".to_owned(),
        "| --- | --- | --- |".to_owned(),
        "| A | Hello world | X |".to_owned(),
    ];
    let result = format_gfm_table(&lines);
    let result_lines: Vec<&str> = result.lines().collect();

    assert_eq!(result_lines.len(), 3);
    // All rows should have the same length due to padding
    assert_eq!(result_lines[0].len(), result_lines[1].len());
    assert_eq!(result_lines[1].len(), result_lines[2].len());
    // Check content is preserved
    assert!(result_lines[0].contains("Short"));
    assert!(result_lines[0].contains("Medium Length"));
    assert!(result_lines[0].contains("This is much longer"));
    assert!(result_lines[2].contains("A"));
    assert!(result_lines[2].contains("Hello world"));
}

#[test]
fn format_gfm_table_preserves_alignment_markers() {
    let lines = vec![
        "| Left | Center | Right |".to_owned(),
        "| :--- | :---: | ---: |".to_owned(),
        "| A | B | C |".to_owned(),
    ];
    let result = format_gfm_table(&lines);
    let sep_line = result.lines().nth(1).unwrap();

    // Extract separator cells (trim leading/trailing pipes and split)
    let cells: Vec<&str> = sep_line
        .trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim())
        .collect();

    assert_eq!(cells.len(), 3);
    // Left alignment: starts with dashes (no leading colon after trimming)
    assert!(
        cells[0].starts_with('-'),
        "Left column should be left-aligned"
    );
    // Center alignment: starts and ends with colon
    assert!(
        cells[1].starts_with(':') && cells[1].ends_with(':'),
        "Center column should be center-aligned"
    );
    // Right alignment: ends with colon but doesn't start with one
    assert!(
        !cells[2].starts_with(':') && cells[2].ends_with(':'),
        "Right column should be right-aligned"
    );
}

#[test]
fn format_gfm_table_handles_rows_with_fewer_columns() {
    let lines = vec![
        "| A | B | C |".to_owned(),
        "| --- | --- | --- |".to_owned(),
        "| X |".to_owned(), // Missing columns
    ];
    let result = format_gfm_table(&lines);
    let result_lines: Vec<&str> = result.lines().collect();

    // Should still produce valid output
    assert_eq!(result_lines.len(), 3);
    // Last row should be padded to have same structure
    assert_eq!(result_lines[0].len(), result_lines[2].len());
}

#[test]
fn format_gfm_table_handles_empty_cells() {
    let lines = vec![
        "| A | | C |".to_owned(),
        "| --- | --- | --- |".to_owned(),
        "| | B | |".to_owned(),
    ];
    let result = format_gfm_table(&lines);

    // Should produce aligned output with empty cells preserved
    assert!(result.contains("| A"));
    assert!(result.contains("| B"));
    assert!(result.contains("| C"));
}
