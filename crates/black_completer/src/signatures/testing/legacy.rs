//! "Legacy" versions of command signatures used for testing.
//!
//! Each signature in this file should be semantically equivalent to a command signature returned
//! by a function of the same name in `super::v2`; this is to ensure that the same test
//! coverage can run with the "v2" Cargo feature both enabled and disabled.
//!
//! Any new test signatures should also have a corresponding "v2" implementation in `super::v2`;
//! and the test should be written using common testing APIs (e.g. create_test_command_registry) to
//! provide coverage for both flag states of "v2".
use std::borrow::Cow;

use itertools::Itertools;
use warp_command_signatures::{
    Alias, AliasGeneratorName, Argument, ArgumentType, CommandBuilder, CommandSignatureGenerators,
    Generator, GeneratorName, GeneratorResults, Importance, IsArgumentOptional, Opt, Order,
    ParserDirectives, Priority, Signature, Suggestion as MetadataSuggestion, Template,
};
use black_util::path::ShellFamily;

use super::{TEST_ALIAS_COMMAND, TEST_GENERATOR_1_COMMAND, TEST_GENERATOR_2_COMMAND};

pub fn test_generators() -> CommandSignatureGenerators {
    CommandSignatureGenerators::new("test")
        .add_generator(
            "test1",
            Generator::script(
                CommandBuilder::single_command(TEST_GENERATOR_1_COMMAND),
                |_| GeneratorResults {
                    suggestions: vec![
                        MetadataSuggestion::new("foo"),
                        MetadataSuggestion::new("bar"),
                    ],
                    is_ordered: false,
                },
            ),
        )
        .add_generator(
            "test2",
            Generator::script(
                CommandBuilder::single_command(TEST_GENERATOR_2_COMMAND),
                |_| GeneratorResults {
                    suggestions: vec![
                        MetadataSuggestion::new("def"),
                        MetadataSuggestion::new("abc"),
                    ],
                    is_ordered: true,
                },
            ),
        )
        .add_alias(
            "alias",
            Alias::new(
                |_| TEST_ALIAS_COMMAND.to_string(),
                |_, tokens, idx| {
                    Some(
                        tokens
                            .iter()
                            .enumerate()
                            .map(|(curr_idx, token)| {
                                if curr_idx == idx {
                                    // Replace the alias (on the left) with token on the right.

                                    // Ensure each token is escaped so that we can produce a valid
                                    // new command to run the completer with.
                                    // TODO(alokedesai): Each alias generator shouldn't be
                                    // responsible for escaping tokens--this should happen higher in
                                    // the stack.
                                    match *token {
                                        "nine" => Cow::Borrowed("twelve"),
                                        "twelve" => "one".into(),
                                        "loop1" => "loop2".into(),
                                        "loop2" => "loop1".into(),
                                        s => ShellFamily::Posix.escape(s),
                                    }
                                } else {
                                    ShellFamily::Posix.escape(token)
                                }
                            })
                            .join(" "),
                    )
                },
            ),
        )
}

/// Dummy signatures to test with
pub fn git_signature() -> Signature {
    Signature {
        name: "git".to_string(),
        alias_generator: None,
        description: Some("the stupid content tracker".into()),
        arguments: None,
        subcommands: Some(vec![
            Signature {
                name: "add".to_string(),
                alias_generator: None,
                description: Some("Add file contents to the index".to_string()),
                arguments: None,
                subcommands: None,
                options: None,
                priority: Priority::default(),
                parser_directives: Default::default(),
            },
            Signature {
                name: "checkout".to_string(),
                alias_generator: None,
                description: Some("Switch branches or restore working tree files".to_string()),
                arguments: Some(vec![Argument {
                    display_name: Some("Branch".to_string()),
                    description: Some("Branch".to_owned()),
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![
                        create_argument_suggestion("漢字"),
                        create_argument_suggestion("bob/卡b卡"),
                    ],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                subcommands: None,
                options: None,
                priority: Priority::default(),
                parser_directives: Default::default(),
            },
            Signature {
                name: "clone".to_string(),
                alias_generator: None,
                description: Some("Clone a repository into a new directory".into()),
                arguments: None,
                subcommands: None,
                options: None,
                priority: Priority::default(),
                parser_directives: Default::default(),
            },
            Signature {
                name: "branch".to_string(),
                alias_generator: None,
                description: Some("List, create, or delete branches".into()),
                arguments: None,
                subcommands: None,
                options: Some(vec![
                    Opt {
                        exact_string: vec!["--delete".to_string()],
                        description: None,
                        arguments: Some(vec![Argument {
                            display_name: Some("branch".to_string()),
                            description: None,
                            is_variadic: true,
                            is_command: false,
                            argument_types: vec![
                                create_argument_suggestion("branch-1"),
                                create_argument_suggestion("second-branch"),
                            ],
                            optional: IsArgumentOptional::Required,
                            skip_generator_validation: false,
                        }]),
                        required: false,
                        priority: Priority::default(),
                    },
                    Opt {
                        exact_string: vec!["-m".to_string()],
                        description: None,
                        arguments: Some(vec![
                            Argument {
                                display_name: Some("from_branch".to_string()),
                                description: None,
                                is_variadic: false,
                                is_command: false,
                                argument_types: vec![
                                    create_argument_suggestion("branch-1"),
                                    create_argument_suggestion("second-branch"),
                                ],
                                optional: IsArgumentOptional::Required,
                                skip_generator_validation: false,
                            },
                            Argument {
                                display_name: Some("to_branch".to_string()),
                                description: None,
                                is_variadic: false,
                                is_command: false,
                                argument_types: vec![
                                    create_argument_suggestion("branch-1"),
                                    create_argument_suggestion("second-branch"),
                                ],
                                optional: IsArgumentOptional::Required,
                                skip_generator_validation: false,
                            },
                        ]),
                        required: false,
                        priority: Priority::default(),
                    },
                ]),
                priority: Priority::default(),
                parser_directives: Default::default(),
            },
        ]),
        options: Some(vec![
            Opt {
                exact_string: vec!["-p".to_string()],
                description: None,
                arguments: None,
                required: false,
                priority: Priority::Default,
            },
            Opt {
                exact_string: vec!["--version".to_string()],
                description: None,
                arguments: None,
                required: false,
                priority: Priority::Default,
            },
            Opt {
                exact_string: vec!["--help".to_string()],
                description: None,
                arguments: None,
                required: false,
                priority: Priority::Default,
            },
            Opt {
                exact_string: vec!["--bare".to_string()],
                description: None,
                arguments: None,
                required: false,
                priority: Priority::Default,
            },
            Opt {
                exact_string: vec!["-c".to_string()],
                description: None,
                arguments: None,
                required: false,
                priority: Priority::Default,
            },
        ]),
        priority: Priority::default(),
        parser_directives: Default::default(),
    }
}

pub fn java_signature() -> Signature {
    Signature {
        name: "java".to_string(),
        alias_generator: None,
        description: Some("Launch a java application".into()),
        arguments: Some(vec![Argument {
            display_name: Some("<mainclass>".to_string()),
            description: None,
            is_variadic: false,
            is_command: false,
            argument_types: vec![],
            optional: IsArgumentOptional::Optional(None),
            skip_generator_validation: false,
        }]),
        subcommands: None,
        options: Some(vec![
            Opt {
                // Java supports both styles of long option.
                exact_string: vec!["-version".to_string(), "--version".to_string()],
                description: Some("print product version to the error stream and exit".into()),
                arguments: None,
                required: false,
                priority: Priority::default(),
            },
            Opt {
                exact_string: vec![
                    "-cp".to_string(),
                    "-classpath".to_string(),
                    "--class-path".to_string(),
                ],
                description: Some("class search path of directories and zip/jar files".to_string()),
                arguments: Some(vec![Argument {
                    display_name: Some("classpath".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                required: false,
                priority: Priority::default(),
            },
        ]),
        priority: Priority::default(),
        parser_directives: Default::default(),
    }
}

/// A signature with a single positional that has no argument types.
pub fn signature_with_empty_positional() -> Signature {
    Signature {
        name: "test-empty".to_string(),
        alias_generator: None,
        description: Some("testing...".to_string()),
        arguments: Some(vec![Argument {
            display_name: Some("test-empty--arg".to_string()),
            description: None,
            is_variadic: false,
            is_command: false,
            argument_types: vec![],
            optional: IsArgumentOptional::Required,
            skip_generator_validation: false,
        }]),
        subcommands: None,
        options: None,
        priority: Priority::default(),
        parser_directives: Default::default(),
    }
}

pub fn test_signature() -> Signature {
    Signature {
        name: "test".to_string(),
        alias_generator: Some(AliasGeneratorName("alias".to_owned())),
        description: Some("testing...".to_string()),
        arguments: None,
        subcommands: Some(vec![
            Signature {
                name: "one".to_string(),
                alias_generator: None,
                description: None,
                arguments: Some(vec![
                    Argument {
                        display_name: Some("first arg".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![
                            create_argument_suggestion("one-one"),
                            create_argument_suggestion("one-two"),
                        ],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                    Argument {
                        display_name: Some("second arg".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![
                            create_argument_suggestion("two-one"),
                            create_argument_suggestion("two-two"),
                        ],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                ]),
                subcommands: None,
                options: None,
                priority: Priority::Global(Importance::More(Order(100))),
                parser_directives: Default::default(),
            },
            Signature {
                name: "two".to_string(),
                alias_generator: None,
                description: None,
                arguments: Some(vec![
                    Argument {
                        display_name: Some("two-one".to_string()),
                        description: None,
                        is_variadic: true,
                        is_command: false,
                        argument_types: vec![create_argument_suggestion("two-one")],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                    Argument {
                        display_name: Some("two-two".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![create_argument_suggestion("two-two")],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                ]),
                subcommands: None,
                options: None,
                priority: Priority::Global(Importance::Less(Order(50))),
                parser_directives: Default::default(),
            },
            Signature {
                name: "three".to_string(),
                alias_generator: None,
                description: None,
                arguments: Some(vec![
                    Argument {
                        display_name: Some("three-one".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![create_argument_suggestion("three-one")],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                    Argument {
                        display_name: Some("three-two".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![create_argument_suggestion("three-two")],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                    Argument {
                        display_name: Some("three-three".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![create_argument_suggestion("three-three")],
                        optional: IsArgumentOptional::Optional(None),
                        skip_generator_validation: false,
                    },
                ]),
                subcommands: None,
                options: None,
                priority: Priority::Global(Importance::Less(Order(1))),
                parser_directives: Default::default(),
            },
            Signature {
                name: "four".to_string(),
                alias_generator: None,
                description: None,
                arguments: Some(vec![
                    Argument {
                        display_name: Some("four-one".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![create_argument_suggestion("four-one")],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                    Argument {
                        display_name: Some("four-two".to_string()),
                        description: None,
                        is_variadic: true,
                        is_command: false,
                        argument_types: vec![create_argument_suggestion("four-two")],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                ]),
                subcommands: None,
                options: None,
                priority: Priority::default(),
                parser_directives: Default::default(),
            },
            Signature {
                name: "five".to_string(),
                alias_generator: None,
                description: None,
                arguments: Some(vec![Argument {
                    display_name: Some("five".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![
                        ArgumentType::Generator(GeneratorName("test1".to_owned())),
                        ArgumentType::Generator(GeneratorName("test2".to_owned())),
                    ],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                subcommands: None,
                options: None,
                priority: Priority::default(),
                parser_directives: Default::default(),
            },
            Signature {
                name: "six".to_owned(),
                alias_generator: None,
                description: None,
                subcommands: None,
                arguments: Some(vec![Argument {
                    display_name: Some("six-one".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![
                        create_argument_suggestion_with_priority("six-arg", Priority::Default),
                        create_argument_suggestion_with_priority(
                            "six-arg-2",
                            Priority::Global(Importance::More(Order(100))),
                        ),
                    ],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                options: None,
                priority: Priority::Default,
                parser_directives: Default::default(),
            },
            Signature {
                name: "seven".to_owned(),
                alias_generator: None,
                description: None,
                subcommands: None,
                arguments: Some(vec![Argument {
                    display_name: Some("seven-arg".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![
                        ArgumentType::Generator(GeneratorName("test2".to_owned())),
                        ArgumentType::Generator(GeneratorName("test2".to_owned())),
                    ],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                options: None,
                priority: Priority::Default,
                parser_directives: Default::default(),
            },
            Signature {
                name: "eight".to_owned(),
                alias_generator: None,
                description: None,
                subcommands: Some(vec![Signature {
                    name: "eight-subcommand".to_string(),
                    alias_generator: None,
                    description: None,
                    arguments: None,
                    subcommands: None,
                    options: None,
                    priority: Default::default(),
                    parser_directives: Default::default(),
                }]),
                arguments: Some(vec![
                    Argument {
                        display_name: Some("eight-arg".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![create_argument_suggestion_with_priority(
                            "eight-arg",
                            Priority::Default,
                        )],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                    Argument {
                        display_name: Some("eight-arg-2".to_string()),
                        description: None,
                        is_variadic: true,
                        is_command: false,
                        argument_types: vec![create_argument_suggestion_with_priority(
                            "eight-arg-2",
                            Priority::Default,
                        )],
                        optional: IsArgumentOptional::Optional(None),
                        skip_generator_validation: false,
                    },
                ]),
                options: None,
                priority: Priority::Default,
                parser_directives: Default::default(),
            },
            Signature {
                name: "nine".to_owned(),
                alias_generator: None,
                description: None,
                subcommands: None,
                arguments: Some(vec![Argument {
                    display_name: Some("nine-arg".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: true,
                    argument_types: vec![create_argument_suggestion_with_priority(
                        "git",
                        Priority::Default,
                    )],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                options: None,
                priority: Priority::Default,
                parser_directives: Default::default(),
            },
        ]),
        options: Some(vec![
            Opt {
                exact_string: vec!["--long".to_string()],
                description: None,
                arguments: Some(vec![Argument {
                    display_name: Some("long-one".to_string()),
                    description: None,
                    is_variadic: true,
                    is_command: false,
                    argument_types: vec![
                        create_argument_suggestion("long-one"),
                        create_argument_suggestion("long-two"),
                    ],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                required: false,
                priority: Priority::Global(Importance::Less(Order(1))),
            },
            Opt {
                exact_string: vec!["--not-long".to_string()],
                description: None,
                arguments: Some(vec![Argument {
                    display_name: Some("long-one".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![
                        create_argument_suggestion("not-long-one"),
                        create_argument_suggestion("not-long-two"),
                    ],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                required: false,
                priority: Priority::Global(Importance::More(Order(100))),
            },
            Opt {
                exact_string: vec!["--required-args".to_string()],
                description: None,
                arguments: Some(vec![
                    Argument {
                        display_name: Some("required-arg-1".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![
                            create_argument_suggestion("arg-1-1"),
                            create_argument_suggestion("arg-1-2"),
                        ],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                    Argument {
                        display_name: Some("required-arg-2".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![
                            create_argument_suggestion("arg-2-1"),
                            create_argument_suggestion("arg-2-2"),
                        ],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                ]),
                required: false,
                priority: Priority::Global(Importance::More(Order(100))),
            },
            Opt {
                exact_string: vec!["--required-args-with-var".to_string()],
                description: None,
                arguments: Some(vec![
                    Argument {
                        display_name: Some("required-arg".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![
                            create_argument_suggestion("arg-1"),
                            create_argument_suggestion("arg-2"),
                        ],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                    Argument {
                        display_name: Some("variadic-arg".to_string()),
                        description: None,
                        is_variadic: true,
                        is_command: false,
                        argument_types: vec![
                            create_argument_suggestion("vararg-1"),
                            create_argument_suggestion("vararg-2"),
                        ],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                ]),
                required: false,
                priority: Priority::Global(Importance::Less(Order(1))),
            },
            Opt {
                exact_string: vec!["--required-and-optional-args".to_string()],
                description: None,
                arguments: Some(vec![
                    Argument {
                        display_name: Some("required-arg".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![
                            create_argument_suggestion("required-1"),
                            create_argument_suggestion("required-2"),
                        ],
                        optional: IsArgumentOptional::Required,
                        skip_generator_validation: false,
                    },
                    Argument {
                        display_name: Some("optional-arg".to_string()),
                        description: None,
                        is_variadic: false,
                        is_command: false,
                        argument_types: vec![
                            create_argument_suggestion("optional-1"),
                            create_argument_suggestion("optional-2"),
                        ],
                        optional: IsArgumentOptional::Optional(None),
                        skip_generator_validation: false,
                    },
                ]),
                required: false,
                priority: Priority::default(),
            },
            Opt {
                exact_string: vec!["--template-args-for-opt".to_string()],
                description: None,
                arguments: Some(vec![Argument {
                    display_name: Some("templated".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![ArgumentType::Template(Template {
                        type_name: warp_command_signatures::TemplateType::FilesAndFolders,
                        filter_name: None,
                    })],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                required: false,
                priority: Priority::default(),
            },
            Opt {
                exact_string: vec!["-r".to_string()],
                description: None,
                arguments: None,
                required: false,
                priority: Priority::default(),
            },
            Opt {
                exact_string: vec!["-V".to_string()],
                description: None,
                arguments: None,
                required: false,
                priority: Priority::default(),
            },
        ]),
        priority: Priority::default(),
        parser_directives: Default::default(),
    }
}

pub fn fuzzy_signature() -> Signature {
    Signature {
        name: "fuzzy".to_string(),
        alias_generator: None,
        description: Some("testing...".to_string()),
        arguments: None,
        subcommands: Some(vec![
            Signature {
                name: "prefix1".to_string(),
                alias_generator: None,
                description: None,
                arguments: None,
                subcommands: None,
                options: None,
                priority: Priority::Default,
                parser_directives: Default::default(),
            },
            Signature {
                name: "prefix2".to_string(),
                alias_generator: None,
                description: None,
                arguments: None,
                subcommands: None,
                options: None,
                priority: Priority::Global(Importance::More(Order(100))),
                parser_directives: Default::default(),
            },
            Signature {
                name: "suffix-pre-fix".to_string(),
                alias_generator: None,
                description: None,
                arguments: None,
                subcommands: None,
                options: None,
                priority: Priority::Default,
                parser_directives: Default::default(),
            },
        ]),
        options: Some(vec![Opt {
            exact_string: vec!["--pre-fx".to_string()],
            description: None,
            arguments: None,
            required: false,
            priority: Priority::Default,
        }]),
        priority: Priority::default(),
        parser_directives: Default::default(),
    }
}

pub fn cd_signature() -> Signature {
    Signature {
        name: "cd".to_string(),
        alias_generator: None,
        description: Some("testing...".to_string()),
        arguments: Some(vec![Argument {
            display_name: Some("directories".to_string()),
            description: None,
            is_variadic: false,
            is_command: false,
            argument_types: vec![
                create_hidden_argument_suggestion('-'),
                ArgumentType::Template(Template {
                    type_name: warp_command_signatures::TemplateType::Folders { must_exist: true },
                    filter_name: None,
                }),
            ],
            optional: IsArgumentOptional::Required,
            skip_generator_validation: false,
        }]),
        priority: Priority::default(),
        subcommands: None,
        options: None,
        parser_directives: Default::default(),
    }
}

pub fn npm_signature() -> Signature {
    Signature {
        name: "npm".to_string(),
        alias_generator: None,
        description: Some("testing...".to_string()),
        priority: Priority::default(),
        arguments: None,
        subcommands: Some(vec![
            Signature {
                name: "r".to_string(),
                alias_generator: None,
                description: None,
                arguments: Some(vec![Argument {
                    display_name: Some("r-arg".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![create_argument_suggestion("r-arg")],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                subcommands: None,
                options: None,
                priority: Priority::Default,
                parser_directives: Default::default(),
            },
            Signature {
                name: "run".to_string(),
                alias_generator: None,
                description: None,
                arguments: Some(vec![Argument {
                    display_name: Some("run-arg".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![create_argument_suggestion("run-arg")],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                subcommands: None,
                options: None,
                priority: Priority::Default,
                parser_directives: Default::default(),
            },
        ]),
        options: None,
        parser_directives: Default::default(),
    }
}

pub fn ls_signature() -> Signature {
    Signature {
        name: "ls".to_string(),
        alias_generator: None,
        description: Some("testing...".to_string()),
        priority: Priority::default(),
        arguments: Some(vec![Argument {
            display_name: Some("filepaths".to_string()),
            description: None,
            is_variadic: true,
            is_command: false,
            argument_types: vec![ArgumentType::Template(Template {
                type_name: warp_command_signatures::TemplateType::FilesAndFolders,
                filter_name: None,
            })],
            optional: IsArgumentOptional::Optional(None),
            skip_generator_validation: false,
        }]),
        subcommands: None,
        options: Some(vec![
            Opt {
                exact_string: vec!["-a".to_string()],
                description: None,
                arguments: None,
                required: false,
                priority: Priority::Default,
            },
            Opt {
                exact_string: vec!["--color".to_string()],
                description: None,
                arguments: Some(vec![Argument {
                    display_name: Some("when".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![
                        create_argument_suggestion("force"),
                        create_argument_suggestion("auto"),
                        create_argument_suggestion("never"),
                    ],
                    optional: IsArgumentOptional::Optional(None),
                    skip_generator_validation: false,
                }]),
                required: false,
                priority: Priority::Default,
            },
            Opt {
                exact_string: vec!["--test".to_string()],
                description: None,
                arguments: Some(vec![Argument {
                    display_name: Some("when".to_string()),
                    description: None,
                    is_variadic: true,
                    is_command: false,
                    argument_types: vec![
                        create_argument_suggestion("force"),
                        create_argument_suggestion("auto"),
                        create_argument_suggestion("never"),
                    ],
                    optional: IsArgumentOptional::Optional(None),
                    skip_generator_validation: false,
                }]),
                required: false,
                priority: Priority::Default,
            },
        ]),
        parser_directives: Default::default(),
    }
}

pub fn add_content_signature() -> Signature {
    Signature {
        name: "Add-Content".to_string(),
        alias_generator: None,
        description: None,
        priority: Priority::default(),
        arguments: Some(vec![
            Argument {
                display_name: Some("-Path".to_string()),
                description: None,
                is_variadic: false,
                is_command: false,
                argument_types: vec![ArgumentType::Template(Template {
                    type_name: warp_command_signatures::TemplateType::FilesAndFolders,
                    filter_name: None,
                })],
                optional: IsArgumentOptional::Optional(None),
                skip_generator_validation: false,
            },
            Argument {
                display_name: Some("-Value".to_string()),
                description: None,
                is_variadic: false,
                is_command: false,
                argument_types: vec![ArgumentType::Template(Template {
                    type_name: warp_command_signatures::TemplateType::FilesAndFolders,
                    filter_name: None,
                })],
                optional: IsArgumentOptional::Optional(None),
                skip_generator_validation: false,
            },
        ]),
        subcommands: None,
        options: Some(vec![
            Opt {
                exact_string: vec!["-Force".to_string()],
                description: None,
                arguments: None,
                required: false,
                priority: Priority::Default,
            },
            Opt {
                exact_string: vec!["-Encoding".to_string()],
                description: None,
                arguments: Some(vec![Argument {
                    display_name: Some("System.Text.Encoding".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![
                        create_argument_suggestion("ASCII"),
                        create_argument_suggestion("UTF8"),
                    ],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                required: false,
                priority: Priority::Default,
            },
            Opt {
                exact_string: vec!["-Exclude".to_string()],
                description: None,
                arguments: Some(vec![Argument {
                    display_name: Some("System.String[]".to_string()),
                    description: None,
                    is_variadic: false,
                    is_command: false,
                    argument_types: vec![ArgumentType::Template(Template {
                        type_name: warp_command_signatures::TemplateType::FilesAndFolders,
                        filter_name: None,
                    })],
                    optional: IsArgumentOptional::Required,
                    skip_generator_validation: false,
                }]),
                required: false,
                priority: Default::default(),
            },
        ]),
        parser_directives: ParserDirectives {
            flags_are_posix_noncompliant: true,
            flags_match_unique_prefix: true,
            always_case_insensitive: true,
        },
    }
}

fn create_argument_suggestion(name: impl Into<String>) -> ArgumentType {
    create_argument_suggestion_with_priority(name, Priority::Default)
}

fn create_argument_suggestion_with_priority(
    name: impl Into<String>,
    priority: Priority,
) -> ArgumentType {
    ArgumentType::Suggestion(MetadataSuggestion::new(name.into()).with_priority(priority))
}

fn create_hidden_argument_suggestion(name: impl Into<String>) -> ArgumentType {
    ArgumentType::Suggestion(MetadataSuggestion {
        exact_string: name.into(),
        description: None,
        priority: Default::default(),
        icon: None,
        is_hidden: true,
        display_name: None,
    })
}
