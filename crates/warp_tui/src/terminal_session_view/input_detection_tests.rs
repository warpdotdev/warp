use super::{
    input_detection_decision, should_apply_input_detection, should_reset_input_to_agent,
    InputDetectionDecision,
};

#[test]
fn current_nld_result_is_applied_without_inline_menu() {
    assert!(should_apply_input_detection(
        "git status",
        "git status",
        false
    ));
}

#[test]
fn stale_nld_result_is_ignored() {
    assert!(!should_apply_input_detection("git", "git status", false));
}

#[test]
fn nld_result_is_ignored_while_inline_menu_is_active() {
    assert!(!should_apply_input_detection("/agent", "/agent", true));
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
fn nonempty_input_is_parsed_before_detection() {
    assert_eq!(
        input_detection_decision("cargo", None, 0, false),
        InputDetectionDecision::Parse
    );
}

#[test]
fn short_or_unknown_single_tokens_reset_to_agent() {
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
fn known_commands_and_multi_token_input_are_classified() {
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
