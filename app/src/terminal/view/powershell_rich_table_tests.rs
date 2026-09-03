use super::*;

fn begin(table_id: &str) -> PowerShellTableBeginValue {
    PowerShellTableBeginValue {
        table_id: table_id.to_owned(),
        columns: vec![
            PowerShellTableColumn {
                name: "Name".to_owned(),
                property_name: "Name".to_owned(),
                type_name: "System.String".to_owned(),
            },
            PowerShellTableColumn {
                name: "Id".to_owned(),
                property_name: "Id".to_owned(),
                type_name: "System.Int32".to_owned(),
            },
        ],
        ..Default::default()
    }
}

#[test]
fn stream_preserves_contiguous_tables_and_normalizes_rows() {
    let mut stream = PowerShellTableStream::default();
    stream.begin(begin("a"));
    stream.rows(&PowerShellTableRowsValue {
        table_id: "a".to_owned(),
        rows: vec![vec!["alpha".to_owned()]],
        ..Default::default()
    });
    stream.end(&PowerShellTableEndValue {
        table_id: "a".to_owned(),
        ..Default::default()
    });
    stream.begin(begin("b"));
    stream.rows(&PowerShellTableRowsValue {
        table_id: "b".to_owned(),
        rows: vec![vec![
            "beta".to_owned(),
            "2".to_owned(),
            "ignored".to_owned(),
        ]],
        ..Default::default()
    });

    let tables = stream.finish_command();
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].rows, vec![vec!["alpha", ""]]);
    assert_eq!(tables[1].rows, vec![vec!["beta", "2"]]);
}

#[test]
fn clear_discards_stale_and_completed_tables() {
    let mut stream = PowerShellTableStream::default();
    stream.begin(begin("stale"));
    stream.rows(&PowerShellTableRowsValue {
        table_id: "stale".to_owned(),
        rows: vec![vec!["discarded".to_owned()]],
        ..Default::default()
    });
    stream.end(&PowerShellTableEndValue {
        table_id: "stale".to_owned(),
        ..Default::default()
    });

    stream.clear();
    assert!(stream.finish_command().is_empty());
}

#[test]
fn mismatched_chunks_do_not_corrupt_the_active_table() {
    let mut stream = PowerShellTableStream::default();
    stream.begin(begin("expected"));
    stream.rows(&PowerShellTableRowsValue {
        table_id: "other".to_owned(),
        rows: vec![vec!["discarded".to_owned()]],
        ..Default::default()
    });
    stream.end(&PowerShellTableEndValue {
        table_id: "other".to_owned(),
        ..Default::default()
    });
    stream.rows(&PowerShellTableRowsValue {
        table_id: "expected".to_owned(),
        rows: vec![vec!["kept".to_owned(), "7".to_owned()]],
        ..Default::default()
    });

    let tables = stream.finish_command();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].rows, vec![vec!["kept", "7"]]);
}
