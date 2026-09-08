use super::{
    InputDetectionDecision, input_detection_decision, parsed_result_is_applicable,
    should_reset_input_to_agent,
};

// These tests exercise only synchronous TUI coordinator decisions; they never invoke the NLD
// classifier and are therefore deterministic. Shared classification behavior is covered by
// `crates/input_classifier/src/heuristic_classifier/mod_tests.rs`, while GUI input-mode behavior
// and shared model transitions are covered by `app/src/terminal/input_tests.rs` and
// `app/src/ai/blocklist/input_model_tests.rs`.

#[test]
fn current_parse_result_is_applicable_without_inline_menu() {
    assert!(parsed_result_is_applicable(
        "git status",
        "git status",
        false
    ));
}

#[test]
fn stale_parse_result_is_not_applicable() {
    assert!(!parsed_result_is_applicable("git", "git status", false));
}

#[test]
fn parse_result_is_not_applicable_while_inline_menu_is_active() {
    assert!(!parsed_result_is_applicable("/agent", "/agent", true));
}

#[test]
fn empty_input_resets_to_agent() {
    assert_eq!(
        input_detection_decision("", None, 0, false),
        InputDetectionDecision::ResetToAgent
    );
    assert_eq!(
        input_detection_decision("  \n", None, 0, false),
        InputDetectionDecision::ResetToAgent
    );
}

#[test]
fn unrecognized_slash_prefix_is_parsed_as_possible_shell_input() {
    assert_eq!(
        input_detection_decision("/usr/bin/env", None, 0, false),
        InputDetectionDecision::Parse
    );
    assert_eq!(
        input_detection_decision("/usr/bin/env", Some(1), 12, true),
        InputDetectionDecision::Classify
    );
}

#[test]
fn nonempty_input_is_parsed_before_detection() {
    assert_eq!(
        input_detection_decision("cargo", None, 0, false),
        InputDetectionDecision::Parse
    );
}

#[test]
fn decision_resets_short_or_unknown_single_tokens_to_agent() {
    assert_eq!(
        input_detection_decision("w", Some(1), 1, true),
        InputDetectionDecision::ResetToAgent
    );
    assert_eq!(
        input_detection_decision("fi", Some(1), 2, false),
        InputDetectionDecision::ResetToAgent
    );
    assert_eq!(
        input_detection_decision("fix", Some(1), 3, false),
        InputDetectionDecision::ResetToAgent
    );
}

#[test]
fn decision_routes_known_commands_and_multi_token_input_to_classifier() {
    assert_eq!(
        input_detection_decision("ls", Some(1), 2, true),
        InputDetectionDecision::Classify
    );
    assert_eq!(
        input_detection_decision("cargo", Some(1), 5, true),
        InputDetectionDecision::Classify
    );
    assert_eq!(
        input_detection_decision("git status", Some(2), 3, true),
        InputDetectionDecision::Classify
    );
    assert_eq!(
        input_detection_decision("fix this", Some(2), 3, false),
        InputDetectionDecision::Classify
    );
}

#[test]
fn only_unlocked_reset_decisions_change_to_agent() {
    assert!(should_reset_input_to_agent(
        InputDetectionDecision::ResetToAgent,
        false
    ));
    assert!(!should_reset_input_to_agent(
        InputDetectionDecision::Classify,
        false
    ));
    assert!(!should_reset_input_to_agent(
        InputDetectionDecision::ResetToAgent,
        true
    ));
}
