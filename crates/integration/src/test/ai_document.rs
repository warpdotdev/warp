use warp::integration_testing::ai_document::{
    assert_ai_document_content_scrolled_after_header, assert_ai_document_has_scroll_header,
    assert_ai_document_header_at_top_with_content_at_top,
    assert_ai_document_header_partially_hidden_before_content_scroll, create_ai_document,
    open_ai_document, scroll_ai_document_by, set_orchestration_config_for_ai_document,
};
use warp::integration_testing::pane_group::assert_num_panes_in_tab;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;

use super::{new_builder, Builder};

const AI_DOCUMENT_KEY: &str = "ai document";

/// Builds enough Markdown to make the AI document editor scrollable.
fn long_plan_content() -> String {
    let mut content = String::from("# Plan\n\n");
    for i in 0..200 {
        content.push_str(&format!(
            "## Section {i}\n\nThis section makes the plan long enough to scroll.\n\n"
        ));
    }
    content
}

/// Verifies orchestration config chrome scrolls with AI document content.
pub fn test_ai_document_orchestration_config_header_scrolls_with_content() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(create_ai_document(
            AI_DOCUMENT_KEY,
            "Scrollable Plan",
            long_plan_content(),
        ))
        .with_step(open_ai_document(AI_DOCUMENT_KEY).add_assertion(assert_num_panes_in_tab(0, 2)))
        .with_step(
            warpui_core::integration::TestStep::new(
                "Assert AI document opens without orchestration header",
            )
            .add_assertion(assert_ai_document_has_scroll_header(false)),
        )
        .with_step(set_orchestration_config_for_ai_document(AI_DOCUMENT_KEY))
        .with_step(
            warpui_core::integration::TestStep::new("Assert orchestration header starts at top")
                .add_assertion(assert_ai_document_header_at_top_with_content_at_top()),
        )
        .with_step(
            scroll_ai_document_by(-20.)
                .add_assertion(assert_ai_document_header_partially_hidden_before_content_scroll()),
        )
        .with_step(
            scroll_ai_document_by(-800.)
                .add_assertion(assert_ai_document_content_scrolled_after_header()),
        )
}
