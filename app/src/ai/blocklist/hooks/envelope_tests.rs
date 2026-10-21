use ai::agent::action_result::RequestCommandOutputResult;
use ai::hooks::{HOOK_PROTOCOL_VERSION, HookBoundary, HookPayload, HookSurface, RunMetadata};
use warp_core::execution_mode::ExecutionMode;
use warpui::App;

use super::*;
use crate::ai::agent::CancellationReason;
use crate::ai::blocklist::hooks::testing::{hook_test_app, install_test_hooks, shell_action};
use crate::features::FeatureFlag;
use crate::test_util::settings::initialize_settings_for_tests_with_mode;

#[test]
fn tool_pre_envelope_carries_the_action_and_common_fields() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let action = shell_action("a1", "ls -la");
        let envelope = app.read(|ctx| {
            let site = EnvelopeSite::new(harness.conversation_id, &harness.active_session);
            envelope_for_tool_pre(&action, &site, ctx)
        });

        assert_eq!(envelope.boundary(), HookBoundary::ToolPre);
        assert_eq!(envelope.conversation_id, harness.conversation_id);
        assert_eq!(envelope.action_id, Some(action.id.clone()));
        assert_eq!(envelope.protocol_version, HOOK_PROTOCOL_VERSION);
        assert_eq!(envelope.surface, HookSurface::Gui);
        assert!(!envelope.warp_version.is_empty());
        assert_eq!(envelope.turn_id, None);
        assert_eq!(envelope.exchange_id, None, "no exchange has been sent yet");
        assert_eq!(
            envelope.cwd, None,
            "the mock session has no working directory"
        );
        assert!(envelope.roots.is_empty());
        let HookPayload::ToolPre {
            action: payload_action,
            tool_name,
        } = &envelope.payload
        else {
            panic!("expected a tool.pre payload");
        };
        assert_eq!(payload_action, &action.action);
        assert_eq!(tool_name.as_str(), "Shell");
    });
}

#[test]
fn result_and_failure_envelopes_carry_the_result_and_cancellation_reason() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let action = shell_action("a1", "ls");
        let result = AIAgentActionResultType::RequestCommandOutput(
            RequestCommandOutputResult::CancelledBeforeExecution,
        );
        let (result_envelope, failure_envelope) = app.read(|ctx| {
            let site = EnvelopeSite::new(harness.conversation_id, &harness.active_session);
            (
                envelope_for_tool_result(&action, &result, &site, ctx),
                envelope_for_tool_failure(
                    &action,
                    &result,
                    Some(CancellationReason::ManuallyCancelled),
                    &site,
                    ctx,
                ),
            )
        });
        assert_eq!(result_envelope.boundary(), HookBoundary::ToolResult);
        assert_eq!(failure_envelope.boundary(), HookBoundary::ToolFailure);
        assert_eq!(failure_envelope.action_id, Some(action.id.clone()));
        let HookPayload::ToolFailure {
            cancellation_reason,
            ..
        } = &failure_envelope.payload
        else {
            panic!("expected a tool.failure payload");
        };
        assert_eq!(cancellation_reason.as_deref(), Some("user_cancelled"));
    });
}

#[test]
fn cancellation_reason_names_are_stable() {
    // Scripts match on these strings. Changing one is a breaking change for hook authors.
    let expected = [
        (CancellationReason::ManuallyCancelled, "user_cancelled"),
        (CancellationReason::AutomaticCloudHandoff, "cloud_handoff"),
        (
            CancellationReason::FollowUpSubmitted {
                is_for_same_conversation: true,
            },
            "follow_up_submitted",
        ),
        (
            CancellationReason::FollowUpSubmitted {
                is_for_same_conversation: false,
            },
            "follow_up_submitted",
        ),
        (CancellationReason::UserCommandExecuted, "user_ran_command"),
        (CancellationReason::Reverted, "reverted"),
        (CancellationReason::Deleted, "conversation_deleted"),
        (
            CancellationReason::CommandFinishedDuringInlineAgentView,
            "command_finished",
        ),
        (
            CancellationReason::CLISubagentUserTakeover,
            "user_took_control",
        ),
        (CancellationReason::AgentExitedShell, "shell_exited"),
    ];
    for (reason, name) in expected {
        assert_eq!(cancellation_reason_name(reason), name, "{reason:?}");
    }
}

#[test]
fn a_new_conversation_site_builds_without_a_session_or_history_lookup() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let envelope = app.read(|ctx| {
            let site = EnvelopeSite::for_new_conversation(harness.conversation_id);
            envelope_for_session_start(false, &site, ctx)
        });
        assert_eq!(envelope.boundary(), HookBoundary::SessionStart);
        assert_eq!(envelope.cwd, None);
        assert_eq!(envelope.exchange_id, None);
        assert_eq!(envelope.model_id, None);
    });
}

#[test]
fn run_metadata_reaches_the_envelope_and_marks_the_oz_surface() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        initialize_settings_for_tests_with_mode(&mut app, ExecutionMode::Sdk, false);
        app.update(install_test_hooks);
        let conversation_id = AIConversationId::new();

        let before = app.read(|ctx| {
            envelope_for_session_start(
                true,
                &EnvelopeSite::for_new_conversation(conversation_id),
                ctx,
            )
        });
        assert_eq!(before.run, None);
        assert_eq!(before.surface, HookSurface::CliRun);

        let run = RunMetadata {
            run_id: "run-1".to_owned(),
            parent_run_id: None,
            task_id: Some("task-1".to_owned()),
        };
        app.update(|ctx| {
            HookRuntime::handle(ctx).update(ctx, |runtime, _| {
                runtime.set_run_metadata(Some(run.clone()));
            });
        });
        let after = app.read(|ctx| {
            envelope_for_session_start(
                true,
                &EnvelopeSite::for_new_conversation(conversation_id),
                ctx,
            )
        });
        assert_eq!(after.run, Some(run));
        assert_eq!(after.surface, HookSurface::OzRun);
    });
}
