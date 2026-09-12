use command::blocking::Command;

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

fn rows(table_id: &str, cells: Vec<Vec<&str>>) -> PowerShellTableRowsValue {
    PowerShellTableRowsValue {
        table_id: table_id.to_owned(),
        rows: cells
            .into_iter()
            .map(|row| row.into_iter().map(str::to_owned).collect())
            .collect(),
        ..Default::default()
    }
}

fn end(table_id: &str) -> PowerShellTableEndValue {
    PowerShellTableEndValue {
        table_id: table_id.to_owned(),
        ..Default::default()
    }
}

#[test]
fn end_makes_table_available_without_waiting_for_command_finished() {
    let mut stream = PowerShellTableStream::default();
    stream.begin(begin("a"));
    stream.rows(&rows("a", vec![vec!["alpha"]]));

    let tables = stream.end(&end("a"));
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].rows, vec![vec!["alpha", ""]]);
    assert!(stream.finish_command().is_empty());
}

#[test]
fn tables_are_emitted_in_stream_order_when_ended() {
    let mut stream = PowerShellTableStream::default();
    let mut inserted = Vec::new();

    stream.begin(begin("a"));
    stream.rows(&rows("a", vec![vec!["alpha"]]));
    inserted.extend(stream.end(&end("a")));

    stream.begin(begin("b"));
    stream.rows(&rows("b", vec![vec!["beta", "2", "ignored"]]));
    inserted.extend(stream.end(&end("b")));

    assert_eq!(inserted.len(), 2);
    assert_eq!(inserted[0].table_id, "a");
    assert_eq!(inserted[0].rows, vec![vec!["alpha", ""]]);
    assert_eq!(inserted[1].table_id, "b");
    assert_eq!(inserted[1].rows, vec![vec!["beta", "2"]]);
    assert!(stream.finish_command().is_empty());
}

#[test]
fn finish_command_recovers_table_when_end_is_missing() {
    let mut stream = PowerShellTableStream::default();
    stream.begin(begin("a"));
    stream.rows(&rows("a", vec![vec!["kept", "7"]]));

    let tables = stream.finish_command();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].rows, vec![vec!["kept", "7"]]);
}

#[test]
fn precmd_recovery_uses_the_same_missing_end_path_as_command_finished() {
    let mut stream = PowerShellTableStream::default();
    stream.begin(begin("pending"));
    stream.rows(&rows("pending", vec![vec!["row"]]));

    let recovered = stream.finish_command();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].table_id, "pending");
    assert!(stream.finish_command().is_empty());
}

#[test]
fn begin_without_end_keeps_both_tables_until_the_next_stable_insertion_point() {
    let mut stream = PowerShellTableStream::default();
    stream.begin(begin("a"));
    stream.rows(&rows("a", vec![vec!["first"]]));

    stream.begin(begin("b"));
    stream.rows(&rows("b", vec![vec!["second"]]));

    let tables = stream.end(&end("b"));
    assert_eq!(
        tables
            .iter()
            .map(|table| table.table_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

#[test]
fn clear_discards_current_and_queued_tables() {
    let mut stream = PowerShellTableStream::default();
    stream.begin(begin("stale"));
    stream.rows(&rows("stale", vec![vec!["discarded"]]));
    stream.begin(begin("also-stale"));
    stream.rows(&rows("also-stale", vec![vec!["discarded"]]));

    stream.clear();
    assert!(stream.finish_command().is_empty());
}

#[test]
fn mismatched_chunks_do_not_corrupt_the_active_table() {
    let mut stream = PowerShellTableStream::default();
    stream.begin(begin("expected"));
    stream.rows(&rows("other", vec![vec!["discarded"]]));
    assert!(stream.end(&end("other")).is_empty());
    stream.rows(&rows("expected", vec![vec!["kept", "7"]]));

    let tables = stream.finish_command();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].rows, vec![vec!["kept", "7"]]);
}

#[test]
fn more_than_64_columns_are_rejected_before_any_rows_are_kept() {
    let columns = (0..65)
        .map(|index| PowerShellTableColumn {
            name: format!("C{index}"),
            property_name: format!("C{index}"),
            type_name: "System.String".to_owned(),
        })
        .collect();
    let mut stream = PowerShellTableStream::default();
    stream.begin(PowerShellTableBeginValue {
        table_id: "wide".to_owned(),
        columns,
        ..Default::default()
    });
    stream.rows(&PowerShellTableRowsValue {
        table_id: "wide".to_owned(),
        rows: vec![vec!["x".to_owned(); 65]],
        ..Default::default()
    });
    assert!(stream.end(&end("wide")).is_empty());
    assert!(stream.finish_command().is_empty());
}

#[test]
fn row_stream_stops_at_ten_thousand_rows() {
    let mut stream = PowerShellTableStream::default();
    stream.begin(begin("bounded"));
    stream.rows(&PowerShellTableRowsValue {
        table_id: "bounded".to_owned(),
        rows: vec![vec!["n".to_owned(), "1".to_owned()]; 10_001],
        ..Default::default()
    });

    let tables = stream.end(&end("bounded"));
    assert_eq!(tables[0].rows.len(), 10_000);
}

#[test]
fn powershell_bootstrap_helpers_cover_table_fallback_and_bounds() {
    let Some(pwsh) = powershell_executable() else {
        eprintln!("skipping powershell helper tests: pwsh/powershell not installed");
        return;
    };
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = manifest_dir.join("src/terminal/view/powershell_rich_tables_behavior.ps1");
    let bootstrap = manifest_dir.join("assets/bundled/bootstrap/pwsh.ps1");
    let output = Command::new(pwsh)
        .args([
            "-NoProfile",
            "-File",
            script.to_str().expect("script path is utf-8"),
            "-BootstrapPath",
            bootstrap.to_str().expect("bootstrap path is utf-8"),
        ])
        .output()
        .expect("failed to spawn PowerShell");
    assert!(
        output.status.success(),
        "powershell helper tests failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn powershell_executable() -> Option<&'static str> {
    ["pwsh", "powershell"].into_iter().find(|candidate| {
        Command::new(candidate)
            .args(["-NoProfile", "-Command", "exit 0"])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    })
}
