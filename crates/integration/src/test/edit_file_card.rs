//! Manual integration test that renders the Agent Mode edit-file card for a
//! single markdown file creation and captures the Rendered/Raw toggle + Open
//! button (both `EditFileCardEnhancements` additions) in Raw and Rendered modes.
//!
//! Run manually with a real display:
//!
//! ```sh
//! WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
//!   cargo run -p integration --bin integration -- test_edit_file_card_enhancements
//! ```
use std::time::Duration;

use warp::features::FeatureFlag;
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::workspace::Workspace;
use warpui_core::async_assert;
use warpui_core::integration::TestStep;
use warpui_core::ViewHandle;

use super::new_builder;
use crate::Builder;

const DEMO_MARKDOWN: &str = concat!(
    "# Demo Feature\n\n",
    "This markdown file was **created** by the agent.\n\n",
    "## Highlights\n\n",
    "- Rendered/Raw toggle in the footer\n",
    "- `Open` button in the header\n\n",
    "```rust\n",
    "fn main() {\n",
    "    println!(\"hello, world\");\n",
    "}\n",
    "```\n",
);

fn active_workspace(
    app: &warpui_core::App,
    window_id: warpui_core::WindowId,
) -> ViewHandle<Workspace> {
    let workspace_views: Vec<ViewHandle<Workspace>> =
        app.views_of_type(window_id).expect("Workspace must exist");
    workspace_views
        .first()
        .expect("Workspace must exist")
        .clone()
}

pub fn test_edit_file_card_enhancements() -> Builder {
    FeatureFlag::EditFileCardEnhancements.set_enabled(true);

    new_builder()
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(
            TestStep::new("Open dummy edit-file card").with_action(|app, window_id, _| {
                FeatureFlag::EditFileCardEnhancements.set_enabled(true);
                let workspace = active_workspace(app, window_id);
                workspace.update(app, |workspace, ctx| {
                    workspace.open_dummy_edit_file_card(
                        "/workspace/DEMO_FEATURE.md".to_string(),
                        DEMO_MARKDOWN.to_string(),
                        ctx,
                    );
                });
            }),
        )
        .with_step(
            TestStep::new("Assert card shows enhancements").add_assertion(|app, window_id| {
                let workspace = active_workspace(app, window_id);
                let state = workspace.read(app, |workspace, ctx| {
                    workspace.dummy_edit_file_card_gate_state(ctx)
                });
                async_assert!(
                    state
                        .as_deref()
                        .is_some_and(|s| s.contains("toggle=true") && s.contains("open=true")),
                    "edit-file card gate state: {state:?}"
                )
            }),
        )
        .with_step(TestStep::new("Start recording").with_start_recording())
        .with_step(
            TestStep::new("Screenshot: Raw mode")
                .set_post_step_pause(Duration::from_secs(2))
                .with_take_screenshot("edit_file_card_raw.png"),
        )
        .with_step(
            TestStep::new("Switch to Rendered mode").with_action(|app, window_id, _| {
                let workspace = active_workspace(app, window_id);
                workspace.update(app, |workspace, ctx| {
                    workspace.set_dummy_edit_file_card_rendered(true, ctx);
                });
            }),
        )
        .with_step(
            TestStep::new("Screenshot: Rendered mode")
                .set_post_step_pause(Duration::from_secs(2))
                .with_take_screenshot("edit_file_card_rendered.png"),
        )
        .with_step(
            TestStep::new("Switch back to Raw mode").with_action(|app, window_id, _| {
                let workspace = active_workspace(app, window_id);
                workspace.update(app, |workspace, ctx| {
                    workspace.set_dummy_edit_file_card_rendered(false, ctx);
                });
            }),
        )
        .with_step(
            TestStep::new("Screenshot: Raw mode again")
                .set_post_step_pause(Duration::from_secs(1))
                .with_take_screenshot("edit_file_card_raw_again.png"),
        )
        .with_step(TestStep::new("Stop recording").with_stop_recording())
}
