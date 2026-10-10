use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use async_channel::unbounded;
use futures::channel::oneshot;
use futures_lite::future::poll_once;
use parking_lot::FairMutex;
use warp_terminal::event::ObservedExitStatus;
use warpui::{App, EntityId, ModelHandle};

use super::{
    ActionResult, AnyActionExecution, BlockSelector, ExecuteActionInput, ShellCommandExecutor,
    ShellCommandExecutorEvent, ShellRecoveryResult,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    ReadShellCommandOutputResult, RequestCommandOutputResult, ShellCommandDelay, ShellCommandError,
    TransferShellCommandControlToUserResult,
};
use crate::ai::blocklist::action_model::recording_controller::RecordingController;
use crate::terminal::event::{BlockMetadataReceivedEvent, BlockWorkingDirectoryUpdatedEvent};
use crate::terminal::model::block::{BlockId, BlockMetadata, BlockState, CURSOR_MARKER};
use crate::terminal::model::session::Sessions;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::terminal_model::{BlockIndex, TerminalModel};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::test_util::terminal::initialize_app_for_terminal_view;

#[test]
fn polling_snapshot_exposes_continuation_prompt_and_cursor() {
    App::test((), |mut app| async move {
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let (_tx, rx) = unbounded();
        let dispatcher = app.add_model(|ctx| ModelEventDispatcher::new(rx, sessions.clone(), ctx));
        let active_session =
            app.add_model(|ctx| ActiveSession::new(sessions, dispatcher.clone(), ctx));
        let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let block_id = {
            let mut model = model.lock();
            model.block_list_mut().active_block_mut().start();
            model.process_bytes("echo \"unterminated\r\ndquote> ");
            assert_eq!(
                model.block_list().active_block().state(),
                BlockState::BeforeExecution
            );
            model.active_block_id().clone()
        };
        let executor = app.add_model(|ctx| {
            ShellCommandExecutor::new(active_session, model, &dispatcher, EntityId::new(), ctx)
        });

        let action = AIAgentAction {
            id: "read-output".to_owned().into(),
            task_id: TaskId::new("root".into()),
            requires_result: true,
            action: AIAgentActionType::ReadShellCommandOutput {
                block_id,
                delay: Some(ShellCommandDelay::Duration(Duration::ZERO)),
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
        let AnyActionExecution::Async {
            execute_future,
            on_complete,
        } = execution
        else {
            panic!("polling an incomplete quote must wait for a snapshot");
        };
        let snapshot = execute_future.await;
        let result = app.update(|ctx| on_complete(snapshot, ctx));

        let AIAgentActionResultType::ReadShellCommandOutput(
            ReadShellCommandOutputResult::LongRunningCommandSnapshot {
                grid_contents,
                cursor,
                ..
            },
        ) = result
        else {
            panic!("an incomplete quote must produce a long-running snapshot");
        };
        assert_eq!(grid_contents, "echo \"unterminated\ndquote> <|cursor|>");
        assert_eq!(cursor, CURSOR_MARKER);
    });
}

#[test]
fn control_handback_snapshot_exposes_continuation_prompt_and_cursor() {
    App::test((), |mut app| async move {
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let (_tx, rx) = unbounded();
        let dispatcher = app.add_model(|ctx| ModelEventDispatcher::new(rx, sessions.clone(), ctx));
        let active_session =
            app.add_model(|ctx| ActiveSession::new(sessions, dispatcher.clone(), ctx));
        let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        {
            let mut model = model.lock();
            model.block_list_mut().active_block_mut().start();
            model.process_bytes("echo \"unterminated\r\ndquote> ");
            model
                .block_list_mut()
                .active_block_mut()
                .set_was_long_running(true.into());
            assert_eq!(
                model.block_list().active_block().state(),
                BlockState::BeforeExecution
            );
        }
        let executor = app.add_model(|ctx| {
            ShellCommandExecutor::new(active_session, model, &dispatcher, EntityId::new(), ctx)
        });
        let action = AIAgentAction {
            id: "transfer-control".to_owned().into(),
            task_id: TaskId::new("root".into()),
            requires_result: true,
            action: AIAgentActionType::TransferShellCommandControlToUser {
                reason: "Complete the unterminated quote".to_owned(),
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
        let AnyActionExecution::Async {
            execute_future,
            on_complete,
        } = execution
        else {
            panic!("control transfer must wait for handback");
        };

        executor.update(&mut app, |executor, _| {
            executor.notify_control_handed_back()
        });
        let snapshot = execute_future.await;
        let result = app.update(|ctx| on_complete(snapshot, ctx));

        let AIAgentActionResultType::TransferShellCommandControlToUser(
            TransferShellCommandControlToUserResult::Snapshot {
                grid_contents,
                cursor,
                ..
            },
        ) = result
        else {
            panic!("handing back an incomplete quote must produce a snapshot");
        };
        assert_eq!(grid_contents, "echo \"unterminated\ndquote> <|cursor|>");
        assert_eq!(cursor, CURSOR_MARKER);
    });
}

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

fn shell_command_executor(
    app: &mut App,
) -> (
    ModelHandle<ShellCommandExecutor>,
    ModelHandle<ModelEventDispatcher>,
    Arc<FairMutex<TerminalModel>>,
) {
    let terminal_view_id = EntityId::new();
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_model_events_tx, model_events_rx) = unbounded();
    let model_event_dispatcher =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let active_session =
        app.add_model(|ctx| ActiveSession::new(sessions, model_event_dispatcher.clone(), ctx));
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
    (executor, model_event_dispatcher, terminal_model)
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
        let (executor, _dispatcher, _model) = shell_command_executor(&mut app);
        let action_id: AIAgentActionId = "recover-after-snapshot".to_owned().into();
        let block_id = BlockId::new();

        executor.update(&mut app, |executor, _| {
            executor.begin_shell_recovery(&action_id, &block_id);
        });
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
                ShellCommandError::BlockNotFound
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

/// The replacement shell's bootstrap precmd finds the interrupted block already finished, which
/// must not resolve the pending poll as a normal completion.
#[test]
fn shell_recovery_waits_through_bootstrap_before_resolving_follow_up_read() {
    App::test((), |mut app| async move {
        let (executor, dispatcher, model) = shell_command_executor(&mut app);
        let action_id: AIAgentActionId = "recover-shell".to_owned().into();
        model.lock().simulate_long_running_block("exit 7", "");
        let block_id = model.lock().active_block_id().clone();
        let mut result = Box::pin(executor.update(&mut app, |executor, ctx| {
            executor.action_result_future(
                BlockSelector::Id(block_id.clone()),
                Some(ShellCommandDelay::OnCompletion),
                ctx,
            )
        }));

        executor.update(&mut app, |executor, _| {
            executor.begin_shell_recovery(&action_id, &block_id);
        });
        assert!(poll_once(&mut result).await.is_none());

        model.lock().finish_block();
        dispatcher.update(&mut app, |_, ctx| {
            ctx.emit(ModelEvent::BlockMetadataReceived(
                BlockMetadataReceivedEvent {
                    block_metadata: BlockMetadata::new(None, Some("/worktree".to_owned())),
                    block_index: BlockIndex::zero(),
                    is_after_in_band_command: false,
                    is_done_bootstrapping: true,
                },
            ));
        });
        assert!(poll_once(&mut result).await.is_none());

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
        let ActionResult::ShellRecovered(result) = result.await else {
            panic!("expected recovered shell result");
        };
        assert_eq!(result.block_id, block_id);
        assert_eq!(result.status, ObservedExitStatus::Code(7));
    });
}
