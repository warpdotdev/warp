use crate::env_vars::{view::command_dialog::EnvVarSecretCommand, EnvVar};

use super::*;

#[test]
fn test_is_command_copied_from_notebook() {
    let notebook = CloudNotebookModel {
        title: String::from(""),
        data: String::from("hello world\n```\n foobar \n```\n"),
        ai_document_id: None,
        conversation_id: None,
    };
    assert!(!is_command_copied_from_notebook("hello world", &notebook));
    assert!(!is_command_copied_from_notebook("foo", &notebook));
    assert!(is_command_copied_from_notebook("foobar", &notebook));
}

#[test]
fn test_is_command_copied_from_env_var_collection() {
    let shell_type = ShellType::Zsh;
    let collection = EnvVarCollection {
        title: None,
        description: None,
        vars: vec![
            EnvVar {
                name: String::from("GCP_TOKEN"),
                description: None,
                value: EnvVarValue::Command(EnvVarSecretCommand {
                    name: String::from("gcloud cmd"),
                    command: String::from("gcloud print auth token"),
                }),
            },
            EnvVar {
                name: String::from("OPENAI_KEY"),
                description: None,
                value: EnvVarValue::Command(EnvVarSecretCommand {
                    name: String::from("openai"),
                    command: String::from("openai --token"),
                }),
            },
        ],
    };
    assert!(!is_command_copied_from_env_var_collection(
        "gcloud",
        &collection,
        shell_type
    ));
    assert!(is_command_copied_from_env_var_collection(
        "gcloud print auth token",
        &collection,
        shell_type
    ));
    assert!(is_command_copied_from_env_var_collection(
        "export GCP_TOKEN=$(gcloud print auth token)",
        &collection,
        shell_type
    ));

    // TODO(suraj): enable this after fixing how we export env-vars
    // assert!(is_command_copied_from_env_var_collection(
    //     "export GCP_TOKEN=$(gcloud print auth token); export OPENAI_KEY=$(openai --token);",
    //     &collection,
    //     shell_type
    // ));
}
