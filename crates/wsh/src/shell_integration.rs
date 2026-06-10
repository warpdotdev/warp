use std::fmt;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const RIGHT_BRACKET: u8 = 0x5d; // ']'
const BACKSLASH: u8 = 0x5c;     // '\'
const SEMICOLON: u8 = 0x3b;     // ';'

#[derive(Debug, Clone, PartialEq)]
pub enum ShellEvent {
    PromptStart,
    PromptEnd,
    CommandStart,
    CommandFinished { exit_code: Option<i32> },
}

impl fmt::Display for ShellEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellEvent::PromptStart => write!(f, "PromptStart"),
            ShellEvent::PromptEnd => write!(f, "PromptEnd"),
            ShellEvent::CommandStart => write!(f, "CommandStart"),
            ShellEvent::CommandFinished { exit_code: Some(c) } => {
                write!(f, "CommandFinished({c})")
            }
            ShellEvent::CommandFinished { exit_code: None } => {
                write!(f, "CommandFinished(?)")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ParserState {
    Normal,
    Escape,
    /// Accumulating OSC header bytes, expecting "133;"
    OscHeader { collected: Vec<u8> },
    /// Inside an OSC 133 body, accumulating the command payload.
    OscBody { payload: Vec<u8>, got_esc: bool },
    /// Inside a non-133 OSC sequence — pass through everything until the terminator.
    OscPassthrough { buffered: Vec<u8>, got_esc: bool },
}

pub struct OscParser {
    state: ParserState,
}

impl Default for OscParser {
    fn default() -> Self {
        Self::new()
    }
}

impl OscParser {
    pub fn new() -> Self {
        Self {
            state: ParserState::Normal,
        }
    }

    pub fn feed(&mut self, input: &[u8]) -> (Vec<u8>, Vec<ShellEvent>) {
        let mut output = Vec::with_capacity(input.len());
        let mut events = Vec::new();

        for &byte in input {
            match std::mem::replace(&mut self.state, ParserState::Normal) {
                ParserState::Normal => {
                    if byte == ESC {
                        self.state = ParserState::Escape;
                    } else {
                        output.push(byte);
                    }
                }

                ParserState::Escape => {
                    if byte == RIGHT_BRACKET {
                        self.state = ParserState::OscHeader {
                            collected: Vec::new(),
                        };
                    } else {
                        // Not an OSC — flush ESC + this byte as normal output.
                        output.push(ESC);
                        output.push(byte);
                    }
                }

                ParserState::OscHeader { mut collected } => {
                    const EXPECTED: &[u8] = b"133;";

                    collected.push(byte);
                    let len = collected.len();

                    if collected[len - 1] != EXPECTED[len - 1] {
                        // Mismatch — this is a non-133 OSC. Flush what we have
                        // and switch to passthrough mode.
                        let mut buffered = Vec::with_capacity(2 + collected.len());
                        buffered.push(ESC);
                        buffered.push(RIGHT_BRACKET);
                        buffered.extend_from_slice(&collected);
                        self.state = ParserState::OscPassthrough {
                            buffered,
                            got_esc: false,
                        };
                    } else if len == EXPECTED.len() {
                        // Matched "133;" — now accumulate the body.
                        self.state = ParserState::OscBody {
                            payload: Vec::new(),
                            got_esc: false,
                        };
                    } else {
                        // Partial match so far, keep going.
                        self.state = ParserState::OscHeader { collected };
                    }
                }

                ParserState::OscBody { mut payload, got_esc } => {
                    if got_esc {
                        if byte == BACKSLASH {
                            // ST terminator — sequence complete.
                            Self::parse_osc133_payload(&payload, &mut events);
                        } else {
                            // ESC was not followed by '\' — it's part of the payload.
                            payload.push(ESC);
                            payload.push(byte);
                            self.state = ParserState::OscBody {
                                payload,
                                got_esc: false,
                            };
                        }
                    } else if byte == BEL {
                        // BEL terminator — sequence complete.
                        Self::parse_osc133_payload(&payload, &mut events);
                    } else if byte == ESC {
                        self.state = ParserState::OscBody {
                            payload,
                            got_esc: true,
                        };
                    } else {
                        payload.push(byte);
                        self.state = ParserState::OscBody {
                            payload,
                            got_esc: false,
                        };
                    }
                }

                ParserState::OscPassthrough { mut buffered, got_esc } => {
                    if got_esc {
                        buffered.push(ESC);
                        if byte == BACKSLASH {
                            // ST terminator — flush entire passthrough sequence.
                            buffered.push(byte);
                            output.extend_from_slice(&buffered);
                        } else {
                            buffered.push(byte);
                            self.state = ParserState::OscPassthrough {
                                buffered,
                                got_esc: false,
                            };
                        }
                    } else if byte == BEL {
                        buffered.push(byte);
                        output.extend_from_slice(&buffered);
                    } else if byte == ESC {
                        self.state = ParserState::OscPassthrough {
                            buffered,
                            got_esc: true,
                        };
                    } else {
                        buffered.push(byte);
                        self.state = ParserState::OscPassthrough {
                            buffered,
                            got_esc: false,
                        };
                    }
                }
            }
        }

        (output, events)
    }

    fn parse_osc133_payload(payload: &[u8], events: &mut Vec<ShellEvent>) {
        if payload.is_empty() {
            return;
        }

        let command = payload[0];
        match command {
            b'A' => events.push(ShellEvent::PromptStart),
            b'B' => events.push(ShellEvent::PromptEnd),
            b'C' => events.push(ShellEvent::CommandStart),
            b'D' => {
                let exit_code = if payload.len() > 1 && payload[1] == SEMICOLON {
                    std::str::from_utf8(&payload[2..])
                        .ok()
                        .and_then(|s| s.parse::<i32>().ok())
                } else {
                    None
                };
                events.push(ShellEvent::CommandFinished { exit_code });
            }
            _ => {
                // Unknown OSC 133 command — silently drop it.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_prompt_start_bel() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(b"\x1b]133;A\x07");
        assert!(output.is_empty());
        assert_eq!(events, vec![ShellEvent::PromptStart]);
    }

    #[test]
    fn parse_prompt_start_st() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(b"\x1b]133;A\x1b\\");
        assert!(output.is_empty());
        assert_eq!(events, vec![ShellEvent::PromptStart]);
    }

    #[test]
    fn parse_prompt_end() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(b"\x1b]133;B\x07");
        assert!(output.is_empty());
        assert_eq!(events, vec![ShellEvent::PromptEnd]);
    }

    #[test]
    fn parse_command_start() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(b"\x1b]133;C\x07");
        assert!(output.is_empty());
        assert_eq!(events, vec![ShellEvent::CommandStart]);
    }

    #[test]
    fn parse_command_finished_with_exit_code() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(b"\x1b]133;D;0\x07");
        assert!(output.is_empty());
        assert_eq!(
            events,
            vec![ShellEvent::CommandFinished {
                exit_code: Some(0)
            }]
        );
    }

    #[test]
    fn parse_command_finished_nonzero_exit() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(b"\x1b]133;D;127\x07");
        assert!(output.is_empty());
        assert_eq!(
            events,
            vec![ShellEvent::CommandFinished {
                exit_code: Some(127)
            }]
        );
    }

    #[test]
    fn parse_command_finished_no_exit_code() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(b"\x1b]133;D\x07");
        assert!(output.is_empty());
        assert_eq!(
            events,
            vec![ShellEvent::CommandFinished { exit_code: None }]
        );
    }

    #[test]
    fn mixed_content_stripped() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(b"hello\x1b]133;A\x07world");
        assert_eq!(output, b"helloworld");
        assert_eq!(events, vec![ShellEvent::PromptStart]);
    }

    #[test]
    fn split_across_feeds_esc_then_rest() {
        let mut parser = OscParser::new();
        let (out1, ev1) = parser.feed(b"before\x1b");
        assert_eq!(out1, b"before");
        assert!(ev1.is_empty());

        let (out2, ev2) = parser.feed(b"]133;A\x07after");
        assert_eq!(out2, b"after");
        assert_eq!(ev2, vec![ShellEvent::PromptStart]);
    }

    #[test]
    fn split_across_feeds_mid_header() {
        let mut parser = OscParser::new();
        let (out1, ev1) = parser.feed(b"\x1b]13");
        assert!(out1.is_empty());
        assert!(ev1.is_empty());

        let (out2, ev2) = parser.feed(b"3;B\x07");
        assert!(out2.is_empty());
        assert_eq!(ev2, vec![ShellEvent::PromptEnd]);
    }

    #[test]
    fn split_across_feeds_mid_body() {
        let mut parser = OscParser::new();
        let (out1, ev1) = parser.feed(b"\x1b]133;D;12");
        assert!(out1.is_empty());
        assert!(ev1.is_empty());

        let (out2, ev2) = parser.feed(b"7\x07");
        assert!(out2.is_empty());
        assert_eq!(
            ev2,
            vec![ShellEvent::CommandFinished {
                exit_code: Some(127)
            }]
        );
    }

    #[test]
    fn split_st_terminator_across_feeds() {
        let mut parser = OscParser::new();
        let (out1, ev1) = parser.feed(b"\x1b]133;C\x1b");
        assert!(out1.is_empty());
        assert!(ev1.is_empty());

        let (out2, ev2) = parser.feed(b"\x5c");
        assert!(out2.is_empty());
        assert_eq!(ev2, vec![ShellEvent::CommandStart]);
    }

    #[test]
    fn non_133_osc_passes_through() {
        let mut parser = OscParser::new();
        // OSC 0 (set window title): ESC ] 0 ; t i t l e BEL
        let input = b"\x1b]0;my title\x07";
        let (output, events) = parser.feed(input);
        assert_eq!(output, input.to_vec());
        assert!(events.is_empty());
    }

    #[test]
    fn non_133_osc_with_st_passes_through() {
        let mut parser = OscParser::new();
        let input = b"\x1b]0;title\x1b\\";
        let (output, events) = parser.feed(input);
        assert_eq!(output, input.to_vec());
        assert!(events.is_empty());
    }

    #[test]
    fn multiple_events_in_one_feed() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(
            b"\x1b]133;D;0\x07\x1b]133;A\x07prompt$ \x1b]133;B\x07",
        );
        assert_eq!(output, b"prompt$ ");
        assert_eq!(
            events,
            vec![
                ShellEvent::CommandFinished {
                    exit_code: Some(0)
                },
                ShellEvent::PromptStart,
                ShellEvent::PromptEnd,
            ]
        );
    }

    #[test]
    fn bare_esc_not_followed_by_bracket_passes_through() {
        let mut parser = OscParser::new();
        // ESC [ is CSI, not OSC — should pass through.
        let (output, events) = parser.feed(b"\x1b[31mred\x1b[0m");
        assert_eq!(output, b"\x1b[31mred\x1b[0m");
        assert!(events.is_empty());
    }

    #[test]
    fn interleaved_normal_escape_sequences_and_osc133() {
        let mut parser = OscParser::new();
        let (output, events) =
            parser.feed(b"\x1b[32mgreen\x1b[0m\x1b]133;A\x07$ ");
        assert_eq!(output, b"\x1b[32mgreen\x1b[0m$ ");
        assert_eq!(events, vec![ShellEvent::PromptStart]);
    }

    #[test]
    fn empty_input() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(b"");
        assert!(output.is_empty());
        assert!(events.is_empty());
    }

    #[test]
    fn plain_text_no_escapes() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(b"just plain text\n");
        assert_eq!(output, b"just plain text\n");
        assert!(events.is_empty());
    }

    #[test]
    fn negative_exit_code() {
        let mut parser = OscParser::new();
        let (output, events) = parser.feed(b"\x1b]133;D;-1\x07");
        assert!(output.is_empty());
        assert_eq!(
            events,
            vec![ShellEvent::CommandFinished {
                exit_code: Some(-1)
            }]
        );
    }

    #[test]
    fn full_prompt_cycle() {
        let mut parser = OscParser::new();
        // Simulate a full prompt cycle: D;0, A, prompt text, B, user types, C
        let stream = b"\x1b]133;D;0\x07\x1b]133;A\x07user@host:~$ \x1b]133;B\x07ls -la\x1b]133;C\x07";
        let (output, events) = parser.feed(stream);
        assert_eq!(output, b"user@host:~$ ls -la");
        assert_eq!(
            events,
            vec![
                ShellEvent::CommandFinished { exit_code: Some(0) },
                ShellEvent::PromptStart,
                ShellEvent::PromptEnd,
                ShellEvent::CommandStart,
            ]
        );
    }

    #[test]
    fn byte_at_a_time() {
        let mut parser = OscParser::new();
        let input = b"x\x1b]133;A\x07y";
        let mut all_output = Vec::new();
        let mut all_events = Vec::new();
        for &byte in input {
            let (out, evts) = parser.feed(&[byte]);
            all_output.extend_from_slice(&out);
            all_events.extend(evts);
        }
        assert_eq!(all_output, b"xy");
        assert_eq!(all_events, vec![ShellEvent::PromptStart]);
    }
}
