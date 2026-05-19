use std::collections::HashMap;

use lazy_static::lazy_static;

use crate::workflows::workflow::{Argument, Workflow};

use super::{compute_workflow_display_data, compute_workflow_display_data_for_history_command};

lazy_static! {
    static ref WORKFLOW: Workflow = Workflow::Command {
        name: "Run single integration test with display".to_owned(),
        command:
            "RUST_BACKTRACE=full WARP_SHELL_PATH={{shell_path}} cargo run -p integration --bin \
          integration --features=with_real_display_in_integration_tests -- {{test_name}}"
                .to_owned(),
        arguments: vec![
            Argument {
                name: "shell_path".to_owned(),
                default_value: Some("/bin/bash".to_owned()),
                description: None,
                arg_type: Default::default()
            },
            Argument {
                name: "test_name".to_owned(),
                default_value: None,
                description: None,
                arg_type: Default::default()
            }
        ],
        description: None,
        tags: vec![],
        source_url: None,
        author: None,
        author_url: None,
        shells: vec![
            warp_workflows::Shell::Zsh,
            warp_workflows::Shell::Bash,
            warp_workflows::Shell::Fish,
        ],
        environment_variables: None,
    };
    static ref WORKFLOW_MULTIPLE_INSTANCES_SAME_PARAMETER: Workflow = Workflow::Command {
        name: "Echo my name 3 times".to_owned(),
        command: r#"echo {{name}} {{name}} {{name}}"#.to_owned(),
        arguments: vec![Argument {
            name: "name".to_owned(),
            default_value: Some("Zach".to_owned()),
            description: None,
            arg_type: Default::default(),
        },],
        description: None,
        tags: vec![],
        source_url: None,
        author: None,
        author_url: None,
        shells: vec![
            warp_workflows::Shell::Zsh,
            warp_workflows::Shell::Bash,
            warp_workflows::Shell::Fish,
        ],
        environment_variables: None,
    };
    static ref WORKFLOW_NO_PARAMETERS: Workflow = Workflow::Command {
        name: "Print numbers 1 to 13".to_owned(),
        command: r#"for i in {0..13}; do echo $i; done"#.to_owned(),
        arguments: vec![],
        description: None,
        tags: vec![],
        source_url: None,
        author: None,
        author_url: None,
        shells: vec![
            warp_workflows::Shell::Bash,
            warp_workflows::Shell::Fish,
            warp_workflows::Shell::Zsh,
        ],
        environment_variables: None,
    };
    static ref WORKFLOW_WITH_ESCAPES: Workflow = Workflow::Command {
        name: "Workflow with escaped arguments".to_owned(),
        command:
            r#"docker history --no-trunc --format {{arg1}} {{{.ID}}}: {{{.CreatedBy}}} {{{arg2}}} {{arg2}}"#
                .to_owned(),
        arguments: vec![
            Argument {
                name: "arg1".to_owned(),
                default_value: Some("default1".to_owned()),
                description: None,
                arg_type: Default::default()
            },
            Argument {
                name: "arg2".to_owned(),
                default_value: None,
                description: None,
                arg_type: Default::default()
            }
        ],
        description: None,
        tags: vec![],
        source_url: None,
        author: None,
        author_url: None,
        shells: vec![
            warp_workflows::Shell::Zsh,
            warp_workflows::Shell::Bash,
            warp_workflows::Shell::Fish,
        ],
        environment_variables: None,
    };
    static ref WORKFLOW_WITH_DUPLICATES_AND_ESCAPES: Workflow = Workflow::Command {
        name: "Workflow with escaped arguments".to_owned(),
        command:
            r#"{{{hi}}} {{hi}} {{{{{{hi}} {{{{hi}}}} {{{{{hi}}} {{hi}}"#
                .to_owned(),
        arguments: vec![
            Argument {
                name: "hi".to_owned(),
                default_value: None,
                description: None,
                arg_type: Default::default()
            },
        ],
        description: None,
        tags: vec![],
        source_url: None,
        author: None,
        author_url: None,
        shells: vec![
            warp_workflows::Shell::Zsh,
            warp_workflows::Shell::Bash,
            warp_workflows::Shell::Fish,
        ],
        environment_variables: None,
    };

    static ref WORKFLOW_WITH_MULTIBYTE_CHARS: Workflow = Workflow::Command {
        name: "Workflow with multiyte chars".to_owned(),
        command:
            r#"echo "hello 😎{{name}}🤠{{name}}"#
                .to_owned(),
        arguments: vec![
            Argument {
                name: "name".to_owned(),
                default_value: None,
                description: None,
                arg_type: Default::default()
            },
        ],
        description: None,
        tags: vec![],
        source_url: None,
        author: None,
        author_url: None,
        shells: vec![
            warp_workflows::Shell::Zsh,
            warp_workflows::Shell::Bash,
            warp_workflows::Shell::Fish,
        ],
        environment_variables: None,
    };
}

#[test]
fn test_compute_workflow_display_data_for_linked_history_command() {
    // This command should be parsed as a workflow-linked command.
    //
    // It passes "/opt/homebrew/bin/fish" for the {{shell_path}} parameter and
    // test_command_search_loads_history for the {{test_name}} parameter.
    let linked_history_command = "RUST_BACKTRACE=full WARP_SHELL_PATH=/opt/homebrew/bin/fish cargo run -p integration --bin \
        integration --features=with_real_display_in_integration_tests -- \
        test_command_search_loads_history";
    let display_data =
        compute_workflow_display_data_for_history_command(linked_history_command, &WORKFLOW)
            .expect("WorkflowDisplayData should be Some()");
    assert_eq!(
        display_data.command_with_replaced_arguments.as_str(),
        linked_history_command,
    );
    assert_eq!(
        display_data.replaced_ranges,
        vec![36.into()..58.into(), 155.into()..188.into()]
    );
    assert_eq!(
        display_data.argument_index_to_char_range_map,
        HashMap::from([
            (0.into(), vec![36.into()..58.into()]),
            (1.into(), vec![155.into()..188.into()])
        ])
    );
}

#[test]
fn test_compute_workflow_display_data_for_linked_history_command_with_multiple_instances_same_parameter(
) {
    // This command should be parsed as a workflow-linked command.
    //
    // The workflow contains multiple instances of the same parameter
    let linked_history_command = r#"echo warp warp warp"#;
    let display_data = compute_workflow_display_data_for_history_command(
        linked_history_command,
        &WORKFLOW_MULTIPLE_INSTANCES_SAME_PARAMETER,
    )
    .expect("WorkflowDisplayData should be Some()");
    assert_eq!(
        display_data.command_with_replaced_arguments.as_str(),
        linked_history_command,
    );
    assert_eq!(
        display_data.replaced_ranges,
        vec![
            5.into()..9.into(),
            10.into()..14.into(),
            15.into()..19.into()
        ]
    );
    assert_eq!(
        display_data.argument_index_to_char_range_map,
        HashMap::from([(
            0.into(),
            vec![
                5.into()..9.into(),
                10.into()..14.into(),
                15.into()..19.into()
            ]
        ),])
    );
}

#[test]
fn test_compute_workflow_display_data_for_linked_history_command_with_no_parameters() {
    let linked_history_command = r#"for i in {0..13}; do echo $i; done"#;
    let display_data = compute_workflow_display_data_for_history_command(
        linked_history_command,
        &WORKFLOW_NO_PARAMETERS,
    )
    .expect("WorkflowDisplayData should be Some()");
    assert_eq!(
        display_data.command_with_replaced_arguments.as_str(),
        linked_history_command,
    );
    assert!(display_data.replaced_ranges.is_empty());
    assert!(display_data.argument_index_to_char_range_map.is_empty());
}

#[test]
fn test_compute_workflow_display_data_for_linked_history_command_with_multibyte_chars() {
    let linked_history_command = r#"echo 😎🤠 😎🤠 😎🤠"#;
    let display_data = compute_workflow_display_data_for_history_command(
        linked_history_command,
        &WORKFLOW_MULTIPLE_INSTANCES_SAME_PARAMETER,
    )
    .expect("WorkflowDisplayData should be Some()");
    assert_eq!(
        display_data.command_with_replaced_arguments.as_str(),
        linked_history_command,
    );
    assert_eq!(
        display_data.replaced_ranges,
        vec![
            5.into()..13.into(),
            14.into()..22.into(),
            23.into()..31.into(),
        ]
    );
    assert_eq!(
        display_data.argument_index_to_char_range_map,
        HashMap::from([(
            0.into(),
            vec![
                5.into()..7.into(),
                8.into()..10.into(),
                11.into()..13.into()
            ]
        )])
    );
}

#[test]
fn test_compute_workflow_display_data_for_unlinked_history_command() {
    let unlinked_history_command = r#"echo foo"#;
    assert!(
        compute_workflow_display_data_for_history_command(unlinked_history_command, &WORKFLOW)
            .is_none()
    );
}

#[test]
fn test_compute_workflow_display_data_for_unlinked_history_command_with_no_parameters() {
    let unlinked_history_command = r#"echo foo"#;
    assert!(compute_workflow_display_data_for_history_command(
        unlinked_history_command,
        &WORKFLOW_NO_PARAMETERS
    )
    .is_none());
}

#[test]
fn test_compute_workflow_display_data_for_similar_but_unlinked_history_command() {
    // This command is missing the "-p" from the workflow's command, so should not be linked to the
    // command.
    let similar_but_unlinked_history_command =
        "RUST_BACKTRACE=full WARP_SHELL_PATH=/opt/homebrew/bin/fish cargo run integration --bin \
        integration --features=with_real_display_in_integration_tests -- \
        test_command_search_loads_history";
    assert!(compute_workflow_display_data_for_history_command(
        similar_but_unlinked_history_command,
        &WORKFLOW
    )
    .is_none());
}

#[test]
fn test_compute_workflow_display_data_with_escaped_arguments() {
    let display_data = compute_workflow_display_data(&WORKFLOW_WITH_ESCAPES);
    let correct_command =
        "docker history --no-trunc --format default1 {{.ID}}: {{.CreatedBy}} {{arg2}} arg2";

    assert_eq!(
        display_data.command_with_replaced_arguments.as_str(),
        correct_command
    );
    assert_eq!(
        display_data.replaced_ranges,
        vec![35.into()..43.into(), 77.into()..81.into()]
    );
    assert_eq!(
        display_data.argument_index_to_char_range_map,
        HashMap::from([
            (0.into(), vec![35.into()..43.into()]),
            (1.into(), vec![77.into()..81.into()])
        ])
    );
}

#[test]
fn test_compute_workflow_display_data_with_duplicates_and_escaped_arguments() {
    let display_data = compute_workflow_display_data(&WORKFLOW_WITH_DUPLICATES_AND_ESCAPES);
    let correct_command = "{{hi}} hi {{{{hi {{{hi}}} {{{{hi}} hi";

    assert_eq!(
        display_data.command_with_replaced_arguments.as_str(),
        correct_command
    );
    assert_eq!(
        display_data.replaced_ranges,
        vec![
            7.into()..9.into(),
            14.into()..16.into(),
            35.into()..37.into()
        ]
    );
    assert_eq!(
        display_data.argument_index_to_char_range_map,
        HashMap::from([(
            0.into(),
            vec![
                7.into()..9.into(),
                14.into()..16.into(),
                35.into()..37.into()
            ]
        ),])
    );
}

#[test]
fn test_compute_workflow_display_data_for_linked_history_command_with_escaped_args() {
    let linked_history_command = "{{hi}} foo {{{{foo {{{hi}}} {{{{hi}} foo";

    let display_data = compute_workflow_display_data_for_history_command(
        linked_history_command,
        &WORKFLOW_WITH_DUPLICATES_AND_ESCAPES,
    )
    .expect("WorkflowDisplayData should be Some()");

    assert_eq!(
        display_data.command_with_replaced_arguments.as_str(),
        linked_history_command,
    );
    assert_eq!(
        display_data.replaced_ranges,
        vec![
            7.into()..10.into(),
            15.into()..18.into(),
            37.into()..40.into()
        ]
    );
}

#[test]
fn test_compute_workflow_display_data_with_multibyte_chars() {
    let display_data = compute_workflow_display_data(&WORKFLOW_WITH_MULTIBYTE_CHARS);
    assert_eq!(
        display_data.command_with_replaced_arguments.as_str(),
        r#"echo "hello 😎name🤠name"#
    );
    assert_eq!(
        display_data.replaced_ranges,
        vec![16.into()..20.into(), 24.into()..28.into()]
    );
    assert_eq!(
        display_data.argument_index_to_char_range_map,
        HashMap::from([(0.into(), vec![13.into()..17.into(), 18.into()..22.into()])])
    );
}
