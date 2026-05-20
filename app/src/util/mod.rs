pub mod bindings;
pub mod clipboard;
pub mod color;
pub mod extensions;
#[cfg(feature = "local_fs")]
pub mod file;
pub mod git;
pub mod image;
pub(crate) mod link_detection;
pub mod links;
pub mod openable_file_type;
#[cfg(feature = "local_tty")]
pub mod path;
pub mod repo_detection;
pub mod time_format;
pub mod tooltips;
pub(crate) mod traffic_lights;
pub(crate) mod truncation;
pub mod vm_detection;
#[cfg(windows)]
pub mod windows;

use itertools::Itertools;
use std::cmp::Ordering;
use std::fmt;
use std::ops::Range;

pub fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut i = 1;
    while i < ranges.len() {
        if ranges[i - 1].end.cmp(&ranges[i].start) >= Ordering::Equal {
            let removed = ranges.remove(i);
            if removed.start.cmp(&ranges[i - 1].start) < Ordering::Equal {
                ranges[i - 1].start = removed.start;
            }
            if removed.end.cmp(&ranges[i - 1].end) > Ordering::Equal {
                ranges[i - 1].end = removed.end;
            }
        } else {
            i += 1;
        }
    }
    ranges
}

pub fn dedupe_from_last(lines: Vec<String>) -> Vec<String> {
    let mut unique_elements = lines.into_iter().rev().unique().collect::<Vec<_>>();
    unique_elements.reverse();
    unique_elements
}

pub fn parse_ascii_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }

    let mut result: u32 = 0;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((byte - b'0') as u32)?;
    }
    Some(result)
}

/// AsciiDebug is intended to make it easy to inspect the contents of byte slices that are mostly ASCII
/// characters (but may not be valid unicode). It changes the output of the wrapped byte slice to
/// a human readable string with non-ASCII characters written as hex escapes.
///
/// E.g. `log::info!("{:?}", &AsciiDebug(some_byte_slice));`
pub struct AsciiDebug<'a>(pub &'a [u8]);

impl fmt::Debug for AsciiDebug<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"")?;
        for &byte in self.0 {
            // Check if the byte is a standard printable character.
            if (32..126).contains(&byte) {
                write!(f, "{}", byte as char)?;
            } else {
                write!(f, "\\{{{byte:02X}}}")?;
            }
        }
        write!(f, "\"")?;
        Ok(())
    }
}

#[test]
fn test_dedupe() {
    let history_lines = vec![
        "1".to_string(),
        "3".to_string(),
        "2".to_string(),
        "1".to_string(),
    ];
    assert_eq!(
        dedupe_from_last(history_lines),
        vec!["3".to_string(), "2".to_string(), "1".to_string()]
    );
}

#[test]
fn test_parse_ascii_u32() {
    assert_eq!(parse_ascii_u32(b"123"), Some(123));
    assert_eq!(parse_ascii_u32(b"0"), Some(0));
    assert_eq!(parse_ascii_u32(b"4294967295"), Some(4294967295)); // Max u32
    assert_eq!(parse_ascii_u32(b"4294967296"), None); // Overflow
    assert_eq!(parse_ascii_u32(b""), None);
    assert_eq!(parse_ascii_u32(b"12a3"), None);
}
