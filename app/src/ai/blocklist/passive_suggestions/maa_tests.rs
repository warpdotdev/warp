use std::fs;

use super::{
    MAX_BLOCK_CONTENTS_BYTES_FOR_PATH_DETECTION, detect_relevant_file_paths_for_block,
    head_and_tail_within_budget,
};

#[test]
fn splits_at_char_boundaries_within_budget() {
    let text = "é".repeat(50_000); // 100,000 bytes; every odd byte index is mid-char.
    for max_bytes in [0, 1, 3, 99_999, 100_000, 100_001] {
        let (head, tail) = head_and_tail_within_budget(&text, max_bytes);
        if text.len() <= max_bytes {
            assert_eq!(head, text);
            assert_eq!(tail, None);
        } else {
            // Slicing mid-char would have panicked; verify the budget holds.
            assert!(head.len() + tail.unwrap().len() <= max_bytes);
        }
    }
}

#[test]
fn detects_paths_in_head_and_tail_of_capped_block_contents() {
    let dir = std::env::temp_dir().join(format!("warp_maa_tests_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for file in ["head.rs", "mid.rs", "tail.rs"] {
        fs::write(dir.join(file), "// rust\n").unwrap();
    }
    let cwd = dir.to_string_lossy().to_string();

    // Real paths planted at the start, middle, and end of a block whose size is
    // well over the cap, separated by path-free filler.
    let filler = "filler ".repeat(MAX_BLOCK_CONTENTS_BYTES_FOR_PATH_DETECTION / 7);
    let text =
        format!("error in head.rs:1\n{filler}\nerror in mid.rs:1\n{filler}\nerror in tail.rs:1\n");
    assert!(text.len() > MAX_BLOCK_CONTENTS_BYTES_FOR_PATH_DETECTION);

    let paths = detect_relevant_file_paths_for_block(&text, &cwd, None);
    assert!(paths.iter().any(|path| path.ends_with("head.rs")));
    assert!(paths.iter().any(|path| path.ends_with("tail.rs")));
    // The middle of an over-budget block is not scanned.
    assert!(!paths.iter().any(|path| path.ends_with("mid.rs")));

    let _ = fs::remove_dir_all(&dir);
}
