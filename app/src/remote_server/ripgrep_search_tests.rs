use std::path::PathBuf;

use string_offset::ByteOffset;
use warp_ripgrep::search::{Match as RipgrepMatch, Submatch};

use super::ripgrep_match_to_proto;

fn submatch(start: usize, end: usize) -> Submatch {
    Submatch {
        byte_start: ByteOffset::from(start),
        byte_end: ByteOffset::from(end),
    }
}

#[test]
fn ripgrep_match_to_proto_maps_fields() {
    let m = RipgrepMatch {
        file_path: PathBuf::from("/repo/src/main.rs"),
        line_number: 42,
        line_text: "fn main() {}".to_string(),
        submatches: vec![submatch(3, 7)],
    };

    let proto = ripgrep_match_to_proto(m);

    assert_eq!(proto.file_path, "/repo/src/main.rs");
    assert_eq!(proto.line_number, 42);
    assert_eq!(proto.line_text, "fn main() {}");
    assert_eq!(proto.submatches.len(), 1);
    assert_eq!(proto.submatches[0].byte_start, 3);
    assert_eq!(proto.submatches[0].byte_end, 7);
}

#[test]
fn ripgrep_match_to_proto_preserves_late_submatch_and_full_line() {
    let line = format!("{}needle", "x".repeat(8_000));
    let m = RipgrepMatch {
        file_path: PathBuf::from("/repo/long.rs"),
        line_number: 1,
        line_text: line.clone(),
        submatches: vec![submatch(8_000, 8_006)],
    };

    let proto = ripgrep_match_to_proto(m);

    assert_eq!(proto.line_text, line);
    assert_eq!(proto.submatches[0].byte_start, 8_000);
    assert_eq!(proto.submatches[0].byte_end, 8_006);
}
