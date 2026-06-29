use super::write_stream_snapshot_delta;

#[test]
fn stream_snapshots_write_only_appended_text() {
    let mut last_output = String::new();
    let mut output = Vec::new();

    for snapshot in ["h", "he", "hello", "hello", "hello world"] {
        write_stream_snapshot_delta(&mut last_output, snapshot, &mut output).unwrap();
    }

    assert_eq!(String::from_utf8(output).unwrap(), "hello world");
    assert_eq!(last_output, "hello world");
}
