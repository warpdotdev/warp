use ai::hooks::test_util::{test_binding, test_source_id};
use ai::hooks::{HookBoundary, HookPayload};
use warpui::{App, SingletonEntity};

use super::*;
use crate::ai::agent::conversation::AIConversation;
use crate::ai::blocklist::hooks::testing::{hook_test_app, settle};
use crate::features::FeatureFlag;

#[test]
fn prompt_submit_reaches_the_adapter_with_the_text_and_conversation() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| {
            harness.hooks.install_bindings(
                vec![test_binding(
                    test_source_id("s"),
                    HookBoundary::PromptSubmit,
                    0,
                    None,
                )],
                ctx,
            )
        });

        app.update(|ctx| {
            notify_prompt_submit(
                "fix the build",
                vec!["file.rs".to_owned()],
                harness.conversation_id,
                &harness.active_session,
                ctx,
            )
        });
        settle(&mut app, |_| harness.hooks.adapter.encoded_count() == 1).await;

        let envelope = &harness.hooks.adapter.encoded_envelopes()[0];
        assert_eq!(envelope.boundary(), HookBoundary::PromptSubmit);
        assert_eq!(envelope.conversation_id, harness.conversation_id);
        let HookPayload::PromptSubmit {
            text,
            attachments_summary,
        } = &envelope.payload
        else {
            panic!("expected a prompt.submit payload");
        };
        assert_eq!(text, "fix the build");
        assert_eq!(attachments_summary, &["file.rs".to_owned()]);
        assert_eq!(harness.hooks.runner.invocation_count(), 1);
    });
}

#[test]
fn prompt_submit_does_not_fire_for_a_shared_session_viewer() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| {
            harness.hooks.install_bindings(
                vec![test_binding(
                    test_source_id("s"),
                    HookBoundary::PromptSubmit,
                    0,
                    None,
                )],
                ctx,
            )
        });
        let viewer = AIConversation::new(true, false);
        let viewer_id = viewer.id();
        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
                model.restore_conversations(harness.terminal_view_id, vec![viewer], ctx);
            });
            notify_prompt_submit(
                "mirrored",
                Vec::new(),
                viewer_id,
                &harness.active_session,
                ctx,
            );
            notify_prompt_submit(
                "unknown conversation",
                Vec::new(),
                AIConversationId::new(),
                &harness.active_session,
                ctx,
            );
        });
        settle(&mut app, |_| true).await;
        assert_eq!(harness.hooks.adapter.encoded_count(), 0);
    });
}
