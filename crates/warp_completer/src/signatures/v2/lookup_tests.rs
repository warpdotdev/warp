use super::*;
use crate::signatures::{Argument, Command, CommandSignature, Opt};

/// Creates a `test_command` signature with a `test_subcommand` subcommand
/// and the given options on the root command.
fn test_command_signature_with_options(options: Vec<Opt>) -> CommandSignature {
    CommandSignature {
        command: Command {
            name: "test_command".to_owned(),
            subcommands: vec![Command {
                name: "test_subcommand".to_owned(),
                ..Default::default()
            }],
            options,
            ..Default::default()
        },
    }
}

fn valued_option() -> Opt {
    Opt {
        name: vec!["-n".to_owned(), "--name".to_owned()],
        arguments: vec![Argument {
            name: "value".to_owned(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Creates a keyword `Command` (with no subcommands of its own) that takes a single required
/// positional argument, for use under a `repeatable_keywords` parent.
fn keyword(name: &str) -> Command {
    Command {
        name: name.to_owned(),
        arguments: vec![Argument {
            name: format!("{name}-value"),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Creates a signature resembling `ip rule add`, where `add`'s subcommands (`iif`, `from`,
/// `table`, `fwmark`) are repeatable, order-independent keywords rather than mutually exclusive
/// subcommands.
fn repeatable_keyword_command_signature() -> CommandSignature {
    CommandSignature {
        command: Command {
            name: "ip".to_owned(),
            subcommands: vec![Command {
                name: "rule".to_owned(),
                subcommands: vec![Command {
                    name: "add".to_owned(),
                    repeatable_keywords: true,
                    subcommands: vec![
                        keyword("iif"),
                        keyword("from"),
                        keyword("table"),
                        keyword("fwmark"),
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
    }
}

#[test]
fn test_get_matching_signature_for_input_on_root_command() {
    let registry = CommandRegistry::new();
    registry.register_signature(CommandSignature {
        command: Command {
            name: "test_command".to_owned(),
            ..Default::default()
        },
    });

    let matched = get_matching_signature_for_input("test_command ", &registry)
        .expect("Signature should exist");
    assert_eq!(matched.command.name, "test_command");
    assert_eq!(matched.token_index, 0);
}

#[test]
fn test_get_matching_signature_for_input_on_root_command_with_argument() {
    let registry = CommandRegistry::new();
    registry.register_signature(CommandSignature {
        command: Command {
            name: "test_command".to_owned(),
            subcommands: vec![Command {
                name: "test_subcommand".to_owned(),
                ..Default::default()
            }],
            arguments: vec![Argument {
                name: "arg1".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        },
    });

    let matched = get_matching_signature_for_input("test_command some_arg_value ", &registry)
        .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_command");
    assert_eq!(matched.token_index, 0);
}

#[test]
fn test_get_matching_signature_for_input_on_subcommand() {
    let registry = CommandRegistry::new();
    registry.register_signature(CommandSignature {
        command: Command {
            name: "test_command".to_owned(),
            subcommands: vec![Command {
                name: "test_subcommand".to_owned(),
                ..Default::default()
            }],
            arguments: vec![Argument {
                name: "arg1".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        },
    });

    let matched = get_matching_signature_for_input("test_command test_subcommand ", &registry)
        .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_subcommand");
    assert_eq!(matched.token_index, 1);
}

#[test]
fn test_get_matching_signature_for_input_on_subcommand_with_argument() {
    let registry = CommandRegistry::new();
    registry.register_signature(CommandSignature {
        command: Command {
            name: "test_command".to_owned(),
            subcommands: vec![
                Command {
                    name: "test_subcommand1".to_owned(),
                    arguments: vec![Argument {
                        name: "test_subcommand_arg".to_owned(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Command {
                    name: "test_subcommand2".to_owned(),
                    arguments: vec![Argument {
                        name: "test_subcommand_arg".to_owned(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            arguments: vec![Argument {
                name: "test_command_arg".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        },
    });

    let matched = get_matching_signature_for_input(
        "test_command test_subcommand1 some_arg_value ",
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_subcommand1");
    assert_eq!(matched.token_index, 1);
}

#[test]
fn test_get_matching_signature_for_input_without_trailing_whitespace() {
    let registry = CommandRegistry::new();
    registry.register_signature(CommandSignature {
        command: Command {
            name: "test_command".to_owned(),
            subcommands: vec![Command {
                name: "test_subcommand".to_owned(),
                ..Default::default()
            }],
            arguments: vec![Argument {
                name: "arg1".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        },
    });

    let matched = get_matching_signature_for_input("test_command test_subcommand", &registry)
        .expect("Signature should be found.");

    // The matched signature should be that of the top-level command. Because there is no trailing
    // whitespace in the input, it's assumed we're still completing on the "test_subcommand", so we
    // should still be using the top-level command signature.
    assert_eq!(matched.command.name, "test_command");
    assert_eq!(matched.token_index, 0);
}

#[test]
fn test_get_matching_signature_for_tokenized_input() {
    let registry = CommandRegistry::new();
    registry.register_signature(CommandSignature {
        command: Command {
            name: "test_command".to_owned(),
            subcommands: vec![
                Command {
                    name: "test_subcommand1".to_owned(),
                    arguments: vec![Argument {
                        name: "test_subcommand_arg".to_owned(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Command {
                    name: "test_subcommand2".to_owned(),
                    arguments: vec![Argument {
                        name: "test_subcommand_arg".to_owned(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            arguments: vec![Argument {
                name: "test_command_arg".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        },
    });

    let matched = get_matching_signature_for_tokenized_input(
        &["test_command", "test_subcommand1", "some_arg_value"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_subcommand1");
    assert_eq!(matched.token_index, 1);
}

#[test]
fn test_get_matching_signature_for_tokenized_input_without_trailing_whitespace() {
    let registry = CommandRegistry::new();
    registry.register_signature(CommandSignature {
        command: Command {
            name: "test_command".to_owned(),
            subcommands: vec![
                Command {
                    name: "test_subcommand1".to_owned(),
                    arguments: vec![Argument {
                        name: "test_subcommand_arg".to_owned(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Command {
                    name: "test_subcommand2".to_owned(),
                    arguments: vec![Argument {
                        name: "test_subcommand_arg".to_owned(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            arguments: vec![Argument {
                name: "test_command_arg".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        },
    });

    let matched = get_matching_signature_for_tokenized_input(
        &["test_command", "test_subcommand1"],
        false,
        &registry,
    )
    .expect("Signature should be found.");

    // The matched signature should be that of the top-level command. Because there is no trailing
    // whitespace in the input, it's assumed we're still completing on the "test_subcommand", so we
    // should still be using the top-level command signature.
    assert_eq!(matched.command.name, "test_command");
    assert_eq!(matched.token_index, 0);
}

#[test]
fn test_get_matching_signature_skips_flag_with_value_before_subcommand() {
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![valued_option()]));

    // -n takes a value, so the parser should skip "-n val" and find test_subcommand.
    let matched = get_matching_signature_for_tokenized_input(
        &["test_command", "-n", "val", "test_subcommand"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_subcommand");
    assert_eq!(matched.token_index, 3);
}

#[test]
fn test_get_matching_signature_skips_long_flag_with_value_before_subcommand() {
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![
        Opt {
            name: vec!["--context".to_owned()],
            arguments: vec![Argument {
                name: "context".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        },
        valued_option(),
    ]));

    // Two valued flags before the subcommand should both be skipped.
    let matched = get_matching_signature_for_tokenized_input(
        &[
            "test_command",
            "--context",
            "staging",
            "-n",
            "project1",
            "test_subcommand",
        ],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_subcommand");
    assert_eq!(matched.token_index, 5);
}

#[test]
fn test_get_matching_signature_skips_switch_flag_before_subcommand() {
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![Opt {
        name: vec!["--verbose".to_owned()],
        arguments: vec![],
        ..Default::default()
    }]));

    let matched = get_matching_signature_for_tokenized_input(
        &["test_command", "--verbose", "test_subcommand"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_subcommand");
    assert_eq!(matched.token_index, 2);
}

#[test]
fn test_get_matching_signature_flag_at_end_without_value_does_not_panic() {
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![valued_option()]));

    // "-n" with no value should not panic.
    let matched =
        get_matching_signature_for_tokenized_input(&["test_command", "-n"], true, &registry)
            .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_command");
    // No subcommand found, so entry_token_index (0) is returned.
    assert_eq!(matched.token_index, 0);
}

#[test]
fn test_get_matching_signature_skips_unrecognized_flag_before_subcommand() {
    // Unrecognized flags (tokens starting with '-' not in the spec) should be
    // skipped so the parser can still discover subcommands after them.
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![]));

    let matched = get_matching_signature_for_tokenized_input(
        &["test_command", "--unknown-flag", "test_subcommand"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_subcommand");
    assert_eq!(matched.token_index, 2);
}

#[test]
fn test_get_matching_signature_flag_arg_consumes_token_matching_subcommand_name() {
    // When a recognized flag takes a required argument, the next token is
    // consumed as the flag's value even if it matches a subcommand name.
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![valued_option()]));

    let matched = get_matching_signature_for_tokenized_input(
        &["test_command", "-n", "test_subcommand", "extra"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    // "test_subcommand" was consumed as -n's value, so no subcommand is found.
    assert_eq!(matched.command.name, "test_command");
    assert_eq!(matched.token_index, 0);
}

#[test]
fn test_get_matching_signature_only_flags_no_subcommand() {
    // When the input consists only of flags with no following subcommand,
    // the parent command should be returned at the entry index.
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![valued_option()]));

    let matched =
        get_matching_signature_for_tokenized_input(&["test_command", "-n", "val"], true, &registry)
            .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_command");
    assert_eq!(matched.token_index, 0);
}

#[test]
fn test_get_matching_signature_optional_flag_arg_does_not_consume_subcommand() {
    // A flag with 1 required + 1 optional argument should only skip the
    // required arg, so the next token can still match a subcommand.
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![Opt {
        name: vec!["--output".to_owned()],
        arguments: vec![
            Argument {
                name: "format".to_owned(),
                ..Default::default()
            },
            Argument {
                name: "extra".to_owned(),
                optional: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }]));

    // "json" is the required arg, "test_subcommand" should not be consumed as
    // the optional arg.
    let matched = get_matching_signature_for_tokenized_input(
        &["test_command", "--output", "json", "test_subcommand"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_subcommand");
    assert_eq!(matched.token_index, 3);
}

#[test]
fn test_get_matching_signature_repeatable_keyword_returns_keyword_and_siblings() {
    let registry = CommandRegistry::new();
    registry.register_signature(repeatable_keyword_command_signature());

    // Right after "iif eth0 " (with trailing whitespace), the matched command should be "iif"
    // itself (so its own argument values still complete), with its unused sibling keywords
    // ("from", "table", "fwmark") surfaced as eligible siblings.
    let matched = get_matching_signature_for_tokenized_input(
        &["ip", "rule", "add", "iif", "eth0"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(matched.command.name, "iif");

    let mut sibling_names: Vec<&str> = matched
        .eligible_sibling_keywords
        .iter()
        .map(|command| command.name.as_str())
        .collect();
    sibling_names.sort_unstable();
    assert_eq!(sibling_names, vec!["from", "fwmark", "table"]);
}

#[test]
fn test_get_matching_signature_repeatable_keyword_excludes_used_keywords() {
    let registry = CommandRegistry::new();
    registry.register_signature(repeatable_keyword_command_signature());

    // After a second keyword ("from") has also been used, it should no longer appear among the
    // eligible siblings, but keywords that haven't been used yet still should.
    let matched = get_matching_signature_for_tokenized_input(
        &["ip", "rule", "add", "iif", "eth0", "from", "10.0.0.0/8"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(matched.command.name, "from");

    let mut sibling_names: Vec<&str> = matched
        .eligible_sibling_keywords
        .iter()
        .map(|command| command.name.as_str())
        .collect();
    sibling_names.sort_unstable();
    assert_eq!(sibling_names, vec!["fwmark", "table"]);
}

#[test]
fn test_get_matching_signature_repeatable_keyword_no_keyword_matched_yet() {
    let registry = CommandRegistry::new();
    registry.register_signature(repeatable_keyword_command_signature());

    // Before any keyword has been typed, the parent ("add") itself should be returned, with no
    // extra eligible siblings (its own `subcommands` already contains the full keyword list).
    let matched =
        get_matching_signature_for_tokenized_input(&["ip", "rule", "add"], true, &registry)
            .expect("Signature should be found.");
    assert_eq!(matched.command.name, "add");
    assert!(matched.eligible_sibling_keywords.is_empty());
}

#[test]
fn test_get_matching_signature_repeatable_keywords_opt_in_does_not_affect_default_commands() {
    // A default (non-repeatable-keywords) parent should retain the ordinary "descend and drop
    // siblings" behavior: once a subcommand is resolved, its exclusive siblings should not be
    // surfaced via `eligible_sibling_keywords`.
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![]));

    let matched = get_matching_signature_for_tokenized_input(
        &["test_command", "test_subcommand"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(matched.command.name, "test_subcommand");
    assert!(matched.eligible_sibling_keywords.is_empty());
}
