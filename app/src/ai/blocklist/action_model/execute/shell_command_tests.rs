use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use async_channel::unbounded;
use futures::channel::oneshot;
use parking_lot::FairMutex;
use warpui::{App, EntityId};

use super::super::{AnyActionExecution, ExecuteActionInput};
use super::{BlockSelector, ShellCommandExecutor, ShellCommandExecutorEvent};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionResultType, AIAgentActionType, RequestCommandOutputResult,
};
use crate::ai::blocklist::action_model::recording_controller::RecordingController;
use crate::terminal::event::{BlockMetadataReceivedEvent, BlockWorkingDirectoryUpdatedEvent};
use crate::terminal::model::block::{BlockId, BlockMetadata};
use crate::terminal::model::session::Sessions;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::terminal_model::{BlockIndex, TerminalModel};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::test_util::terminal::initialize_app_for_terminal_view;

#[test]
fn terminal_busy_does_not_write_or_cancel_the_running_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(|ctx| {
            ctx.add_singleton_model(|_| RecordingController::new());
        });
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let (_tx, rx) = unbounded();
        let dispatcher = app.add_model(|ctx| ModelEventDispatcher::new(rx, sessions.clone(), ctx));
        let active_session =
            app.add_model(|ctx| ActiveSession::new(sessions, dispatcher.clone(), ctx));
        let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        model.lock().simulate_long_running_block("lint", "working");
        let block_id = model.lock().active_block_id().clone();
        let executor = app.add_model(|ctx| {
            ShellCommandExecutor::new(
                active_session,
                model.clone(),
                &dispatcher,
                EntityId::new(),
                ctx,
            )
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&executor, move |_, event: &ShellCommandExecutorEvent, _| {
                observed.borrow_mut().push(event.clone());
            });
        });
        let action = AIAgentAction {
            id: "new-command".to_owned().into(),
            task_id: TaskId::new("root".into()),
            requires_result: true,
            action: AIAgentActionType::RequestCommandOutput {
                command: "ls".into(),
                is_read_only: Some(true),
                is_risky: Some(false),
                wait_until_completion: true,
                uses_pager: None,
                rationale: None,
                citations: vec![],
            },
        };
        let execution: AnyActionExecution = executor.update(&mut app, |executor, ctx| {
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
            panic!("busy terminal must synchronously return an error");
        };
        assert!(
            matches!(&result, AIAgentActionResultType::RequestCommandOutput(
            RequestCommandOutputResult::TerminalBusy { block_id: active, command }
        ) if active == &block_id && command == "ls")
        );
        assert!(result.should_trigger_request_upon_completion());
        assert!(!result.is_cancelled());
        assert!(events.borrow().is_empty());
        model
            .lock()
            .block_list_mut()
            .set_active_conversation_context(AIConversationId::new(), false, false);
        let displaced: AnyActionExecution = executor.update(&mut app, |executor, ctx| {
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
        assert!(matches!(
            displaced,
            AnyActionExecution::Sync(AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::CancelledBeforeExecution,
            ),)
        ));
        assert!(events.borrow().is_empty());
        model
            .lock()
            .block_list_mut()
            .clear_active_conversation_context();
        {
            let model = model.lock();
            assert_eq!(model.active_block_id(), &block_id);
            assert!(model.block_list().active_block().is_executing());
        }
        model.lock().finish_block();
        let execution: AnyActionExecution = executor.update(&mut app, |executor, ctx| {
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
        assert!(matches!(execution, AnyActionExecution::Async { .. }));
        assert!(matches!(events.borrow().as_slice(),
            [ShellCommandExecutorEvent::ExecuteCommand { command, .. }] if command == "ls"));
    });
}

#[test]
fn injection_interrupt_keeps_completion_waiter_until_normal_precmd() {
    App::test((), |mut app| async move {
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let (_tx, rx) = unbounded();
        let dispatcher = app.add_model(|ctx| ModelEventDispatcher::new(rx, sessions.clone(), ctx));
        let active_session =
            app.add_model(|ctx| ActiveSession::new(sessions, dispatcher.clone(), ctx));
        let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        model.lock().simulate_long_running_block("lint", "working");
        let block_id = model.lock().active_block_id().clone();
        let executor = app.add_model(|ctx| {
            ShellCommandExecutor::new(
                active_session,
                model.clone(),
                &dispatcher,
                EntityId::new(),
                ctx,
            )
        });
        let (tx, mut rx) = oneshot::channel();
        executor.update(&mut app, |executor, ctx| {
            executor
                .block_finished_senders
                .insert(BlockSelector::Id(block_id.clone()), tx);
            executor.interrupt_for_injected_followup(
                AIConversationId::new(),
                block_id.clone(),
                ctx,
            );
            assert!(
                executor
                    .block_finished_senders
                    .contains_key(&BlockSelector::Id(block_id.clone()))
            );
        });
        assert_eq!(rx.try_recv().unwrap(), None);
        model.lock().finish_block();
        dispatcher.update(&mut app, |_, ctx| {
            ctx.emit(ModelEvent::BlockMetadataReceived(
                BlockMetadataReceivedEvent {
                    block_metadata: BlockMetadata::new(None, Some("/tmp".into())),
                    block_index: BlockIndex::zero(),
                    is_after_in_band_command: false,
                    is_done_bootstrapping: true,
                },
            ));
        });
        assert_eq!(rx.try_recv().unwrap(), Some(()));
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
