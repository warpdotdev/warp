use warpui::{App, ViewHandle};

use super::*;
use crate::terminal::TerminalView;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

// Blocks persisted before `should_auto_resume` stored `Stop` as a bare unit variant. These must
// still deserialize (with resume disabled) so restoring an older session doesn't drop the block's
// AI metadata wholesale.
#[test]
fn legacy_stop_reason_deserializes_with_resume_disabled() {
    let reason: UserTakeOverReason = serde_json::from_str("\"Stop\"").unwrap();
    assert_eq!(
        reason,
        UserTakeOverReason::Stop {
            should_auto_resume: false
        }
    );

    let state: LongRunningCommandControlState =
        serde_json::from_str(r#"{"User":{"reason":"Stop"}}"#).unwrap();
    assert_eq!(
        state,
        LongRunningCommandControlState::User {
            reason: UserTakeOverReason::Stop {
                should_auto_resume: false
            }
        }
    );
}

fn running_requested_command(
    app: &mut App,
) -> (
    ViewHandle<TerminalView>,
    ModelHandle<CLISubagentController>,
    AIConversationId,
) {
    initialize_app_for_terminal_view(app);
    let terminal = add_window_with_terminal(app, None);
    // Scope history events to this controller without constructing unrelated CLI transcript views.
    let terminal_surface_id = EntityId::new();
    let cli_controller = terminal.update(app, |view, ctx| {
        ctx.add_model(|ctx| {
            CLISubagentController::new(
                view.ai_controller(),
                view.ai_action_model(),
                None,
                view.model.clone(),
                view.model_event_dispatcher(),
                terminal_surface_id,
                ctx,
            )
        })
    });
    let conversation_id = BlocklistAIHistoryModel::handle(app).update(app, |history, ctx| {
        history.start_new_conversation(terminal_surface_id, false, false, false, ctx)
    });
    terminal.update(app, |view, _| {
        let mut model = view.model.lock();
        model.simulate_long_running_block("sleep 12", "");
        model
            .block_list_mut()
            .active_block_mut()
            .set_agent_interaction_mode_for_requested_command(
                AIAgentActionId::from("requested-command".to_owned()),
                None,
                conversation_id,
            );
    });
    (terminal, cli_controller, conversation_id)
}

#[test]
fn completed_command_stays_hidden_when_cli_subtask_arrives_late() {
    App::test((), |mut app| async move {
        let (terminal, cli_controller, conversation_id) = running_requested_command(&mut app);
        let (block_id, dispatcher) = terminal.read(&app, |view, _| {
            (
                view.model.lock().active_block_id().clone(),
                view.model_event_dispatcher().clone(),
            )
        });
        let (completed_tx, completed_rx) = async_channel::bounded(1);
        let completed_block_id = block_id.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&dispatcher, move |_, event, _| {
                if let ModelEvent::BlockCompleted(event) = event
                    && event.block_id == completed_block_id
                {
                    completed_tx.try_send(()).unwrap();
                }
            });
        });
        terminal.update(&mut app, |view, _| view.model.lock().finish_block());
        completed_rx.recv().await.unwrap();
        let terminal_surface_id =
            cli_controller.read(&app, |controller, _| controller.terminal_view_id);
        BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
            history
                .create_cli_subagent_task_for_conversation(
                    block_id.clone(),
                    conversation_id,
                    terminal_surface_id,
                    ctx,
                )
                .unwrap();
        });

        cli_controller.read(&app, |controller, _| {
            let model = controller.terminal_model.lock();
            let block = model.block_list().block_with_id(&block_id).unwrap();
            assert!(block.interaction_mode().should_hide_block());
            assert!(block.cli_subagent_task_id().is_none());
        });
    });
}

#[test]
fn running_command_cli_subtask_finishes_when_command_completes() {
    App::test((), |mut app| async move {
        let (terminal, cli_controller, conversation_id) = running_requested_command(&mut app);
        let block_id = terminal.read(&app, |view, _| view.model.lock().active_block_id().clone());
        let (finished_tx, finished_rx) = async_channel::bounded(1);
        app.update(|ctx| {
            ctx.subscribe_to_model(&cli_controller, move |_, event, _| {
                if let CLISubagentEvent::FinishedSubagent {
                    initial_requested_command_action_id,
                    ..
                } = event
                {
                    finished_tx
                        .try_send(initial_requested_command_action_id.clone())
                        .unwrap();
                }
            });
        });
        let terminal_surface_id =
            cli_controller.read(&app, |controller, _| controller.terminal_view_id);
        BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
            history
                .create_cli_subagent_task_for_conversation(
                    block_id,
                    conversation_id,
                    terminal_surface_id,
                    ctx,
                )
                .unwrap();
        });
        assert!(cli_controller.read(&app, |controller, _| controller.active_target().is_some()));

        terminal.update(&mut app, |view, _| view.model.lock().finish_block());
        assert_eq!(
            finished_rx.recv().await.unwrap(),
            Some(AIAgentActionId::from("requested-command".to_owned()))
        );
    });
}

#[test]
fn stop_reason_round_trips() {
    for should_auto_resume in [true, false] {
        let reason = UserTakeOverReason::Stop { should_auto_resume };
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(
            serde_json::from_str::<UserTakeOverReason>(&json).unwrap(),
            reason
        );
    }
}
