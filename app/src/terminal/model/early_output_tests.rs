use super::TypeaheadMode;
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
        None,
    )
    .merge_from_bootstrapped_value(bootstrapped_value.clone());

    block_list.bootstrapped(bootstrapped_value);
    block_list.early_output_mut().init_session(&session_info);
    assert_eq!(block_list.early_output_mut().mode, mode);

    block_list.command_finished(Default::default());
    block_list.prompt_only_precmd(Default::default());
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
    block_list.prompt_only_precmd(Default::default());

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
fn test_push_expected_echo_is_swallowed_for_shell_reported() {
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    // Register "git ch" as expected echo -- e.g. `PtyController` restoring the input buffer
    // after a generator command that necessarily cleared it to run as a foreground command.
    // The caller already has this text elsewhere (the input editor's own buffer, which was
    // never cleared), so it must be dropped entirely here rather than surfaced as typeahead --
    // surfacing it would duplicate it in the editor.
    block_list.early_output_mut().push_expected_echo("git ch");

    // Echoing the exact same characters should be swallowed: no background block, unlike
    // plain, unregistered echo for this mode (see `test_queued_typeahead_shell_reported` below,
    // where the same characters without a prior `push_expected_echo` call go straight to a
    // background block), and no typeahead either.
    for ch in "git ch".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(block_list.background_block_mut().is_none());
    assert_eq!(block_list.early_output().typeahead(), "");
}

#[test]
fn test_push_expected_echo_survives_a_partial_echo_before_a_carriage_return() {
    // Reproduces zsh's ZLE redraw for a native-completions buffer restore: it echoes one
    // character, returns to column 0, then re-echoes the whole line -- so the echo stream for
    // a two-character restore is "g", CR, "g", "i", not "g", "i" once.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list.early_output_mut().push_expected_echo("gi");

    block_list.input('g');
    block_list.carriage_return();
    block_list.input('g');
    block_list.input('i');
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "the redraw's repeat of the restored buffer must not start a phantom background block"
    );
    assert_eq!(block_list.early_output().typeahead(), "");
}

#[test]
fn test_push_expected_echo_survives_a_full_echo_repeated_after_a_carriage_return() {
    // Reproduces fish's redraw for a native-completions buffer restore: it echoes the whole
    // line, returns to column 0 (twice), then echoes the whole line again -- so the echo
    // stream for a two-character restore is "g", "i", CR, CR, "g", "i", CR.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list.early_output_mut().push_expected_echo("gi");

    block_list.input('g');
    block_list.input('i');
    block_list.carriage_return();
    block_list.carriage_return();
    block_list.input('g');
    block_list.input('i');
    block_list.carriage_return();
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "a repeated full echo of the restored buffer must not start a phantom background block"
    );
    assert_eq!(block_list.early_output().typeahead(), "");
}

#[test]
fn test_push_expected_echo_survives_a_carriage_return_mid_pass_that_continues_rather_than_restarts()
{
    // Reproduces fish's redraw for a longer native-completions buffer restore: unlike the
    // all-or-nothing repeats above, fish sometimes returns to column 0 *mid-line* and continues
    // echoing the same line from wherever it left off, rather than restarting it from the
    // beginning -- so the echo stream for a six-character restore can be "git ", CR, "ch", not
    // a clean restart. A carriage return must not discard an in-progress match position just
    // because it also opens up the possibility of a restart.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list.early_output_mut().push_expected_echo("git ch");

    for ch in "git ".chars() {
        block_list.input(ch);
    }
    block_list.carriage_return();
    for ch in "ch".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "a carriage return that turns out to be mid-line must not make the continuation miss"
    );
    assert_eq!(block_list.early_output().typeahead(), "");
}

#[test]
fn test_push_expected_echo_handles_a_carriage_return_between_two_matching_characters() {
    // The minimal version of the same shape as above, isolating just the carriage return
    // boundary: "g", CR, "i" against a registered "gi".
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list.early_output_mut().push_expected_echo("gi");

    block_list.input('g');
    block_list.carriage_return();
    block_list.input('i');
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "a carriage return between two characters of a single continued echo must not misfire"
    );
}

#[test]
fn test_push_expected_echo_tolerates_ambiguous_candidates_matching_the_same_character() {
    // When a carriage return leaves both a restart-at-0 candidate and a mid-pass candidate
    // live, and the buffer contains a repeated character, more than one candidate can match
    // the very same incoming character (e.g. registering "gg": after matching the first "g"
    // and a carriage return, both position 0 and position 1 expect a "g" next). Both must be
    // tracked rather than the matcher picking one arbitrarily and losing the other.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list.early_output_mut().push_expected_echo("gg");

    block_list.input('g');
    block_list.carriage_return();
    // Ambiguous: matches both the position-0 restart candidate and the position-1 continuation
    // candidate.
    block_list.input('g');
    // Only the continuation candidate (now at position 2, fully matched) predicts nothing
    // further; a fresh full echo would need another full "gg", which this test doesn't send,
    // so nothing more should be consumed.
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "an ambiguous character that matches multiple live candidates must still be consumed"
    );
}

#[test]
fn test_push_expected_echo_survives_a_precmd_within_the_same_restore_window() {
    // `CompletionsFinished` (and the `push_expected_echo` call it triggers) fires when `9280;B`
    // is parsed, which precedes the shell's own in-band-command precmd DCS -- so in practice a
    // `precmd` call normally lands *inside* the restore window, before the echo has arrived.
    // `precmd` must not clear the registration out from under it.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list.early_output_mut().push_expected_echo("gi");

    block_list.command_finished(Default::default());
    block_list.prompt_only_precmd(Default::default());

    // The echo, arriving only now, must still be recognized.
    block_list.input('g');
    block_list.input('i');
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "the registration must survive an intervening precmd within the same restore"
    );
}

#[test]
fn test_push_expected_echo_staleness_is_bounded_by_the_next_registration_not_by_precmd() {
    // A restore whose echo never fully arrives (e.g. the request was superseded by a newer
    // one) must not leave state that spuriously matches unrelated output once a *new*
    // registration has superseded it -- `push_expected_echo` replacing its content outright on
    // every call is what bounds staleness, not any clearing on `precmd`.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list.early_output_mut().push_expected_echo("gi");
    block_list.input('g'); // Only a partial echo arrives before a newer request supersedes it.

    block_list.command_finished(Default::default());
    block_list.prompt_only_precmd(Default::default());

    // A newer restore's registration must fully replace the stale one.
    block_list.early_output_mut().push_expected_echo("go");
    block_list.input('g');
    block_list.input('o');
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "the newer registration must be matched cleanly, not confused by the stale one"
    );
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
    block_list.prompt_only_precmd(Default::default());

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
    assert!(
        block_list
            .background_block_mut()
            .expect("Block should exist")
            .is_empty(&crate::terminal::model::block::TranscriptScope::Terminal)
    );
}
