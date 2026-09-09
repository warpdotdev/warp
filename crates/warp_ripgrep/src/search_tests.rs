use std::fs;
use std::io::{self, Write};

use tempfile::TempDir;

use super::{SEARCHER_MULTILINE_HEAP_LIMIT, search_to_writer};
use crate::types::RipgrepMessage;
#[derive(Default)]
struct WriteStats {
    total_bytes: usize,
    largest_write: usize,
}

impl Write for WriteStats {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.total_bytes += buf.len();
        self.largest_write = self.largest_write.max(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn multiline_search_streams_many_matches_without_buffering_the_file_output() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("many-matches.txt");
    fs::write(&path, "alpha\nbeta\n".repeat(20_000)).unwrap();

    let stats = search_to_writer(
        &["alpha\nbeta".to_string()],
        vec![temp_dir.path().to_path_buf()],
        false,
        true,
        WriteStats::default(),
    )
    .unwrap();

    assert!(stats.total_bytes > 1024 * 1024);
    assert!(stats.largest_write < 64 * 1024);
}

#[test]
fn multiline_search_skips_files_above_the_heap_limit() {
    let temp_dir = TempDir::new().unwrap();
    let small_path = temp_dir.path().join("small.txt");
    fs::write(&small_path, "alpha\nbeta\n").unwrap();

    let large_path = temp_dir.path().join("large.txt");
    let mut large_contents = b"alpha\nbeta\n".to_vec();
    large_contents.resize(SEARCHER_MULTILINE_HEAP_LIMIT + 1, b'x');
    fs::write(&large_path, large_contents).unwrap();

    let output = search_to_writer(
        &["alpha\nbeta".to_string()],
        vec![temp_dir.path().to_path_buf()],
        false,
        true,
        Vec::new(),
    )
    .unwrap();

    let mut matched_paths = Vec::new();
    let mut limit_reached_count = 0;
    for line in String::from_utf8(output).unwrap().lines() {
        match serde_json::from_str(line).unwrap() {
            RipgrepMessage::Match { data } => matched_paths.push(data.path.text),
            RipgrepMessage::LimitReached => limit_reached_count += 1,
            RipgrepMessage::Begin | RipgrepMessage::End => {}
        }
    }
    assert_eq!(matched_paths, vec![small_path.to_string_lossy()]);
    assert_eq!(limit_reached_count, 1);
}
