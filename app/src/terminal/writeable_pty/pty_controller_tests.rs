use crate::terminal::color::List;
use crate::terminal::event::UserBlockCompleted;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::block::{BlockSize, SerializedBlock};
use crate::terminal::shell::ShellType;
use crate::terminal::BlockPadding;
use crate::theme::WarpTheme;

use super::*;

impl PtyController {
    /// Flushes PTY writes by sending a "dummy" test write, and blocking until the test write is
    /// handled.
    ///
    /// When the "dummy write" is handled, an event is emitted through a channel. This method
    /// blocks on receiving this event through the channel. Since async writes are handled in order
    /// that they are queued, this ensures all queued writes have been handled/"flushed".
    fn flush_pty_writes(&mut self) {
        let (tx, rx) = futures::channel::oneshot::channel();
        let _ = self.queue_async_write(AsyncPtyWrite {
            bytes: vec![],
            delay: None,
            on_write_fn: Some(Box::new(move || {
                let _ = tx.send(());
            })),
        });
        let _ = warpui::r#async::block_on(rx);
    }
}

fn terminal_model(background_executor: Arc<Background>) -> Arc<FairMutex<TerminalModel>> {
    Arc::new(FairMutex::new(TerminalModel::new_for_test(
        // This BlockSize contains arbitrary values.
        BlockSize {
            block_padding: BlockPadding {
                padding_top: 0.5,
                command_padding_top: 0.5,
                middle: 0.5,
                bottom: 0.5,
            },
            size: SizeInfo::new_for_test_with_width_and_height(7., 10.5),
            max_block_scroll_limit: 1000,
            prompt_height: 1.,
        },
        List::from(&WarpTheme::default().into()),
        ChannelEventListener::new_for_test(),
        background_executor,
        false,
        None,
        false,
        None,
    )))
}

fn assert_input_matches(message: &Message, expected_bytes: Vec<u8>) {
    assert!(matches!(message, Message::Input(bytes) if bytes.to_vec() == expected_bytes));
}

/// Returns a vector containing the bytes we expect to be written to the PTY to execute the given
/// `command` in the given `Shell`.
fn expected_command_bytes(command: &str, shell: &Shell) -> Vec<u8> {
    let mut bytes = shell.shell_type().kill_buffer_bytes().to_vec();
    bytes.extend(command.as_bytes().to_vec());
    bytes.extend(shell.shell_type().execute_command_bytes());
    bytes
}

#[test]
fn test_pty_controller_writes_user_command() {
    let (event_loop_tx, event_loop_rx) = mio_extras::channel::channel();
    let background_executor = Arc::new(Background::default());
    let mut controller = PtyController::new(
        event_loop_tx,
        background_executor.clone(),
        terminal_model(background_executor),
    );

    let shell = Shell::new(ShellType::Zsh, None, None, Default::default(), None);
    assert!(controller.write_user_command("echo foo", &shell).is_ok());
    controller.flush_pty_writes();

    let message = event_loop_rx
        .try_recv()
        .expect("PtyController should have sent write.");
    assert!(
        matches!(message, Message::Input(bytes) if bytes.to_vec() == expected_command_bytes("echo foo", &shell))
    );
}

#[test]
fn test_pty_controller_writes_in_band_command() {
    let (event_loop_tx, event_loop_rx) = mio_extras::channel::channel();
    let background_executor = Arc::new(Background::default());
    let mut controller = PtyController::new(
        event_loop_tx,
        background_executor.clone(),
        terminal_model(background_executor),
    );

    let shell = Shell::new(ShellType::Zsh, None, None, Default::default(), None);
    assert!(controller.write_in_band_command("echo foo", &shell).is_ok());
    controller.flush_pty_writes();

    let message = event_loop_rx
        .try_recv()
        .expect("PtyController should have sent write.");
    assert_input_matches(&message, expected_command_bytes("echo foo", &shell));
}

#[test]
fn test_pty_controller_updates_block_list_when_writing_in_band_command() {
    let (event_loop_tx, _) = mio_extras::channel::channel();
    let background_executor = Arc::new(Background::default());
    let terminal_model = terminal_model(background_executor.clone());
    let mut controller =
        PtyController::new(event_loop_tx, background_executor, terminal_model.clone());

    let shell = Shell::new(ShellType::Zsh, None, None, Default::default(), None);
    assert!(controller.write_in_band_command("echo foo", &shell).is_ok());
    controller.flush_pty_writes();

    assert!(terminal_model
        .lock()
        .block_list()
        .is_writing_or_executing_in_band_command());
}

#[test]
fn test_pty_controller_writes_input_buffer_sequence_after_block_completed() {
    let (event_loop_tx, event_loop_rx) = mio_extras::channel::channel();
    let background_executor = Arc::new(Background::default());

    let mut controller = PtyController::new(
        event_loop_tx,
        background_executor.clone(),
        terminal_model(background_executor),
    );
    controller.set_state_after_block_completed(
        &BlockType::User(UserBlockCompleted {
            serialized_block: SerializedBlock::new_for_test("echo foo".as_bytes().to_vec(), vec![])
                .into(),
            command: "echo foo".to_owned(),
            output_truncated: "".to_owned(),
            started_at: None,
            num_output_lines: 0,
            num_output_lines_truncated: 0,
            shell_type: None,
        }),
        true,
    );
    controller.flush_pty_writes();

    let message = event_loop_rx
        .try_recv()
        .expect("PtyController should have sent write.");
    assert_input_matches(&message, vec![escape_sequences::C0::ESC, b'i']);
}

#[test]
fn test_pty_controller_writes_in_band_command_after_input_buffer_sequence() {
    let (event_loop_tx, event_loop_rx) = mio_extras::channel::channel();
    let background_executor = Arc::new(Background::default());
    let mut controller = PtyController::new(
        event_loop_tx,
        background_executor.clone(),
        terminal_model(background_executor),
    );

    let shell = Shell::new(ShellType::Zsh, None, None, Default::default(), None);
    controller.set_state_after_block_completed(
        &BlockType::User(UserBlockCompleted {
            serialized_block: SerializedBlock::new_for_test("echo foo".as_bytes().to_vec(), vec![])
                .into(),
            command: "echo foo".to_owned(),
            output_truncated: "".to_owned(),
            started_at: None,
            num_output_lines: 0,
            num_output_lines_truncated: 0,
            shell_type: None,
        }),
        true,
    );
    assert!(controller.write_in_band_command("echo foo", &shell).is_ok());
    controller.flush_pty_writes();

    let mut messages = vec![];
    while let Ok(message) = event_loop_rx.try_recv() {
        messages.push(message);
    }
    assert_eq!(messages.len(), 2);
    assert_input_matches(&messages[0], vec![escape_sequences::C0::ESC, b'i']);

    assert_input_matches(&messages[1], expected_command_bytes("echo foo", &shell));
}

#[test]
fn test_pty_controller_cancels_async_writes_upon_user_command() {
    let (event_loop_tx, event_loop_rx) = mio_extras::channel::channel();
    let background_executor = Arc::new(Background::default());
    let mut controller = PtyController::new(
        event_loop_tx,
        background_executor.clone(),
        terminal_model(background_executor),
    );

    controller.set_state_after_block_completed(
        &BlockType::User(UserBlockCompleted {
            serialized_block: SerializedBlock::new_for_test("echo foo".as_bytes().to_vec(), vec![])
                .into(),
            command: "echo foo".to_owned(),
            output_truncated: "".to_owned(),
            started_at: None,
            num_output_lines: 0,
            num_output_lines_truncated: 0,
            shell_type: None,
        }),
        true,
    );
    let shell = Shell::new(ShellType::Zsh, None, None, Default::default(), None);
    // Writing this command should cancel writing the input buffer escape sequence, which is
    // written after a 50ms delay. Since only ~25 ms has passed the write should not have
    // occurred yet.
    assert!(controller.write_user_command("echo foo", &shell).is_ok());
    controller.flush_pty_writes();

    let mut messages = vec![];
    while let Ok(message) = event_loop_rx.try_recv() {
        messages.push(message);
    }
    assert_eq!(messages.len(), 1);

    assert_input_matches(&messages[0], expected_command_bytes("echo foo", &shell));
}
