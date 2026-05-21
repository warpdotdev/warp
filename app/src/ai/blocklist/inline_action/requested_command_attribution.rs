//! Module to attribute AI-generated requested commands
//! to known documents (e.g. Warp Drive objects).

use warpui::AppContext;
use warpui::SingletonEntity;

use crate::env_vars::EnvVarCollection;
use crate::env_vars::EnvVarValue;
use crate::notebooks::CloudNotebookModel;
use crate::terminal::shell::ShellType;
use crate::{
    ai::agent::AIAgentCitation, cloud_object::model::persistence::CloudModel,
    workflows::command_parser::command_matches_workflow,
};
use markdown_parser::{parse_markdown, FormattedTextLine};

/// Returns true iff the `command` is directly copied from the `document`.
pub(crate) fn is_command_copied_from_document(
    command: &str,
    document: &AIAgentCitation,
    shell_type: Option<ShellType>,
    ctx: &AppContext,
) -> bool {
    let command = command.trim();

    match document {
        AIAgentCitation::WarpDriveObject { uid } => {
            is_command_copied_from_warp_drive_object(command, uid, shell_type, ctx)
        }
        _ => false,
    }
}

/// Returns true iff the `command` is directly copied from the
/// Warp Drive object identified by `object_uid`.
fn is_command_copied_from_warp_drive_object(
    command: &str,
    object_uid: &str,
    shell_type: Option<ShellType>,
    ctx: &AppContext,
) -> bool {
    if let Some(workflow) = CloudModel::as_ref(ctx).get_workflow_by_uid(object_uid) {
        command_matches_workflow(command, &workflow.model().data)
    } else if let Some(notebook) = CloudModel::as_ref(ctx).get_notebook_by_uid(object_uid) {
        is_command_copied_from_notebook(command, notebook.model())
    } else if let Some((env_var_collection, shell_type)) = CloudModel::as_ref(ctx)
        .get_env_var_collection_by_uid(object_uid)
        .zip(shell_type)
    {
        is_command_copied_from_env_var_collection(
            command,
            &env_var_collection.model().string_model,
            shell_type,
        )
    } else {
        false
    }
}

/// Returns true iff the `command` was copied directly from one of the
/// notebook's code blocks.
fn is_command_copied_from_notebook(command: &str, notebook: &CloudNotebookModel) -> bool {
    let Ok(md) = parse_markdown(notebook.data.as_str()) else {
        return false;
    };

    for line in md.lines {
        if let FormattedTextLine::CodeBlock(code) = line {
            if command == code.code.trim() {
                return true;
            }
        }
    }

    false
}

/// Returns true iff the `command` was copied from one of the env-vars
/// in the `collection`.
///
/// TODO: it'd be ideal to attribute the command to the env-var (collection)
/// if the name of the var didn't necessarily match but the value still did.
fn is_command_copied_from_env_var_collection(
    command: &str,
    collection: &EnvVarCollection,
    shell_type: ShellType,
) -> bool {
    // Check if the command is an instantiation of all the env-vars in the collection.
    if collection.export_variables_for_shell(shell_type) == command {
        return true;
    }

    for var in &collection.vars {
        // Check if the env-var is defined as a command and matches the given command exactly.
        if let EnvVarValue::Command(secret_command) = &var.value {
            if secret_command.command == command {
                return true;
            }
        }

        // Check if the command is an initialization of the specific env-var.
        let init_string = var.get_initialization_string(shell_type);
        if init_string == command || init_string.trim_end_matches(";") == command {
            return true;
        }
    }

    false
}

#[cfg(test)]
#[path = "requested_command_attribution_tests.rs"]
mod tests;
