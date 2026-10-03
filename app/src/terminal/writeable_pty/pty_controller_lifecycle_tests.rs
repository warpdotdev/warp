use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::{FairMutex, Mutex};
use warp_core::telemetry::testing::MockTelemetryContextProvider;
use warpui::App;
use warpui::r#async::FutureExt as _;

use super::*;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::StartCommandOutcome;
use crate::terminal::model::ansi::{Handler, PreexecValue, PromptMarker};
use crate::terminal::model::session::{SessionId, SessionInfo, Sessions};
use crate::test_util::assert_eventually;

#[derive(Clone, Default)]
struct TestEventLoopSender {
    messages: Arc<Mutex<Vec<Message>>>,
}
impl TestEventLoopSender {
    fn written_bytes(&self) -> Vec<u8> {
        self.messages
            .lock()
            .iter()
            .filter_map(|message| match message {
                Message::Input(bytes) => Some(&bytes[..]),
                _ => None,
            })
            .flatten()
            .copied()
            .collect()
    }
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

struct NativeCompletionController {
    controller: ModelHandle<PtyController<TestEventLoopSender>>,
    model: Arc<FairMutex<TerminalModel>>,
    sender: TestEventLoopSender,
    model_events: ModelHandle<ModelEventDispatcher>,
}

fn native_completion_controller(app: &mut App) -> NativeCompletionController {
    let (model_events_tx, model_events_rx) = async_channel::unbounded();
    let event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(model_events_tx)
        .build();
    let model = Arc::new(FairMutex::new(TerminalModel::mock(None, Some(event_proxy))));
    while model_events_rx.try_recv().is_ok() {}
    let (_executor_command_tx, executor_command_rx) = async_channel::unbounded();
    let sessions = app.add_model(|_| {
        let mut sessions = Sessions::new_for_test();
        sessions
            .register_session_for_test(SessionInfo::new_for_test().with_shell_type(ShellType::Zsh));
        sessions
    });
    let model_events = app.add_model(|ctx| {
        let mut dispatcher = ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx);
        dispatcher.set_active_session_id(SessionInfo::new_for_test().session_id);
        dispatcher
    });
    let line_editor_status =
        app.add_model(|ctx| LineEditorStatus::new(model_events.clone(), sessions.clone(), ctx));
    let sender = TestEventLoopSender::default();
    let controller = app.add_model(|ctx| {
        PtyController::new(
            sender.clone(),
            model_events.clone(),
            line_editor_status,
            sessions,
            executor_command_rx,
            model.clone(),
            ctx,
        )
    });
    NativeCompletionController {
        controller,
        model,
        sender,
        model_events,
    }
}

fn native_completion_prompt(model: &mut TerminalModel) {
    model.simulate_cmd("initial prompt");
    model.finish_block();
    model.prompt_marker(PromptMarker::EndPrompt);
}

#[test]
fn retired_native_completion_cannot_deliver_a_late_reply_to_another_request() {
    App::test((), |mut app| async move {
        app.update(MockTelemetryContextProvider::register);
        let NativeCompletionController {
            controller,
            model,
            sender,
            model_events,
        } = native_completion_controller(&mut app);
        let NativeCompletionController {
            controller: other_controller,
            model: other_model,
            sender: other_sender,
            ..
        } = native_completion_controller(&mut app);
        let late_reply_dispatched = Rc::new(Cell::new(false));
        let command_finished = Rc::new(Cell::new(false));
        let late_reply_dispatched_for_subscription = late_reply_dispatched.clone();
        let command_finished_for_subscription = command_finished.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&model_events, move |_, event, _| match event {
                ModelEvent::CompletionsFinished(..) => {
                    late_reply_dispatched_for_subscription.set(true);
                }
                ModelEvent::Handler(AnsiHandlerEvent::InBandCommandFinished) => {
                    command_finished_for_subscription.set(true);
                }
                _ => (),
            });
        });
        let (first_tx, first_rx) = async_channel::unbounded();
        controller.update(&mut app, |controller, ctx| {
            controller.run_native_shell_completions("first ".to_owned(), first_tx, ctx);
        });
        assert!(sender.written_bytes().is_empty());
        native_completion_prompt(&mut model.lock());
        assert_eventually!(
            String::from_utf8_lossy(&sender.written_bytes())
                .contains("warp_run_generator_command_native_completions 666972737420"),
            "the prompt should dispatch the first native command"
        );
        sender.messages.lock().clear();
        first_rx.close();

        let (second_tx, second_rx) = async_channel::unbounded();
        controller.update(&mut app, |controller, ctx| {
            controller.run_native_shell_completions("second ".to_owned(), second_tx, ctx);
        });
        assert!(second_rx.is_closed());
        assert!(sender.messages.lock().is_empty());
        model
            .lock()
            .process_bytes(b"\x1b]9280;A\x07\x1b]9280;C;7374616c65\x07\x1b]9280;B\x07" as &[u8]);
        assert_eventually!(
            late_reply_dispatched.get(),
            "the late completion event should reach the controller before the next request"
        );
        let (blocked_tx, blocked_rx) = async_channel::unbounded();
        controller.update(&mut app, |controller, ctx| {
            controller.run_native_shell_completions("still blocked ".to_owned(), blocked_tx, ctx);
        });
        assert!(blocked_rx.is_closed());
        assert!(sender.written_bytes().is_empty());

        let (other_tx, other_rx) = async_channel::unbounded();
        other_controller.update(&mut app, |controller, ctx| {
            controller.run_native_shell_completions("other ".to_owned(), other_tx, ctx);
        });
        native_completion_prompt(&mut other_model.lock());
        assert_eventually!(
            String::from_utf8_lossy(&other_sender.written_bytes())
                .contains("warp_run_generator_command_native_completions 6f7468657220"),
            "quarantine should not block another PTY"
        );
        assert!(!other_rx.is_closed());

        model
            .lock()
            .process_bytes(b"\x1b]9280;A\x07\x1b]9280;C;7374616c65\x07" as &[u8]);
        model.lock().finish_block();
        assert_eventually!(
            command_finished.get(),
            "the completed native command should release quarantine"
        );

        let (fresh_tx, fresh_rx) = async_channel::unbounded();
        controller.update(&mut app, |controller, ctx| {
            controller.run_native_shell_completions("fresh ".to_owned(), fresh_tx, ctx);
        });
        assert!(sender.written_bytes().is_empty());
        model.lock().process_bytes(b"\x1b]9280;B\x07" as &[u8]);
        model.lock().prompt_marker(PromptMarker::EndPrompt);
        assert_eventually!(
            String::from_utf8_lossy(&sender.written_bytes())
                .contains("warp_run_generator_command_native_completions 667265736820"),
            "the next prompt should dispatch the fresh native command"
        );
        model
            .lock()
            .process_bytes(b"\x1b]9280;A\x07\x1b]9280;C;6672657368\x07\x1b]9280;B\x07" as &[u8]);
        let (completions, span) = fresh_rx
            .recv()
            .with_timeout(std::time::Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        let completions = completions
            .into_iter()
            .map(|completion| {
                warp_completer::completer::MatchedSuggestion::from(completion)
                    .suggestion
                    .display
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(completions, vec!["fresh"]);
        assert_eq!(span, None);
    });
}

#[test]
fn native_command_finished_without_a_reply_closes_the_receiver_and_allows_another_request() {
    App::test((), |mut app| async move {
        app.update(MockTelemetryContextProvider::register);
        let NativeCompletionController {
            controller,
            model,
            sender,
            ..
        } = native_completion_controller(&mut app);
        let (first_tx, first_rx) = async_channel::unbounded();
        controller.update(&mut app, |controller, ctx| {
            controller.run_native_shell_completions("first ".to_owned(), first_tx, ctx);
        });
        native_completion_prompt(&mut model.lock());
        assert_eventually!(
            String::from_utf8_lossy(&sender.written_bytes())
                .contains("warp_run_generator_command_native_completions 666972737420"),
            "the prompt should dispatch the first native command"
        );
        model.lock().finish_block();
        assert_eventually!(
            first_rx.is_closed(),
            "the missing response should not wait after command completion"
        );

        let (second_tx, second_rx) = async_channel::unbounded();
        controller.update(&mut app, |controller, ctx| {
            controller.run_native_shell_completions("second ".to_owned(), second_tx, ctx);
        });
        first_rx.close();
        model.lock().prompt_marker(PromptMarker::EndPrompt);
        assert_eventually!(
            String::from_utf8_lossy(&sender.written_bytes())
                .contains("warp_run_generator_command_native_completions 7365636f6e6420"),
            "a completed command should not leave the next request quarantined"
        );
        model
            .lock()
            .process_bytes(b"\x1b]9280;A\x07\x1b]9280;C;76616c6964\x07\x1b]9280;B\x07" as &[u8]);
        let (completions, _) = second_rx
            .recv()
            .with_timeout(std::time::Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completions.len(), 1);
    });
}

#[test]
fn retired_queued_native_completion_does_not_write_to_the_pty() {
    App::test((), |mut app| async move {
        app.update(MockTelemetryContextProvider::register);
        let NativeCompletionController {
            controller,
            model,
            sender,
            ..
        } = native_completion_controller(&mut app);
        let (results_tx, results_rx) = async_channel::unbounded();
        controller.update(&mut app, |controller, ctx| {
            controller.run_native_shell_completions("expired ".to_owned(), results_tx, ctx);
        });
        results_rx.close();
        native_completion_prompt(&mut model.lock());
        assert_eventually!(
            controller.read(&app, |controller, ctx| controller
                .line_editor_status
                .as_ref(ctx)
                .is_line_editor_active()),
            "the prompt should drain expired native writes without consuming line-editor readiness"
        );
        assert!(
            !String::from_utf8_lossy(&sender.written_bytes())
                .contains("warp_run_generator_command_native_completions")
        );

        let (fresh_tx, fresh_rx) = async_channel::unbounded();
        controller.update(&mut app, |controller, ctx| {
            controller.run_native_shell_completions("fresh ".to_owned(), fresh_tx, ctx);
        });
        assert!(!fresh_rx.is_closed());
        assert!(
            String::from_utf8_lossy(&sender.written_bytes())
                .contains("warp_run_generator_command_native_completions 667265736820")
        );
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
