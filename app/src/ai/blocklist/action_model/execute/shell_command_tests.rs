use std::sync::Arc;

use async_channel::unbounded;
use futures::channel::oneshot;
use parking_lot::FairMutex;
use warpui::{App, Entity, EntityId};

use super::{BlockSelector, ShellCommandExecutor, ShellCommandExecutorEvent};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType};
use crate::terminal::event::{BlockMetadataReceivedEvent, BlockWorkingDirectoryUpdatedEvent};
use crate::terminal::model::block::{BlockId, BlockMetadata};
use crate::terminal::model::session::Sessions;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::terminal_model::{BlockIndex, TerminalModel};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
struct ShellCommandEventRecorder {
    event: Option<ShellCommandExecutorEvent>,
}

impl Entity for ShellCommandEventRecorder {
    type Event = ();
}

fn request_command_action(action_id: AIAgentActionId, command: &str) -> AIAgentAction {
    AIAgentAction {
        id: action_id,
        task_id: TaskId::new("fake-task".to_owned()),
        action: AIAgentActionType::RequestCommandOutput {
            command: command.to_owned(),
            is_read_only: None,
            is_risky: None,
            rationale: None,
            uses_pager: None,
            wait_until_completion: true,
            citations: vec![],
        },
        requires_result: false,
    }
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
        let (tx, _rx) = oneshot::channel::<()>();
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
fn execute_command_event_includes_source_conversation_id() {
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
        let recorder = app.add_model(|ctx| {
            ctx.subscribe_to_model(
                &executor,
                |recorder: &mut ShellCommandEventRecorder, event, _| {
                    recorder.event = Some(event.clone());
                },
            );
            ShellCommandEventRecorder { event: None }
        });

        let action_id = AIAgentActionId::from("requested-command".to_owned());
        let conversation_id = AIConversationId::new();
        let action = request_command_action(action_id.clone(), "pwd");
        executor.update(&mut app, |executor, ctx| {
            let _ = executor.execute(
                super::super::ExecuteActionInput {
                    action: &action,
                    conversation_id,
                },
                ctx,
            );
        });

        let event = recorder
            .read(&app, |recorder, _| recorder.event.clone())
            .expect("expected requested-command execution event");
        let ShellCommandExecutorEvent::ExecuteCommand {
            action_id: emitted_action_id,
            conversation_id: emitted_conversation_id,
            command,
        } = event
        else {
            panic!("expected ExecuteCommand event");
        };

        assert_eq!(emitted_action_id, action_id);
        assert_eq!(emitted_conversation_id, conversation_id);
        assert_eq!(command, "pwd");
    });
}
