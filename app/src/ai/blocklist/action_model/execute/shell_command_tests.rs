use std::sync::Arc;

use async_channel::unbounded;
use parking_lot::FairMutex;
use warp_terminal::event::ObservedExitStatus;
use warpui::{App, EntityId, ModelHandle};

use super::{
    ActionResult, AnyActionExecution, BlockSelector, BlockWaitEvent, ExecuteActionInput,
    ShellCommandExecutor, ShellRecoveryResult, action_result_for_read_shell_command_output,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    ReadShellCommandOutputResult, ShellCommandDelay,
};
use crate::terminal::event::{BlockMetadataReceivedEvent, BlockWorkingDirectoryUpdatedEvent};
use crate::terminal::model::block::{BlockId, BlockMetadata};
use crate::terminal::model::session::Sessions;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::terminal_model::{BlockIndex, TerminalModel};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
fn shell_command_executor(app: &mut App) -> ModelHandle<ShellCommandExecutor> {
    let terminal_view_id = EntityId::new();
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_model_events_tx, model_events_rx) = unbounded();
    let model_event_dispatcher =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let active_session =
        app.add_model(|ctx| ActiveSession::new(sessions, model_event_dispatcher.clone(), ctx));
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    app.add_model(|ctx| {
        ShellCommandExecutor::new(
            active_session,
            terminal_model,
            &model_event_dispatcher,
            terminal_view_id,
            ctx,
        )
    })
}

fn read_shell_command_output(
    executor: &ModelHandle<ShellCommandExecutor>,
    block_id: &BlockId,
    app: &mut App,
) -> AIAgentActionResultType {
    let action = AIAgentAction {
        id: AIAgentActionId::from(format!("read-{block_id}")),
        task_id: TaskId::new("read-recovered-shell".to_owned()),
        action: AIAgentActionType::ReadShellCommandOutput {
            block_id: block_id.clone(),
            delay: Some(ShellCommandDelay::OnCompletion),
        },
        requires_result: true,
    };
    let execution: AnyActionExecution = executor.update(app, |executor, ctx| {
        executor
            .execute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id: AIConversationId::new(),
                },
                ctx,
            )
            .into()
    });
    let AnyActionExecution::Sync(result) = execution else {
        panic!("expected synchronous recovered shell read");
    };
    result
}
#[test]
fn shell_recovery_persists_result_until_later_read_and_consumes_it_once() {
    App::test((), |mut app| async move {
        let executor = shell_command_executor(&mut app);
        let action_id: AIAgentActionId = "recover-after-snapshot".to_owned().into();
        let block_id = BlockId::new();

        assert!(executor.update(&mut app, |executor, _| {
            executor.begin_shell_recovery(&action_id, &block_id)
        }));
        executor.update(&mut app, |executor, _| {
            executor.finish_shell_recovery(ShellRecoveryResult {
                block_id: block_id.clone(),
                output: "replacement ready".to_owned(),
                status: ObservedExitStatus::Unavailable,
                restored_working_directory: "/home/agent".to_owned(),
                used_fallback_directory: false,
                start_ts: None,
                completed_ts: None,
            });
        });

        let first = read_shell_command_output(&executor, &block_id, &mut app);
        assert!(matches!(
            first,
            AIAgentActionResultType::ReadShellCommandOutput(
                ReadShellCommandOutputResult::ShellRecovered {
                    status: ObservedExitStatus::Unavailable,
                    ..
                }
            )
        ));
        assert!(matches!(
            read_shell_command_output(&executor, &block_id, &mut app),
            AIAgentActionResultType::ReadShellCommandOutput(ReadShellCommandOutputResult::Error(
                crate::ai::agent::ShellCommandError::BlockNotFound
            ))
        ));
    });
}

/// Locks in the contract that `ShellCommandExecutor`'s requested-command finish
/// detector reacts only to `BlockMetadataReceived` (precmd) and not to
/// `BlockWorkingDirectoryUpdated` (OSC 7). The detector relies on
/// `BlockMetadataReceived` firing exactly once per block; OSC 7 can fire many
/// times per block, so wiring it into the detector would resolve the wait
/// future before the requested command actually finishes.
#[test]
fn block_working_directory_updated_does_not_drain_finish_senders() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let (_model_events_tx, model_events_rx) = unbounded();
        let model_event_dispatcher =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        let active_session = app.add_model(|ctx| {
            ActiveSession::new(sessions.clone(), model_event_dispatcher.clone(), ctx)
        });
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let executor = app.add_model(|ctx| {
            ShellCommandExecutor::new(
                active_session,
                terminal_model.clone(),
                &model_event_dispatcher,
                terminal_view_id,
                ctx,
            )
        });

        let block_id = BlockId::new();
        let selector = BlockSelector::Id(block_id);
        let (tx, _rx) = async_channel::bounded::<BlockWaitEvent>(1);
        executor.update(&mut app, |executor, _ctx| {
            executor.block_finished_senders.insert(selector, tx);
        });
        assert_eq!(
            app.read(|ctx| executor.as_ref(ctx).block_finished_senders.len()),
            1
        );

        // OSC 7 update — must NOT drain or resolve the finish sender.
        model_event_dispatcher.update(&mut app, |_dispatcher, ctx| {
            ctx.emit(ModelEvent::BlockWorkingDirectoryUpdated(
                BlockWorkingDirectoryUpdatedEvent {
                    block_metadata: BlockMetadata::new(None, Some("/tmp/new".to_string())),
                    block_index: BlockIndex::zero(),
                    is_for_in_band_command: false,
                    is_done_bootstrapping: true,
                },
            ));
        });
        assert_eq!(
            app.read(|ctx| executor.as_ref(ctx).block_finished_senders.len()),
            1,
            "BlockWorkingDirectoryUpdated must not touch block_finished_senders — \
             that map is reserved for precmd (BlockMetadataReceived)"
        );

        // Precmd event — the senders map should be drained (and since the
        // block isn't in the terminal model, the sender is dropped).
        model_event_dispatcher.update(&mut app, |_dispatcher, ctx| {
            ctx.emit(ModelEvent::BlockMetadataReceived(
                BlockMetadataReceivedEvent {
                    block_metadata: BlockMetadata::new(None, Some("/tmp/precmd".to_string())),
                    block_index: BlockIndex::zero(),
                    is_after_in_band_command: false,
                    is_done_bootstrapping: true,
                },
            ));
        });
        assert_eq!(
            app.read(|ctx| executor.as_ref(ctx).block_finished_senders.len()),
            0,
            "BlockMetadataReceived should drain the finish senders"
        );
    });
}

#[test]
fn shell_recovery_rebinds_requested_command_waiter() {
    App::test((), |mut app| async move {
        let executor = shell_command_executor(&mut app);
        let action_id: AIAgentActionId = "recover-shell".to_owned().into();
        let block_id = BlockId::new();
        let selector = BlockSelector::RequestedCommandId(action_id.clone());
        let (tx, rx) = async_channel::bounded(2);
        executor.update(&mut app, |executor, _| {
            executor.block_finished_senders.insert(selector, tx);
        });

        assert!(executor.update(&mut app, |executor, _| {
            executor.begin_shell_recovery(&action_id, &block_id)
        }));
        assert!(matches!(rx.try_recv(), Ok(BlockWaitEvent::RecoveryStarted)));

        executor.update(&mut app, |executor, _| {
            executor.finish_shell_recovery(ShellRecoveryResult {
                block_id: block_id.clone(),
                output: "replacement ready".to_owned(),
                status: ObservedExitStatus::Signal(9),
                restored_working_directory: "/home/agent".to_owned(),
                used_fallback_directory: true,
                start_ts: None,
                completed_ts: None,
            });
        });
        let BlockWaitEvent::Recovered(result) =
            rx.try_recv().expect("recovery result should be delivered")
        else {
            panic!("expected recovered shell result");
        };
        assert_eq!(result.block_id, block_id);
        assert_eq!(result.status, ObservedExitStatus::Signal(9));
    });
}

#[test]
fn shell_recovery_rebinds_follow_up_read_waiter_after_initial_snapshot() {
    App::test((), |mut app| async move {
        let executor = shell_command_executor(&mut app);
        let action_id: AIAgentActionId = "recover-shell".to_owned().into();
        let block_id = BlockId::new();
        let selector = BlockSelector::Id(block_id.clone());
        let (tx, rx) = async_channel::bounded(2);
        executor.update(&mut app, |executor, _| {
            executor.block_finished_senders.insert(selector, tx);
        });

        assert!(executor.update(&mut app, |executor, _| {
            executor.begin_shell_recovery(&action_id, &block_id)
        }));
        assert!(matches!(rx.try_recv(), Ok(BlockWaitEvent::RecoveryStarted)));

        executor.update(&mut app, |executor, _| {
            executor.finish_shell_recovery(ShellRecoveryResult {
                block_id: block_id.clone(),
                output: "replacement ready".to_owned(),
                status: ObservedExitStatus::Code(7),
                restored_working_directory: "/worktree".to_owned(),
                used_fallback_directory: false,
                start_ts: None,
                completed_ts: None,
            });
        });
        let BlockWaitEvent::Recovered(result) = rx
            .try_recv()
            .expect("follow-up read should receive recovery")
        else {
            panic!("expected recovered shell result");
        };
        assert_eq!(result.block_id, block_id);
        assert_eq!(result.status, ObservedExitStatus::Code(7));
    });
}

#[test]
fn shell_recovery_resolves_exactly_one_matching_waiter() {
    App::test((), |mut app| async move {
        let executor = shell_command_executor(&mut app);
        let action_id: AIAgentActionId = "recover-shell".to_owned().into();
        let block_id = BlockId::new();
        let (requested_tx, requested_rx) = async_channel::bounded(2);
        let (read_tx, read_rx) = async_channel::bounded(2);
        executor.update(&mut app, |executor, _| {
            executor.block_finished_senders.insert(
                BlockSelector::RequestedCommandId(action_id.clone()),
                requested_tx,
            );
            executor
                .block_finished_senders
                .insert(BlockSelector::Id(block_id.clone()), read_tx);
        });

        assert!(executor.update(&mut app, |executor, _| {
            executor.begin_shell_recovery(&action_id, &block_id)
        }));
        assert!(matches!(
            requested_rx.try_recv(),
            Ok(BlockWaitEvent::RecoveryStarted)
        ));
        assert!(read_rx.try_recv().is_err());

        executor.update(&mut app, |executor, _| {
            executor.finish_shell_recovery(ShellRecoveryResult {
                block_id,
                output: "replacement ready".to_owned(),
                status: ObservedExitStatus::Unavailable,
                restored_working_directory: "/home/agent".to_owned(),
                used_fallback_directory: true,
                start_ts: None,
                completed_ts: None,
            });
        });
        assert!(matches!(
            requested_rx.try_recv(),
            Ok(BlockWaitEvent::Recovered(_))
        ));
        assert!(read_rx.try_recv().is_err());
    });
}

#[test]
fn shell_recovery_follow_up_read_returns_stable_recovery_output() {
    let block_id = BlockId::new();
    let result = action_result_for_read_shell_command_output(
        "exit 7".to_owned(),
        ActionResult::ShellRecovered(ShellRecoveryResult {
            block_id: block_id.clone(),
            output: "replacement ready".to_owned(),
            status: ObservedExitStatus::Code(7),
            restored_working_directory: "/worktree".to_owned(),
            used_fallback_directory: false,
            start_ts: None,
            completed_ts: None,
        }),
    );

    let AIAgentActionResultType::ReadShellCommandOutput(
        ReadShellCommandOutputResult::ShellRecovered {
            block_id: recovered_block_id,
            output,
            status,
            ..
        },
    ) = result
    else {
        panic!("expected recovered follow-up read result");
    };
    assert_eq!(recovered_block_id, block_id);
    assert_eq!(output, "replacement ready");
    assert_eq!(status, ObservedExitStatus::Code(7));
}
