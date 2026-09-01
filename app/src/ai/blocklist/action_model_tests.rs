use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use warpui::App;

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionResultType, RunAgentsAgentOutcome, RunAgentsAgentOutcomeKind,
    RunAgentsLaunchedExecutionMode, RunAgentsResult,
};
use crate::ai::blocklist::{BlocklistAIControllerEvent, BlocklistAIHistoryModel};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

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

fn assert_run_agents_result_sends_one_follow_up(result: RunAgentsResult) {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let sent_request_count = Arc::new(Mutex::new(0));

        let (conversation_id, task_id, action_model) = terminal.update(&mut app, |view, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id =
                        history.start_new_conversation(view.id(), false, false, false, ctx);
                    history.set_server_conversation_token_for_conversation(
                        conversation_id,
                        "server-conversation".to_owned(),
                    );
                    conversation_id
                });
            let task_id = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .expect("conversation exists")
                .get_root_task_id()
                .clone();
            let controller = view.ai_controller().clone();
            let sent_request_count_for_subscription = Arc::clone(&sent_request_count);
            ctx.subscribe_to_model(&controller, move |_, _, event, _| {
                if matches!(event, BlocklistAIControllerEvent::SentRequest { .. }) {
                    *sent_request_count_for_subscription.lock().unwrap() += 1;
                }
            });
            (conversation_id, task_id, view.ai_action_model().clone())
        });

        action_model.update(&mut app, |action_model, ctx| {
            action_model.handle_action_result(
                conversation_id,
                Arc::new(AIAgentActionResult {
                    id: AIAgentActionId::from("run-agents".to_owned()),
                    task_id,
                    result: AIAgentActionResultType::RunAgents(result),
                }),
                None,
                ctx,
            );
        });

        assert_eq!(*sent_request_count.lock().unwrap(), 1);
    });
}

#[test]
fn local_run_agents_failure_sends_exactly_one_follow_up() {
    assert_run_agents_result_sends_one_follow_up(RunAgentsResult::Failure {
        error: "child launch failed".to_owned(),
    });
}

#[test]
fn partial_local_run_agents_launch_sends_exactly_one_follow_up() {
    assert_run_agents_result_sends_one_follow_up(RunAgentsResult::Launched {
        model_id: "auto".to_owned(),
        harness_type: "oz".to_owned(),
        execution_mode: RunAgentsLaunchedExecutionMode::Local,
        agents: vec![
            RunAgentsAgentOutcome {
                name: "started".to_owned(),
                kind: RunAgentsAgentOutcomeKind::Launched {
                    agent_id: "agent-id".to_owned(),
                },
                resolved_model_id: "auto".to_owned(),
            },
            RunAgentsAgentOutcome {
                name: "failed".to_owned(),
                kind: RunAgentsAgentOutcomeKind::Failed {
                    error: "launch failed".to_owned(),
                },
                resolved_model_id: "auto".to_owned(),
            },
        ],
    });
}
