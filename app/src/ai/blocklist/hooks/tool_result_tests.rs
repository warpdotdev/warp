use ai::agent::action_result::{
    CallMCPToolResult, ReadFilesResult, RequestCommandOutputResult, RunAgentsResult,
};
use ai::hooks::test_util::{ScriptedRunner, test_binding, test_source_id};
use ai::hooks::{DeclaredBinding, HookBoundary};
use chrono::Local;
use warpui::{App, ModelHandle, SingletonEntity};

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionId, AIAgentActionResult, AIAgentPtyWriteMode, RunAgentsExecutionMode,
    RunAgentsRequest,
};
use crate::ai::blocklist::hooks::testing::{
    HookTestApp, action_model, hook_test_app, init_project_action, read_nothing_action, settle,
    shell_action,
};
use crate::ai::blocklist::{BlocklistAIActionModel, BlocklistAIHistoryModel};
use crate::features::FeatureFlag;
use crate::terminal::model::block::BlockId;

fn observe(
    app: &mut App,
    harness: &HookTestApp,
    action: &AIAgentAction,
    result: AIAgentActionResultType,
) {
    app.update(|ctx| {
        observe_terminal_result(
            action,
            &result,
            None,
            harness.conversation_id,
            &harness.active_session,
            ctx,
        )
    });
}

fn boundaries(app: &App) -> Vec<HookBoundary> {
    app.read(observed_terminal_results)
        .into_iter()
        .map(|observed| observed.boundary)
        .collect()
}

fn completed(command: &str, block_id: &BlockId, exit_code: i32) -> AIAgentActionResultType {
    AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::Completed {
        block_id: block_id.clone(),
        command: command.to_owned(),
        output: "out".to_owned(),
        exit_code: exit_code.into(),
        start_ts: Some(Local::now()),
        completed_ts: Some(Local::now()),
    })
}

fn snapshot(command: &str, block_id: &BlockId) -> AIAgentActionResultType {
    AIAgentActionResultType::RequestCommandOutput(
        RequestCommandOutputResult::LongRunningCommandSnapshot {
            block_id: block_id.clone(),
            command: command.to_owned(),
            grid_contents: String::new(),
            cursor: String::new(),
            is_alt_screen_active: false,
            activity: None,
        },
    )
}

fn read_action(id: &str, block_id: &BlockId) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        requires_result: true,
        action: AIAgentActionType::ReadShellCommandOutput {
            block_id: block_id.clone(),
            delay: None,
        },
    }
}

fn write_action(id: &str, block_id: &BlockId) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        requires_result: true,
        action: AIAgentActionType::WriteToLongRunningShellCommand {
            block_id: block_id.clone(),
            input: bytes::Bytes::from_static(b"q"),
            mode: AIAgentPtyWriteMode::Raw,
        },
    }
}

fn mcp_action(id: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        requires_result: true,
        action: AIAgentActionType::CallMCPTool {
            server_id: None,
            name: "tool".to_owned(),
            input: serde_json::Value::Null,
        },
    }
}

/// The payload of the last envelope the adapter saw for `action_id`.
fn last_result_for(harness: &HookTestApp, action_id: &str) -> Option<HookPayload> {
    harness
        .hooks
        .adapter
        .encoded_envelopes()
        .into_iter()
        .rev()
        .find(|envelope| {
            envelope
                .action_id
                .as_ref()
                .is_some_and(|id| id.to_string() == action_id)
        })
        .map(|envelope| envelope.payload)
}

fn observing_bindings() -> Vec<DeclaredBinding> {
    let source = test_source_id("observer");
    vec![
        test_binding(source.clone(), HookBoundary::ToolResult, 0, None),
        test_binding(source, HookBoundary::ToolFailure, 0, None),
    ]
}

fn observed(app: &App) -> Vec<ObservedTerminalResult> {
    app.read(observed_terminal_results)
}

fn has_result(
    model: &ModelHandle<BlocklistAIActionModel>,
    action_id: &AIAgentActionId,
    app: &mut App,
) -> bool {
    model.read(app, |model, _| model.get_action_result(action_id).is_some())
}

#[test]
fn a_synchronous_completion_is_observed_once_as_a_result() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let model = action_model(&mut app, &harness);
        let action = init_project_action("init");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| has_result(&model, &action.id, app)).await;

        assert_eq!(
            observed(&app),
            vec![ObservedTerminalResult {
                action_id: action.id.clone(),
                boundary: HookBoundary::ToolResult,
            }]
        );
        settle(&mut app, |_| harness.hooks.adapter.encoded_count() == 1).await;
    });
}

#[test]
fn an_asynchronous_completion_is_observed_once_as_a_result() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let model = action_model(&mut app, &harness);
        let action = read_nothing_action("read");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| has_result(&model, &action.id, app)).await;

        assert_eq!(
            observed(&app),
            vec![ObservedTerminalResult {
                action_id: action.id.clone(),
                boundary: HookBoundary::ToolResult,
            }]
        );
    });
}

#[test]
fn a_hook_denial_is_observed_once_as_a_failure() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let deny = test_binding(
            test_source_id("policy"),
            HookBoundary::ToolPre,
            0,
            Some("Shell"),
        );
        harness
            .hooks
            .runner
            .script(deny.id.clone(), ScriptedRunner::deny_with("not today"));
        let mut bindings = observing_bindings();
        bindings.push(deny);
        app.update(|ctx| harness.hooks.install_bindings(bindings, ctx));
        let model = action_model(&mut app, &harness);
        let action = shell_action("shell", "ls");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| has_result(&model, &action.id, app)).await;

        let result = model.read(&app, |model, _| {
            model.get_action_result(&action.id).cloned()
        });
        assert!(matches!(
            result.as_deref().map(|result| &result.result),
            Some(AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::HookDenied { .. }
            ))
        ));
        assert_eq!(
            observed(&app),
            vec![ObservedTerminalResult {
                action_id: action.id.clone(),
                boundary: HookBoundary::ToolFailure,
            }]
        );
    });
}

#[test]
fn cancelling_a_running_action_is_observed_once_as_a_failure() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let model = action_model(&mut app, &harness);
        let action = shell_action("shell", "sleep 100");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| {
            model.read(app, |model, _| {
                model
                    .get_action_status(&action.id)
                    .is_some_and(|s| s.is_blocked())
            })
        })
        .await;
        model.update(&mut app, |model, ctx| {
            model.execute_action(&action.id, harness.conversation_id, ctx)
        });
        assert!(model.read(&app, |model, _| {
            model
                .get_action_status(&action.id)
                .is_some_and(|s| s.is_running())
        }));

        model.update(&mut app, |model, ctx| {
            model.cancel_action_with_id(
                harness.conversation_id,
                &action.id,
                CancellationReason::ManuallyCancelled,
                ctx,
            )
        });
        settle(&mut app, |app| has_result(&model, &action.id, app)).await;

        assert_eq!(
            observed(&app),
            vec![ObservedTerminalResult {
                action_id: action.id.clone(),
                boundary: HookBoundary::ToolFailure,
            }]
        );
    });
}

#[test]
fn cancelling_a_pending_action_is_observed_once_as_a_failure() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let model = action_model(&mut app, &harness);
        let action = shell_action("shell", "ls");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| {
            model.read(app, |model, _| {
                model
                    .get_action_status(&action.id)
                    .is_some_and(|s| s.is_blocked())
            })
        })
        .await;

        model.update(&mut app, |model, ctx| {
            model.cancel_action_with_id(
                harness.conversation_id,
                &action.id,
                CancellationReason::ManuallyCancelled,
                ctx,
            )
        });
        assert!(has_result(&model, &action.id, &mut app));
        assert_eq!(
            observed(&app),
            vec![ObservedTerminalResult {
                action_id: action.id.clone(),
                boundary: HookBoundary::ToolFailure,
            }]
        );
    });
}

#[test]
fn a_shared_session_viewer_does_not_observe_mirrored_results() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let viewer_conversation_id = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.start_new_conversation(harness.terminal_view_id, false, true, false, ctx)
            })
        });
        let model = action_model(&mut app, &harness);
        model.update(&mut app, |model, _| model.set_view_only(true));
        let first = shell_action("first", "sleep 100");
        let second = shell_action("second", "ls");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(
                vec![first.clone(), second.clone()],
                viewer_conversation_id,
                ctx,
            )
        });
        settle(&mut app, |app| {
            model.read(app, |model, _| {
                model
                    .get_pending_actions_for_conversation(&viewer_conversation_id)
                    .count()
                    == 2
            })
        })
        .await;

        // The sharer's snapshot for the first command arrives, which drains the second one.
        model.update(&mut app, |model, ctx| {
            model.apply_finished_action_result(
                viewer_conversation_id,
                AIAgentActionResult {
                    id: first.id.clone(),
                    task_id: first.task_id.clone(),
                    result: AIAgentActionResultType::RequestCommandOutput(
                        RequestCommandOutputResult::LongRunningCommandSnapshot {
                            block_id: BlockId::new(),
                            command: "sleep 100".to_owned(),
                            grid_contents: String::new(),
                            cursor: String::new(),
                            is_alt_screen_active: false,
                            activity: None,
                        },
                    ),
                },
                ctx,
            )
        });
        assert!(has_result(&model, &first.id, &mut app));
        assert!(has_result(&model, &second.id, &mut app));
        assert!(observed(&app).is_empty());
    });
}

#[test]
fn a_tool_that_ran_is_a_result_whatever_it_returned_and_one_that_did_not_is_a_failure() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let block = BlockId::new();
        let shell = shell_action("shell", "make");
        let cases: Vec<(AIAgentAction, AIAgentActionResultType, HookBoundary)> = vec![
            (
                shell.clone(),
                completed("make", &block, 1),
                HookBoundary::ToolResult,
            ),
            (
                shell.clone(),
                completed("make", &block, 130),
                HookBoundary::ToolResult,
            ),
            (
                shell.clone(),
                AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::HookDenied {
                        command: "make".to_owned(),
                        reason: "no".to_owned(),
                    },
                ),
                HookBoundary::ToolFailure,
            ),
            (
                shell.clone(),
                AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::Denylisted {
                        command: "make".to_owned(),
                    },
                ),
                HookBoundary::ToolFailure,
            ),
            (
                shell,
                AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::CancelledBeforeExecution,
                ),
                HookBoundary::ToolFailure,
            ),
            (
                mcp_action("mcp"),
                AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Error("boom".to_owned())),
                HookBoundary::ToolResult,
            ),
            (
                mcp_action("mcp"),
                AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Error(format!(
                    "{DENIED_BY_HOOK_PREFIX}not allowed"
                ))),
                HookBoundary::ToolFailure,
            ),
            (
                mcp_action("mcp"),
                AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Cancelled),
                HookBoundary::ToolFailure,
            ),
            (
                read_nothing_action("read"),
                AIAgentActionResultType::ReadFiles(ReadFilesResult::Error("missing".to_owned())),
                HookBoundary::ToolResult,
            ),
            (
                AIAgentAction {
                    id: AIAgentActionId::from("agents".to_owned()),
                    task_id: TaskId::new("task".to_owned()),
                    requires_result: true,
                    action: AIAgentActionType::RunAgents(Box::new(RunAgentsRequest {
                        metadata: Default::default(),
                        summary: String::new(),
                        base_prompt: String::new(),
                        skills: Vec::new(),
                        model_id: String::new(),
                        harness_type: String::new(),
                        execution_mode: RunAgentsExecutionMode::Local,
                        agent_run_configs: Vec::new(),
                        plan_id: String::new(),
                        harness_auth_secret_name: None,
                    })),
                },
                AIAgentActionResultType::RunAgents(RunAgentsResult::Denied {
                    reason: "no".to_owned(),
                }),
                HookBoundary::ToolFailure,
            ),
        ];
        let expected = cases
            .iter()
            .map(|(_, _, boundary)| *boundary)
            .collect::<Vec<_>>();
        for (action, result, _) in cases {
            observe(&mut app, &harness, &action, result);
        }
        assert_eq!(boundaries(&app), expected);

        // A cancellation reason makes any result a failure.
        app.update(|ctx| {
            observe_terminal_result(
                &shell_action("shell", "make"),
                &completed("make", &block, 0),
                Some(CancellationReason::ManuallyCancelled),
                harness.conversation_id,
                &harness.active_session,
                ctx,
            )
        });
        assert_eq!(boundaries(&app).last(), Some(&HookBoundary::ToolFailure));
    });
}

#[test]
fn a_long_running_commands_real_exit_is_reported_again_on_its_own_action() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let block = BlockId::new();
        let command = shell_action("shell", "sleep 100");

        observe(&mut app, &harness, &command, snapshot("sleep 100", &block));
        assert_eq!(boundaries(&app), vec![HookBoundary::ToolResult]);
        assert_eq!(
            app.read(|ctx| HookRuntime::as_ref(ctx).shell_completions.pending_count()),
            1
        );

        let poll = read_action("read", &block);
        let start = Local::now();
        let end = start + chrono::Duration::milliseconds(1500);
        observe(
            &mut app,
            &harness,
            &poll,
            AIAgentActionResultType::ReadShellCommandOutput(
                ReadShellCommandOutputResult::CommandFinished {
                    command: "sleep 100".to_owned(),
                    block_id: block.clone(),
                    output: "done".to_owned(),
                    exit_code: 3.into(),
                    start_ts: Some(start),
                    completed_ts: Some(end),
                },
            ),
        );
        let observed = app.read(observed_terminal_results);
        assert_eq!(
            observed
                .iter()
                .map(|o| (o.action_id.to_string(), o.boundary))
                .collect::<Vec<_>>(),
            vec![
                ("shell".to_owned(), HookBoundary::ToolResult),
                ("read".to_owned(), HookBoundary::ToolResult),
                ("shell".to_owned(), HookBoundary::ToolResult),
            ]
        );
        settle(&mut app, |_| harness.hooks.adapter.encoded_count() == 3).await;
        let Some(HookPayload::ToolResult {
            action,
            result,
            duration,
            ..
        }) = last_result_for(&harness, "shell")
        else {
            panic!("the command's own action carries its exit");
        };
        assert_eq!(action, command.action);
        assert!(matches!(
            result,
            AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::Completed {
                ref command, ref output, exit_code, ..
            }) if command == "sleep 100" && output == "done" && exit_code.value() == 3
        ));
        assert_eq!(duration, Some(Duration::from_millis(1500)));
        assert_eq!(
            app.read(|ctx| HookRuntime::as_ref(ctx).shell_completions.pending_count()),
            0
        );

        // A second poll that sees the same block finished reports nothing more for the command.
        observe(
            &mut app,
            &harness,
            &read_action("read-2", &block),
            AIAgentActionResultType::ReadShellCommandOutput(
                ReadShellCommandOutputResult::CommandFinished {
                    command: "sleep 100".to_owned(),
                    block_id: block.clone(),
                    output: "done".to_owned(),
                    exit_code: 3.into(),
                    start_ts: None,
                    completed_ts: None,
                },
            ),
        );
        assert_eq!(app.read(observed_terminal_results).len(), 4);
    });
}

#[test]
fn a_write_that_sees_the_exit_reports_it_and_a_plain_command_is_reported_once() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let block = BlockId::new();
        let command = shell_action("shell", "vim");

        observe(&mut app, &harness, &command, snapshot("vim", &block));
        observe(
            &mut app,
            &harness,
            &write_action("write", &block),
            AIAgentActionResultType::WriteToLongRunningShellCommand(
                WriteToLongRunningShellCommandResult::CommandFinished {
                    block_id: block.clone(),
                    output: "bye".to_owned(),
                    exit_code: 0.into(),
                    start_ts: None,
                    completed_ts: None,
                },
            ),
        );
        let observed = app.read(observed_terminal_results);
        assert_eq!(observed.len(), 3);
        assert_eq!(observed[2].action_id.to_string(), "shell");
        assert_eq!(observed[2].boundary, HookBoundary::ToolResult);

        // A command that finished on its own, and a poll of a block the agent did not start, are
        // each reported exactly once.
        let other_block = BlockId::new();
        observe(
            &mut app,
            &harness,
            &shell_action("quick", "ls"),
            completed("ls", &other_block, 0),
        );
        observe(
            &mut app,
            &harness,
            &read_action("read-user", &BlockId::new()),
            AIAgentActionResultType::ReadShellCommandOutput(
                ReadShellCommandOutputResult::CommandFinished {
                    command: "the user's".to_owned(),
                    block_id: BlockId::new(),
                    output: String::new(),
                    exit_code: 0.into(),
                    start_ts: None,
                    completed_ts: None,
                },
            ),
        );
        assert_eq!(app.read(observed_terminal_results).len(), 5);
    });
}

#[test]
fn a_command_still_waiting_for_its_exit_is_forgotten_with_its_conversation() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let block = BlockId::new();
        observe(
            &mut app,
            &harness,
            &shell_action("shell", "sleep 100"),
            snapshot("sleep 100", &block),
        );
        app.update(|ctx| {
            HookRuntime::handle(ctx).update(ctx, |runtime, _| {
                runtime.forget_conversation(harness.conversation_id);
            });
        });
        assert_eq!(
            app.read(|ctx| HookRuntime::as_ref(ctx).shell_completions.pending_count()),
            0
        );
    });
}

#[test]
fn nothing_is_observed_when_no_binding_is_active() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let model = action_model(&mut app, &harness);
        let action = init_project_action("init");
        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| has_result(&model, &action.id, app)).await;
        assert!(observed(&app).is_empty());
    });
}
