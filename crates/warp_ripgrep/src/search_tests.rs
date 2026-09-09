use std::fs;

use tempfile::TempDir;

use super::{SEARCHER_MULTILINE_HEAP_LIMIT, search_to_writer};
use crate::types::RipgrepMessage;

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

    let matched_paths: Vec<_> = String::from_utf8(output)
        .unwrap()
        .lines()
        .filter_map(|line| match serde_json::from_str(line).unwrap() {
            RipgrepMessage::Match { data } => Some(data.path.text),
            RipgrepMessage::Begin | RipgrepMessage::End => None,
        })
        .collect();
    assert_eq!(matched_paths, vec![small_path.to_string_lossy()]);
}
