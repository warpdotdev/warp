use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
use ai::agent::action_result::RunAgentsResult;
use warpui::r#async::Timer;
use warpui::{App, SingletonEntity};

use super::*;
use crate::ai::agent::AIAgentActionResultType;
use crate::ai::agent::task::TaskId;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn make_action_result(id: &str) -> Arc<AIAgentActionResult> {
    Arc::new(AIAgentActionResult {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        result: AIAgentActionResultType::InitProject,
    })
}

fn run_agents_action(id: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        action: AIAgentActionType::RunAgents(RunAgentsRequest {
            summary: "Run child".to_owned(),
            base_prompt: "Investigate".to_owned(),
            skills: vec![],
            model_id: String::new(),
            harness_type: String::new(),
            execution_mode: RunAgentsExecutionMode::Local,
            agent_run_configs: vec![RunAgentsAgentRunConfig {
                name: "child".to_owned(),
                prompt: String::new(),
                title: String::new(),
                agent_identity_uid: String::new(),
                model_id: String::new(),
            }],
            plan_id: String::new(),
            harness_auth_secret_name: None,
        }),
        requires_result: true,
    }
}

fn server_owned_failure(id: &str) -> AIAgentActionResult {
    AIAgentActionResult {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        result: AIAgentActionResultType::RunAgents(RunAgentsResult::Failure {
            error: "server rejected request".to_owned(),
        }),
    }
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

#[test]
fn server_owned_run_agents_failure_suppresses_preprocessed_action_and_outbound_result() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let (conversation_id, action_model) = terminal.update(&mut app, |terminal, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(terminal.id(), false, false, false, ctx)
                });
            (conversation_id, terminal.ai_action_model().clone())
        });
        let action_id = AIAgentActionId::from("server-resolved".to_owned());

        action_model.update(&mut app, |model, ctx| {
            model.queue_actions(
                vec![run_agents_action("server-resolved")],
                conversation_id,
                ctx,
            );
            model.apply_server_owned_run_agents_failure(
                conversation_id,
                server_owned_failure("server-resolved"),
                ctx,
            );
            model.apply_server_owned_run_agents_failure(
                conversation_id,
                server_owned_failure("server-resolved"),
                ctx,
            );
        });
        Timer::after(Duration::from_millis(1)).await;

        action_model.read(&app, |model, _| {
            let Some(AIActionStatus::Finished(result)) = model.get_action_status(&action_id) else {
                panic!("expected terminal action status");
            };
            assert!(matches!(
                result.result,
                AIAgentActionResultType::RunAgents(RunAgentsResult::Failure { .. })
            ));
            assert!(
                model
                    .get_pending_actions_for_conversation(&conversation_id)
                    .next()
                    .is_none()
            );
            assert!(model.get_finished_action_results(conversation_id).is_none());
            assert!(!model.has_unfinished_actions_for_conversation(conversation_id));
        });
    });
}

#[test]
fn server_owned_run_agents_failure_is_scoped_to_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let (first_conversation_id, second_conversation_id, action_model) =
            terminal.update(&mut app, |terminal, ctx| {
                let (first, second) =
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        (
                            history.start_new_conversation(terminal.id(), false, false, false, ctx),
                            history.start_new_conversation(terminal.id(), false, false, false, ctx),
                        )
                    });
                (first, second, terminal.ai_action_model().clone())
            });
        let action_id = AIAgentActionId::from("conversation-local".to_owned());

        action_model.update(&mut app, |model, ctx| {
            model.apply_server_owned_run_agents_failure(
                first_conversation_id,
                server_owned_failure("conversation-local"),
                ctx,
            );
            model.queue_actions(
                vec![run_agents_action("conversation-local")],
                second_conversation_id,
                ctx,
            );
        });
        Timer::after(Duration::from_millis(1)).await;

        action_model.read(&app, |model, _| {
            assert!(
                model
                    .get_pending_actions_for_conversation(&second_conversation_id)
                    .any(|action| action.id == action_id)
            );
            assert!(model.has_unfinished_actions_for_conversation(second_conversation_id));
            assert!(
                model
                    .get_finished_action_results(second_conversation_id)
                    .is_none()
            );
        });
    });
}
