use ai::agent::action_result::StartAgentVersion;
use warpui::{App, EntityId};

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    StartAgentExecutionMode, StartAgentResult,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;

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

fn assert_start_agent_disabled(result: AIAgentActionResultType, version: StartAgentVersion) {
    assert!(matches!(
        result,
        AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version: actual })
            if error == START_AGENT_DISABLED_ERROR && actual == version
    ));
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
fn execute_rejects_start_agent_without_creating_pending_request() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let action = build_start_agent_action(
            StartAgentVersion::V2,
            StartAgentExecutionMode::local_with_defaults(),
        );

        let execution = executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: parent_conversation_id,
                    },
                    ctx,
                )
                .into()
        });

        let AnyActionExecution::Sync(result) = execution else {
            panic!("expected sync execution");
        };
        assert_start_agent_disabled(result, StartAgentVersion::V2);
        executor.read(&app, |executor, _| {
            assert!(executor.pending.is_empty());
        });
    });
}

#[test]
fn execute_rejects_legacy_local_codex_command_without_validation_side_effects() {
    App::test((), |mut app| async move {
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
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: parent_conversation_id,
                    },
                    ctx,
                )
                .into()
        });

        let AnyActionExecution::Sync(result) = execution else {
            panic!("expected sync execution");
        };
        assert_start_agent_disabled(result, StartAgentVersion::V1);
        executor.read(&app, |executor, _| {
            assert!(executor.pending.is_empty());
        });
    });
}

#[test]
fn dispatch_returns_disabled_error_without_emitting_create_agent() {
    App::test((), |mut app| async move {
        let executor = app.add_model(StartAgentExecutor::new);
        let parent_conversation_id = AIConversationId::new();

        let receiver = executor.update(&mut app, |executor, ctx| {
            executor.dispatch(
                "Agent 1".to_string(),
                "Investigate the failure".to_string(),
                StartAgentExecutionMode::local_with_defaults(),
                None,
                parent_conversation_id,
                Some("parent-run-id".to_string()),
                ctx,
            )
        });

        assert!(matches!(
            receiver.recv().await,
            Ok(StartAgentOutcome::Error(error)) if error == START_AGENT_DISABLED_ERROR
        ));
        executor.read(&app, |executor, _| {
            assert!(executor.pending.is_empty());
        });
    });
}
