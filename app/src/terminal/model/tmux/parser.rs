//! This module parses output from tmux control mode.
//!
//! Control mode is a special interface to tmux that allows programs to interact with tmux
//! in a structured manner, receiving information about windows, panes, and the state
//! of the server in a machine-readable format.
//!
//! Refer to the tmux control mode protocol documentation for more details:
//! https://github.com/tmux/tmux/wiki/Control-Mode

use crate::util::AsciiDebug;
#[derive(PartialEq, Eq)]
pub enum TmuxMessage {
    /// This is output from a tmux command, like send-keys or new-window. This is different from
    /// pane output, which is handled separately.
    CommandOutput {
        /// Commands can succeed or fail, which is captured by the Result variant, and can have
        /// multiple lines of output.
        output_lines: Result<Vec<Vec<u8>>, Vec<Vec<u8>>>,
    },
    WindowClose {
        window_id: u32,
    },
    Exit,
    /// Only used in development.
    Unknown {
        tag: Vec<u8>,
        rest: Vec<u8>,
    },
    ParseError {
        message: &'static str,
        byte: u8,
    },
}

/// Debug impl which formats the byte vecs as readable unicode strings.
impl std::fmt::Debug for TmuxMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TmuxMessage::CommandOutput {
                output_lines: output,
            } => match output {
                Ok(v) => {
                    let output: Vec<_> = v.iter().map(|vec| AsciiDebug(vec)).collect();
                    f.debug_struct("CommandOutput")
                        .field("output", &format!("Ok({output:?})"))
                        .finish()
                }
                Err(e) => {
                    let errors: Vec<_> = e.iter().map(|vec| AsciiDebug(vec)).collect();
                    f.debug_struct("CommandOutput")
                        .field("output", &format!("Err({errors:?})"))
                        .finish()
                }
            },
            TmuxMessage::Exit => write!(f, "Exit"),
            TmuxMessage::Unknown { tag, rest } => f
                .debug_struct("Unknown")
                .field("tag", &AsciiDebug(tag))
                .field("rest", &AsciiDebug(rest))
                .finish(),
            TmuxMessage::ParseError { message, byte } => f
                .debug_struct("ParseError")
                .field("message", message)
                .field("byte", byte)
                .finish(),
            TmuxMessage::WindowClose { window_id } => f
                .debug_struct("WindowClose")
                .field("window_id", window_id)
                .finish(),
        }
    }
}

/// Performs actions sent by `TmuxControlModeParser`.
///
/// The functions in this trait are called by the parser immediately upon parsing relevant input.
pub trait TmuxControlModeHandler {
    /// This is called by the parser whenever a byte of output from a particular pane is parsed.
    fn pane_output(&mut self, pane: u32, byte: u8);

    /// This is called by the parser for all other tmux messages, as soon as they've been parsed.
    /// See `TmuxMessage` for more details.
    fn tmux_control_mode_message(&mut self, message: TmuxMessage);
}

#[derive(Debug)]
enum ParserState {
    BeginningOfLine,
    ReadingTag {
        // The longest known tag is 'unlinked-window-renamed', which is 23 characters long.
        tag: [u8; 23],
        len: u8,
    },

    TagExit,
    TagUnknown {
        tag: Vec<u8>,  // Only set in debug builds
        args: Vec<u8>, // Only set in debug builds
    },

    TagBegin,
    ReadingCommandOutput {
        current_line: Vec<u8>,
        lines: Vec<Vec<u8>>,
    },

    TagOutput {
        maybe_pane: Option<u32>,
    },
    ReadingPaneOutput {
        pane: u32,
        maybe_escape_sequence: Option<EscapeSequence>,
    },

    TagWindowClose {
        maybe_window: Option<u32>,
    },
    Error,
}

#[derive(Debug)]
struct EscapeSequence {
    char: u8,
    remaining_digits: u8,
}

/// This parses output from tmux control mode. It expects the output to be well formed and does a
/// best-effort job to identify, report, and recover from errors.
/// Specifically:
/// - The parser correctly parses all well-formed output.
/// - The parser will not crash or infinite loop, whether or not the input is well-formed.
/// - Some mal-formed input may result in an incorrect parse, but most will result in a parse error.
/// - If the parser continues to get input after a parse error, it will attempt to recover after
///   the next newline.
///
/// N.B. When working with control mode output, it's important to understand the distinction
/// between tmux command output (e.g. the output of running the `list-windows` tmux command) with
/// tmux pane output (e.g. the output produced by running `ls` in pane 0).
///
/// For more information on the expected output format, see: https://github.com/tmux/tmux/wiki/Control-Mode
#[derive(Debug)]
pub struct TmuxControlModeParser {
    state: ParserState,
}

impl TmuxControlModeParser {
    pub fn new() -> Self {
        TmuxControlModeParser {
            state: ParserState::BeginningOfLine,
        }
    }

    /// The primary interface to the parser. Takes a handler and the next byte of output, and
    /// calls one of the handler methods when it parses a byte of pane output or a complete tmux
    /// message.
    pub fn advance(&mut self, handler: &mut impl TmuxControlModeHandler, byte: u8) {
        if byte == b'\r' {
            // This should only ever appear directly before a \n and it simplifies parsing
            // if we just discard carriage returns and only look for newlines.
            return;
        }
        match &mut self.state {
            ParserState::BeginningOfLine => {
                // Input prior to this state:  ""
                // Input parsed in this state: "%"
                if byte != b'%' {
                    report_parse_error(
                        handler,
                        "Received non-% character at the beginning of a line",
                        byte,
                    );
                    self.state = ParserState::Error;
                    return;
                }

                self.state = ParserState::ReadingTag {
                    tag: Default::default(),
                    len: 0,
                };
            }
            ParserState::ReadingTag { tag, len } => {
                // Input prior to this state:  "%"
                // Input parsed in this state: "%[tag]"
                let current_len = *len as usize;
                if byte == b' ' || byte == b'\n' {
                    // A space or a new line means we've reached the end of the tag (e.g.
                    // "%something").
                    match &tag[..current_len] {
                        b"begin" => {
                            self.state = ParserState::TagBegin;
                        }
                        b"exit" => {
                            self.state = ParserState::TagExit;
                        }
                        b"output" => {
                            self.state = ParserState::TagOutput { maybe_pane: None };
                        }
                        b"window-close" | b"unlinked-window-close" => {
                            self.state = ParserState::TagWindowClose { maybe_window: None };
                        }
                        _ => {
                            self.state = ParserState::TagUnknown {
                                tag: if cfg!(debug_assertions) {
                                    // We only care about the contents of unknown tags in debug
                                    // builds.
                                    tag[..current_len].to_owned()
                                } else {
                                    Vec::new()
                                },
                                args: Vec::new(),
                            };
                        }
                    }
                    // The parser is now set up to parse the particular message based on the tag we
                    // matched. That message may be parsed differently depending on whether we've
                    // encountered a space or a newline, so we call into this method again to
                    // continue the parse with the same byte.
                    return self.advance(handler, byte);
                }

                // Ignore any bits that are longer than our max tag length.
                if current_len < tag.len() {
                    tag[current_len] = byte;
                    *len += 1;
                }
            }
            ParserState::Error => {
                // Input prior to this state:  "[any unexpected input]"
                // Input parsed in this state: "[any more input]\n"
                //
                // Discard input until we see a newline and then try to recover.
                if byte == b'\n' {
                    self.state = ParserState::BeginningOfLine;
                }
            }

            ParserState::TagExit => {
                // Input prior to this state:  "%exit"
                // Input parsed in this state: "\n"
                if byte != b'\n' {
                    report_parse_error(handler, "Extraneous byte after %exit", byte);
                    self.state = ParserState::Error;
                    return;
                }

                handler.tmux_control_mode_message(TmuxMessage::Exit);
                self.state = ParserState::BeginningOfLine;
            }

            ParserState::TagUnknown { tag, args } => {
                // Input prior to this state:  "%[unknown tag]"
                // Input parsed in this state: " [rest of the line]\n"
                if byte == b'\n' {
                    if cfg!(debug_assertions) {
                        handler.tmux_control_mode_message(TmuxMessage::Unknown {
                            tag: std::mem::take(tag),
                            rest: std::mem::take(args),
                        });
                    }
                    self.state = ParserState::BeginningOfLine;
                    return;
                }

                if cfg!(debug_assertions) {
                    // We only care about the contents of unknown tags in debug builds.
                    args.push(byte);
                }
            }
            ParserState::TagBegin => {
                // Input prior to this state:  "%begin"
                // Input parsed in this state: " [seconds from epoch] [unique command number] [flags]\n"
                // We don't care about any of the arguments, so discard input until we get a newline.
                if byte == b'\n' {
                    self.state = ParserState::ReadingCommandOutput {
                        current_line: Vec::new(),
                        lines: Vec::new(),
                    };
                }
            }
            ParserState::ReadingCommandOutput {
                current_line,
                lines,
            } => {
                // Input prior to this state:  "%begin [seconds from epoch] [unique command number] [flags]\n"
                // Input parsed in this state: some number of "[command output line]\n" followed by "%end\n" or "%error\n"
                if byte == b'\n' {
                    // Check to see if command output is complete.
                    if current_line.starts_with(b"%end") {
                        handler.tmux_control_mode_message(TmuxMessage::CommandOutput {
                            output_lines: Ok(std::mem::take(lines)),
                        });
                        self.state = ParserState::BeginningOfLine;
                    } else if current_line.starts_with(b"%error") {
                        handler.tmux_control_mode_message(TmuxMessage::CommandOutput {
                            output_lines: Err(std::mem::take(lines)),
                        });
                        self.state = ParserState::BeginningOfLine;
                    } else {
                        // Command output still ongoing -- append to list of lines.
                        lines.push(std::mem::take(current_line));
                    }
                    return;
                }

                // In the middle of a line.
                current_line.push(byte);
            }
            ParserState::TagOutput { maybe_pane } => {
                // Input prior to this state:  "%output"
                // Input parsed in this state: " %[pane id] "
                if let &mut Some(pane) = maybe_pane {
                    // We're parsing the pane number.
                    if byte.is_ascii_digit() {
                        // Got another digit of the pane number.
                        *maybe_pane = Some(pane * 10 + (byte - b'0') as u32);
                    } else if byte == b' ' {
                        // Pane number finished.
                        self.state = ParserState::ReadingPaneOutput {
                            pane,
                            maybe_escape_sequence: None,
                        };
                    } else {
                        report_parse_error(
                            handler,
                            "Non-digit character in %output pane number",
                            byte,
                        );
                        self.state = ParserState::Error;
                    }
                } else {
                    // We haven't started parsing the pane number yet.
                    if byte == b'%' {
                        // Pane number starting.
                        *maybe_pane = Some(0);
                    } else if byte == b' ' {
                        // Ignore spaces.
                    } else {
                        report_parse_error(handler, "Unexpected character after %output", byte);
                    }
                }
            }

            ParserState::ReadingPaneOutput {
                pane,
                maybe_escape_sequence,
            } => {
                // Input prior to this state:  "%output %[pane id] "
                // Input parsed in this state: "[escaped command output]\n"
                //
                // The escaped output format, according to the docs:
                //   The output has any characters less than ASCII 32 and the \ character replaced
                //   with their octal equivalent, so \ becomes \134. Otherwise, it is exactly what
                //   the application running in the pane sent to tmux. It may not be valid UTF-8 and
                //   may contain escape sequences which will be as expected by tmux (so for
                //   TERM=screen or TERM=tmux).
                if let Some(escape_sequence) = maybe_escape_sequence {
                    match byte {
                        b'0'..=b'8' => {
                            escape_sequence.remaining_digits -= 1;
                            let octal_digit = byte - b'0';
                            // Put each octal digit in the right spot.
                            // The left shift is equivalent to a multiplication by
                            // (8^remaining_digits).
                            escape_sequence.char |=
                                octal_digit << (escape_sequence.remaining_digits * 3);

                            if escape_sequence.remaining_digits == 0 {
                                handler.pane_output(*pane, escape_sequence.char);
                                *maybe_escape_sequence = None;
                            }
                        }
                        _ => {
                            // Not 0-8 in escape sequence
                            report_parse_error(
                                handler,
                                "Non-octal digit found in output escape sequence",
                                byte,
                            );
                            self.state = ParserState::Error;
                        }
                    }
                } else {
                    match byte {
                        b'\n' => {
                            // Pane output over
                            self.state = ParserState::BeginningOfLine;
                        }
                        b'\\' => {
                            // Begin char escape
                            *maybe_escape_sequence = Some(EscapeSequence {
                                char: 0,
                                remaining_digits: 3,
                            })
                        }
                        byte if byte < 32 => {
                            // All bytes < 32 are supposed to be escaped in output mode.
                            report_parse_error(
                                handler,
                                "Unescaped character < ASCII 32 found in output",
                                byte,
                            );
                            self.state = ParserState::Error;
                        }
                        byte => {
                            // Standard input character
                            handler.pane_output(*pane, byte);
                        }
                    }
                }
            }
            ParserState::TagWindowClose { maybe_window } => {
                // Input prior to this state:  "%window-close" | "%unlinked-window-close"
                // Input parsed in this state: " @[window id]\n"
                if let &mut Some(window) = maybe_window {
                    // We're parsing the window number.
                    if byte.is_ascii_digit() {
                        // Got another digit of the pane number.
                        *maybe_window = Some(window * 10 + (byte - b'0') as u32);
                    } else if byte == b'\n' {
                        // Message finished.
                        handler.tmux_control_mode_message(TmuxMessage::WindowClose {
                            window_id: window,
                        });
                        self.state = ParserState::BeginningOfLine;
                    } else {
                        report_parse_error(
                            handler,
                            "Non-digit character in %window-close window number",
                            byte,
                        );
                        self.state = ParserState::Error;
                    }
                } else {
                    // We haven't started parsing the pane number yet.
                    if byte == b'@' {
                        // Window number starting.
                        *maybe_window = Some(0);
                    } else if byte == b' ' {
                        // Ignore spaces.
                    } else {
                        report_parse_error(
                            handler,
                            "Unexpected character after %window-close",
                            byte,
                        );
                    }
                }
            }
        }
    }
}

impl Default for TmuxControlModeParser {
    fn default() -> Self {
        Self::new()
    }
}

fn report_parse_error(handler: &mut impl TmuxControlModeHandler, message: &'static str, byte: u8) {
    handler.tmux_control_mode_message(TmuxMessage::ParseError { message, byte })
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
