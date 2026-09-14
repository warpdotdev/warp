use std::io::{BufReader, Error, Read};

use serde_json::json;

use super::{JsonlReadStatus, parse_jsonl};

#[test]
fn distinguishes_interior_damage_from_an_appending_tail() {
    let capture =
        parse_jsonl(b"\n{\"input_tokens\":9007199254740993}\nnot json\n{\"type\":".as_slice());
    assert_eq!(
        capture.entries,
        vec![json!({"input_tokens": 9007199254740993_i64})]
    );
    assert_eq!(capture.diagnostics.records_read, 1);
    assert_eq!(capture.diagnostics.malformed_records, 1);
    assert!(capture.diagnostics.incomplete_trailing_record);
    assert_eq!(capture.diagnostics.status, JsonlReadStatus::Readable);
}

struct Interrupted;

impl Read for Interrupted {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(Error::other("synthetic read failure"))
    }
}

#[test]
fn retains_records_before_read_failure() {
    let reader = b"{}\n".as_slice().chain(Interrupted);
    let capture = parse_jsonl(BufReader::new(reader));
    assert_eq!(capture.entries, vec![json!({})]);
    assert_eq!(capture.diagnostics.status, JsonlReadStatus::Unreadable);
    assert!(!capture.diagnostics.is_complete());
}
