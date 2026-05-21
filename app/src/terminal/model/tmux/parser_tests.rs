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
