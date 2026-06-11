use ai::agent::action_result::StartAgentVersion;
use warp_core::features::FeatureFlag;
use warpui::{App, EntityId};

use super::*;
use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    StartAgentExecutionMode, StartAgentResult,
};
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::server::server_api::ServerApiProvider;
use crate::test_util::settings::initialize_history_persistence_for_tests;

const FIRST_REQUEST_ID: StartAgentRequestId = StartAgentRequestId::from_raw_for_test(0);

/// Stable placeholder run_id assigned to the parent conversation in tests
/// that dispatch an Oz local child. The Oz `Local` arm of
/// `StartAgentExecutor::execute` bails out synchronously if the parent has
/// no `run_id`, so every `local_with_defaults` test needs to assign one.
const PARENT_RUN_ID: &str = "00000000-0000-0000-0000-000000000001";

fn build_start_agent_action(
    version: StartAgentVersion,
    execution_mode: StartAgentExecutionMode,
) -> AIAgentAction {
    build_start_agent_action_with_prompt(version, execution_mode, "Investigate the failure")
}

fn build_start_agent_action_with_prompt(
    version: StartAgentVersion,
    execution_mode: StartAgentExecutionMode,
    prompt: &str,
) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from("start-agent-action".to_string()),
        action: AIAgentActionType::StartAgent {
            version,
            name: "Agent 1".to_string(),
            prompt: prompt.to_string(),
            execution_mode,
            lifecycle_subscription: None,
        },
        task_id: TaskId::new("start-agent-task".to_string()),
        requires_result: false,
    }
}

#[test]
fn legacy_local_codex_command_prompt_normalizes_to_local_harness() {
    let (prompt, execution_mode) = normalize_legacy_local_child_harness_command(
        "codex --dangerously-bypass-approvals-and-sandbox 'Investigate the failure'".to_string(),
        StartAgentExecutionMode::local_with_defaults(),
    );

    assert_eq!(prompt, "Investigate the failure");
    assert_eq!(
        execution_mode,
        StartAgentExecutionMode::local_harness("codex".to_string())
    );
}

#[test]
fn execute_normalizes_legacy_local_codex_command_before_validation() {
    App::test((), |mut app| async move {
        let _local_codex = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let action = build_start_agent_action_with_prompt(
            StartAgentVersion::V1,
            StartAgentExecutionMode::local_with_defaults(),
            "codex --dangerously-bypass-approvals-and-sandbox 'Investigate the failure'",
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: parent_conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Sync(result) = execution else {
            panic!("expected sync execution");
        };

        assert!(matches!(
            result,
            AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version })
                if error
                    == "Local harness child agents require the parent run_id to be available."
                    && version == StartAgentVersion::V1
        ));
    });
}

#[test]
fn execute_returns_error_when_child_startup_is_blocked_before_initialization() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        history_model.update(&mut app, |model, ctx| {
            model.assign_run_id_for_conversation(
                parent_conversation_id,
                PARENT_RUN_ID.to_string(),
                None,
                terminal_view_id,
                ctx,
            );
        });
        let action = build_start_agent_action(
            StartAgentVersion::V1,
            StartAgentExecutionMode::local_with_defaults(),
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: parent_conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Async {
            execute_future,
            on_complete,
        } = execution
        else {
            panic!("expected async execution");
        };

        let child_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "Agent 1".to_string(),
                parent_conversation_id,
                None,
                ctx,
            )
        });

        history_model.update(&mut app, |model, ctx| {
            model.record_new_conversation_request_complete(
                FIRST_REQUEST_ID,
                child_conversation_id,
                ctx,
            );
        });

        executor.read(&app, |executor, _| {
            assert_eq!(
                executor
                    .pending
                    .values()
                    .find_map(|pending| pending.child_conversation_id),
                Some(child_conversation_id)
            );
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                child_conversation_id,
                ConversationStatus::Blocked {
                    blocked_action:
                        "GitHub authentication required before starting the child agent."
                            .to_string(),
                },
                ctx,
            );
        });

        let async_result = execute_future.await;
        let result = app.update(|ctx| on_complete(async_result, ctx));
        assert!(matches!(
            result,
            AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version })
                if error
                    == "GitHub authentication required before starting the child agent."
                    && version == StartAgentVersion::V1
        ));

        executor.read(&app, |executor, _| {
            assert!(executor.pending.is_empty());
        });
    });
}

#[test]
fn execute_resolves_error_when_request_linkage_happens_after_child_already_failed() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        history_model.update(&mut app, |model, ctx| {
            model.assign_run_id_for_conversation(
                parent_conversation_id,
                PARENT_RUN_ID.to_string(),
                None,
                terminal_view_id,
                ctx,
            );
        });
        let action = build_start_agent_action(
            StartAgentVersion::V1,
            StartAgentExecutionMode::local_with_defaults(),
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: parent_conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Async {
            execute_future,
            on_complete,
        } = execution
        else {
            panic!("expected async execution");
        };

        let child_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "Agent 1".to_string(),
                parent_conversation_id,
                None,
                ctx,
            )
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status_with_error_message(
                terminal_view_id,
                child_conversation_id,
                ConversationStatus::Error,
                Some("'codex' CLI not found on your machine.".to_string()),
                ctx,
            );
        });

        history_model.update(&mut app, |model, ctx| {
            model.record_new_conversation_request_complete(
                FIRST_REQUEST_ID,
                child_conversation_id,
                ctx,
            );
        });

        let async_result = execute_future.await;
        let result = app.update(|ctx| on_complete(async_result, ctx));
        assert!(matches!(
            result,
            AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version })
                if error == "'codex' CLI not found on your machine."
                    && version == StartAgentVersion::V1
        ));

        executor.read(&app, |executor, _| {
            assert!(executor.pending.is_empty());
        });
    });
}

#[test]
fn execute_resolves_success_when_request_linkage_happens_after_child_already_started() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        app.add_singleton_model(OrchestrationEventStreamer::new);
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        history_model.update(&mut app, |model, ctx| {
            model.assign_run_id_for_conversation(
                parent_conversation_id,
                PARENT_RUN_ID.to_string(),
                None,
                terminal_view_id,
                ctx,
            );
        });
        let action = build_start_agent_action(
            StartAgentVersion::V1,
            StartAgentExecutionMode::local_with_defaults(),
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: parent_conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Async {
            execute_future,
            on_complete,
        } = execution
        else {
            panic!("expected async execution");
        };

        let child_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "Agent 1".to_string(),
                parent_conversation_id,
                None,
                ctx,
            )
        });
        let run_id = uuid::Uuid::new_v4().to_string();

        history_model.update(&mut app, |history_model, ctx| {
            history_model.assign_run_id_for_conversation(
                child_conversation_id,
                run_id.clone(),
                None,
                terminal_view_id,
                ctx,
            );
        });

        history_model.update(&mut app, |model, ctx| {
            model.record_new_conversation_request_complete(
                FIRST_REQUEST_ID,
                child_conversation_id,
                ctx,
            );
        });

        let async_result = execute_future.await;
        let result = app.update(|ctx| on_complete(async_result, ctx));
        assert!(matches!(
            result,
            AIAgentActionResultType::StartAgent(StartAgentResult::Success {
                agent_id,
                version,
            }) if agent_id == run_id && version == StartAgentVersion::V1
        ));

        executor.read(&app, |executor, _| {
            assert!(executor.pending.is_empty());
        });
    });
}

#[test]
fn execute_returns_detailed_error_when_child_startup_fails_before_initialization() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        history_model.update(&mut app, |model, ctx| {
            model.assign_run_id_for_conversation(
                parent_conversation_id,
                PARENT_RUN_ID.to_string(),
                None,
                terminal_view_id,
                ctx,
            );
        });
        let action = build_start_agent_action(
            StartAgentVersion::V1,
            StartAgentExecutionMode::local_with_defaults(),
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: parent_conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Async {
            execute_future,
            on_complete,
        } = execution
        else {
            panic!("expected async execution");
        };

        let child_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "Agent 1".to_string(),
                parent_conversation_id,
                None,
                ctx,
            )
        });

        history_model.update(&mut app, |model, ctx| {
            model.record_new_conversation_request_complete(
                FIRST_REQUEST_ID,
                child_conversation_id,
                ctx,
            );
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status_with_error_message(
                terminal_view_id,
                child_conversation_id,
                ConversationStatus::Error,
                Some("Failed to resolve child agent skills: review-comments".to_string()),
                ctx,
            );
        });

        let async_result = execute_future.await;
        let result = app.update(|ctx| on_complete(async_result, ctx));
        assert!(matches!(
            result,
            AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version })
                if error == "Failed to resolve child agent skills: review-comments"
                    && version == StartAgentVersion::V1
        ));
    });
}

#[test]
fn execute_accepts_local_harness_child_when_parent_run_id_is_available() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        history_model.update(&mut app, |model, ctx| {
            model.assign_run_id_for_conversation(
                parent_conversation_id,
                PARENT_RUN_ID.to_string(),
                None,
                terminal_view_id,
                ctx,
            );
        });
        let action = build_start_agent_action(
            StartAgentVersion::V2,
            StartAgentExecutionMode::local_harness("claude".to_string()),
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: parent_conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });
        let AnyActionExecution::Async {
            execute_future,
            on_complete,
        } = execution
        else {
            panic!("expected async execution");
        };

        executor.read(&app, |executor, _| {
            assert!(executor.pending.contains_key(&FIRST_REQUEST_ID));
        });

        drop(execute_future);
        drop(on_complete);
    });
}

#[test]
fn execute_returns_error_when_local_harness_child_missing_parent_run_id() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let action = build_start_agent_action(
            StartAgentVersion::V2,
            StartAgentExecutionMode::local_harness("claude".to_string()),
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: parent_conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Sync(result) = execution else {
            panic!("expected sync execution");
        };

        assert!(matches!(
            result,
            AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version })
                if error
                    == "Local harness child agents require the parent run_id to be available."
                    && version == StartAgentVersion::V2
        ));
    });
}

#[test]
fn execute_rejects_invalid_local_harness_names_before_pane_creation() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let action = build_start_agent_action(
            StartAgentVersion::V2,
            StartAgentExecutionMode::local_harness("gemini".to_string()),
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: parent_conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Sync(result) = execution else {
            panic!("expected sync execution");
        };

        assert!(matches!(
            result,
            AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version })
                if error == "Unsupported local child harness 'gemini'."
                    && version == StartAgentVersion::V2
        ));
    });
}

#[test]
fn execute_rejects_disabled_local_codex_before_other_local_harness_validation() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let action = build_start_agent_action(
            StartAgentVersion::V2,
            StartAgentExecutionMode::local_harness("codex".to_string()),
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: parent_conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Sync(result) = execution else {
            panic!("expected sync execution");
        };

        assert!(matches!(
            result,
            AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version })
                if error == "Local Codex child agents are temporarily disabled."
                    && version == StartAgentVersion::V2
        ));
    });
}

#[test]
fn execute_allows_local_codex_when_flag_is_enabled() {
    App::test((), |mut app| async move {
        let _local_codex = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let action = build_start_agent_action(
            StartAgentVersion::V2,
            StartAgentExecutionMode::local_harness("codex".to_string()),
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: parent_conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Sync(result) = execution else {
            panic!("expected sync execution");
        };

        assert!(matches!(
            result,
            AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version })
                if error
                    == "Local harness child agents require the parent run_id to be available."
                    && version == StartAgentVersion::V2
        ));
    });
}

#[test]
fn parallel_dispatch_keeps_two_pendings_distinguishable_by_request_id() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        history_model.update(&mut app, |model, ctx| {
            model.assign_run_id_for_conversation(
                parent_conversation_id,
                PARENT_RUN_ID.to_string(),
                None,
                terminal_view_id,
                ctx,
            );
        });

        let action_a = build_start_agent_action(
            StartAgentVersion::V1,
            StartAgentExecutionMode::local_with_defaults(),
        );
        let action_b = build_start_agent_action(
            StartAgentVersion::V1,
            StartAgentExecutionMode::local_with_defaults(),
        );
        executor.update(&mut app, |executor, ctx| {
            let _: AnyActionExecution = executor
                .execute(
                    ExecuteActionInput {
                        action: &action_a,
                        conversation_id: parent_conversation_id,
                    },
                    ctx,
                )
                .into();
            let _: AnyActionExecution = executor
                .execute(
                    ExecuteActionInput {
                        action: &action_b,
                        conversation_id: parent_conversation_id,
                    },
                    ctx,
                )
                .into();
        });

        executor.read(&app, |executor, _| {
            assert_eq!(executor.pending.len(), 2, "both pendings should be live");
            assert!(executor.pending.contains_key(&FIRST_REQUEST_ID));
            assert!(executor
                .pending
                .contains_key(&StartAgentRequestId::from_raw_for_test(1)));
        });
    });
}

#[test]
fn parallel_pendings_each_resolve_independently_via_recorded_child_id() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        history_model.update(&mut app, |model, ctx| {
            model.assign_run_id_for_conversation(
                parent_conversation_id,
                PARENT_RUN_ID.to_string(),
                None,
                terminal_view_id,
                ctx,
            );
        });

        let action_a = build_start_agent_action(
            StartAgentVersion::V1,
            StartAgentExecutionMode::local_with_defaults(),
        );
        let action_b = build_start_agent_action(
            StartAgentVersion::V1,
            StartAgentExecutionMode::local_with_defaults(),
        );
        let exec_a = executor.update(&mut app, |executor, ctx| {
            let result: AnyActionExecution = executor
                .execute(
                    ExecuteActionInput {
                        action: &action_a,
                        conversation_id: parent_conversation_id,
                    },
                    ctx,
                )
                .into();
            result
        });
        let exec_b = executor.update(&mut app, |executor, ctx| {
            let result: AnyActionExecution = executor
                .execute(
                    ExecuteActionInput {
                        action: &action_b,
                        conversation_id: parent_conversation_id,
                    },
                    ctx,
                )
                .into();
            result
        });
        let (
            AnyActionExecution::Async {
                execute_future: future_a,
                on_complete: complete_a,
            },
            AnyActionExecution::Async {
                execute_future: future_b,
                on_complete: complete_b,
            },
        ) = (exec_a, exec_b)
        else {
            panic!("expected async executions");
        };

        let child_a = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "Agent A".to_string(),
                parent_conversation_id,
                None,
                ctx,
            )
        });
        let child_b = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "Agent B".to_string(),
                parent_conversation_id,
                None,
                ctx,
            )
        });

        history_model.update(&mut app, |model, ctx| {
            model.record_new_conversation_request_complete(FIRST_REQUEST_ID, child_a, ctx);
            model.record_new_conversation_request_complete(
                StartAgentRequestId::from_raw_for_test(1),
                child_b,
                ctx,
            );
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status_with_error_message(
                terminal_view_id,
                child_b,
                ConversationStatus::Error,
                Some("Agent B init failed".to_string()),
                ctx,
            );
        });

        let async_b = future_b.await;
        let result_b = app.update(|ctx| complete_b(async_b, ctx));
        assert!(matches!(
            result_b,
            AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, .. })
                if error == "Agent B init failed"
        ));

        executor.read(&app, |executor, _| {
            assert_eq!(
                executor.pending.len(),
                1,
                "only child_b's pending should have been removed"
            );
            assert!(executor.pending.contains_key(&FIRST_REQUEST_ID));
        });

        drop(future_a);
        drop(complete_a);
    });
}

#[test]
fn execute_returns_error_when_remote_opencode_harness_is_requested() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let action = build_start_agent_action(
            StartAgentVersion::V2,
            StartAgentExecutionMode::Remote {
                environment_id: "env-123".to_string(),
                skill_references: vec![],
                model_id: String::new(),
                computer_use_enabled: false,
                worker_host: String::new(),
                harness_type: "opencode".to_string(),
                title: String::new(),
                auth_secret_name: None,
            },
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: parent_conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Sync(result) = execution else {
            panic!("expected sync execution");
        };

        assert!(matches!(
            result,
            AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version })
                if error == "Remote child agents do not support the opencode harness yet."
                    && version == StartAgentVersion::V2
        ));
    });
}
