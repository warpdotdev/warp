//! V2 versions of command signatures used for testing.
//!
//! Each signature in this file should be semantically equivalent with a command signature returned
//! by a function of the same name in `super::legacy`; this is to ensure that the same test
//! coverage can run with the "v2" Cargo feature both enabled and disabled.
use warp_js::TypedJsFunctionRef;

use super::{TEST_GENERATOR_1_COMMAND, TEST_GENERATOR_2_COMMAND};
use crate::signatures::{
    Argument, ArgumentValue, Arity, Command, CommandSignature, GeneratorFn, GeneratorResults,
    GeneratorScript, Opt, Priority, Suggestion, TemplateType,
};

lazy_static::lazy_static! {
    pub(crate) static ref TEST_GENERATOR_1_JS_FUNCTION: TypedJsFunctionRef<String, GeneratorResults> = TypedJsFunctionRef::<String, GeneratorResults>::new_for_test();
    pub(crate) static ref TEST_GENERATOR_2_JS_FUNCTION: TypedJsFunctionRef<String, GeneratorResults> = TypedJsFunctionRef::<String, GeneratorResults>::new_for_test();

    static ref TEST_GENERATOR_1: ArgumentValue = ArgumentValue::Generator(GeneratorFn::ShellCommand {
        script: GeneratorScript::Static(TEST_GENERATOR_1_COMMAND.to_owned()),
        post_process: Some(TEST_GENERATOR_1_JS_FUNCTION.clone()),
    });

    static ref TEST_GENERATOR_2: ArgumentValue = ArgumentValue::Generator(GeneratorFn::ShellCommand {
        script: GeneratorScript::Static(TEST_GENERATOR_2_COMMAND.to_owned()),
        post_process: Some(TEST_GENERATOR_2_JS_FUNCTION.clone()),
    });
}

fn create_argument_value(name: impl Into<String>) -> ArgumentValue {
    ArgumentValue::Suggestion(Suggestion {
        value: name.into(),
        ..Default::default()
    })
}

fn create_argument_value_with_priority(
    name: impl Into<String>,
    priority: Priority,
) -> ArgumentValue {
    ArgumentValue::Suggestion(Suggestion {
        value: name.into(),
        priority,
        ..Default::default()
    })
}

// TODO(zachbai): Use this function to create hidden suggestions when hidden suggestions are
// implemented in V2.
#[allow(dead_code)]
fn create_hidden_argument_suggestion(name: impl Into<String>) -> ArgumentValue {
    ArgumentValue::Suggestion(Suggestion {
        value: name.into(),
        display_value: None,
        description: None,
        priority: Priority::default(),
    })
}

pub fn test_signature() -> CommandSignature {
    CommandSignature {
        command: Command {
            name: "test".to_owned(),
            alias: vec!["alias".to_owned()],
            description: Some("testing...".to_owned()),
            subcommands: vec![
                Command {
                    name: "one".to_owned(),
                    arguments: vec![
                        Argument {
                            name: "first arg".to_owned(),
                            values: vec![
                                create_argument_value("one-one"),
                                create_argument_value("one-two"),
                            ],
                            ..Default::default()
                        },
                        Argument {
                            name: "second arg".to_owned(),
                            values: vec![
                                create_argument_value("two-one"),
                                create_argument_value("two-two"),
                            ],
                            ..Default::default()
                        },
                    ],
                    priority: Priority::max(),
                    ..Default::default()
                },
                Command {
                    name: "two".to_owned(),
                    arguments: vec![
                        Argument {
                            name: "two-one".to_owned(),
                            arity: Some(Arity {
                                limit: None,
                                delimiter: None,
                            }),
                            values: vec![create_argument_value("two-one")],
                            ..Default::default()
                        },
                        Argument {
                            name: "two-two".to_owned(),
                            values: vec![create_argument_value("two-two")],
                            ..Default::default()
                        },
                    ],
                    priority: Priority::new(-50),
                    ..Default::default()
                },
                Command {
                    name: "three".to_owned(),
                    arguments: vec![
                        Argument {
                            name: "three-one".to_owned(),
                            values: vec![create_argument_value("three-one")],
                            ..Default::default()
                        },
                        Argument {
                            name: "three-two".to_owned(),
                            values: vec![create_argument_value("three-two")],
                            ..Default::default()
                        },
                        Argument {
                            name: "three-three".to_owned(),
                            values: vec![create_argument_value("three-three")],
                            optional: true,
                            ..Default::default()
                        },
                    ],
                    priority: Priority::min(),
                    ..Default::default()
                },
                Command {
                    name: "four".to_owned(),
                    arguments: vec![
                        Argument {
                            name: "four-one".to_owned(),
                            values: vec![create_argument_value("four-one")],
                            ..Default::default()
                        },
                        Argument {
                            name: "four-two".to_owned(),
                            arity: Some(Arity {
                                limit: None,
                                delimiter: None,
                            }),
                            values: vec![create_argument_value("four-two")],
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                Command {
                    name: "five".to_owned(),
                    arguments: vec![Argument {
                        name: "five".to_owned(),
                        values: vec![TEST_GENERATOR_1.clone(), TEST_GENERATOR_2.clone()],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Command {
                    name: "six".to_owned(),
                    arguments: vec![Argument {
                        name: "six-one".to_owned(),
                        values: vec![
                            create_argument_value("six-arg"),
                            create_argument_value_with_priority("six-arg-2", Priority::max()),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Command {
                    name: "seven".to_owned(),
                    arguments: vec![Argument {
                        name: "seven-arg".to_owned(),
                        values: vec![TEST_GENERATOR_2.clone(), TEST_GENERATOR_2.clone()],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Command {
                    name: "eight".to_owned(),
                    subcommands: vec![Command {
                        name: "eight-subcommand".to_owned(),
                        ..Default::default()
                    }],
                    arguments: vec![
                        Argument {
                            name: "eight-arg".to_owned(),
                            values: vec![create_argument_value("eight-arg")],
                            ..Default::default()
                        },
                        Argument {
                            name: "eight-arg-2".to_owned(),
                            arity: Some(Arity {
                                limit: None,
                                delimiter: None,
                            }),
                            values: vec![create_argument_value("eight-arg-2")],
                            optional: true,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                Command {
                    name: "nine".to_owned(),
                    arguments: vec![Argument {
                        name: "nine-arg".to_owned(),
                        values: vec![create_argument_value("git")],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            options: vec![
                Opt {
                    name: vec!["--long".to_owned()],
                    arguments: vec![Argument {
                        name: "long-one".to_owned(),
                        arity: Some(Arity {
                            limit: None,
                            delimiter: None,
                        }),
                        values: vec![
                            create_argument_value("long-one"),
                            create_argument_value("long-two"),
                        ],
                        ..Default::default()
                    }],
                    priority: Priority::min(),
                    ..Default::default()
                },
                Opt {
                    name: vec!["--not-long".to_owned()],
                    arguments: vec![Argument {
                        name: "long-one".to_owned(),
                        values: vec![
                            create_argument_value("not-long-one"),
                            create_argument_value("not-long-two"),
                        ],
                        ..Default::default()
                    }],
                    priority: Priority::max(),
                    ..Default::default()
                },
                Opt {
                    name: vec!["--required-args".to_owned()],
                    arguments: vec![
                        Argument {
                            name: "required-arg-1".to_owned(),
                            values: vec![
                                create_argument_value("arg-1-1"),
                                create_argument_value("arg-1-2"),
                            ],
                            ..Default::default()
                        },
                        Argument {
                            name: "required-arg-2".to_owned(),
                            values: vec![
                                create_argument_value("arg-2-1"),
                                create_argument_value("arg-2-2"),
                            ],
                            ..Default::default()
                        },
                    ],
                    priority: Priority::max(),
                    ..Default::default()
                },
                Opt {
                    name: vec!["--required-args-with-var".to_owned()],
                    arguments: vec![
                        Argument {
                            name: "required-arg".to_owned(),
                            values: vec![
                                create_argument_value("arg-1"),
                                create_argument_value("arg-2"),
                            ],
                            ..Default::default()
                        },
                        Argument {
                            name: "variadic-arg".to_owned(),
                            arity: Some(Arity {
                                limit: None,
                                delimiter: None,
                            }),
                            values: vec![
                                create_argument_value("vararg-1"),
                                create_argument_value("vararg-2"),
                            ],
                            ..Default::default()
                        },
                    ],
                    priority: Priority::min(),
                    ..Default::default()
                },
                Opt {
                    name: vec!["--required-and-optional-args".to_owned()],
                    arguments: vec![
                        Argument {
                            name: "required-arg".to_owned(),
                            values: vec![
                                create_argument_value("required-1"),
                                create_argument_value("required-2"),
                            ],
                            ..Default::default()
                        },
                        Argument {
                            name: "optional-arg".to_owned(),
                            values: vec![
                                create_argument_value("optional-1"),
                                create_argument_value("optional-2"),
                            ],
                            optional: true,
                            ..Default::default()
                        },
                    ],
                    required: false,
                    priority: Priority::default(),
                    ..Default::default()
                },
                Opt {
                    name: vec!["--template-args-for-opt".to_owned()],
                    arguments: vec![Argument {
                        name: "templated".to_owned(),
                        values: vec![ArgumentValue::Template {
                            type_name: TemplateType::FilesAndFolders,
                            filter_name: None,
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Opt {
                    name: vec!["-r".to_owned()],
                    ..Default::default()
                },
                Opt {
                    name: vec!["-V".to_owned()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    }
}

pub fn cd_signature() -> CommandSignature {
    CommandSignature {
        command: Command {
            name: "cd".to_owned(),
            alias: vec![],
            description: Some("testing...".to_owned()),
            arguments: vec![Argument {
                name: "directories".to_owned(),
                description: None,
                arity: None,
                values: vec![
                    // TODO(completions-v2): Uncomment when "hidden" suggestions are implemented.
                    // A "hidden" suggestion is only shown if it is an exact match for the current
                    // token. In this case, "-" is only shown as a suggestion if the user has
                    // exactly typed "cd -" in the input.
                    // create_hidden_argument_suggestion('-'),
                    ArgumentValue::Template {
                        type_name: TemplateType::Folders,
                        filter_name: None,
                    },
                ],
                optional: false,
            }],
            subcommands: vec![],
            options: vec![],
            priority: Priority::default(),
        },
    }
}

pub fn ls_signature() -> CommandSignature {
    CommandSignature {
        command: Command {
            name: "ls".to_owned(),
            alias: vec![],
            description: Some("testing...".to_owned()),
            arguments: vec![Argument {
                name: "filepaths".to_owned(),
                description: None,
                arity: Some(Arity {
                    limit: None,
                    delimiter: None,
                }),
                values: vec![ArgumentValue::Template {
                    type_name: TemplateType::FilesAndFolders,
                    filter_name: None,
                }],
                optional: true,
            }],
            subcommands: vec![],
            options: vec![
                Opt {
                    name: vec!["-a".to_owned()],
                    ..Default::default()
                },
                Opt {
                    name: vec!["--color".to_owned()],
                    arguments: vec![Argument {
                        name: "when".to_owned(),
                        values: vec![
                            create_argument_value("force"),
                            create_argument_value("auto"),
                            create_argument_value("never"),
                        ],
                        optional: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Opt {
                    name: vec!["--test".to_owned()],
                    arguments: vec![Argument {
                        name: "when".to_owned(),
                        arity: Some(Arity {
                            limit: None,
                            delimiter: None,
                        }),
                        values: vec![
                            create_argument_value("force"),
                            create_argument_value("auto"),
                            create_argument_value("never"),
                        ],
                        optional: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            priority: Priority::default(),
        },
    }
}

/// A signature with a single positional that has no argument types.
pub fn signature_with_empty_positional() -> CommandSignature {
    CommandSignature {
        command: Command {
            name: "test-empty".to_owned(),
            alias: vec![],
            description: Some("testing...".to_owned()),
            arguments: vec![Argument {
                name: "test-empty--arg".to_owned(),
                description: None,
                arity: None,
                values: vec![],
                optional: false,
            }],
            subcommands: vec![],
            options: vec![],
            priority: Priority::default(),
        },
    }
}

pub fn git_signature() -> CommandSignature {
    CommandSignature {
        command: Command {
            name: "git".to_owned(),
            description: Some("the stupid content tracker".to_owned()),
            subcommands: vec![
                Command {
                    name: "add".to_owned(),
                    description: Some("Add file contents to the index".to_owned()),
                    ..Default::default()
                },
                Command {
                    name: "checkout".to_owned(),
                    description: Some("Switch branches or restore working tree files".to_owned()),
                    arguments: vec![Argument {
                        name: "branch".to_owned(),
                        description: Some("Branch".to_owned()),
                        values: vec![
                            create_argument_value("漢字"),
                            create_argument_value("bob/卡b卡"),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Command {
                    name: "clone".to_owned(),
                    description: Some("Clone a repository into a new directory".into()),
                    ..Default::default()
                },
                Command {
                    name: "branch".to_owned(),
                    description: Some("List, create, or delete branches".into()),
                    options: vec![
                        Opt {
                            name: vec!["--delete".to_owned()],
                            arguments: vec![Argument {
                                name: "branch".to_owned(),
                                arity: Some(Arity {
                                    limit: None,
                                    delimiter: None,
                                }),
                                values: vec![
                                    create_argument_value("branch-1"),
                                    create_argument_value("second-branch"),
                                ],
                                ..Default::default()
                            }],
                            ..Default::default()
                        },
                        Opt {
                            name: vec!["-m".to_owned()],
                            arguments: vec![
                                Argument {
                                    name: "from_branch".to_owned(),
                                    values: vec![
                                        create_argument_value("branch-1"),
                                        create_argument_value("second-branch"),
                                    ],
                                    ..Default::default()
                                },
                                Argument {
                                    name: "to_branch".to_owned(),
                                    values: vec![
                                        create_argument_value("branch-1"),
                                        create_argument_value("second-branch"),
                                    ],
                                    ..Default::default()
                                },
                            ],
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
            ],
            options: vec![
                Opt {
                    name: vec!["-p".to_string()],
                    ..Default::default()
                },
                Opt {
                    name: vec!["--version".to_string()],
                    ..Default::default()
                },
                Opt {
                    name: vec!["--help".to_string()],
                    ..Default::default()
                },
                Opt {
                    name: vec!["--bare".to_string()],
                    ..Default::default()
                },
                Opt {
                    name: vec!["-c".to_string()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    }
}

pub fn java_signature() -> CommandSignature {
    CommandSignature {
        command: Command {
            name: "java".to_string(),
            description: Some("Launch a java application".into()),
            arguments: vec![Argument {
                name: "<mainclass>".to_string(),
                optional: true,
                ..Default::default()
            }],
            options: vec![
                Opt {
                    // Java supports both styles of long-hand options.
                    name: vec!["-version".to_string(), "--version".to_string()],
                    description: Some("print product version to the error stream and exit".into()),
                    ..Default::default()
                },
                Opt {
                    name: vec![
                        "-cp".to_string(),
                        "-classpath".to_string(),
                        "--class-path".to_string(),
                    ],
                    description: Some(
                        "class search path of directories and zip/jar files".to_string(),
                    ),
                    arguments: vec![Argument {
                        name: "classpath".to_string(),
                        ..Default::default()
                    }],
                    required: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    }
}

pub fn fuzzy_signature() -> CommandSignature {
    CommandSignature {
        command: Command {
            name: "fuzzy".to_owned(),
            description: Some("testing...".to_owned()),
            arguments: vec![],
            subcommands: vec![
                Command {
                    name: "prefix1".to_owned(),
                    ..Default::default()
                },
                Command {
                    name: "prefix2".to_owned(),
                    priority: Priority::max(),
                    ..Default::default()
                },
                Command {
                    name: "suffix-pre-fix".to_owned(),
                    ..Default::default()
                },
            ],
            options: vec![Opt {
                name: vec!["--pre-fx".to_owned()],
                ..Default::default()
            }],
            ..Default::default()
        },
    }
}

pub fn npm_signature() -> CommandSignature {
    CommandSignature {
        command: Command {
            name: "npm".to_owned(),
            description: Some("testing...".to_owned()),
            subcommands: vec![
                Command {
                    name: "r".to_owned(),
                    arguments: vec![Argument {
                        name: "r-arg".to_owned(),
                        values: vec![create_argument_value("r-arg")],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Command {
                    name: "run".to_owned(),
                    arguments: vec![Argument {
                        name: "run-arg".to_owned(),
                        values: vec![create_argument_value("run-arg")],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            options: vec![],
            ..Default::default()
        },
    }
}
