//! Unit tests for [`resolve_history_match`], pinning down the NLD history-match
//! decision matrix between command history and agent prompt history.
//!
//! Each `Option<Option<DateTime<Local>>>` argument models one history source:
//! the outer `Option` is whether that source had a close match, and the inner
//! `Option` is that match's timestamp (command-history-file entries may have no
//! timestamp; agent prompt entries always carry one).

use chrono::Duration;

use super::*;

/// Returns a timestamp and a strictly-later timestamp, for ordering assertions.
fn earlier_and_later() -> (DateTime<Local>, DateTime<Local>) {
    let earlier = Local::now();
    let later = earlier + Duration::seconds(1);
    (earlier, later)
}

const HISTORY_MATCH_AI: Option<(InputType, InputTypeAutoDetectionSource)> =
    Some((InputType::AI, InputTypeAutoDetectionSource::HistoryMatch));
const HISTORY_MATCH_SHELL: Option<(InputType, InputTypeAutoDetectionSource)> =
    Some((InputType::Shell, InputTypeAutoDetectionSource::HistoryMatch));

#[test]
fn no_match_from_either_source_is_not_history_match() {
    // Neither command nor prompt history matched: the caller must fall through
    // to the classifier, so we cannot report a `HistoryMatch` decision.
    assert_eq!(resolve_history_match(None, None), None);
}

#[test]
fn prompt_only_match_locks_to_ai_history_match() {
    let (_, prompt_ts) = earlier_and_later();
    assert_eq!(
        resolve_history_match(None, Some(Some(prompt_ts))),
        HISTORY_MATCH_AI,
    );
}

#[test]
fn command_only_match_locks_to_shell_history_match() {
    let (command_ts, _) = earlier_and_later();
    assert_eq!(
        resolve_history_match(Some(Some(command_ts)), None),
        HISTORY_MATCH_SHELL,
    );
}

#[test]
fn command_only_match_without_timestamp_locks_to_shell_history_match() {
    // History-file commands can match without carrying a timestamp.
    assert_eq!(resolve_history_match(Some(None), None), HISTORY_MATCH_SHELL);
}

#[test]
fn both_match_prompt_newer_locks_to_ai() {
    let (command_ts, prompt_ts) = earlier_and_later();
    assert_eq!(
        resolve_history_match(Some(Some(command_ts)), Some(Some(prompt_ts))),
        HISTORY_MATCH_AI,
    );
}

#[test]
fn both_match_command_newer_locks_to_shell() {
    let (prompt_ts, command_ts) = earlier_and_later();
    assert_eq!(
        resolve_history_match(Some(Some(command_ts)), Some(Some(prompt_ts))),
        HISTORY_MATCH_SHELL,
    );
}

#[test]
fn both_match_equal_timestamps_prefer_shell() {
    // The newer-wins check is strict, so a tie cannot prove the prompt is more
    // recent and we preserve the Shell short-circuit.
    let ts = Local::now();
    assert_eq!(
        resolve_history_match(Some(Some(ts)), Some(Some(ts))),
        HISTORY_MATCH_SHELL,
    );
}

#[test]
fn both_match_command_without_timestamp_prefer_shell() {
    // Cannot prove the prompt is newer when the command carries no timestamp.
    let (_, prompt_ts) = earlier_and_later();
    assert_eq!(
        resolve_history_match(Some(None), Some(Some(prompt_ts))),
        HISTORY_MATCH_SHELL,
    );
}
