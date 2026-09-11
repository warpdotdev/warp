use std::collections::BTreeMap;
use std::io::BufRead;

use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum JsonlReadStatus {
    #[default]
    Missing,
    Readable,
    Unreadable,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JsonlDiagnostics {
    pub status: JsonlReadStatus,
    pub records_read: usize,
    pub malformed_records: u32,
    pub incomplete_trailing_record: bool,
}

impl JsonlDiagnostics {
    pub fn is_complete(&self) -> bool {
        self.status == JsonlReadStatus::Readable
            && self.malformed_records == 0
            && !self.incomplete_trailing_record
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CaptureDiagnostics {
    pub root: JsonlDiagnostics,
    pub subagents: BTreeMap<String, JsonlDiagnostics>,
    pub subagent_discovery_incomplete: bool,
}

#[derive(Debug)]
pub struct JsonlCapture {
    pub entries: Vec<Value>,
    pub diagnostics: JsonlDiagnostics,
}

/// Capture valid records without interpreting an interrupted read as an empty history.
pub fn parse_jsonl(mut reader: impl BufRead) -> JsonlCapture {
    let mut capture = JsonlCapture {
        entries: Vec::new(),
        diagnostics: JsonlDiagnostics {
            status: JsonlReadStatus::Readable,
            ..Default::default()
        },
    };
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => {
                capture.diagnostics.status = JsonlReadStatus::Unreadable;
                break;
            }
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice(&line) {
            Ok(value) => capture.entries.push(value),
            Err(error) if !line.ends_with(b"\n") && error.is_eof() => {
                capture.diagnostics.incomplete_trailing_record = true;
            }
            Err(_) => {
                capture.diagnostics.malformed_records =
                    capture.diagnostics.malformed_records.saturating_add(1);
            }
        }
    }
    capture.diagnostics.records_read = capture.entries.len();
    capture
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
