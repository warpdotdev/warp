use std::io::{BufReader, Error, Read};

use serde_json::json;

use super::{JsonlLimits, JsonlReadStatus, parse_jsonl};

const UNRESTRICTED: JsonlLimits = JsonlLimits {
    max_line_bytes: usize::MAX,
    max_total_bytes: usize::MAX,
    max_records: usize::MAX,
};

#[test]
fn distinguishes_interior_damage_from_an_appending_tail() {
    let capture = parse_jsonl(
        b"\n{\"input_tokens\":9007199254740993}\nnot json\n{\"type\":".as_slice(),
        UNRESTRICTED,
    );
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
    let capture = parse_jsonl(BufReader::new(reader), UNRESTRICTED);
    assert_eq!(capture.entries, vec![json!({})]);
    assert_eq!(capture.diagnostics.status, JsonlReadStatus::Unreadable);
    assert!(!capture.diagnostics.is_complete());
}

#[test]
fn enforces_configured_resource_limits() {
    let line_limited = parse_jsonl(
        b"{}\n".as_slice(),
        JsonlLimits {
            max_line_bytes: 2,
            max_total_bytes: 6,
            max_records: 2,
        },
    );
    assert_eq!(
        line_limited.diagnostics.status,
        JsonlReadStatus::ResourceLimited
    );
    assert!(line_limited.entries.is_empty());

    let total_limited = parse_jsonl(
        b"{}\n{}\n".as_slice(),
        JsonlLimits {
            max_line_bytes: 3,
            max_total_bytes: 3,
            max_records: 2,
        },
    );
    assert_eq!(
        total_limited.diagnostics.status,
        JsonlReadStatus::ResourceLimited
    );
    assert_eq!(total_limited.entries, vec![json!({})]);

    let record_limited = parse_jsonl(
        b"{}\n{}\n".as_slice(),
        JsonlLimits {
            max_line_bytes: 3,
            max_total_bytes: 6,
            max_records: 1,
        },
    );
    assert_eq!(
        record_limited.diagnostics.status,
        JsonlReadStatus::ResourceLimited
    );
    assert_eq!(record_limited.entries, vec![json!({})]);
}
