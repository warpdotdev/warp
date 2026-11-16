use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt as _;
use futures::channel::oneshot;
use regex::Regex;
use serial_test::serial;
use session_sharing_protocol::sharer::SessionRetentionReason;
use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity as _};

use super::{ShareSessionError, TerminalDriver, WaitingCommand};
use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::terminal::input::CommandExecutionSource;
use crate::terminal::model::ansi::{Handler, PreexecValue};
use crate::terminal::model::block::{BlockId, BlockState, SerializedBlock};
use crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes;
use crate::terminal::shared_session::SharedSessionStatus;
use crate::terminal::view::{Event, ExecuteCommandEvent};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::workspaces::user_workspaces::{HeadlessTeamScope, UserWorkspaces};

#[test]
fn prewarmed_binding_updates_controller_and_waits_for_real_sharing() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _sharing = FeatureFlag::CreatingSharedSessions.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal.clone(), ctx));
        let window_id = app.read(|ctx| terminal.window_id(ctx));
        let task_id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        let working_dir = std::path::PathBuf::from("/workspace");
        let starts = Rc::new(RefCell::new(Vec::new()));
        app.update(|ctx| {
            let starts = starts.clone();
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::StartSharingCurrentSession { source, .. } = event {
                    starts.borrow_mut().push(source.clone());
                }
            });
        });
        driver.update(&mut app, |driver, ctx| {
            driver
                .bind_prewarmed_task(
                    super::PrewarmedTaskBinding {
                        task_id,
                        working_dir,
                        team_scope: &HeadlessTeamScope::Personal,
                        share_requests: Vec::new(),
                        should_share: true,
                        restored_conversation_id: None,
                    },
                    ctx,
                )
                .unwrap();
            assert!(driver.session_share_rx.is_some());
            assert!(driver.shared_session_id.is_none());
        });
        terminal.read(&app, |terminal, ctx| {
            let controller = terminal.ai_controller().as_ref(ctx);
            assert_eq!(controller.get_ambient_agent_task_id(), Some(task_id));
            assert!(
                UserWorkspaces::as_ref(ctx)
                    .team_uid_for_window(window_id)
                    .is_none()
            );
        });
        assert_eq!(starts.borrow().len(), 1);
        assert_eq!(
            starts.borrow()[0].orchestrator_task_id(),
            Some(task_id.to_string().as_str())
        );
        let shared = driver.update(&mut app, |driver, _| driver.wait_for_session_shared());
        terminal.update(&mut app, |_, ctx| {
            ctx.emit(Event::EstablishedSharedSession {
                session_id: session_sharing_protocol::common::SessionId::new(),
            })
        });
        shared.await.unwrap();
        driver.read(&app, |driver, _| {
            assert!(driver.shared_session_id.is_some())
        });
    });
}

fn command_started_event(command: &str) -> Event {
    Event::ExecuteCommand(ExecuteCommandEvent {
        command: command.to_owned(),
        session_id: 123.into(),
        workflow_id: None,
        workflow_command: None,
        should_add_command_to_history: false,
        source: CommandExecutionSource::User,
    })
}

fn block_completed_event(id: BlockId, exit_code: i32) -> Event {
    Event::BlockCompleted {
        block: Arc::new(SerializedBlock {
            id,
            exit_code: exit_code.into(),
            did_execute: false,
            ..Default::default()
        }),
        is_local: true,
    }
}

#[test]
fn block_completion_before_command_start_does_not_resolve_exit() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal_view.clone(), ctx));
        let command_future = driver
            .update(&mut app, |driver, ctx| {
                driver.execute_command("echo ready", ctx)
            })
            .expect("command should be accepted");

        terminal_view.update(&mut app, |_, ctx| {
            ctx.emit(block_completed_event(BlockId::new(), 0));
        });
        terminal_view.update(&mut app, |_, ctx| {
            ctx.emit(command_started_event("echo ready"));
        });
        let mut handle = command_future.await.expect("command should start");

        assert!((&mut handle).now_or_never().is_none());
    });
}

#[test]
fn matching_block_completion_resolves_exit_without_preexec() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal_view.clone(), ctx));
        let active_block_id =
            terminal_view.read(&app, |view, _| view.model.lock().active_block_id().clone());
        let command_future = driver
            .update(&mut app, |driver, ctx| driver.execute_command("false", ctx))
            .expect("command should be accepted");

        terminal_view.update(&mut app, |_, ctx| {
            ctx.emit(command_started_event("false"));
        });
        let handle = command_future.await.expect("command should start");
        assert_eq!(handle.block_id(), &active_block_id);

        terminal_view.update(&mut app, |_, ctx| {
            ctx.emit(block_completed_event(active_block_id, 17));
        });

        assert_eq!(handle.await.expect("command should finish").value(), 17);
    });
}

#[test]
fn unrelated_block_completion_after_command_start_does_not_resolve_exit() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal_view.clone(), ctx));
        let command_future = driver
            .update(&mut app, |driver, ctx| {
                driver.execute_command("echo ready", ctx)
            })
            .expect("command should be accepted");
        terminal_view.update(&mut app, |_, ctx| {
            ctx.emit(command_started_event("echo ready"));
        });
        let mut handle = command_future.await.expect("command should start");

        terminal_view.update(&mut app, |_, ctx| {
            ctx.emit(block_completed_event(BlockId::new(), 0));
        });

        assert!((&mut handle).now_or_never().is_none());
        let block_id = handle.block_id().clone();
        terminal_view.update(&mut app, |_, ctx| {
            ctx.emit(block_completed_event(block_id, 0));
        });
        assert_eq!(handle.await.expect("command should finish").value(), 0);
    });
}

#[test]
fn different_command_text_does_not_record_started_block() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal_view.clone(), ctx));
        let command_future = driver
            .update(&mut app, |driver, ctx| {
                driver.execute_command("echo ready", ctx)
            })
            .expect("command should be accepted");
        futures::pin_mut!(command_future);

        terminal_view.update(&mut app, |_, ctx| {
            ctx.emit(command_started_event("echo unrelated"));
        });

        assert!(command_future.as_mut().now_or_never().is_none());
        let active_block_id =
            terminal_view.read(&app, |view, _| view.model.lock().active_block_id().clone());
        terminal_view.update(&mut app, |_, ctx| {
            ctx.emit(block_completed_event(active_block_id, 0));
            ctx.emit(command_started_event("echo ready"));
        });
        let mut handle = command_future.await.expect("expected command should start");
        assert!((&mut handle).now_or_never().is_none());
    });
}

#[test]
fn shell_exit_fails_both_command_waiters() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal_view.clone(), ctx));
        let (start_tx, start_rx) = oneshot::channel();
        let (exit_tx, exit_rx) = oneshot::channel();
        driver.update(&mut app, |driver, _| {
            driver.pending_command_start = Some(("echo ready".to_owned(), start_tx));
            driver.waiting_command = Some(WaitingCommand {
                exit_tx,
                started_block_id: None,
            });
            driver.last_command = Some("echo ready".to_owned());
        });

        terminal_view.update(&mut app, |_, ctx| ctx.emit(Event::Exited));

        assert!(matches!(
            start_rx.await.expect("start waiter should receive an error"),
            Err(AgentDriverError::SetupCommandExitedShell { command }) if command == "echo ready"
        ));
        assert!(matches!(
            exit_rx.await.expect("exit waiter should receive an error"),
            Err(AgentDriverError::SetupCommandExitedShell { command }) if command == "echo ready"
        ));
    });
}

#[test]
fn block_plaintext_exposes_continuation_prompt_until_preexec() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal_view.clone(), ctx));
        let block_id = terminal_view.update(&mut app, |view, _| {
            let mut model = view.model.lock();
            model.block_list_mut().active_block_mut().start();
            model.process_bytes("echo \"unterminated\r\ndquote> ");
            assert_eq!(
                model.block_list().active_block().state(),
                BlockState::BeforeExecution
            );
            model.active_block_id().clone()
        });

        assert_eq!(
            app.read(|ctx| driver.as_ref(ctx).block_output_plaintext(&block_id, ctx)),
            Some("echo \"unterminated\ndquote> ".to_owned())
        );

        terminal_view.update(&mut app, |view, _| {
            let mut model = view.model.lock();
            model.preexec(PreexecValue {
                command: "echo \"unterminated\"".to_owned(),
                session_id: None,
            });
            model.process_bytes("executed");
        });

        assert_eq!(
            app.read(|ctx| driver.as_ref(ctx).block_output_plaintext(&block_id, ctx)),
            Some("executed".to_owned())
        );
    });
}

#[test]
fn extend_shared_session_retention_emits_event_for_active_sharer() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal_view = add_window_with_terminal(&mut app, None);
        let terminal_driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal_view.clone(), ctx));
        let emitted_reasons = Rc::new(RefCell::new(Vec::new()));

        app.update(|ctx| {
            let emitted_reasons = emitted_reasons.clone();
            ctx.subscribe_to_view(&terminal_view, move |_, event, _| {
                if let Event::ExtendSessionRetention { reason } = event {
                    emitted_reasons.borrow_mut().push(*reason);
                }
            });
        });

        terminal_driver.update(&mut app, |driver, ctx| {
            driver.extend_shared_session_retention(SessionRetentionReason::SetupFailed, ctx);
        });

        assert!(
            emitted_reasons.borrow().is_empty(),
            "retention should not be extended before session sharing is active"
        );

        terminal_view.update(&mut app, |view, _| {
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ActiveSharer);
        });

        terminal_driver.update(&mut app, |driver, ctx| {
            driver.extend_shared_session_retention(SessionRetentionReason::SetupFailed, ctx);
        });

        let emitted_reasons = emitted_reasons.borrow();
        assert_eq!(emitted_reasons.len(), 1);
        assert!(matches!(
            emitted_reasons[0],
            SessionRetentionReason::SetupFailed
        ));
    });
}

#[test]
fn shared_session_failure_propagates_to_agent_driver_error() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal_view = add_window_with_terminal(&mut app, None);
        let terminal_driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal_view.clone(), ctx));
        let failure = terminal_driver.update(&mut app, |driver, _| {
            driver.wait_for_session_share_failure()
        });

        let reason = "Reached maximum number of session sharing reconnection attempts without making ordered event progress";
        terminal_view.update(&mut app, |_, ctx| {
            ctx.emit(Event::SharedSessionFailed {
                reason: reason.to_string(),
            });
        });

        match failure.await {
            AgentDriverError::ShareSessionFailed {
                error: ShareSessionError::Failed(actual_reason),
            } => assert_eq!(actual_reason, reason),
            other => panic!("expected fatal session sharing failure, got {other:?}"),
        }
    });
}

// #[serial] because the secret regexes configured below are global state.
#[test]
#[serial]
fn shell_exit_fails_in_flight_and_subsequent_commands() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        // Configure a secret pattern (a GitHub classic PAT). In production
        // these are populated from the user's/enterprise's privacy settings
        // via CustomSecretRegexUpdater.
        set_user_and_enterprise_secret_regexes(
            [&Regex::new(r"\bghp_[A-Za-z0-9_]{36}\b").expect("pattern should compile")],
            std::iter::empty(),
        );

        let terminal_view = add_window_with_terminal(&mut app, None);
        let terminal_driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal_view.clone(), ctx));

        // A command containing a secret matching the configured pattern. The
        // attributed command in the shell-exit error must have the token
        // redacted, since the error flows into server task status and Sentry
        // reports.
        let token = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij";
        let submitted = format!("echo {token}");
        let expected_redacted = format!("echo {}", "*".repeat(token.len()));

        // Start a command (e.g. an environment setup command) so the driver
        // has an in-flight command waiting on the terminal session.
        let command_future = terminal_driver
            .update(&mut app, |driver, ctx| {
                driver.execute_command(&submitted, ctx)
            })
            .expect("command should be accepted before the shell exits");

        // The shell process dies (e.g. the command ran `exit 1`).
        terminal_view.update(&mut app, |_, ctx| ctx.emit(Event::Exited));

        // The in-flight command must resolve with the shell-exit error
        // instead of hanging forever, regardless of whether it had already
        // started executing. The error must name the (redacted) command that
        // was running when the shell died.
        let result = match command_future.await {
            Ok(handle) => handle.await,
            Err(error) => Err(error),
        };
        match &result {
            Err(AgentDriverError::SetupCommandExitedShell { command }) => {
                assert_eq!(command, &expected_redacted);
            }
            other => {
                panic!("in-flight command should fail with SetupCommandExitedShell, got {other:?}")
            }
        }

        // Any further command must fail fast with the same error, still
        // attributing the (redacted) command that killed the shell (not the
        // newly attempted one).
        let fail_fast = terminal_driver.update(&mut app, |driver, ctx| {
            driver.execute_command("echo again", ctx).err()
        });
        match &fail_fast {
            Some(AgentDriverError::SetupCommandExitedShell { command }) => {
                assert_eq!(command, &expected_redacted);
            }
            other => panic!(
                "commands after shell exit should fail fast with SetupCommandExitedShell, got {other:?}"
            ),
        }
    });
}
