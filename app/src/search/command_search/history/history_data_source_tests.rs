use std::sync::Arc;

use super::*;
use crate::terminal::HistoryEntry;
use crate::terminal::model::session::SessionId;

#[test]
fn truncate_for_fuzzy_match_respects_char_boundaries() {
    let text = "日本語"; // 3 chars, each 3 bytes in UTF-8.
    assert_eq!(truncate_for_fuzzy_match(text, 10), text);
    assert_eq!(truncate_for_fuzzy_match(text, 2), "日本");
    assert_eq!(truncate_for_fuzzy_match(text, 0), "");
}

#[test]
fn matched_indices_from_truncated_text_stay_valid_against_the_full_text() {
    // A multi-byte prefix ahead of the match makes char and byte offsets diverge: if any layer
    // in the path (the matcher or the truncation) treated the returned indices as byte offsets
    // instead of char offsets, indexing `full_chars` with them would land on the wrong
    // characters here, where an all-ASCII command could not have caught that.
    let full_command = format!("日本語target{}", "x".repeat(MAX_FUZZY_MATCH_CHARS + 1_000));
    let truncated = truncate_for_fuzzy_match(&full_command, MAX_FUZZY_MATCH_CHARS);
    assert!(truncated.len() < full_command.len());

    let result = fuzzy_match::match_indices_case_insensitive(truncated, "target")
        .expect("query should match within the truncated prefix");

    // The indices came from the truncated text, but HistorySearchItem highlights them against
    // the untruncated `entry.command` -- so they must still land on the right characters there.
    let full_chars: Vec<char> = full_command.chars().collect();
    let matched_text: String = result
        .matched_indices
        .iter()
        .map(|&idx| full_chars[idx])
        .collect();
    assert_eq!(matched_text, "target");
}

#[test]
fn legacy_history_search_only_finds_matches_within_the_truncation_cutoff() {
    let filler = "x".repeat(MAX_FUZZY_MATCH_CHARS + 1_000);
    let match_near_start = format!("target{filler}");
    let match_past_cutoff = format!("{filler}target");

    let snapshot = HistorySnapshot {
        commands: Arc::new([
            Arc::new(HistoryEntry::command_only(match_near_start.clone())),
            Arc::new(HistoryEntry::command_only(match_past_cutoff.clone())),
        ]),
        query_text: "target".to_owned(),
        current_session_id: SessionId::from(0),
    };

    let results = futures_lite::future::block_on(fuzzy_match_history_legacy(snapshot)).unwrap();

    let matched_commands: Vec<String> = results
        .into_iter()
        .map(|result| match result.accept_result() {
            CommandSearchItemAction::AcceptHistory(item) => item.command,
            other => panic!("expected AcceptHistory, got {other:?}"),
        })
        .collect();

    assert!(matched_commands.contains(&match_near_start));
    assert!(
        !matched_commands.contains(&match_past_cutoff),
        "a match past MAX_FUZZY_MATCH_CHARS should not be found: {matched_commands:?}"
    );
}
