use itertools::Itertools;
use string_offset::ByteOffset;
use warp_util::path::EscapeChar;

use super::LocationType;
use crate::completer::testing::FakeCompletionContext;
use crate::completer::CompletionContext;
use crate::meta::{Span, SpannedItem};
use crate::parsers::simple::{command_at_cursor_position, parse_for_completions};
use crate::parsers::{classify_command, ParsedToken};
use crate::signatures::testing::{create_test_command_registry, test_signature};
use crate::signatures::CommandRegistry;

fn location(line: &str, registry: CommandRegistry, pos: usize) -> Vec<LocationType> {
    let ctx = FakeCompletionContext::new(registry);
    let line = &line[..pos];

    let command_to_complete = parse_for_completions(line, EscapeChar::Backslash, false)
        .expect("test command should be able to parse");
    let mut tokens = command_to_complete
        .parts
        .iter()
        .map(|s| s.as_str())
        .collect_vec();

    let classified_command = classify_command(
        command_to_complete.clone(),
        &mut tokens,
        ctx.command_registry(),
        ctx.command_case_sensitivity(),
    );

    crate::completer::engine::completion_location(&ctx, line, classified_command.as_ref())
        .into_iter()
        .map(|v| v.item)
        .collect()
}

#[test]
fn test_command_at_cursor_parse() {
    let line = r"git status $(git stash) && git checkout main";

    let parse_first_command =
        command_at_cursor_position(line, EscapeChar::Backslash, ByteOffset::from(4));
    assert!(parse_first_command.is_some());
    assert_eq!(
        Span::from_list(&parse_first_command.unwrap().parts),
        Span::new(0, 23)
    );

    let parse_second_command =
        command_at_cursor_position(line, EscapeChar::Backslash, ByteOffset::from(17));
    assert!(parse_second_command.is_some());
    assert_eq!(
        Span::from_list(&parse_second_command.unwrap().parts),
        Span::new(13, 22)
    );

    let parse_third_command =
        command_at_cursor_position(line, EscapeChar::Backslash, ByteOffset::from(28));
    assert!(parse_third_command.is_some());
    assert_eq!(
        Span::from_list(&parse_third_command.unwrap().parts),
        Span::new(27, 44)
    );

    let parse_on_boundary =
        command_at_cursor_position(line, EscapeChar::Backslash, ByteOffset::from(25));
    assert!(parse_on_boundary.is_none());

    let parse_out_of_range =
        command_at_cursor_position(line, EscapeChar::Backslash, ByteOffset::from(47));
    assert!(parse_out_of_range.is_none());
}

#[test]
fn completes_command_names() {
    assert_eq!(
        location("cargo", CommandRegistry::default(), 3),
        vec![LocationType::Command {
            is_recognized: false,
            parsed_token: ParsedToken::new("car")
        }]
    );

    assert_eq!(
        location("cargo", CommandRegistry::default(), 5),
        vec![LocationType::Command {
            is_recognized: true,
            parsed_token: ParsedToken::new("cargo")
        }]
    );

    assert_eq!(
        location("cd path/to | echo 1", CommandRegistry::default(), 13),
        vec![LocationType::Command {
            is_recognized: true,
            parsed_token: ParsedToken::empty()
        }]
    );
}

#[test]
fn completes_unregistered_command_names() {
    assert_eq!(
        location("warp", CommandRegistry::empty(), 4),
        vec![LocationType::Command {
            is_recognized: false,
            parsed_token: ParsedToken::new("warp")
        }]
    );

    assert_eq!(
        location("echo hola | sdfsd", CommandRegistry::default(), 17),
        vec![LocationType::Command {
            is_recognized: false,
            parsed_token: ParsedToken::new("sdfsd")
        }]
    );
}

#[test]
fn completes_argument_command() {
    let command = "cd ".spanned(Span::new(0, 2));
    let cursor_at_whitespace = Span::new(3, 3);
    assert_eq!(command.span.slice(command.item), "cd");
    assert_eq!(
        command.span.until(cursor_at_whitespace).slice(command.item),
        "cd "
    );
    assert_eq!(
        location(
            command.item,
            CommandRegistry::empty(),
            cursor_at_whitespace.end()
        ),
        vec![
            LocationType::Argument {
                command_name: ("cd".to_string().spanned(command.span)),
                argument_name: None,
                parsed_token: ParsedToken::empty()
            },
            LocationType::Flag {
                command_name: "cd".to_string().spanned(command.span),
                flag_name: None
            }
        ]
    );

    let command = "echo hola | clang ".spanned(Span::new(12, 17));
    let cursor_at_whitespace = Span::new(18, 18);
    assert_eq!(command.span.slice(command.item), "clang");
    assert_eq!(
        command.span.until(cursor_at_whitespace).slice(command.item),
        "clang "
    );
    assert_eq!(
        location(
            command.item,
            CommandRegistry::default(),
            cursor_at_whitespace.end()
        ),
        vec![
            LocationType::Argument {
                command_name: ("clang".to_string().spanned(command.span)),
                argument_name: None,
                parsed_token: ParsedToken::empty(),
            },
            LocationType::Flag {
                command_name: "clang".to_string().spanned(command.span),
                flag_name: None
            }
        ]
    );
}

#[test]
fn completes_flags_having_one_hyphen() {
    assert_eq!(
        location("bundle -", CommandRegistry::default(), 8),
        vec![LocationType::Flag {
            command_name: "bundle".to_owned().spanned(Span::new(0, 6)),
            flag_name: Some("-".to_owned().spanned(Span::new(7, 8)))
        }]
    );

    assert_eq!(
        location("git add -", CommandRegistry::default(), 9),
        vec![
            LocationType::Argument {
                command_name: ("git add".to_string().spanned(Span::new(0, 7))),
                argument_name: None,
                parsed_token: ParsedToken::new("-")
            },
            LocationType::Flag {
                command_name: "git add".to_string().spanned(Span::new(0, 7)),
                flag_name: Some("-".to_owned().spanned(Span::new(8, 9)))
            }
        ]
    );

    assert_eq!(
        location("echo hola | clang -", CommandRegistry::default(), 19),
        vec![
            LocationType::Argument {
                command_name: ("clang".to_string().spanned(Span::new(12, 17))),
                argument_name: None,
                parsed_token: ParsedToken::new("-")
            },
            LocationType::Flag {
                command_name: "clang".to_owned().spanned(Span::new(12, 17)),
                flag_name: Some("-".to_owned().spanned(Span::new(18, 19)))
            },
        ]
    );
}

#[test]
fn completes_flag_argument_after_equal_sign_no_value() {
    let cmd = "test".to_string().spanned(Span::new(0, 4));
    let registry = create_test_command_registry([test_signature()]);
    assert_eq!(
        location("test --long=", registry, 12),
        vec![LocationType::Argument {
            command_name: cmd,
            argument_name: Some("--long".to_owned()),
            parsed_token: ParsedToken::new(""),
        }]
    );
}

#[test]
fn completes_flag_argument_after_equal_sign_with_partial_value() {
    let cmd = "test".to_string().spanned(Span::new(0, 4));
    let registry = create_test_command_registry([test_signature()]);
    assert_eq!(
        location("test --long=lo", registry, 14),
        vec![LocationType::Argument {
            command_name: cmd,
            argument_name: Some("--long".to_owned()),
            parsed_token: ParsedToken::new("lo"),
        }]
    );
}

#[test]
fn completes_flag_argument_after_equal_sign_with_preceding_switch() {
    let cmd = "test".to_string().spanned(Span::new(0, 4));
    let registry = create_test_command_registry([test_signature()]);
    assert_eq!(
        location("test -r --long=", registry, 15),
        vec![LocationType::Argument {
            command_name: cmd,
            argument_name: Some("--long".to_owned()),
            parsed_token: ParsedToken::new(""),
        }]
    );
}

#[test]
fn completes_flag_argument_after_equal_sign_with_multiple_preceding_flags() {
    let cmd = "test".to_string().spanned(Span::new(0, 4));
    let registry = create_test_command_registry([test_signature()]);
    assert_eq!(
        location("test --not-long=bar -r --long=", registry, 30),
        vec![LocationType::Argument {
            command_name: cmd,
            argument_name: Some("--long".to_owned()),
            parsed_token: ParsedToken::new(""),
        }]
    );
}

#[test]
fn completes_flag_argument_after_equal_sign_with_preceding_space_delimited_flag() {
    let cmd = "test".to_string().spanned(Span::new(0, 4));
    let registry = create_test_command_registry([test_signature()]);
    assert_eq!(
        location("test --not-long bar --long=", registry, 27),
        vec![LocationType::Argument {
            command_name: cmd,
            argument_name: Some("--long".to_owned()),
            parsed_token: ParsedToken::new(""),
        }]
    );
}

#[test]
fn completes_after_completed_equal_sign_flag() {
    let cmd = "test".to_string().spanned(Span::new(0, 4));
    let registry = create_test_command_registry([test_signature()]);
    assert_eq!(
        location("test --long=foo ", registry, 16),
        vec![
            LocationType::Argument {
                command_name: cmd.clone(),
                argument_name: None,
                parsed_token: ParsedToken::empty(),
            },
            LocationType::Flag {
                command_name: cmd,
                flag_name: None,
            },
        ]
    );
}

#[test]
fn completes_flag_argument_after_equal_sign_with_two_preceding_switches() {
    let cmd = "test".to_string().spanned(Span::new(0, 4));
    let registry = create_test_command_registry([test_signature()]);
    assert_eq!(
        location("test -r -V --long=", registry, 18),
        vec![LocationType::Argument {
            command_name: cmd,
            argument_name: Some("--long".to_owned()),
            parsed_token: ParsedToken::new(""),
        }]
    );
}

#[test]
fn completes_flag_argument_with_all_three_flag_styles() {
    let cmd = "test".to_string().spanned(Span::new(0, 4));
    let registry = create_test_command_registry([test_signature()]);
    assert_eq!(
        location("test -r --not-long bar --long=", registry, 30),
        vec![LocationType::Argument {
            command_name: cmd,
            argument_name: Some("--long".to_owned()),
            parsed_token: ParsedToken::new(""),
        }]
    );
}
