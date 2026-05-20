use std::collections::HashSet;

use warp_util::path::EscapeChar;

use crate::parsers::simple::{decompose_command, top_level_command};

use super::super::lexer::Lexer;
use super::super::{Command, Part};
use super::*;

#[test]
fn test_parse_open_subshell() {
    let source = r#"cat "Hello $(ls -la"#;
    let command = Parser::new(Lexer::new(source, EscapeChar::Backslash, false)).parse_command();

    assert_eq!(
        command,
        Command::new(vec![
            Part::Literal("cat".into()).spanned((0, 3)),
            Part::Concatenated(vec![
                Part::Literal("Hello ".into()).spanned((4, 11)),
                Part::OpenSubshell(vec![Command::new(vec![
                    Part::Literal("ls".into()).spanned((13, 15)),
                    Part::Literal("-la".into()).spanned((16, 19)),
                ])
                .spanned((13, 19)),])
                .spanned((11, 19)),
            ])
            .spanned((4, 19)),
        ])
        .spanned((0, 19)),
    );
}

#[test]
fn test_parse_nested_command() {
    let source = r#"cat "Hello $(ls -la)""#;
    let command = Parser::new(Lexer::new(source, EscapeChar::Backslash, false)).parse_command();

    assert_eq!(
        command,
        Command::new(vec![
            Part::Literal("cat".into()).spanned((0, 3)),
            Part::Concatenated(vec![
                Part::Literal("Hello ".into()).spanned((4, 11)),
                Part::ClosedSubshell(vec![Command::new(vec![
                    Part::Literal("ls".into()).spanned((13, 15)),
                    Part::Literal("-la".into()).spanned((16, 19)),
                ])
                .spanned((13, 19)),])
                .spanned((11, 20)),
            ])
            .spanned((4, 21)),
        ])
        .spanned((0, 21))
    );
}

#[test]
fn test_parse() {
    let source = r#"ls | rm -rf || touch 'hello.txt\' &
cat "Hello $(ls -la)" && echo `ps \`; {echo Goodbye😀}"#;

    let commands = Parser::new(Lexer::new(source, EscapeChar::Backslash, false))
        .parse()
        .commands;

    assert_eq!(
        commands,
        [
            Command::new(vec![Part::Literal("ls".into()).spanned((0, 2))]).spanned((0, 3)),
            Command::new(vec![
                Part::Literal("rm".into()).spanned((5, 7)),
                Part::Literal("-rf".into()).spanned((8, 11))
            ])
            .spanned((5, 12)),
            Command::new(vec![
                Part::Literal("touch".into()).spanned((15, 20)),
                Part::Literal("hello.txt\\".into()).spanned((21, 33))
            ])
            .spanned((15, 34)),
            Command::new(vec![
                Part::Literal("cat".into()).spanned((36, 39)),
                Part::Concatenated(vec![
                    Part::Literal("Hello ".into()).spanned((40, 47)),
                    Part::ClosedSubshell(vec![Command::new(vec![
                        Part::Literal("ls".into()).spanned((49, 51)),
                        Part::Literal("-la".into()).spanned((52, 55)),
                    ])
                    .spanned((49, 55))])
                    .spanned((47, 56)),
                ])
                .spanned((40, 57))
            ])
            .spanned((36, 58)),
            Command::new(vec![
                Part::Literal("echo".into()).spanned((61, 65)),
                Part::ClosedSubshell(vec![Command::new(vec![
                    Part::Literal("ps".into()).spanned((67, 69)),
                    Part::Literal("\\".into()).spanned((70, 71)),
                ])
                .spanned((67, 71))])
                .spanned((66, 72)),
            ])
            .spanned((61, 72)),
            Command::new(vec![
                Part::Literal("echo".into()).spanned((75, 79)),
                Part::Literal("Goodbye😀".into()).spanned((80, 91))
            ])
            .spanned((75, 91)),
        ]
    );
}

// Test that a backslash is retained when preceding a command.
#[test]
fn test_backslash_before_command() {
    let source = r#"\ls"#;
    let command = Parser::new(Lexer::new(source, EscapeChar::Backslash, false)).parse_command();

    assert_eq!(
        command,
        Command::new(vec![Part::Literal(r"\ls".into()).spanned((0, 3))]).spanned((0, 3))
    );
}

// Test that a backslash is not retained in the middle of a command.
#[test]
fn test_backslash_in_command() {
    let source = r#"ls \-la"#;
    let command = Parser::new(Lexer::new(source, EscapeChar::Backslash, false)).parse_command();

    assert_eq!(
        command,
        Command::new(vec![
            Part::Literal("ls".into()).spanned((0, 2)),
            Part::Literal("-la".into()).spanned((3, 7))
        ])
        .spanned((0, 7))
    );
}

#[test]
fn test_decompose_command() {
    let test_data = vec![
        ("ls", vec!["ls"]),
        ("$(ls)", vec!["ls"]),
        ("ls -la", vec!["ls -la"]),
        ("ls && cat", vec!["ls", "cat"]),
        (
            "ls $(foo | echo)",
            vec!["foo", "echo", "foo | echo", "ls $(foo | echo)"],
        ),
    ];

    for (input, expected_output) in test_data {
        // Compare with hashsets bc we don't care about ordering.
        assert_eq!(
            HashSet::<String>::from_iter(decompose_command(input, EscapeChar::Backslash).0),
            HashSet::from_iter(expected_output.into_iter().map(ToString::to_string)),
        );
    }
}

#[test]
fn test_contains_redirection() {
    let test_data = vec![
        ("ls < \"file.txt < tmp\"", true),
        ("echo $(ls > file.txt)", true),
        ("ls >> file.txt", true),
        ("ls < file.txt", true),
        ("foo arg1 arg2 > file.txt", true),
        ("foo && ls > file.txt", true),
        ("echo \"hello world\" > output.txt", true),
        ("echo \"5>4\"", false),
        ("echo \"This message -> shows direction\"", false),
        ("print(\"Value must be > 0 and < 100\")", false),
    ];

    for (cmd, should_contain_redirection) in test_data {
        let parser = Parser::new(Lexer::new(cmd, EscapeChar::Backslash, false));
        let contains_redirection = parser.parse().contains_redirection;
        assert_eq!(contains_redirection, should_contain_redirection);
    }
}

#[test]
fn test_top_level_command() {
    let test_data = vec![
        ("PAGER=0 git log", Some("git")),
        ("PAGER= git log", Some("git")),
        ("ls && git status", Some("ls")),
        ("$(git status)", None),
    ];

    for (input, expected_output) in test_data {
        assert_eq!(
            top_level_command(input, EscapeChar::Backslash),
            expected_output.map(ToString::to_string)
        );
    }
}
