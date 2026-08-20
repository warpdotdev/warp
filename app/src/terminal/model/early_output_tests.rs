use super::TypeaheadMode;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::{self, Handler};
use crate::terminal::model::blocks::BlockList;
use crate::terminal::model::index::VisibleRow;
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
fn test_expected_echo_never_swallows_output_that_does_not_match_the_registered_pattern() {
    // Every other test in this file feeds characters that *do* eventually match the registered
    // pattern, since they're all reproducing a real redraw's echo. This asserts the opposite
    // direction: replacing `consume_expected_echo`'s body with `!self.expected_echo.is_empty()`
    // -- i.e. absorbing everything whenever anything is registered, regardless of whether it
    // matches -- would leave every other test in this file green, since none of them feed a
    // character that fails to match from the very first position. "xyz" shares no prefix at
    // all with the registered "git ch", so it must render exactly as sent.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list.early_output_mut().push_expected_echo("git ch");

    for ch in "xyz".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(
        block_list
            .background_block_mut()
            .expect("unmatched output must render as real background output")
            .output_to_string(),
        "xyz"
    );
}

#[test]
fn test_expected_echo_does_not_corrupt_unrelated_output_once_reset() {
    // Reproduces a real defect a standalone extraction of the matcher found: with the pattern
    // "git ch" still live, unrelated background output "grep -rn foo" rendered as "rep -rn foo"
    // -- the leading "g" was silently consumed, since it happens to match the registered
    // pattern's own first character, and a mismatch never clears what a previous match already
    // advanced. `EarlyOutput` alone cannot bound how long a registration stays live -- that is
    // `PtyController`'s job, calling `reset_expected_echo` once the line editor is active again
    // after a restore (see its own doc comment) -- so this simulates that boundary directly by
    // calling `reset_expected_echo` before the unrelated output arrives, exactly as production
    // code does once that boundary is crossed.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list.early_output_mut().push_expected_echo("git ch");
    for ch in "git ch".chars() {
        block_list.input(ch);
    }

    // The restore's own redraw window has closed.
    block_list.early_output_mut().reset_expected_echo();

    // Unrelated output that happens to share a leading character with the stale pattern must
    // render in full, not lose that character to a pattern that no longer applies.
    for ch in "grep -rn foo".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(
        block_list
            .background_block_mut()
            .expect("unrelated output must render as real background output")
            .output_to_string(),
        "grep -rn foo"
    );
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
fn test_push_expected_echo_survives_a_backspace_before_a_full_reprint() {
    // Reproduces zsh's redraw with `terminal.input.honor_ps1 = true` (the shell draws its own
    // prompt): the rewind is a literal backspace, not a carriage return -- the echo stream for
    // an 11-character restore is "s", backspace, then the whole line again ("starship pr").
    // Measured: 0 backspaces in 16 requests with Warp drawing the prompt, exactly 1 in each of
    // 10 requests with the shell drawing it. A carriage-return-only rearm cannot help here,
    // since none of these redraws ever emit a carriage return at all.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list
        .early_output_mut()
        .push_expected_echo("starship pr");

    block_list.input('s');
    block_list.backspace();
    for ch in "starship pr".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "a backspace-based rewind followed by a full reprint must not leak"
    );
}

#[test]
fn test_push_expected_echo_survives_a_cub_rewind_after_a_full_match() {
    // Reproduces a further redraw pass with zsh-syntax-highlighting loaded: after a full
    // match, the command word gets recoloured via CUB (cursor-backward, e.g. `\x1b[11D` for an
    // 11-character buffer) rather than a carriage return or backspace, followed by the
    // recoloured characters (interspersed with SGR codes, irrelevant to matching) and a CUF
    // skipping over the untouched remainder. Unlike a carriage return, CUB's distance is known
    // from the escape sequence's own parameter, so this should rearm precisely rather than
    // seeding every position.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list
        .early_output_mut()
        .push_expected_echo("starship pr");
    for ch in "starship pr".chars() {
        block_list.input(ch);
    }

    // Recolour just "starship" (8 characters): rewind by the full 11, then re-echo only the
    // first 8 characters (SGR codes in between are handled by other ansi::Handler methods, not
    // `input`, so they're omitted here).
    block_list.move_backward(11);
    for ch in "starship".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "a CUB-based rewind and partial recolour reprint must not leak"
    );
}

#[test]
fn test_push_expected_echo_survives_a_cuf_advance_leaving_a_trailing_fragment() {
    // Reproduces the measured redraw with `terminal.input.honor_ps1 = false` (Warp draws the
    // prompt, so the restored line starts at column 0 and the rewind is a plain carriage
    // return rather than a backspace/CUB): after a full match, a syntax-highlighting recolour
    // pass rewinds with a carriage return, re-echoes just the 8-character command word
    // ("starship"), then skips forward over the unchanged argument with a CUF (`\x1b[<n>C`)
    // rather than re-echoing it. Without handling the CUF, the sole live candidate is left
    // stranded at position 8 (right after the recoloured word) instead of advancing past what
    // the CUF skipped over, which mismatches -- and leaks -- whatever arrives next. Uses a
    // longer buffer than the exactly-measured "starship pr" (where the CUF happens to land
    // exactly at the pattern's end) so the CUF instead lands mid-pattern, leaving a genuine
    // trailing fragment to demonstrate the fix against.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list
        .early_output_mut()
        .push_expected_echo("starship prompt");

    for ch in "starship prompt".chars() {
        block_list.input(ch);
    }

    block_list.carriage_return();
    // Recolour just the 8-character command word.
    for ch in "starship".chars() {
        block_list.input(ch);
    }
    // Skip forward over the unchanged " pr" (3 columns) without re-echoing it.
    block_list.move_forward(3);
    // The rest of the line, starting right where the CUF left off.
    for ch in "ompt".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "a CUF-based forward skip followed by the rest of the line must not leak"
    );
}

#[test]
fn test_push_expected_echo_survives_absolute_cursor_addressing_before_a_full_reprint() {
    // Reproduces PSReadLine's own redraw, measured live: it rewinds not with a carriage
    // return, a backspace or CUB, but with absolute cursor addressing (CUP, `\x1b[1;1H`), which
    // none of those three motions cover -- a candidate left only at its post-match position
    // would see the reprint's very next character as a genuine mismatch, which (per
    // `EarlyOutputHandler::input()`) would end the window on real echo. `goto`/`goto_col` are
    // treated the same way as a carriage return, just to any column (see `rearm_at_column`).
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list
        .early_output_mut()
        .push_expected_echo("starship pr");

    block_list.input('s');
    block_list.goto(VisibleRow(0), 0);
    for ch in "starship pr".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "an absolute-cursor-addressing rewind followed by a full reprint must not leak"
    );
}

#[test]
fn test_push_expected_echo_survives_absolute_cursor_addressing_to_a_nonzero_column() {
    // Reproduces PSReadLine's redraw with a nonempty prompt, measured live: the CUP addresses
    // column 29 (a 29-column prompt), not column 0, but PSReadLine always redraws the *entire*
    // buffer from its own start regardless of the column that redraw happens to land on -- so
    // this must rearm exactly like the column-0 case does, not be treated as no information
    // (which an earlier, more conservative version of this rule did, causing every restore
    // with a nonempty prompt to leak in full).
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list
        .early_output_mut()
        .push_expected_echo("starship pr");

    block_list.input('s');
    block_list.goto(VisibleRow(1), 29);
    for ch in "starship pr".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "a CUP to a nonzero column matching the prompt's own width must still rearm and not leak"
    );
}

#[test]
fn test_push_expected_echo_survives_cumulative_reecho_across_many_cup_redraws() {
    // PSReadLine's redraw of a restored buffer is not one-shot: measured, a 12-character
    // buffer is re-echoed as an increasingly long prefix across 12 separate redraws ("1",
    // "12", "123", ... rather than the full buffer once) -- each preceded by its own CUP back
    // to the buffer's start. A rearm that only fired on the *first* CUP would leave every
    // later, longer echo mismatching partway through and leak the remainder; each CUP must
    // rearm independently, which is already true here since `goto` calls `rearm_at_column`
    // unconditionally on every occurrence.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    let buffer = "echo hi";
    block_list.early_output_mut().push_expected_echo(buffer);

    for end in 1..=buffer.chars().count() {
        block_list.goto(VisibleRow(1), 29);
        for ch in buffer.chars().take(end) {
            block_list.input(ch);
        }
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "a cumulative re-echo across many CUP-preceded redraws must not leak"
    );
}

#[test]
fn test_a_rearm_can_still_leak_exactly_one_coincidentally_matching_character() {
    // The residual, acknowledged exposure of clearing on the first genuine mismatch rather
    // than on a signal that reliably lands only after the real echo has arrived: if a fully-
    // matched (and therefore already-exhausted) pattern gets rearmed by some *unrelated*
    // command's own carriage return, and that unrelated command's output happens to start
    // with a character the rearm predicts, that one coincidental character is swallowed
    // before the very next, non-matching character closes the window. This is progress over
    // no bound at all (unbounded, this could keep eating characters indefinitely across many
    // later commands), not a full fix -- the bound is "at most one character", not zero.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list.early_output_mut().push_expected_echo("gi");
    block_list.input('g');
    block_list.input('i');

    // An unrelated command's own carriage return rearms position 0 (see `rearm_at_column`),
    // without going through `start_active_block` -- e.g. the registering restore's own
    // `CompletionsFinished` never got a chance to reach `start_active_block` in between, or
    // this is that command's own prompt redraw.
    block_list.carriage_return();

    // Unrelated output starting with "g" -- coincidentally matches the rearmed position 0.
    for ch in "grep".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(
        block_list
            .background_block_mut()
            .expect("the mismatching tail must still render as real background output")
            .output_to_string(),
        "rep",
        "exactly the coincidentally-matching leading \"g\" is lost, not the whole word"
    );
}

#[test]
fn test_starting_a_real_command_clears_a_stale_expected_echo_registration() {
    // Without a clear on the transition that starts a real command, a pattern left over from
    // the last restore would otherwise persist indefinitely across every later command's own
    // carriage returns/backspaces/CUB/CUF, risking silently swallowing a character of that
    // command's own, completely unrelated output if it happened to match something in the
    // stale pattern.
    //
    // Asserts on `expected_echo`/`expected_echo_positions` directly rather than inferring the
    // reset from block routing: once `start_active_block` runs, the active block is
    // `started()`, and `BlockList`'s own dispatch routes all further input straight to it,
    // bypassing `EarlyOutputHandler` -- and therefore `consume_expected_echo` -- entirely. So a
    // character fed in after `start_active_block` can never land in a background block
    // regardless of whether the stale pattern was cleared, making that outcome unobservable
    // and not what this test is meant to pin down.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    // A restore's echo fully arrives and matches, as usual.
    block_list
        .early_output_mut()
        .push_expected_echo("starship pr");
    for ch in "starship pr".chars() {
        block_list.input(ch);
    }

    // The user then submits a real, unrelated command -- this is the transition that must
    // clear the stale registration.
    block_list.start_active_block();

    assert!(
        block_list.early_output_mut().expected_echo.is_empty(),
        "starting a real command must clear the registered text"
    );
    assert!(
        block_list
            .early_output_mut()
            .expected_echo_positions
            .is_empty(),
        "starting a real command must clear the live candidate positions"
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
fn test_push_expected_echo_survives_wrapped_line_space_fill_at_the_wrap_boundary() {
    // Reproduces a wrapped-line redraw, measured on Linux zsh: once the restored buffer is
    // longer than the terminal width minus the prompt's own width, ZLE's redraw clears the
    // remainder of the first display row with literal space characters (not an erase-to-
    // end-of-line escape) before moving to the next row with CUD (`\x1b[1B`) -- so the byte
    // stream for a buffer that exactly fills the first row and continues onto a second is the
    // first row's own characters, then space-fill, then CUD, then the rest of the buffer.
    // Measured: every byte from the wrap point on leaked into a background block, exactly
    // `total - (columns - prompt_width)` bytes -- i.e. the *first* character that fails to
    // match (a space, not part of the actual buffer text) ends the window per
    // `EarlyOutputHandler::input()`'s existing rule, so the rest of the real redraw that
    // follows leaks in full even though it does match the buffer.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list
        .early_output_mut()
        .push_expected_echo("echo xxxxxxxxxx");

    // First display row: exactly fills the (simulated) terminal width.
    for ch in "echo xxxxx".chars() {
        block_list.input(ch);
    }
    // Space-fill clearing the remainder of the row before wrapping -- doesn't match the
    // buffer's own text at this position.
    block_list.input(' ');
    block_list.input(' ');
    // CUD onto the second display row.
    block_list.move_down(1);
    // The rest of the buffer, continuing correctly on the second row.
    for ch in "xxxxx".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(
        block_list.background_block_mut().is_none(),
        "space-fill padding at a wrap boundary must not leak the rest of the redraw"
    );
}

#[test]
fn test_a_stray_space_after_a_full_match_still_leaks_normally() {
    // The space-fill swallow added for the wrapped-line case above is deliberately scoped to
    // redraws that are still genuinely in progress (see `awaiting_more_expected_echo`), not to
    // every space received while the window happens to still be open. Once the whole pattern
    // has already been matched, there is nothing left to redraw, so a stray space at that
    // point must render like any other unmatched character.
    let mut block_list = new_block_list(
        ChannelEventListener::new_for_test(),
        TypeaheadMode::ShellReported,
    );

    block_list.early_output_mut().push_expected_echo("gi");
    for ch in "gi".chars() {
        block_list.input(ch);
    }

    // Unrelated output that happens to start with a space, after the pattern is fully matched.
    for ch in " ls".chars() {
        block_list.input(ch);
    }
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(
        block_list
            .background_block_mut()
            .expect("a space arriving after the pattern is fully matched must still render")
            .output_to_string(),
        " ls"
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
