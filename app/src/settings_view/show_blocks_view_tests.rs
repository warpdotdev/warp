use chrono::DateTime;

use super::{UnshareBlockRequestState, UserOwnedBlock, visible_block_indices};

fn block(id: &str, command: &str) -> UserOwnedBlock {
    UserOwnedBlock::new(
        id,
        command,
        DateTime::parse_from_rfc3339("2026-01-01T12:00:00+00:00").unwrap(),
    )
}

#[test]
fn search_matches_empty_and_whitespace_queries() {
    let block = block("ABC123", "git status");

    assert!(block.matches_search_query(""));
    assert!(block.matches_search_query("   "));
}

#[test]
fn search_matches_command_case_insensitively() {
    let block = block("ABC123", "Git Status");

    assert!(block.matches_search_query("  STATUS  "));
    assert!(block.matches_search_query("git"));
}

#[test]
fn search_matches_displayed_url_case_insensitively() {
    let legacy_block = block("ABC123", "ls");
    let new_block = block("0123456789012345678901", "pwd");

    assert!(legacy_block.matches_search_query("abc123"));
    assert!(new_block.matches_search_query("BLOCK/0123456789012345678901"));
}

#[test]
fn search_does_not_match_unrelated_text() {
    let block = block("ABC123", "git status");

    assert!(!block.matches_search_query("deployment"));
}

#[test]
fn filtered_rows_keep_original_indices_for_actions() {
    let mut blocks = vec![
        block("first1", "git status"),
        block("second", "git log"),
        block("third3", "cargo test"),
    ];

    assert_eq!(visible_block_indices(&blocks, "cargo"), vec![2]);
    assert_eq!(blocks[visible_block_indices(&blocks, "cargo")[0]].id, "third3");

    blocks[1].unshare_request_status = UnshareBlockRequestState::Done;
    assert_eq!(visible_block_indices(&blocks, ""), vec![0, 2]);
    assert_eq!(visible_block_indices(&blocks, "git log"), Vec::<usize>::new());
}
