use std::collections::HashMap;
use std::sync::Arc;

use warp_multi_agent_api as api;
use warpui::integration::TestStep;
use warpui::{SingletonEntity, TypedActionView, async_assert_eq};

use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::features::FeatureFlag;
use crate::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use crate::integration_testing::view_getters::{single_terminal_view_for_tab, workspace_view};
use crate::integration_testing::workspace::{
    assert_focused_tab_index, assert_tab_count, trigger_undo_close,
};
use crate::terminal::model::blocks::BlockHeightItem;
use crate::terminal::model::rich_content::RichContentType;
use crate::terminal::view::TerminalAction;
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};
use crate::workspace::{RestoreConversationLayout, WorkspaceAction};

fn restored_conversation(task_id: &str, output: &str) -> AIConversation {
    AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: task_id.to_string(),
            messages: vec![
                api::Message {
                    id: format!("{task_id}-query"),
                    task_id: task_id.to_string(),
                    request_id: format!("{task_id}-request"),
                    message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                        query: "Explain the architecture".to_string(),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                api::Message {
                    id: format!("{task_id}-output"),
                    task_id: task_id.to_string(),
                    request_id: format!("{task_id}-request"),
                    message: Some(api::message::Message::AgentOutput(
                        api::message::AgentOutput {
                            text: output.to_string(),
                        },
                    )),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        None,
    )
    .expect("valid restored conversation fixture")
}

fn assert_visible_conversation(tab_index: usize, conversation_id: AIConversationId) -> TestStep {
    TestStep::new("Verify the visible conversation and its transcript")
        .add_named_assertion(
            "The visible terminal contains the expected conversation's AI block",
            move |app, window_id| {
                let terminal = single_terminal_view_for_tab(app, window_id, tab_index);
                let state = terminal.read(app, |view, ctx| {
                    let has_ai_block = view.model.lock().block_list()
                    .has_visible_block_height_item_where(|item| matches!(
                        item,
                        BlockHeightItem::RichContent(content)
                            if content.content_type == Some(RichContentType::AIBlock)
                                && content.agent_view_conversation_id == Some(conversation_id)
                    ));
                    (view.active_conversation_id(ctx), has_ai_block)
                });
                async_assert_eq!(state, (Some(conversation_id), true))
            },
        )
        .add_named_assertion(
            "The expected tab is focused",
            assert_focused_tab_index(tab_index),
        )
}

pub fn child_pill_after_reopening_closed_parent_tab() -> Vec<TestStep> {
    FeatureFlag::AgentView.set_enabled(true);

    let parent = restored_conversation("orchestrator-task", "Ask the architect for details.");
    let parent_id = parent.id();
    let mut child = restored_conversation("architect-task", "The architecture has three layers.");
    child.set_parent_conversation_id(parent_id);
    child.set_agent_name("architect".to_string());
    let child_id = child.id();

    vec![
        wait_until_bootstrapped_single_pane_for_tab(0),
        TestStep::new("Restore an orchestrator with a hidden child").with_action(
            move |app, window_id, _| {
                let terminal = single_terminal_view_for_tab(app, window_id, 0);
                terminal.update(app, |view, ctx| {
                    let terminal_view_id = ctx.view_id();
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        history.restore_conversations(terminal_view_id, vec![child.clone()], ctx);
                    });
                    view.restore_conversation_after_view_creation(
                        RestoredAIConversation::new(parent.clone()),
                        true,
                        RestoreConversationEntryBehavior::EnterRestoredConversation,
                        ctx,
                    );
                });
            },
        ),
        assert_visible_conversation(0, parent_id),
        TestStep::new("Keep another tab open").with_action(|app, window_id, _| {
            workspace_view(app, window_id).update(app, |workspace, ctx| {
                workspace.add_tab_with_pane_layout(
                    Default::default(),
                    Arc::new(HashMap::new()),
                    None,
                    ctx,
                );
            });
        }),
        wait_until_bootstrapped_single_pane_for_tab(1),
        TestStep::new("Close the original orchestrator tab")
            .with_action(|app, window_id, _| {
                workspace_view(app, window_id).update(app, |workspace, ctx| {
                    workspace.handle_action(&WorkspaceAction::CloseTab(0), ctx);
                });
            })
            .add_named_assertion("Only the spare tab remains open", assert_tab_count(1)),
        TestStep::new("Reopen the parent through conversation navigation").with_action(
            move |app, window_id, _| {
                workspace_view(app, window_id).update(app, |workspace, ctx| {
                    workspace.handle_action(
                        &WorkspaceAction::RestoreOrNavigateToConversation {
                            pane_view_locator: None,
                            window_id: None,
                            conversation_id: parent_id,
                            terminal_view_id: None,
                            restore_layout: Some(RestoreConversationLayout::NewTab),
                        },
                        ctx,
                    );
                });
            },
        ),
        assert_visible_conversation(1, parent_id),
        TestStep::new("Click the architect pill after reopening")
            .with_click_on_saved_position(format!("orchestration-pill-body-{child_id}")),
        assert_visible_conversation(1, child_id),
        TestStep::new("Return to the orchestrator")
            .with_click_on_saved_position(format!("orchestration-pill-body-{parent_id}")),
        assert_visible_conversation(1, parent_id),
        TestStep::new("Open the architect again")
            .with_click_on_saved_position(format!("orchestration-pill-body-{child_id}")),
        assert_visible_conversation(1, child_id),
        trigger_undo_close().add_named_assertion(
            "Undo restores the original tab without discarding the reopened tab",
            assert_tab_count(3),
        ),
        TestStep::new("Navigate to the child from the undo-restored tab")
            .with_click_on_saved_position(format!("orchestration-pill-body-{child_id}")),
        assert_visible_conversation(2, child_id),
        TestStep::new("Reveal the child through the pane-group fallback").with_action(
            move |app, window_id, _| {
                let origin = single_terminal_view_for_tab(app, window_id, 0);
                workspace_view(app, window_id).update(app, |workspace, ctx| {
                    workspace.handle_action(
                        &WorkspaceAction::FocusTerminalViewInWorkspace {
                            terminal_view_id: origin.id(),
                        },
                        ctx,
                    );
                });
                origin.update(app, |view, ctx| {
                    view.handle_action(
                        &TerminalAction::RevealChildAgent {
                            conversation_id: child_id,
                        },
                        ctx,
                    );
                });
            },
        ),
        assert_visible_conversation(2, child_id),
    ]
}
