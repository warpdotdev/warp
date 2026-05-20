use crate::signatures::{Argument, Command, CommandSignature, Opt};

use super::*;

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

#[test]
fn test_get_matching_signature_for_input_on_root_command() {
    let registry = CommandRegistry::new();
    registry.register_signature(CommandSignature {
        command: Command {
            name: "test_command".to_owned(),
            ..Default::default()
        },
    });

    let (found_signature, index) = get_matching_signature_for_input("test_command ", &registry)
        .expect("Signature should exist");
    assert_eq!(found_signature.name, "test_command");
    assert_eq!(index, 0);
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

    let (found_signature, index) =
        get_matching_signature_for_input("test_command some_arg_value ", &registry)
            .expect("Signature should be found.");
    assert_eq!(found_signature.name, "test_command");
    assert_eq!(index, 0);
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

    let (found_signature, index) =
        get_matching_signature_for_input("test_command test_subcommand ", &registry)
            .expect("Signature should be found.");
    assert_eq!(found_signature.name, "test_subcommand");
    assert_eq!(index, 1);
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

    let (found_signature, index) = get_matching_signature_for_input(
        "test_command test_subcommand1 some_arg_value ",
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(found_signature.name, "test_subcommand1");
    assert_eq!(index, 1);
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

    let (found_signature, index) =
        get_matching_signature_for_input("test_command test_subcommand", &registry)
            .expect("Signature should be found.");

    // The matched signature should be that of the top-level command. Because there is no trailing
    // whitespace in the input, it's assumed we're still completing on the "test_subcommand", so we
    // should still be using the top-level command signature.
    assert_eq!(found_signature.name, "test_command");
    assert_eq!(index, 0);
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

    let (found_signature, token_index) = get_matching_signature_for_tokenized_input(
        &["test_command", "test_subcommand1", "some_arg_value"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(found_signature.name, "test_subcommand1");
    assert_eq!(token_index, 1);
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

    let (found_signature, token_index) = get_matching_signature_for_tokenized_input(
        &["test_command", "test_subcommand1"],
        false,
        &registry,
    )
    .expect("Signature should be found.");

    // The matched signature should be that of the top-level command. Because there is no trailing
    // whitespace in the input, it's assumed we're still completing on the "test_subcommand", so we
    // should still be using the top-level command signature.
    assert_eq!(found_signature.name, "test_command");
    assert_eq!(token_index, 0);
}

#[test]
fn test_get_matching_signature_skips_flag_with_value_before_subcommand() {
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![valued_option()]));

    // -n takes a value, so the parser should skip "-n val" and find test_subcommand.
    let (found_signature, token_index) = get_matching_signature_for_tokenized_input(
        &["test_command", "-n", "val", "test_subcommand"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(found_signature.name, "test_subcommand");
    assert_eq!(token_index, 3);
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
    let (found_signature, token_index) = get_matching_signature_for_tokenized_input(
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
    assert_eq!(found_signature.name, "test_subcommand");
    assert_eq!(token_index, 5);
}

#[test]
fn test_get_matching_signature_skips_switch_flag_before_subcommand() {
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![Opt {
        name: vec!["--verbose".to_owned()],
        arguments: vec![],
        ..Default::default()
    }]));

    let (found_signature, token_index) = get_matching_signature_for_tokenized_input(
        &["test_command", "--verbose", "test_subcommand"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(found_signature.name, "test_subcommand");
    assert_eq!(token_index, 2);
}

#[test]
fn test_get_matching_signature_flag_at_end_without_value_does_not_panic() {
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![valued_option()]));

    // "-n" with no value should not panic.
    let (found_signature, token_index) =
        get_matching_signature_for_tokenized_input(&["test_command", "-n"], true, &registry)
            .expect("Signature should be found.");
    assert_eq!(found_signature.name, "test_command");
    // No subcommand found, so entry_token_index (0) is returned.
    assert_eq!(token_index, 0);
}

#[test]
fn test_get_matching_signature_skips_unrecognized_flag_before_subcommand() {
    // Unrecognized flags (tokens starting with '-' not in the spec) should be
    // skipped so the parser can still discover subcommands after them.
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![]));

    let (found_signature, token_index) = get_matching_signature_for_tokenized_input(
        &["test_command", "--unknown-flag", "test_subcommand"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(found_signature.name, "test_subcommand");
    assert_eq!(token_index, 2);
}

#[test]
fn test_get_matching_signature_flag_arg_consumes_token_matching_subcommand_name() {
    // When a recognized flag takes a required argument, the next token is
    // consumed as the flag's value even if it matches a subcommand name.
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![valued_option()]));

    let (found_signature, token_index) = get_matching_signature_for_tokenized_input(
        &["test_command", "-n", "test_subcommand", "extra"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    // "test_subcommand" was consumed as -n's value, so no subcommand is found.
    assert_eq!(found_signature.name, "test_command");
    assert_eq!(token_index, 0);
}

#[test]
fn test_get_matching_signature_only_flags_no_subcommand() {
    // When the input consists only of flags with no following subcommand,
    // the parent command should be returned at the entry index.
    let registry = CommandRegistry::new();
    registry.register_signature(test_command_signature_with_options(vec![valued_option()]));

    let (found_signature, token_index) =
        get_matching_signature_for_tokenized_input(&["test_command", "-n", "val"], true, &registry)
            .expect("Signature should be found.");
    assert_eq!(found_signature.name, "test_command");
    assert_eq!(token_index, 0);
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
    let (found_signature, token_index) = get_matching_signature_for_tokenized_input(
        &["test_command", "--output", "json", "test_subcommand"],
        true,
        &registry,
    )
    .expect("Signature should be found.");
    assert_eq!(found_signature.name, "test_subcommand");
    assert_eq!(token_index, 3);
}
