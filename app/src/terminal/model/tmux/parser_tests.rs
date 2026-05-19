use super::*;
use std::collections::HashMap;

struct TestHandler {
    output: HashMap<u32, Vec<u8>>,
    messages: Vec<TmuxMessage>,
}

impl TestHandler {
    fn new() -> Self {
        TestHandler {
            output: HashMap::new(),
            messages: Vec::new(),
        }
    }
}

impl TmuxControlModeHandler for TestHandler {
    fn pane_output(&mut self, pane: u32, byte: u8) {
        self.output.entry(pane).or_default().push(byte);
    }

    fn tmux_control_mode_message(&mut self, message: TmuxMessage) {
        self.messages.push(message);
    }
}

#[test]
fn test_valid_command_output_ok() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%begin 1622462330 1\ndummy output\n%end\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    assert_eq!(
        &handler.messages[0],
        &TmuxMessage::CommandOutput {
            output_lines: Ok(vec![b"dummy output".to_vec()])
        }
    );
}

#[test]
fn test_valid_command_output_err() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%begin 1622462330 1\ndummy error\n%error\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    assert_eq!(
        &handler.messages[0],
        &TmuxMessage::CommandOutput {
            output_lines: Err(vec![b"dummy error".to_vec()])
        }
    );
}

#[test]
fn test_exit_message() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%exit\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    assert_eq!(&handler.messages[0], &TmuxMessage::Exit);
}

#[test]
fn test_unknown_message() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%unknown something\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    assert_eq!(
        &handler.messages[0],
        &TmuxMessage::Unknown {
            tag: b"unknown".to_vec(),
            rest: b" something".to_vec(),
        }
    );
}

#[test]
fn test_error_message() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"non-percent\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    match &handler.messages[0] {
        &TmuxMessage::ParseError { message: _, byte } => {
            assert_eq!(byte, b'n');
        }
        _ => panic!("Expected Error message"),
    }
}

#[test]
fn test_pane_output() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%output %0 here is some output\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(
        handler.output.get(&0),
        Some(&b"here is some output".to_vec())
    )
}

#[test]
fn test_pane_output_escape_sequence() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%output %1 \\1345\\015\\012\\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.output.get(&1), Some(&b"\\5\r\n".to_vec()))
}

#[test]
fn test_pane_output_multiline() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%output %0 first line\\012second line\\012third line\\012\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(
        handler.output.get(&0),
        Some(&b"first line\nsecond line\nthird line\n".to_vec())
    )
}

#[test]
fn test_pane_output_split() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%output %0 one\n%output %0 two\n%output %0 three\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.output.get(&0), Some(&b"onetwothree".to_vec()))
}

#[test]
fn test_pane_output_incomplete() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%output %0 incomplete";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 0);
    // Output should be written immediately.
    assert_eq!(handler.output.get(&0), Some(&b"incomplete".to_vec()))
}

#[test]
fn test_pane_output_incomplete_escape() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%output %0 incomplete\\01";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 0);
    // Output should be written as soon as it's ready. Incomplete escape sequences should not be
    // written.
    assert_eq!(handler.output.get(&0), Some(&b"incomplete".to_vec()))
}

#[test]
fn test_pane_output_invalid_escape_sequence() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%output %0 \\a";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    match &handler.messages[0] {
        &TmuxMessage::ParseError { message: _, byte } => {
            assert_eq!(byte, b'a');
        }
        _ => panic!("Expected Error message"),
    }
}

#[test]
fn test_pane_output_invalid_character() {
    // Characters below ASCII 32 should be escaped in the output. Tab is ASCII 9.
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%output %0 \t";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    match &handler.messages[0] {
        &TmuxMessage::ParseError { message: _, byte } => {
            assert_eq!(byte, b'\t');
        }
        _ => panic!("Expected Error message"),
    }
}

#[test]
fn test_begin_without_end() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%begin 1622462330 1\nsome output\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 0);
}

// Regression test for issue #9900. Previously the parser was permanently
// stuck inside `ReadingCommandOutput` if `%end`/`%error` never arrived: the
// SSH client's disconnect diagnostics, the local zsh re-prompt, and Warp's
// still-queued wrapper command bytes all got swallowed as "command output
// content." Now, an OpenSSH disconnect line breaks the begin/end block via
// ParseError so the existing recovery path tears down control mode.
#[test]
fn ssh_disconnect_inside_begin_end_emits_parse_error() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    // 1. tmux had started responding to a wrapper command.
    let begin = b"%begin 1622462330 1 0\n";
    // 2. SSH transport dies. Local OpenSSH prints its disconnect diagnostics
    //    to the PTY. None of this is tmux protocol, but it arrives between
    //    `%begin` and an `%end` that will never come.
    let ssh_garbage: &[u8] = b"Read from remote host 10.0.0.1: Operation timed out\n";

    for &byte in begin.iter().chain(ssh_garbage.iter()) {
        parser.advance(&mut handler, byte);
    }

    // The OpenSSH-specific phrase is detected inside the begin/end block,
    // so we emit a ParseError. The performer (in ansi/mod.rs) translates
    // that into `exited = true` plus a full control-mode state reset.
    assert_eq!(handler.messages.len(), 1);
    assert!(
        matches!(handler.messages[0], TmuxMessage::ParseError { .. }),
        "expected ParseError on OpenSSH disconnect inside %begin/%end, got {:?}",
        handler.messages[0]
    );
}

#[test]
fn ssh_disconnect_client_loop_send_disconnect_pattern_triggers_parse_error() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%begin 1 1 0\nclient_loop: send disconnect: Broken pipe\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    assert!(matches!(
        handler.messages[0],
        TmuxMessage::ParseError { .. }
    ));
}

#[test]
fn ssh_disconnect_connection_to_host_closed_pattern_triggers_parse_error() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%begin 1 1 0\nConnection to 192.0.2.1 closed.\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    assert!(matches!(
        handler.messages[0],
        TmuxMessage::ParseError { .. }
    ));
}

// `Broken pipe` alone is too generic — many unrelated tools print it. We
// must NOT escape the begin/end block on this string alone, otherwise a
// remote command whose stderr happens to mention `Broken pipe` would
// incorrectly tear down control mode mid-flight.
#[test]
fn lone_broken_pipe_does_not_trigger_ssh_disconnect_detection() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%begin 1 1 0\nsome remote tool said: Broken pipe\nfurther output\n%end\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    // The whole begin/end block completes normally with both output lines
    // captured. No spurious ParseError.
    assert_eq!(handler.messages.len(), 1);
    match &handler.messages[0] {
        TmuxMessage::CommandOutput {
            output_lines: Ok(lines),
        } => {
            assert_eq!(lines.len(), 2);
        }
        other => panic!("expected CommandOutput Ok with 2 lines, got {other:?}"),
    }
}

// `Connection to ` and ` closed` must both appear on the SAME line to count.
// A remote command that prints them on separate lines (or with other words
// between them on separate lines) should not trigger detection.
#[test]
fn connection_to_and_closed_on_different_lines_does_not_trigger() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%begin 1 1 0\nConnection to remote\nthe port was closed\n%end\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    assert!(matches!(
        handler.messages[0],
        TmuxMessage::CommandOutput {
            output_lines: Ok(_)
        }
    ));
}

#[test]
fn ssh_disconnect_packet_write_wait_pattern_triggers_parse_error() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%begin 1 1 0\npacket_write_wait: Connection to 10.0.0.1 port 22: Broken pipe\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    assert!(matches!(
        handler.messages[0],
        TmuxMessage::ParseError { .. }
    ));
}

#[test]
fn ssh_disconnect_ssh_exchange_identification_pattern_triggers_parse_error() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%begin 1 1 0\nssh_exchange_identification: read: Connection reset by peer\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    assert!(matches!(
        handler.messages[0],
        TmuxMessage::ParseError { .. }
    ));
}

#[test]
fn ssh_disconnect_kex_exchange_identification_pattern_triggers_parse_error() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = b"%begin 1 1 0\nkex_exchange_identification: Connection closed by remote host\n";
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 1);
    assert!(matches!(
        handler.messages[0],
        TmuxMessage::ParseError { .. }
    ));
}

// End-to-end byte stream from issue #9900: tmux had just sent `%begin`, then
// the wrapper command's own echo (from the `(builtin echo -n "^^^..."; ...)`
// payload) starts arriving, the SSH transport dies, the local OpenSSH client
// prints its disconnect diagnostics, and the local zsh's prompt would come
// next. We must escape the begin/end block at the disconnect line so the
// trailing local-shell bytes do not get silently absorbed.
#[test]
fn issue_9900_repro_wrapper_echo_then_ssh_death_emits_parse_error() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input: &[u8] = b"%begin 1622462330 1 0\n\
        ^^^1777710953501548|||\n\
        Read from remote host 10.0.0.1: Operation timed out\n\
        Connection to 10.0.0.1 closed.\n\
        client_loop: send disconnect: Broken pipe\n";

    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    // The wrapper echo line `^^^...|||` was absorbed as content (expected -- it
    // is not a disconnect signal). On the very next line we hit
    // `Read from remote host`, which fires ParseError. Subsequent OpenSSH
    // diagnostic lines may also fire ParseError because the test loop feeds
    // every byte; in production, the outer `ansi/mod.rs` byte loop breaks on
    // the first `exited = true` so only one fires. Either way, the FIRST
    // message must be ParseError -- that is what tears down control mode
    // before any further bytes can leak.
    assert!(
        !handler.messages.is_empty(),
        "expected at least one ParseError, got no messages"
    );
    assert!(
        matches!(handler.messages[0], TmuxMessage::ParseError { .. }),
        "expected first message to be ParseError, got {:?}",
        handler.messages[0]
    );
}

#[test]
fn test_example_output() {
    let mut parser = TmuxControlModeParser::new();
    let mut handler = TestHandler::new();

    let input = br#"%begin 1578920019 258 0
%end 1578920019 258 0
%window-add @1
%sessions-changed
%session-changed $1 1
%window-renamed @1 tmux
%output %1 nicholas@yelena:~$
%window-renamed @1 ksh
%exit
"#;
    for &byte in input {
        parser.advance(&mut handler, byte);
    }

    assert_eq!(handler.messages.len(), 7);
    assert_eq!(
        &handler.messages[0],
        &TmuxMessage::CommandOutput {
            output_lines: Ok(vec![])
        }
    );
    assert_eq!(
        &handler.messages[1],
        &TmuxMessage::Unknown {
            tag: b"window-add".to_vec(),
            rest: b" @1".to_vec(),
        }
    );
    assert_eq!(
        &handler.messages[2],
        &TmuxMessage::Unknown {
            tag: b"sessions-changed".to_vec(),
            rest: b"".to_vec(),
        }
    );
    assert_eq!(
        &handler.messages[3],
        &TmuxMessage::Unknown {
            tag: b"session-changed".to_vec(),
            rest: b" $1 1".to_vec(),
        }
    );
    assert_eq!(
        &handler.messages[4],
        &TmuxMessage::Unknown {
            tag: b"window-renamed".to_vec(),
            rest: b" @1 tmux".to_vec(),
        }
    );
    assert_eq!(
        &handler.messages[5],
        &TmuxMessage::Unknown {
            tag: b"window-renamed".to_vec(),
            rest: b" @1 ksh".to_vec(),
        }
    );
    assert_eq!(&handler.messages[6], &TmuxMessage::Exit);
    assert_eq!(
        handler.output.get(&1),
        Some(&b"nicholas@yelena:~$".to_vec())
    )
}
