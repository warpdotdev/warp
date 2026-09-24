use std::sync::Arc;

use async_channel::unbounded;
use parking_lot::FairMutex;
use warp_terminal::event::ObservedExitStatus;
use warpui::{App, EntityId};

use super::{BlockSelector, BlockWaitEvent, ShellCommandExecutor, ShellRecoveryResult};
use crate::ai::agent::AIAgentActionId;
use crate::terminal::event::{BlockMetadataReceivedEvent, BlockWorkingDirectoryUpdatedEvent};
use crate::terminal::model::block::{BlockId, BlockMetadata};
use crate::terminal::model::session::Sessions;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::terminal_model::{BlockIndex, TerminalModel};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};

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
                terminal_model,
                &model_event_dispatcher,
                terminal_view_id,
                ctx,
            )
        });
        let action_id: AIAgentActionId = "recover-shell".to_owned().into();
        let selector = BlockSelector::RequestedCommandId(action_id.clone());
        let (tx, rx) = async_channel::bounded(2);
        executor.update(&mut app, |executor, _| {
            executor.block_finished_senders.insert(selector, tx);
        });

        assert!(executor.update(&mut app, |executor, _| {
            executor.begin_shell_recovery(&action_id)
        }));
        assert!(matches!(rx.try_recv(), Ok(BlockWaitEvent::RecoveryStarted)));

        let block_id = BlockId::new();
        executor.update(&mut app, |executor, _| {
            executor.finish_shell_recovery(
                &action_id,
                ShellRecoveryResult {
                    block_id: block_id.clone(),
                    output: "replacement ready".to_owned(),
                    status: ObservedExitStatus::Signal(9),
                    restored_working_directory: "/home/agent".to_owned(),
                    used_fallback_directory: true,
                    start_ts: None,
                    completed_ts: None,
                },
            );
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
