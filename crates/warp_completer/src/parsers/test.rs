use itertools::Itertools;
use warp_util::path::EscapeChar;

use super::*;
use crate::parsers::hir::{CommandCallInfo, Flags, ShellCommand};
#[cfg(not(feature = "v2"))]
use crate::parsers::hir::{Flag, FlagType};
use crate::parsers::simple::parse_for_completions;
use crate::parsers::{classify_command, ClassifiedCommand};
use crate::signatures::testing::{create_test_command_registry, test_signature};

#[test]
pub fn test_classify_command_classifies_known_command() {
    let registry = create_test_command_registry([test_signature()]);

    let lite_command = parse_for_completions("test ", EscapeChar::Backslash, false)
        .expect("Should be able to parse input into LiteCommand");
    let mut tokens = lite_command.parts.iter().map(|s| s.as_str()).collect_vec();

    let classified_command = classify_command(
        lite_command.clone(),
        &mut tokens,
        &registry,
        TopLevelCommandCaseSensitivity::CaseSensitive,
    );
    assert_eq!(
        classified_command,
        Some(ClassifiedCommand {
            env_vars: vec![],
            command: Command::Classified(ShellCommand {
                name: "test".to_owned(),
                name_span: Span::from((0, 4)),
                args: CommandCallInfo {
                    command_name: Spanned {
                        span: Span::from((0, 4)),
                        item: ParsedExpression::new(
                            Expression::Command,
                            ParsedToken("test".to_owned())
                        ),
                    },
                    positionals: None,
                    flags: Some(Flags::new()),
                    ending_whitespace: Some(Span::from((4, 5))),
                    span: Span::from((0, 5))
                }
            }),
            error: None,
        })
    )
}

/// TODO(CORE-2797)
#[cfg(not(feature = "v2"))]
#[test]
pub fn test_classify_command_classifies_known_command_with_flags() {
    let registry = create_test_command_registry([test_signature()]);

    let lite_command = parse_for_completions("test -r --long foo", EscapeChar::Backslash, false)
        .expect("Should be able to parse input into LiteCommand");
    let mut tokens = lite_command.parts.iter().map(|s| s.as_str()).collect_vec();

    let classified_command = classify_command(
        lite_command.clone(),
        &mut tokens,
        &registry,
        TopLevelCommandCaseSensitivity::CaseSensitive,
    );
    assert_eq!(
        classified_command,
        Some(ClassifiedCommand {
            env_vars: vec![],
            command: Command::Classified(ShellCommand {
                name: "test".to_owned(),
                name_span: Span::from((0, 4)),
                args: CommandCallInfo {
                    command_name: Spanned {
                        span: Span::from((0, 4)),
                        item: ParsedExpression::new(
                            Expression::Command,
                            ParsedToken("test".to_owned())
                        ),
                    },
                    positionals: None,
                    flags: Some(Flags {
                        flags: vec![
                            Flag {
                                name: "-r".to_owned(),
                                name_span: Span::from((5, 7)),
                                flag_type: FlagType::NoArgument
                            },
                            Flag {
                                name: "--long".to_owned(),
                                name_span: Span::from((8, 14)),
                                flag_type: FlagType::Argument {
                                    value: Spanned {
                                        span: Span::from((15, 18)),
                                        item: ParsedExpression::new(
                                            Expression::Literal,
                                            ParsedToken("foo".to_owned())
                                        )
                                    }
                                }
                            },
                        ]
                    }),
                    ending_whitespace: None,
                    span: Span::from((0, 18))
                }
            }),
            error: None
        })
    )
}

/// TODO(CORE-2797)
///
/// With exact option matching, `-r` correctly matches the `-r` switch (no arguments),
/// so the parser advances past it and discovers the `one` subcommand. The command path
/// becomes `"test -r one"` (the legacy parser's convention for subcommand paths).
#[cfg(not(feature = "v2"))]
#[test]
pub fn test_classify_command_classifies_known_command_with_subcommand() {
    let registry = create_test_command_registry([test_signature()]);

    let lite_command = parse_for_completions("test -r one foo bar", EscapeChar::Backslash, false)
        .expect("Should be able to parse input into LiteCommand");
    let mut tokens = lite_command.parts.iter().map(|s| s.as_str()).collect_vec();

    let classified_command = classify_command(
        lite_command.clone(),
        &mut tokens,
        &registry,
        TopLevelCommandCaseSensitivity::CaseSensitive,
    );
    assert_eq!(
        classified_command,
        Some(ClassifiedCommand {
            env_vars: vec![],
            command: Command::Classified(ShellCommand {
                name: "test -r one".to_owned(),
                name_span: Span::from((0, 11)),
                args: CommandCallInfo {
                    command_name: Spanned {
                        span: Span::from((0, 11)),
                        item: ParsedExpression::new(
                            Expression::Command,
                            ParsedToken("test -r one".to_owned())
                        ),
                    },
                    positionals: Some(vec![
                        Spanned {
                            span: Span::from((12, 15)),
                            item: ParsedExpression::new(
                                Expression::Literal,
                                ParsedToken("foo".to_owned())
                            ),
                        },
                        Spanned {
                            span: Span::from((16, 19)),
                            item: ParsedExpression::new(
                                Expression::Literal,
                                ParsedToken("bar".to_owned())
                            ),
                        },
                    ]),
                    flags: Some(Flags::new()),
                    ending_whitespace: None,
                    span: Span::from((0, 19))
                }
            }),
            error: None
        })
    )
}

#[test]
pub fn test_classify_command_classifies_unknown_command() {
    let registry = create_test_command_registry([]);

    let lite_command = parse_for_completions("test ", EscapeChar::Backslash, false)
        .expect("Should be able to parse input into LiteCommand");
    let mut tokens = lite_command.parts.iter().map(|s| s.as_str()).collect_vec();

    let classified_command = classify_command(
        lite_command.clone(),
        &mut tokens,
        &registry,
        TopLevelCommandCaseSensitivity::CaseSensitive,
    );
    assert_eq!(
        classified_command,
        Some(ClassifiedCommand {
            env_vars: vec![],
            command: Command::Unclassified(ExternalCommand {
                name: ParsedToken("test".to_owned()),
                name_span: Span::from((0, 4)),
                args: CommandCallInfo {
                    command_name: Spanned {
                        span: Span::from((0, 5)),
                        item: ParsedExpression::new(
                            Expression::Literal,
                            ParsedToken("test".to_owned())
                        ),
                    },
                    positionals: None,
                    flags: None,
                    ending_whitespace: Some(Span::from((4, 5))),
                    span: Span::from((0, 5))
                }
            }),
            error: None,
        })
    )
}

#[test]
pub fn test_classify_command_classifies_unknown_command_with_flags() {
    let registry = create_test_command_registry([]);

    let lite_command = parse_for_completions("test -r --long foo", EscapeChar::Backslash, false)
        .expect("Should be able to parse input into LiteCommand");
    let mut tokens = lite_command.parts.iter().map(|s| s.as_str()).collect_vec();

    let classified_command = classify_command(
        lite_command.clone(),
        &mut tokens,
        &registry,
        TopLevelCommandCaseSensitivity::CaseSensitive,
    );
    assert_eq!(
        classified_command,
        Some(ClassifiedCommand {
            env_vars: vec![],
            command: Command::Unclassified(ExternalCommand {
                name: ParsedToken("test".to_owned()),
                name_span: Span::from((0, 4)),
                args: CommandCallInfo {
                    command_name: Spanned {
                        span: Span::from((0, 18)),
                        item: ParsedExpression::new(
                            Expression::Literal,
                            ParsedToken("test".to_owned()),
                        ),
                    },
                    positionals: Some(vec![
                        Spanned {
                            span: Span::from((5, 7)),
                            item: ParsedExpression::new(
                                Expression::Literal,
                                ParsedToken("-r".to_owned()),
                            ),
                        },
                        Spanned {
                            span: Span::from((8, 14)),
                            item: ParsedExpression::new(
                                Expression::Literal,
                                ParsedToken("--long".to_owned()),
                            ),
                        },
                        Spanned {
                            span: Span::from((15, 18)),
                            item: ParsedExpression::new(
                                Expression::Literal,
                                ParsedToken("foo".to_owned()),
                            ),
                        },
                    ]),
                    flags: None,
                    ending_whitespace: None,
                    span: Span::from((0, 18)),
                },
            }),
            error: None,
        })
    )
}

#[test]
pub fn test_classify_command_classifies_unknown_command_with_subcommand() {
    let registry = create_test_command_registry([]);

    let lite_command = parse_for_completions("test -r one foo bar", EscapeChar::Backslash, false)
        .expect("Should be able to parse input into LiteCommand");
    let mut tokens = lite_command.parts.iter().map(|s| s.as_str()).collect_vec();

    let classified_command = classify_command(
        lite_command.clone(),
        &mut tokens,
        &registry,
        TopLevelCommandCaseSensitivity::CaseSensitive,
    );
    assert_eq!(
        classified_command,
        Some(ClassifiedCommand {
            env_vars: vec![],
            command: Command::Unclassified(ExternalCommand {
                name: ParsedToken("test".to_owned()),
                name_span: Span::from((0, 4)),
                args: CommandCallInfo {
                    command_name: Spanned {
                        span: Span::from((0, 19)),
                        item: ParsedExpression::new(
                            Expression::Literal,
                            ParsedToken("test".to_owned()),
                        ),
                    },
                    positionals: Some(vec![
                        Spanned {
                            span: Span::from((5, 7)),
                            item: ParsedExpression::new(
                                Expression::Literal,
                                ParsedToken("-r".to_owned()),
                            ),
                        },
                        Spanned {
                            span: Span::from((8, 11)),
                            item: ParsedExpression::new(
                                Expression::Literal,
                                ParsedToken("one".to_owned()),
                            ),
                        },
                        Spanned {
                            span: Span::from((12, 15)),
                            item: ParsedExpression::new(
                                Expression::Literal,
                                ParsedToken("foo".to_owned()),
                            ),
                        },
                        Spanned {
                            span: Span::from((16, 19)),
                            item: ParsedExpression::new(
                                Expression::Literal,
                                ParsedToken("bar".to_owned())
                            ),
                        },
                    ]),
                    flags: None,
                    ending_whitespace: None,
                    span: Span::from((0, 19)),
                },
            }),
            error: None,
        }),
    )
}

#[test]
fn test_classify_command_case_sensitive() {
    let registry = create_test_command_registry([test_signature()]);

    let lite_command = parse_for_completions("TEST ", EscapeChar::Backslash, false)
        .expect("Should be able to parse input into LiteCommand");
    let mut tokens = lite_command.parts.iter().map(|s| s.as_str()).collect_vec();

    let classified_command = classify_command(
        lite_command.clone(),
        &mut tokens,
        &registry,
        TopLevelCommandCaseSensitivity::CaseSensitive,
    );

    assert_eq!(
        classified_command,
        Some(ClassifiedCommand {
            env_vars: vec![],
            command: Command::Unclassified(ExternalCommand {
                name: ParsedToken("TEST".to_owned()),
                name_span: Span::from((0, 4)),
                args: CommandCallInfo {
                    command_name: Spanned {
                        span: Span::from((0, 5)),
                        item: ParsedExpression::new(
                            Expression::Literal,
                            ParsedToken("TEST".to_owned())
                        ),
                    },
                    positionals: None,
                    flags: None,
                    ending_whitespace: Some(Span::from((4, 5))),
                    span: Span::from((0, 5))
                }
            }),
            error: None,
        })
    )
}

/// TODO(CORE-2810)
#[cfg(not(feature = "v2"))]
#[test]
fn test_classify_command_case_insensitive() {
    let registry = create_test_command_registry([test_signature()]);

    let lite_command = parse_for_completions("TEST ", EscapeChar::Backslash, false)
        .expect("Should be able to parse input into LiteCommand");
    let mut tokens = lite_command.parts.iter().map(|s| s.as_str()).collect_vec();

    let classified_command = classify_command(
        lite_command.clone(),
        &mut tokens,
        &registry,
        TopLevelCommandCaseSensitivity::CaseInsensitive,
    );

    assert_eq!(
        classified_command,
        Some(ClassifiedCommand {
            env_vars: vec![],
            command: Command::Classified(ShellCommand {
                name: "TEST".to_owned(),
                name_span: Span::from((0, 4)),
                args: CommandCallInfo {
                    command_name: Spanned {
                        span: Span::from((0, 4)),
                        item: ParsedExpression::new(
                            Expression::Command,
                            ParsedToken("TEST".to_owned())
                        ),
                    },
                    positionals: None,
                    flags: Some(Flags::new()),
                    ending_whitespace: Some(Span::from((4, 5))),
                    span: Span::from((0, 5))
                }
            }),
            error: None,
        })
    )
}
