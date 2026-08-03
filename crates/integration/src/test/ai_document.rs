use std::time::Duration;

use warp::integration_testing::ai_document::{
    ai_document_overflow_button_position_id, assert_ai_document_overflow_button_position_exists,
    create_and_open_ai_document,
};
use warp::integration_testing::clipboard::assert_clipboard_contains_string;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;

use super::new_builder;
use crate::Builder;

const PLAN_MARKDOWN: &str = "# Migration Plan\n\n## Steps\n\n1. Audit the call sites\n   - Inventory each module\n   - Note breaking changes\n2. Land the refactor\n3. Verify with `cargo test`\n\n```rust path=null start=null\nfn migrate() {\n    println!(\"done\");\n}\n```";

const EXPECTED_CLIPBOARD: &str = "# Migration Plan\n\n## Steps\n\n1. Audit the call sites\n    * Inventory each module\n    * Note breaking changes\n2. Land the refactor\n3. Verify with `cargo test`\n\n```rust\nfn migrate() {\n    println!(\"done\");\n}\n```\n";

pub fn test_copy_ai_document_as_markdown_from_overflow_menu() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(create_and_open_ai_document("Migration Plan", PLAN_MARKDOWN))
        .with_step(
            new_step_with_default_assertions("Wait for AI document overflow menu button")
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_ai_document_overflow_button_position_exists()),
        )
        .with_step(
            new_step_with_default_assertions("Open AI document overflow menu")
                .with_click_on_saved_position_fn(ai_document_overflow_button_position_id),
        )
        .with_step(
            new_step_with_default_assertions("Copy AI document as Markdown")
                .with_click_on_saved_position("Copy as Markdown")
                .add_assertion(assert_clipboard_contains_string(
                    EXPECTED_CLIPBOARD.to_string(),
                )),
        )
}
