use super::TypeaheadMode;
use crate::ai::blocklist::agent_view::AgentViewState;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::{self, Handler};
use crate::terminal::model::blocks::BlockList;
use crate::terminal::model::session::SessionInfo;
use crate::terminal::model::test_utils::TestBlockListBuilder;
use crate::terminal::shell::ShellType;

/// Create a new bootstrapped block list that will use the set typeahead mode.
fn new_block_list(event_proxy: ChannelEventListener, mode: TypeaheadMode) -> BlockList {
    let mut block_list = TestBlockListBuilder::new()
        .with_channel_event_proxy(event_proxy)
        .build();

    let (shell, shell_version) = match mode {
        TypeaheadMode::ShellReported => ("zsh", "5.0"),
        TypeaheadMode::InputMatching => ("bash", "3.2"),
    };
    let init_shell_value = ansi::InitShellValue {
        shell: shell.into(),
        ..Default::default()
    };

    let bootstrapped_value = ansi::BootstrappedValue {
        shell: shell.into(),
        shell_version: Some(shell_version.into()),
        ..Default::default()
    };
    let session_info = SessionInfo::create_pending(
        ShellType::from_name(shell).unwrap(),
        init_shell_value,
        None,
        None,
        None,
        false,
        None,
    )
    .merge_from_bootstrapped_value(bootstrapped_value.clone(), false);

    block_list.bootstrapped(bootstrapped_value);
    block_list.early_output_mut().init_session(&session_info);
    assert_eq!(block_list.early_output_mut().mode, mode);

    block_list.command_finished(Default::default());
    block_list.precmd(Default::default());
    assert!(block_list.is_bootstrapping_precmd_done());
    block_list
}

#[test]
fn test_lazy_background_insertion() {
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    // Mimic the shell resetting terminal styles between commands.
    block_list.carriage_return();
    block_list.clear_line(ansi::LineClearMode::Right);
    block_list.terminal_attribute(ansi::Attr::Reset);

    // At this point, the background block should not have been inserted.
    assert!(block_list.background_block_mut().is_none());
    assert!(block_list.is_empty());

    // Write actual background output.
    block_list.input('h');
    block_list.input('i');
    block_list.linefeed();
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(!block_list.is_empty());
    let background = block_list
        .background_block_mut()
        .expect("Background block should exist");
    assert_eq!(background.output_to_string(), "hi\n");
}

#[test]
fn test_background_triggers_wakeup() {
    let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
    let mut block_list = new_block_list(
        ChannelEventListener::builder_for_test()
            .with_wakeups_tx(wakeups_tx)
            .build(),
        TypeaheadMode::ShellReported,
    );
    while !wakeups_rx.is_empty() {
        let _ = wakeups_rx.try_recv();
    }

    // Write background output.
    block_list.input('b');

    // There should now be a background block and a wakeup call.
    assert!(block_list.background_block_mut().is_some());
    assert!(wakeups_rx.recv_blocking().is_ok());
}

#[test]
fn test_queued_typeahead_input_matching() {
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::InputMatching,
    );

    // Provide two lines of typeahead.
    block_list
        .early_output_mut()
        .push_user_input("first\rsecond");

    // Mimic the shell echoing and executing the first command.
    block_list.input('f');
    block_list.input('i');
    block_list.input('r');
    block_list.input('s');
    block_list.input('t');
    block_list.carriage_return();
    block_list.linefeed();
    assert!(block_list.active_block().is_command_empty());
    // With input matching, typeahead is never written to the background block.
    assert!(block_list.background_block_mut().is_none());
    // On preexec, the block list detects that the command is missing and restores
    // it from typeahead.
    block_list.preexec(ansi::PreexecValue {
        command: "first".into(),
        session_id: None,
    });
    assert_eq!(block_list.active_block().command_to_string(), "first");
    block_list.command_finished(Default::default());
    block_list.precmd(Default::default());

    // Once the second line of typeahead is echoed, it should be recognized as typeahead.
    block_list.input('s');
    block_list.input('e');
    block_list.input('c');
    block_list.input('o');
    block_list.input('n');
    block_list.input('d');
    assert_eq!(block_list.early_output().typeahead(), "second");
}

#[test]
fn test_queued_typeahead_shell_reported() {
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    // Mimic the shell echoing and executing the first command.
    block_list.input('f');
    block_list.input('i');
    block_list.input('r');
    block_list.input('s');
    block_list.input('t');
    block_list.carriage_return();
    block_list.linefeed();
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert!(block_list.active_block().is_command_empty());
    assert_eq!(
        block_list
            .background_block_mut()
            .expect("Background block should exist")
            .output_to_string(),
        "first\n"
    );
    // On preexec, the block list detects that the command is missing and restores
    // it from background output, removing the background block in the process.
    block_list.preexec(ansi::PreexecValue {
        command: "first".into(),
        session_id: None,
    });
    assert_eq!(block_list.active_block().command_to_string(), "first");
    assert!(block_list.background_block_mut().is_none());

    block_list.command_finished(Default::default());
    block_list.precmd(Default::default());

    // Now, when the second line is echoed, it should be recognized as typeahead.
    block_list.input('s');
    block_list.input('e');
    block_list.input('c');
    block_list.input('o');
    block_list.input('n');
    block_list.input('d');
    // Mimic the ESC-i keybinding, which clears the input buffer.
    block_list.input_buffer(ansi::InputBufferValue {
        buffer: "second".into(),
        session_id: None,
    });
    // zsh appears to use `\r\e[J` (carriage return and clear from cursor to end of screen)
    // to clear the line. There are lots of ways of doing this, and it doesn't
    // matter exactly which one the shell uses as long as the effect on the grid is the same.
    block_list.carriage_return();
    block_list.clear_screen(ansi::ClearMode::Below);
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(block_list.early_output().typeahead(), "second");
    // Unlike regular blocks, if the output grid of a background block is cleared
    // then it becomes hidden.
    assert!(block_list
        .background_block_mut()
        .expect("Block should exist")
        .is_empty(&AgentViewState::Inactive));
}
