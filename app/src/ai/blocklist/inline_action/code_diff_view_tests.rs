use std::collections::HashSet;

use super::*;

fn create_edit(file: &str, content: &str) -> FileEdit {
    FileEdit::Create {
        file: Some(file.to_string()),
        content: Some(content.to_string()),
        allow_overwrite: true,
    }
}

#[test]
fn truncate_to_byte_ceiling_keeps_short_strings_unchanged() {
    assert_eq!(truncate_to_byte_ceiling("hello", 10), "hello");
}

#[test]
fn truncate_to_byte_ceiling_snaps_back_to_a_char_boundary() {
    // "é" is 2 bytes, so a ceiling landing mid-character must snap back to "a".
    assert_eq!(truncate_to_byte_ceiling("aé", 2), "a");
}

#[test]
fn convert_file_edits_to_file_diffs_keeps_all_files_under_the_cap() {
    let edits: Vec<FileEdit> = (0..3)
        .map(|i| create_edit(&format!("/repo/file-{i}.txt"), "content"))
        .collect();

    let diffs = convert_file_edits_to_file_diffs(edits, &None, &None);

    let actual: HashSet<String> = diffs.iter().map(|d| d.file_path()).collect();
    let expected: HashSet<String> = (0..3).map(|i| format!("/repo/file-{i}.txt")).collect();
    assert_eq!(actual, expected);
}

#[test]
fn convert_file_edits_to_file_diffs_keeps_only_the_most_recently_touched_files() {
    let total_files = MAX_RESTORED_FILE_DIFF_FILES + 5;
    let edits: Vec<FileEdit> = (0..total_files)
        .map(|i| create_edit(&format!("/repo/file-{i}.txt"), "content"))
        .collect();

    let diffs = convert_file_edits_to_file_diffs(edits, &None, &None);

    // Only the most recently touched files survive, asserted by the exact path set kept -- not
    // merely the resulting count.
    let actual: HashSet<String> = diffs.iter().map(|d| d.file_path()).collect();
    let expected: HashSet<String> = (5..total_files)
        .map(|i| format!("/repo/file-{i}.txt"))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(diffs.len(), MAX_RESTORED_FILE_DIFF_FILES);
}

/// A file-creation edit is represented as a single `DiffDelta` inserted at the start of the
/// file (`replacement_line_range: 0..0`), wrapped in `DiffType::Update`.
fn sole_create_insertion(diff_type: &DiffType) -> &str {
    let DiffType::Update { deltas, .. } = diff_type else {
        panic!("expected an Update diff");
    };
    let [delta] = deltas.as_slice() else {
        panic!("expected exactly one delta, got {deltas:?}");
    };
    assert_eq!(delta.replacement_line_range, 0..0);
    &delta.insertion
}

#[test]
fn convert_file_edits_to_file_diffs_truncates_create_content_before_cloning() {
    let huge_content = "x".repeat(MAX_RESTORED_FILE_DIFF_CONTENT_BYTES + 100);
    let edits = vec![create_edit("/repo/huge.txt", &huge_content)];

    let diffs = convert_file_edits_to_file_diffs(edits, &None, &None);

    assert_eq!(diffs.len(), 1);
    assert_eq!(
        sole_create_insertion(&diffs[0].diff_type).len(),
        MAX_RESTORED_FILE_DIFF_CONTENT_BYTES
    );
}

#[test]
fn convert_file_edits_to_file_diffs_keeps_small_create_content_unchanged() {
    let content = "fn main() {}\n";
    let edits = vec![create_edit("/repo/small.rs", content)];

    let diffs = convert_file_edits_to_file_diffs(edits, &None, &None);

    assert_eq!(diffs.len(), 1);
    assert_eq!(sole_create_insertion(&diffs[0].diff_type), content);
}

fn v4a_hunk_with_pre_context(pre_context: String) -> V4AHunk {
    V4AHunk {
        change_context: vec![],
        pre_context,
        old: String::new(),
        new: String::new(),
        post_context: String::new(),
    }
}

#[test]
fn convert_file_edits_to_file_diffs_bounds_v4a_hunk_content_before_cloning() {
    // Three hunks, each half the byte ceiling: the first two exhaust the budget, so the third
    // must never be cloned or contribute to the reconstructed content.
    let hunk_size = MAX_RESTORED_FILE_DIFF_CONTENT_BYTES / 2;
    let hunks = vec![
        v4a_hunk_with_pre_context("a".repeat(hunk_size)),
        v4a_hunk_with_pre_context("b".repeat(hunk_size)),
        v4a_hunk_with_pre_context("c".repeat(hunk_size)),
    ];
    let edits = vec![FileEdit::Edit(ParsedDiff::V4AEdit {
        file: Some("/repo/big.rs".to_string()),
        move_to: None,
        hunks,
    })];

    let diffs = convert_file_edits_to_file_diffs(edits, &None, &None);

    assert_eq!(diffs.len(), 1);
    // Bounded to roughly two hunks' worth, not the full three -- the third hunk's 'c's never
    // made it into the reconstructed content.
    assert!(!diffs[0].base.content.contains('c'));
    assert!(diffs[0].base.content.len() < 3 * hunk_size);
}

#[test]
fn convert_file_edits_to_file_diffs_skips_str_replace_edit_when_replace_is_oversized() {
    // `search` is tiny; `replace` alone exceeds the ceiling. The budget must be checked against
    // both fields, not just the one that happens to feed the dummy content.
    let huge_replace = "y".repeat(MAX_RESTORED_FILE_DIFF_CONTENT_BYTES + 100);
    let edits = vec![FileEdit::Edit(ParsedDiff::StrReplaceEdit {
        file: Some("/repo/file.rs".to_string()),
        search: Some("1|small search\n".to_string()),
        replace: Some(huge_replace),
    })];

    let diffs = convert_file_edits_to_file_diffs(edits, &None, &None);

    assert_eq!(diffs.len(), 1);
    assert!(diffs[0].base.content.is_empty());
    let DiffType::Update { deltas, .. } = &diffs[0].diff_type else {
        panic!("expected an Update diff");
    };
    assert!(deltas.is_empty());
}

#[test]
fn convert_file_edits_to_file_diffs_skips_v4a_hunk_when_new_field_is_oversized() {
    // `pre_context`, `old`, and `post_context` are tiny; `new` alone exceeds the ceiling. The
    // budget must account for `new` even though it never feeds the dummy content.
    let hunk = V4AHunk {
        change_context: vec![],
        pre_context: "small".to_string(),
        old: "small".to_string(),
        new: "y".repeat(MAX_RESTORED_FILE_DIFF_CONTENT_BYTES + 100),
        post_context: "small".to_string(),
    };
    let edits = vec![FileEdit::Edit(ParsedDiff::V4AEdit {
        file: Some("/repo/file.rs".to_string()),
        move_to: None,
        hunks: vec![hunk],
    })];

    let diffs = convert_file_edits_to_file_diffs(edits, &None, &None);

    assert_eq!(diffs.len(), 1);
    assert!(diffs[0].base.content.is_empty());
}

#[test]
fn convert_file_edits_to_file_diffs_treats_a_refreshed_file_as_recently_touched() {
    // file-0 is touched first, then touched again last. A second, later touch must count as
    // recent -- it must not be evicted as if it were still sitting at its first-touch position.
    let mut edits: Vec<FileEdit> = vec![create_edit("/repo/file-0.txt", "first touch")];
    edits.extend(
        (1..MAX_RESTORED_FILE_DIFF_FILES)
            .map(|i| create_edit(&format!("/repo/file-{i}.txt"), "content")),
    );
    // One more distinct file, so the cap must drop exactly one file.
    edits.push(create_edit("/repo/file-new.txt", "content"));
    edits.push(create_edit("/repo/file-0.txt", "second touch"));

    let diffs = convert_file_edits_to_file_diffs(edits, &None, &None);

    let actual: HashSet<String> = diffs.iter().map(|d| d.file_path()).collect();
    assert_eq!(diffs.len(), MAX_RESTORED_FILE_DIFF_FILES);
    assert!(
        actual.contains("/repo/file-0.txt"),
        "the re-touched file should survive the cap"
    );
    assert!(
        !actual.contains("/repo/file-1.txt"),
        "file-1 was never refreshed and is now the oldest, so it should be dropped"
    );
}
