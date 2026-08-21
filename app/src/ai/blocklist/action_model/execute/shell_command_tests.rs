use std::sync::Arc;

use async_channel::unbounded;
use command::blocking::Command;
use futures::channel::oneshot;
use parking_lot::FairMutex;
use warpui::{App, Entity, EntityId};

use super::{
    AnyActionExecution, BlockSelector, ExecuteActionInput, ShellCommandExecutor,
    ShellCommandExecutorEvent,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType};
use crate::ai::blocklist::action_model::recording_controller::RecordingController;
use crate::terminal::event::{BlockMetadataReceivedEvent, BlockWorkingDirectoryUpdatedEvent};
use crate::terminal::model::block::{BlockId, BlockMetadata};
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::{SessionId, SessionInfo, Sessions};
use crate::terminal::model::terminal_model::{BlockIndex, TerminalModel};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::shell::{Shell, ShellType};

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

#[derive(Default)]
struct CapturedExecutedCommands(Vec<String>);

impl Entity for CapturedExecutedCommands {
    type Event = ();
}

/// Builds a `ShellCommandExecutor` backed by a bootstrapped session of the given
/// shell type, plus a model that captures every `ExecuteCommand` event it emits.
fn build_executor(
    app: &mut App,
    terminal_view_id: EntityId,
    shell_type: ShellType,
) -> (
    warpui::ModelHandle<ShellCommandExecutor>,
    warpui::ModelHandle<CapturedExecutedCommands>,
) {
    app.add_singleton_model(|_| RecordingController::new());

    let sessions = app.add_model(|_| Sessions::new_for_test());
    let session_id = SessionId::from(0);
    sessions.update(app, |sessions, _ctx| {
        let mut session_info = SessionInfo::new_for_test().with_id(session_id);
        session_info.shell = Shell::new(shell_type, None, None, Default::default(), None);
        sessions.register_session_for_test(session_info);
    });

    let (_model_events_tx, model_events_rx) = unbounded();
    let model_event_dispatcher =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    model_event_dispatcher.update(app, |dispatcher, _ctx| {
        dispatcher.set_active_session_id(session_id);
    });

    let active_session = app
        .add_model(|ctx| ActiveSession::new(sessions.clone(), model_event_dispatcher.clone(), ctx));
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

    let captured = app.add_model(|_| CapturedExecutedCommands::default());
    captured.update(app, |_, ctx| {
        ctx.subscribe_to_model(&executor, |captured, _, event, _ctx| {
            if let ShellCommandExecutorEvent::ExecuteCommand { command, .. } = event {
                captured.0.push(command.clone());
            }
        });
    });

    (executor, captured)
}

/// Builds a `RequestCommandOutput` action for `command` with the given
/// `uses_pager`/`wait_until_completion` flags.
fn build_request_command_output_action(
    command: &str,
    uses_pager: bool,
    wait_until_completion: bool,
) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from("action-1".to_string()),
        task_id: TaskId::new("task-1".to_owned()),
        requires_result: false,
        action: AIAgentActionType::RequestCommandOutput {
            command: command.to_string(),
            is_read_only: Some(true),
            is_risky: Some(false),
            wait_until_completion,
            uses_pager: Some(uses_pager),
            rationale: None,
            citations: vec![],
        },
    }
}

fn executed_command(
    app: &mut App,
    executor: &warpui::ModelHandle<ShellCommandExecutor>,
    captured: &warpui::ModelHandle<CapturedExecutedCommands>,
    action: &AIAgentAction,
) -> String {
    let conversation_id = AIConversationId::new();
    executor.update(app, |executor, ctx| {
        let input = ExecuteActionInput {
            action,
            conversation_id,
        };
        let _: AnyActionExecution = executor.execute(input, ctx).into();
    });

    let executed_commands = app.read(|ctx| captured.as_ref(ctx).0.clone());
    assert_eq!(
        executed_commands.len(),
        1,
        "expected exactly one ExecuteCommand event"
    );
    executed_commands[0].clone()
}

/// Regression test for the reported bug: a `wait`-mode command (server reports
/// `wait_until_completion: true`) with `uses_pager: true` — e.g. `gh pr view` or
/// `git log` — must be decorated so it doesn't drop into the user's pager.
#[test]
fn execute_decorates_pager_command_when_waiting_for_completion() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (executor, captured) = build_executor(&mut app, terminal_view_id, ShellType::Bash);
        let action = build_request_command_output_action("gh pr view 123", true, true);

        let executed = executed_command(&mut app, &executor, &captured, &action);

        assert!(
            executed.contains("| command cat"),
            "expected pager decoration to be applied for a wait-mode uses_pager command, got: {executed}"
        );
    });
}

/// Regression guard for review finding 1: an `interact`-mode command (server
/// reports `wait_until_completion: false`) must NOT be decorated even when
/// `uses_pager` is set, since the subagent driving it may need live PTY control
/// (e.g. a REPL, dev server, or a pager it intends to page through itself).
#[test]
fn execute_does_not_decorate_pager_command_in_interact_mode() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (executor, captured) = build_executor(&mut app, terminal_view_id, ShellType::Bash);
        let action = build_request_command_output_action("less some_long_file.txt", true, false);

        let executed = executed_command(&mut app, &executor, &captured, &action);

        assert_eq!(
            executed, "less some_long_file.txt",
            "an interact-mode command must be run as-is, without pager decoration"
        );
    });
}

/// Regression test for review finding 2: piping a command's output through
/// `cat` must not mask the original command's exit status (in Bash, `cat` being
/// the pipeline's last stage would otherwise always report success).
#[cfg(unix)]
#[test]
fn turn_off_pager_for_command_preserves_nonzero_exit_status_in_bash() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (executor, _captured) = build_executor(&mut app, terminal_view_id, ShellType::Bash);
        let command = "false".to_string();

        let decorated = executor.update(&mut app, |executor, ctx| {
            executor.turn_off_pager_for_command(&command, ctx)
        });

        let status = Command::new("bash")
            .arg("-c")
            .arg(&decorated)
            .status()
            .expect("bash should be available to run the decorated command");
        assert_eq!(
            status.code(),
            Some(1),
            "decorated command should preserve the original nonzero exit status, got: {decorated}"
        );
    });
}

/// Regression test for review finding 3 (CSAT-10167 class): a command whose
/// last line is a bare heredoc terminator must not be corrupted by decoration
/// appending characters to that same line.
#[cfg(unix)]
#[test]
fn turn_off_pager_for_command_preserves_heredoc_in_bash() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (executor, _captured) = build_executor(&mut app, terminal_view_id, ShellType::Bash);
        let command = "cat <<'EOF'\nhello from heredoc\nEOF".to_string();

        let decorated = executor.update(&mut app, |executor, ctx| {
            executor.turn_off_pager_for_command(&command, ctx)
        });

        let output = Command::new("bash")
            .arg("-c")
            .arg(&decorated)
            .output()
            .expect("bash should be available to run the decorated command");
        assert!(
            output.status.success(),
            "decorated command should not corrupt the heredoc, got: {decorated}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            "hello from heredoc"
        );
    });
}

/// Companion to the heredoc regression test: a command with no trailing
/// newline at all must still be decorated safely.
#[cfg(unix)]
#[test]
fn turn_off_pager_for_command_handles_command_without_trailing_newline_in_bash() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (executor, _captured) = build_executor(&mut app, terminal_view_id, ShellType::Bash);
        let command = "printf 'no newline at end'".to_string();

        let decorated = executor.update(&mut app, |executor, ctx| {
            executor.turn_off_pager_for_command(&command, ctx)
        });

        let output = Command::new("bash")
            .arg("-c")
            .arg(&decorated)
            .output()
            .expect("bash should be available to run the decorated command");
        assert!(
            output.status.success(),
            "decorated command failed: {decorated}"
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), "no newline at end");
    });
}

/// Structural check for Zsh: it uses `$pipestatus` (lowercase, 1-indexed),
/// not Bash's `$PIPESTATUS`, and the closing group syntax lands on its own
/// line so heredocs stay intact.
#[test]
fn turn_off_pager_for_command_zsh_uses_pipestatus_and_safe_closer() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (executor, _captured) = build_executor(&mut app, terminal_view_id, ShellType::Zsh);
        let command = "cat <<'EOF'\nhi\nEOF".to_string();

        let decorated = executor.update(&mut app, |executor, ctx| {
            executor.turn_off_pager_for_command(&command, ctx)
        });

        assert!(decorated.contains("${pipestatus[1]}"), "got: {decorated}");
        assert!(
            decorated.contains("EOF\n) | command cat"),
            "closing group syntax must be on its own line after the heredoc terminator, got: {decorated}"
        );
    });
}

/// Structural check for Fish: it has no bare `exit`/`PIPESTATUS`-style array,
/// so the decoration re-asserts the exit status via a nested `fish -c`
/// process, and the closing `end` lands on its own line.
#[test]
fn turn_off_pager_for_command_fish_uses_nested_process_for_exit_status() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (executor, _captured) = build_executor(&mut app, terminal_view_id, ShellType::Fish);
        let command = "cat <<'EOF'\nhi\nEOF".to_string();

        let decorated = executor.update(&mut app, |executor, ctx| {
            executor.turn_off_pager_for_command(&command, ctx)
        });

        assert!(
            decorated.contains("fish -c \"exit $pipestatus[1]\""),
            "got: {decorated}"
        );
        assert!(
            decorated.contains("EOF\nend | command cat"),
            "closing `end` must be on its own line after the heredoc terminator, got: {decorated}"
        );
    });
}

/// Structural check for PowerShell: the closing group syntax must land on its
/// own line so multi-line commands (e.g. here-strings) aren't corrupted.
#[test]
fn turn_off_pager_for_command_powershell_closer_on_own_line() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (executor, _captured) =
            build_executor(&mut app, terminal_view_id, ShellType::PowerShell);
        let command = "Write-Host hi".to_string();

        let decorated = executor.update(&mut app, |executor, ctx| {
            executor.turn_off_pager_for_command(&command, ctx)
        });

        assert!(
            decorated.contains("hi\n) | \\Out-Host"),
            "closing group syntax must be on its own line, got: {decorated}"
        );
    });
}
