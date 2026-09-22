use std::sync::Arc;

use parking_lot::{FairMutex, Mutex};
use warpui::App;

use super::*;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::StartCommandOutcome;
use crate::terminal::model::ansi::{Handler, PreexecValue};
use crate::terminal::model::session::{SessionId, SessionInfo, Sessions};

#[derive(Clone, Default)]
struct TestEventLoopSender {
    messages: Arc<Mutex<Vec<Message>>>,
}

impl EventLoopSender for TestEventLoopSender {
    fn send(&self, message: Message) -> Result<(), EventLoopSendError> {
        self.messages.lock().push(message);
        Ok(())
    }
}

fn terminal_model() -> Arc<FairMutex<TerminalModel>> {
    Arc::new(FairMutex::new(TerminalModel::mock(
        None,
        Some(ChannelEventListener::new_for_test()),
    )))
}

#[test]
fn rejected_and_coalesced_starts_do_not_mutate_controller_or_write_bytes() {
    App::test((), |mut app| async move {
        let model = terminal_model();
        let (model_events_tx, model_events_rx) = async_channel::unbounded();
        let (_executor_command_tx, executor_command_rx) = async_channel::unbounded();
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let model_events =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        let line_editor_status =
            app.add_model(|ctx| LineEditorStatus::new(model_events.clone(), sessions.clone(), ctx));
        let sender = TestEventLoopSender::default();
        let controller = app.add_model(|ctx| {
            PtyController::new(
                sender.clone(),
                model_events,
                line_editor_status,
                sessions,
                executor_command_rx,
                model.clone(),
                ctx,
            )
        });
        controller.update(&mut app, |controller, _| {
            controller.pending_writes.push_back(PtyWrite::Bytes {
                bytes: b"existing-pending-write".to_vec().into(),
            });
        });

        assert_eq!(
            model.lock().start_command_execution(),
            StartCommandOutcome::Accepted
        );
        let coalesced = controller.update(&mut app, |controller, ctx| {
            controller.write_command(
                "coalesced",
                ShellType::Zsh,
                CommandExecutionSource::User,
                ctx,
            )
        });
        assert_eq!(coalesced, StartCommandOutcome::Coalesced);
        controller.read(&app, |controller, _| {
            assert!(!controller.is_user_command_executing);
            assert_eq!(controller.pending_writes.len(), 1);
        });
        assert!(sender.messages.lock().is_empty());

        model.lock().preexec(PreexecValue {
            command: "running".to_owned(),
            session_id: None,
        });
        let rejected = controller.update(&mut app, |controller, ctx| {
            controller.write_command(
                "rejected",
                ShellType::Zsh,
                CommandExecutionSource::User,
                ctx,
            )
        });
        assert_eq!(rejected, StartCommandOutcome::RejectedExecuting);
        controller.read(&app, |controller, _| {
            assert!(!controller.is_user_command_executing);
            assert_eq!(controller.pending_writes.len(), 1);
        });
        assert!(sender.messages.lock().is_empty());

        drop(model_events_tx);
    });
}

#[test]
fn native_shell_completions_queues_the_generator_command_for_the_active_sessions_shell() {
    App::test((), |mut app| async move {
        let model = terminal_model();
        let (model_events_tx, model_events_rx) = async_channel::unbounded();
        let (_executor_command_tx, executor_command_rx) = async_channel::unbounded();
        let mut sessions = Sessions::new_for_test();
        let session_id = SessionId::from(42);
        sessions.register_session_for_test(
            SessionInfo::new_for_test()
                .with_id(session_id)
                .with_shell_type(ShellType::Fish),
        );
        let sessions = app.add_model(|_| sessions);
        let model_events = app.add_model(|ctx| {
            let mut dispatcher = ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx);
            dispatcher.set_active_session_id(session_id);
            dispatcher
        });
        let line_editor_status =
            app.add_model(|ctx| LineEditorStatus::new(model_events.clone(), sessions.clone(), ctx));
        let sender = TestEventLoopSender::default();
        let controller = app.add_model(|ctx| {
            PtyController::new(
                sender.clone(),
                model_events,
                line_editor_status,
                sessions,
                executor_command_rx,
                model,
                ctx,
            )
        });

        let (results_tx, _results_rx) = async_channel::unbounded();
        controller.update(&mut app, |controller, ctx| {
            controller.run_native_shell_completions("git ch".to_owned(), results_tx, ctx);
        });

        // The line editor isn't active by default, so the write should still be queued rather
        // than sent to the event loop.
        assert!(sender.messages.lock().is_empty());
        controller.read(&app, |controller, _| {
            assert_eq!(controller.pending_writes.len(), 1);
            let Some(PtyWrite::RunNativeShellCompletions {
                command,
                shell_type,
                ..
            }) = controller.pending_writes.front()
            else {
                panic!("expected a queued RunNativeShellCompletions write");
            };
            assert_eq!(*shell_type, ShellType::Fish);
            assert_eq!(
                command,
                " warp_run_generator_command_native_completions 676974206368"
            );
        });

        drop(model_events_tx);
    });
}

#[test]
fn native_shell_completions_reports_no_matches_without_an_active_session() {
    App::test((), |mut app| async move {
        let model = terminal_model();
        let (model_events_tx, model_events_rx) = async_channel::unbounded();
        let (_executor_command_tx, executor_command_rx) = async_channel::unbounded();
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let model_events =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        let line_editor_status =
            app.add_model(|ctx| LineEditorStatus::new(model_events.clone(), sessions.clone(), ctx));
        let sender = TestEventLoopSender::default();
        let controller = app.add_model(|ctx| {
            PtyController::new(
                sender.clone(),
                model_events,
                line_editor_status,
                sessions,
                executor_command_rx,
                model,
                ctx,
            )
        });

        let (results_tx, results_rx) = async_channel::unbounded();
        controller.update(&mut app, |controller, ctx| {
            controller.run_native_shell_completions("git ch".to_owned(), results_tx, ctx);
        });

        let (completions, replacement_span) = results_rx
            .try_recv()
            .expect("should immediately receive empty results");
        assert!(completions.is_empty());
        assert!(replacement_span.is_none());
        controller.read(&app, |controller, _| {
            assert!(controller.pending_writes.is_empty());
        });
        assert!(sender.messages.lock().is_empty());

        drop(model_events_tx);
    });
}

#[test]
fn rejected_queued_in_band_start_is_cancelled_without_writing_bytes() {
    App::test((), |mut app| async move {
        let model = terminal_model();
        model.lock().start_command_execution();
        model.lock().preexec(PreexecValue {
            command: "running".to_owned(),
            session_id: None,
        });

        let (model_events_tx, model_events_rx) = async_channel::unbounded();
        let (_executor_command_tx, executor_command_rx) = async_channel::unbounded();
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let model_events =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        let line_editor_status =
            app.add_model(|ctx| LineEditorStatus::new(model_events.clone(), sessions.clone(), ctx));
        let sender = TestEventLoopSender::default();
        let controller = app.add_model(|ctx| {
            PtyController::new(
                sender.clone(),
                model_events,
                line_editor_status.clone(),
                sessions,
                executor_command_rx,
                model.clone(),
                ctx,
            )
        });
        let (cancel_tx, cancel_rx) = async_channel::unbounded();

        controller.update(&mut app, |controller, ctx| {
            controller.queue_in_band_command(
                "rejected-in-band",
                ShellType::Zsh,
                "command-id".to_owned(),
                cancel_tx,
                ctx,
            );
            let write = controller
                .pending_writes
                .pop_front()
                .expect("The inactive line editor should leave the in-band command queued.");
            assert!(!controller.send_write_to_event_loop(write, ctx));
        });

        assert_eq!(
            cancel_rx
                .try_recv()
                .expect("The rejected in-band command should be cancelled.")
                .command_id,
            "command-id"
        );
        assert!(sender.messages.lock().is_empty());
        line_editor_status.read(&app, |line_editor_status, _| {
            assert!(!line_editor_status.is_line_editor_active());
        });
        drop(model_events_tx);
    });
}
