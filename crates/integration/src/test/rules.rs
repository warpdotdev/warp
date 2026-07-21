use warp::integration_testing::rules::{
    assert_rule_count, assert_rule_exists, assert_rule_pane_open, create_a_personal_rule,
    open_rule_pane, update_rule_content,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::window::save_active_window_id;

use super::{Builder, new_builder};

/// Test creating a rule
pub fn test_rule_creation() -> Builder {
    let key = "rule";
    let rule_content = "Never use unwrap in Rust.";
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            create_a_personal_rule(key, "Rust", rule_content).add_assertion(assert_rule_count(1)),
        )
        .with_step(
            new_step_with_default_assertions("Verify rule content")
                .add_named_assertion_with_data_from_prior_step(
                    "Check rule exists with correct content",
                    assert_rule_exists(key, rule_content),
                ),
        )
}

/// Test updating a rule's content
pub fn test_rule_update() -> Builder {
    let key = "rule";
    let rule_content = "Old rule content";
    let new_rule_content = "New rule content";
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            create_a_personal_rule(key, "Test Rule", rule_content)
                .add_assertion(assert_rule_count(1)),
        )
        .with_step(
            new_step_with_default_assertions("Verify original content")
                .add_named_assertion_with_data_from_prior_step(
                    "Check original rule content",
                    assert_rule_exists(key, rule_content),
                ),
        )
        .with_step(update_rule_content(key, new_rule_content))
        .with_step(
            new_step_with_default_assertions("Verify updated content")
                .add_named_assertion_with_data_from_prior_step(
                    "Check updated rule content",
                    assert_rule_exists(key, new_rule_content),
                ),
        )
}

// Test opening rule pane at the correct rule
pub fn test_rule_pane_opening() -> Builder {
    let key = "rule";
    let window_id = "main_window";
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            create_a_personal_rule(
                key,
                "Pane Test Rule",
                "This rule is for testing pane opening",
            )
            .add_assertion(assert_rule_count(1))
            .add_assertion(save_active_window_id(window_id)),
        )
        .with_step(
            create_a_personal_rule(
                "rule_2",
                "Pane Test Rule #2",
                "This rule is for testing pane opening #2",
            )
            .add_assertion(assert_rule_count(2)),
        )
        .with_step(
            open_rule_pane(window_id, key).add_named_assertion_with_data_from_prior_step(
                "Check rule pane opens",
                assert_rule_pane_open(key),
            ),
        )
}
