use ai::agent::action_result::RequestCommandOutputResult;
use ai::hooks::{HOOK_PROTOCOL_VERSION, HookBoundary, HookPayload, HookSurface, RunMetadata};
use warp_core::execution_mode::ExecutionMode;
use warpui::App;

use super::*;
use crate::ai::agent::CancellationReason;
use crate::ai::blocklist::hooks::RunIdentity;
use crate::ai::blocklist::hooks::testing::{
    add_child_conversation, hook_test_app, install_test_hooks, mock_active_session, shell_action,
};
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
        (CancellationReason::HookEndedRun, "hook_ended_run"),
    ];
    for (reason, name) in expected {
        assert_eq!(cancellation_reason_name(reason), name, "{reason:?}");
    }
}

#[test]
fn a_child_conversation_in_a_desktop_process_is_a_local_child() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let child_id = add_child_conversation(&mut app, &harness);
        let action = shell_action("a1", "ls");
        let (parent_surface, child_surface) = app.read(|ctx| {
            let parent = EnvelopeSite::new(harness.conversation_id, &harness.active_session);
            let child = EnvelopeSite::new(child_id, &harness.active_session);
            (
                envelope_for_tool_pre(&action, &parent, ctx).surface,
                envelope_for_tool_pre(&action, &child, ctx).surface,
            )
        });
        assert_eq!(parent_surface, HookSurface::Gui);
        assert_eq!(child_surface, HookSurface::LocalChild);
        assert!(child_surface.is_headless());
    });
}

#[test]
fn envelopes_carry_the_conversations_turn_id_once_a_turn_started() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let action = shell_action("a1", "ls");
        let before = app.read(|ctx| {
            let site = EnvelopeSite::new(harness.conversation_id, &harness.active_session);
            envelope_for_session_start(false, &site, ctx)
        });
        assert_eq!(before.boundary(), HookBoundary::SessionStart);
        assert_eq!(before.turn_id, None);
        assert_eq!(before.exchange_id, None);

        let turn_id = app.update(|ctx| {
            HookRuntime::handle(ctx).update(ctx, |runtime, _| {
                runtime.start_turn(harness.conversation_id)
            })
        });
        let (same_conversation, other_conversation) = app.read(|ctx| {
            let site = EnvelopeSite::new(harness.conversation_id, &harness.active_session);
            let other = EnvelopeSite::new(AIConversationId::new(), &harness.active_session);
            (
                envelope_for_tool_pre(&action, &site, ctx).turn_id,
                envelope_for_tool_pre(&action, &other, ctx).turn_id,
            )
        });
        assert_eq!(same_conversation, Some(turn_id));
        assert_eq!(other_conversation, None);
    });
}

#[test]
fn run_metadata_reaches_the_envelope_and_the_run_surface_decides_oz_or_cli() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        initialize_settings_for_tests_with_mode(&mut app, ExecutionMode::Sdk, false);
        app.update(install_test_hooks);
        let active_session = mock_active_session(&mut app);
        let conversation_id = AIConversationId::new();

        let before = app.read(|ctx| {
            envelope_for_session_start(
                true,
                &EnvelopeSite::new(conversation_id, &active_session),
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
        for (surface, expected) in [
            (RunSurface::CommandLine, HookSurface::CliRun),
            (RunSurface::Oz, HookSurface::OzRun),
        ] {
            app.update(|ctx| {
                HookRuntime::handle(ctx).update(ctx, |runtime, _| {
                    runtime.set_run(Some(RunIdentity {
                        metadata: run.clone(),
                        surface,
                    }));
                });
            });
            let after = app.read(|ctx| {
                envelope_for_session_start(
                    true,
                    &EnvelopeSite::new(conversation_id, &active_session),
                    ctx,
                )
            });
            assert_eq!(after.run.as_ref(), Some(&run));
            assert_eq!(after.surface, expected);
        }
    });
}
