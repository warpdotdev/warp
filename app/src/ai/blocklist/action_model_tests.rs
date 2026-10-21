use std::collections::HashMap;
use std::sync::Arc;

use super::*;
use crate::ai::agent::AIAgentActionResultType;
use crate::ai::agent::task::TaskId;

fn make_action_result(id: &str) -> Arc<AIAgentActionResult> {
    Arc::new(AIAgentActionResult {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        result: AIAgentActionResultType::InitProject,
    })
}

fn count_startable_actions_for_pass(phases: &[(RunningActionPhase, bool)]) -> usize {
    let mut current_phase = None;
    let mut count = 0;

    for (phase, can_autoexecute) in phases {
        if let Some(current_phase) = current_phase
            && !can_start_action_with_current_phase(current_phase, *phase, *can_autoexecute)
        {
            break;
        }

        count += 1;
        current_phase = Some(*phase);

        if matches!(*phase, RunningActionPhase::Serial) {
            break;
        }
    }

    count
}

#[test]
fn parallel_phase_only_admits_matching_autoexecutable_actions() {
    let phase =
        RunningActionPhase::Parallel(execute::ParallelExecutionPolicy::ReadOnlyLocalContext);

    assert!(can_start_action_with_current_phase(phase, phase, true));
    assert!(!can_start_action_with_current_phase(phase, phase, false));
    assert!(!can_start_action_with_current_phase(
        phase,
        RunningActionPhase::Serial,
        true
    ));
    assert!(!can_start_action_with_current_phase(
        RunningActionPhase::Serial,
        phase,
        true
    ));
}

#[test]
fn phased_scheduling_stops_at_serial_barrier_and_resumes_afterward() {
    let read_only_phase =
        RunningActionPhase::Parallel(execute::ParallelExecutionPolicy::ReadOnlyLocalContext);
    let actions = vec![
        (read_only_phase, true),
        (read_only_phase, true),
        (RunningActionPhase::Serial, true),
        (read_only_phase, true),
        (read_only_phase, true),
    ];

    assert_eq!(count_startable_actions_for_pass(&actions), 2);
    assert_eq!(count_startable_actions_for_pass(&actions[2..]), 1);
    assert_eq!(count_startable_actions_for_pass(&actions[3..]), 2);
}

#[test]
fn finished_results_stay_in_original_action_order() {
    let action_order = HashMap::from([
        (AIAgentActionId::from("first".to_owned()), 0),
        (AIAgentActionId::from("second".to_owned()), 1),
        (AIAgentActionId::from("third".to_owned()), 2),
    ]);
    let mut finished_results = [
        make_action_result("third"),
        make_action_result("first"),
        make_action_result("second"),
    ];

    finished_results
        .sort_by_key(|result| action_order.get(&result.id).copied().unwrap_or(usize::MAX));

    assert_eq!(
        finished_results[0].id,
        AIAgentActionId::from("first".to_owned())
    );
    assert_eq!(
        finished_results[1].id,
        AIAgentActionId::from("second".to_owned())
    );
    assert_eq!(
        finished_results[2].id,
        AIAgentActionId::from("third".to_owned())
    );
}

mod hooks {
    use ai::agent::action_result::RequestCommandOutputResult;
    use ai::hooks::test_util::{ScriptedRunner, test_binding, test_source_id};
    use ai::hooks::{HookBoundary, HookPayload};
    use warpui::App;

    use crate::ai::agent::{AIAgentActionResultType, AIAgentActionType, CancellationReason};
    use crate::ai::blocklist::hooks::pre_tool::{PreToolHookState, cached_state};
    use crate::ai::blocklist::hooks::testing::{
        HookTestApp, action_model, hook_test_app, settle, shell_action,
    };
    use crate::features::FeatureFlag;

    fn install_shell_hook(harness: &HookTestApp, app: &mut App, exit: i32, stdout: &str) {
        let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, Some("Shell"));
        harness
            .hooks
            .runner
            .script(binding.id.clone(), ScriptedRunner::exit_with(exit, stdout));
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));
    }

    fn shell_commands_seen(harness: &HookTestApp) -> Vec<String> {
        harness
            .hooks
            .adapter
            .encoded_envelopes()
            .iter()
            .filter_map(|envelope| match &envelope.payload {
                HookPayload::ToolPre {
                    action: AIAgentActionType::RequestCommandOutput { command, .. },
                    ..
                } => Some(command.clone()),
                HookPayload::ToolPre { .. }
                | HookPayload::SessionStart { .. }
                | HookPayload::PromptSubmit { .. }
                | HookPayload::ToolResult { .. }
                | HookPayload::ToolFailure { .. }
                | HookPayload::TurnStop { .. }
                | HookPayload::ResponseComplete { .. }
                | HookPayload::ReasoningComplete { .. } => None,
            })
            .collect()
    }

    #[test]
    fn a_queued_shell_action_waits_for_its_verdict_and_is_denied_through_the_queue() {
        App::test((), |mut app| async move {
            let _flag = FeatureFlag::AgentHooks.override_enabled(true);
            let harness = hook_test_app(&mut app);
            install_shell_hook(&harness, &mut app, 2, "policy says no");
            let model = action_model(&mut app, &harness);
            let action = shell_action("shell", "ls");
            let action_id = action.id.clone();

            model.update(&mut app, |model, ctx| {
                model.queue_actions(vec![action], harness.conversation_id, ctx)
            });
            settle(&mut app, |app| {
                model.read(app, |model, _| {
                    model.get_action_result(&action_id).is_some()
                })
            })
            .await;

            let result = model.read(&app, |model, _| {
                model.get_action_result(&action_id).cloned()
            });
            assert_eq!(
                result.map(|result| result.result.clone()),
                Some(AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::HookDenied {
                        command: "ls".to_owned(),
                        reason: "policy says no".to_owned(),
                    }
                ))
            );
            assert!(model.read(&app, |model, _| {
                model
                    .get_pending_actions_for_conversation(&harness.conversation_id)
                    .next()
                    .is_none()
            }));
            assert!(
                app.read(|ctx| cached_state(&action_id, ctx)).is_none(),
                "a finished action leaves no verdict behind"
            );
        });
    }

    #[test]
    fn accepting_an_edited_command_runs_the_hook_again_on_the_new_text() {
        App::test((), |mut app| async move {
            let _flag = FeatureFlag::AgentHooks.override_enabled(true);
            let harness = hook_test_app(&mut app);
            install_shell_hook(&harness, &mut app, 0, r#"{"verdict":"allow"}"#);
            let model = action_model(&mut app, &harness);
            let action = shell_action("shell", "ls");
            let action_id = action.id.clone();

            model.update(&mut app, |model, ctx| {
                model.queue_actions(vec![action], harness.conversation_id, ctx)
            });
            // The hook allows, Warp itself still asks, so the action ends up blocked on the user.
            settle(&mut app, |app| {
                model.read(app, |model, _| {
                    model
                        .get_action_status(&action_id)
                        .is_some_and(|status| status.is_blocked())
                })
            })
            .await;
            settle(&mut app, |app| {
                matches!(
                    app.read(|ctx| cached_state(&action_id, ctx)),
                    Some(PreToolHookState::Resolved(_))
                )
            })
            .await;
            assert_eq!(shell_commands_seen(&harness), vec!["ls"]);

            model.update(&mut app, |model, ctx| {
                model.handle_requested_command_accepted(&action_id, "ls -la".to_owned(), ctx)
            });
            // The edit threw the old verdict away and asked the hook about the new command.
            settle(&mut app, |_| shell_commands_seen(&harness).len() == 2).await;
            assert_eq!(shell_commands_seen(&harness), vec!["ls", "ls -la"]);
            settle(&mut app, |app| {
                model.read(app, |model, _| {
                    model
                        .get_action_status(&action_id)
                        .is_some_and(|status| status.is_running())
                })
            })
            .await;
        });
    }

    #[test]
    fn cancelling_the_conversation_while_a_verdict_is_pending_drops_it() {
        App::test((), |mut app| async move {
            let _flag = FeatureFlag::AgentHooks.override_enabled(true);
            let harness = hook_test_app(&mut app);
            install_shell_hook(&harness, &mut app, 2, "denied");
            let gate = harness.hooks.runner.hold();
            let model = action_model(&mut app, &harness);
            let action = shell_action("shell", "ls");
            let action_id = action.id.clone();

            model.update(&mut app, |model, ctx| {
                model.queue_actions(vec![action], harness.conversation_id, ctx)
            });
            settle(&mut app, |_| harness.hooks.runner.invocation_count() == 1).await;
            assert!(matches!(
                app.read(|ctx| cached_state(&action_id, ctx)),
                Some(PreToolHookState::Pending)
            ));

            model.update(&mut app, |model, ctx| {
                model.cancel_all_pending_actions(
                    harness.conversation_id,
                    Some(CancellationReason::ManuallyCancelled),
                    ctx,
                )
            });
            gate.release();
            let result = model.read(&app, |model, _| {
                model.get_action_result(&action_id).cloned()
            });
            assert_eq!(
                result.map(|result| result.result.clone()),
                Some(AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::CancelledBeforeExecution
                ))
            );
            assert!(app.read(|ctx| cached_state(&action_id, ctx)).is_none());

            // Let any late verdict land: it must not revive the action or deny it.
            for _ in 0..10 {
                warpui::r#async::Timer::after(std::time::Duration::from_millis(5)).await;
                app.update(|_| {});
            }
            let result = model.read(&app, |model, _| {
                model.get_action_result(&action_id).cloned()
            });
            assert!(matches!(
                result.as_deref().map(|result| &result.result),
                Some(AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::CancelledBeforeExecution
                ))
            ));
            assert!(model.read(&app, |model, _| {
                model
                    .get_pending_actions_for_conversation(&harness.conversation_id)
                    .next()
                    .is_none()
            }));
        });
    }
}
