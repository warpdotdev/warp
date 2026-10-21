use ai::hooks::test_util::{test_binding, test_source_id};
use ai::hooks::{HookBoundary, HookPayload};
use warpui::App;

use super::*;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::hooks::testing::{hook_test_app, settle};
use crate::features::FeatureFlag;

#[test]
fn creating_a_native_conversation_fires_session_start_once() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| {
            harness.hooks.install_bindings(
                vec![test_binding(
                    test_source_id("s"),
                    HookBoundary::SessionStart,
                    0,
                    None,
                )],
                ctx,
            )
        });

        let new_conversation_id = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
                model.start_new_conversation(harness.terminal_view_id, false, false, false, ctx)
            })
        });
        settle(&mut app, |_| harness.hooks.adapter.encoded_count() == 1).await;

        let envelope = &harness.hooks.adapter.encoded_envelopes()[0];
        assert_eq!(envelope.boundary(), HookBoundary::SessionStart);
        assert_eq!(envelope.conversation_id, new_conversation_id);
        assert!(matches!(
            envelope.payload,
            HookPayload::SessionStart {
                is_background_agent: false
            }
        ));
    });
}

#[test]
fn viewer_and_transcript_conversations_do_not_fire_session_start() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| {
            harness.hooks.install_bindings(
                vec![test_binding(
                    test_source_id("s"),
                    HookBoundary::SessionStart,
                    0,
                    None,
                )],
                ctx,
            )
        });
        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
                model.start_new_conversation(harness.terminal_view_id, false, true, false, ctx);
                model.start_new_conversation(harness.terminal_view_id, false, false, true, ctx);
            })
        });
        settle(&mut app, |_| true).await;
        assert_eq!(harness.hooks.adapter.encoded_count(), 0);
    });
}
