use std::collections::BTreeMap;
use std::io::{self, BufRead};

use serde_json::Value;

/// Read outcome for one JSONL source.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum JsonlReadStatus {
    #[default]
    /// The source was not available to read.
    Missing,
    /// The source was read to EOF.
    Readable,
    /// Reading stopped because the source returned an I/O error.
    Unreadable,
    /// Reading stopped because a configured resource limit was exceeded.
    ResourceLimited,
}
/// Resource limits applied while parsing one JSONL source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JsonlLimits {
    /// Maximum raw bytes retained for one line, including its newline terminator.
    pub max_line_bytes: usize,
    /// Maximum raw bytes read across the source.
    pub max_total_bytes: usize,
    /// Maximum number of valid records retained.
    pub max_records: usize,
}

/// Diagnostics collected while parsing one JSONL source.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JsonlDiagnostics {
    /// Whether the source was readable.
    pub status: JsonlReadStatus,
    /// Number of valid JSON records retained in the capture.
    pub records_read: usize,
    /// Number of non-empty lines that were not valid JSON.
    pub malformed_records: u32,
    /// Whether the final unterminated line looked like an incomplete JSON record.
    pub incomplete_trailing_record: bool,
}

impl JsonlDiagnostics {
    /// Whether the source was read completely without malformed or truncated records.
    pub fn is_complete(&self) -> bool {
        self.status == JsonlReadStatus::Readable
            && self.malformed_records == 0
            && !self.incomplete_trailing_record
    }
}

/// Read diagnostics for a root history and any discovered subagent histories.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CaptureDiagnostics {
    /// Diagnostics for the root history.
    pub root: JsonlDiagnostics,
    /// Diagnostics keyed by captured subagent scope.
    pub subagents: BTreeMap<String, JsonlDiagnostics>,
    /// Whether discovering all expected subagent sources was incomplete.
    pub subagent_discovery_incomplete: bool,
}
/// Valid JSON records and read diagnostics from one JSONL source.
///
/// A resource-limited capture contains only a prefix and must not be persisted as a complete raw
/// transcript.
#[derive(Debug)]
pub struct JsonlCapture {
    /// JSON values retained from valid, non-empty lines.
    pub entries: Vec<Value>,
    /// Read and parse diagnostics for the source.
    pub diagnostics: JsonlDiagnostics,
}

/// Parse valid JSONL records while preserving evidence of interrupted or malformed input.
///
/// Valid records are returned in source order. Malformed interior lines are skipped and counted,
/// while an incomplete trailing JSON record is reported separately so extractors can degrade
/// coverage instead of treating the source as an empty history. Parsing stops before retaining
/// data beyond `limits`.
pub fn parse_jsonl(mut reader: impl BufRead, limits: JsonlLimits) -> JsonlCapture {
    let mut capture = JsonlCapture {
        entries: Vec::new(),
        diagnostics: JsonlDiagnostics {
            status: JsonlReadStatus::Readable,
            ..Default::default()
        },
    };
    let mut line = Vec::new();
    let mut total_bytes = 0;
    loop {
        line.clear();
        let remaining_bytes = limits.max_total_bytes.saturating_sub(total_bytes);
        match read_line(
            &mut reader,
            &mut line,
            limits.max_line_bytes.min(remaining_bytes),
        ) {
            Ok(ReadLine::Eof) => break,
            Ok(ReadLine::Complete) => total_bytes += line.len(),
            Ok(ReadLine::ResourceLimited) => {
                capture.diagnostics.status = JsonlReadStatus::ResourceLimited;
                break;
            }
            Err(_) => {
                capture.diagnostics.status = JsonlReadStatus::Unreadable;
                break;
            }
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice(&line) {
            Ok(value) => {
                if capture.entries.len() >= limits.max_records {
                    capture.diagnostics.status = JsonlReadStatus::ResourceLimited;
                    break;
                }
                capture.entries.push(value);
            }
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
enum ReadLine {
    Eof,
    Complete,
    ResourceLimited,
}

fn read_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    max_bytes: usize,
) -> io::Result<ReadLine> {
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(if line.is_empty() {
                ReadLine::Eof
            } else {
                ReadLine::Complete
            });
        }
        let end = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |index| index + 1);
        let Some(next_len) = line.len().checked_add(end) else {
            return Ok(ReadLine::ResourceLimited);
        };
        if next_len > max_bytes {
            return Ok(ReadLine::ResourceLimited);
        }
        line.extend_from_slice(&buffer[..end]);
        reader.consume(end);
        if line.ends_with(b"\n") {
            return Ok(ReadLine::Complete);
        }
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
