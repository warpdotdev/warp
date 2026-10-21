use ai::hooks::test_util::{test_binding, test_source_id};
use ai::hooks::{HookBoundary, HookPayload};
use warpui::App;

use super::*;
use crate::ai::blocklist::hooks::testing::{hook_test_app, settle};
use crate::features::FeatureFlag;

#[test]
fn turn_stop_reaches_the_adapter_with_its_status() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| {
            harness.hooks.install_bindings(
                vec![test_binding(
                    test_source_id("s"),
                    HookBoundary::TurnStop,
                    0,
                    None,
                )],
                ctx,
            )
        });

        app.update(|ctx| {
            notify_turn_stop(
                TurnStopStatus::Completed,
                harness.conversation_id,
                &harness.active_session,
                ctx,
            )
        });
        settle(&mut app, |_| harness.hooks.adapter.encoded_count() == 1).await;

        let envelope = &harness.hooks.adapter.encoded_envelopes()[0];
        assert_eq!(envelope.boundary(), HookBoundary::TurnStop);
        assert_eq!(envelope.conversation_id, harness.conversation_id);
        assert!(matches!(
            envelope.payload,
            HookPayload::TurnStop {
                status: TurnStopStatus::Completed,
                continuation_count: 0,
            }
        ));
    });
}

#[test]
fn turn_stop_is_silent_without_a_binding_for_it() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| {
            harness.hooks.install_bindings(
                vec![test_binding(
                    test_source_id("s"),
                    HookBoundary::ToolPre,
                    0,
                    None,
                )],
                ctx,
            )
        });
        app.update(|ctx| {
            notify_turn_stop(
                TurnStopStatus::Completed,
                harness.conversation_id,
                &harness.active_session,
                ctx,
            )
        });
        settle(&mut app, |_| true).await;
        assert_eq!(harness.hooks.adapter.encoded_count(), 0);
        assert_eq!(harness.hooks.runner.invocation_count(), 0);
    });
}
