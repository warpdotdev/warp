pub mod render;
pub mod settings;
pub mod success_block;
pub mod trigger_state;

use crate::terminal::model::terminal_model::SubshellInitializationInfo;
use crate::terminal::shell::ShellType;
use crate::ASSETS;
use channel_versions::overrides::TargetOS;
use warpui::AssetProvider;

#[derive(Debug)]
pub enum WarpificationSource {
    Ssh,
    Subshell,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SubshellSource {
    Command(String),
    EnvVarCollection(String),
}

/// This template is for the snippet that appears in the output grid for the success block if the
/// subshell is local.
fn get_subshell_bootstrap_success_block_path(shell_type: ShellType) -> Option<&'static str> {
    match shell_type {
        ShellType::Bash | ShellType::Zsh => {
            Some("bundled/bootstrap/bash_zsh_subshell_bootstrap_block_output.txt")
        }
        ShellType::Fish => Some("bundled/bootstrap/fish_subshell_bootstrap_block_output.txt"),
        ShellType::PowerShell => None,
    }
}

/// Returns OutputGrid bytes to be rendered in the hardcoded "Warpified subshell" block that's added
/// to the blocklist upon successful subshell bootstrap.
///
/// The exact block contents varies based on whether or not the session is local or remote, in
/// addition to the given `shell_type`.
pub fn subshell_bootstrap_success_block_bytes(
    subshell_initialization_info: &SubshellInitializationInfo,
    shell_type: ShellType,
    os: TargetOS,
    disable_tmux: bool,
) -> (Vec<u8>, bool) {
    let from_env_var_collection = subshell_initialization_info
        .env_var_collection_name
        .is_some();

    if from_env_var_collection {
        return (vec![], false);
    }

    let Some(subshell_bootstrap_success_block_path) =
        get_subshell_bootstrap_success_block_path(shell_type)
    else {
        return ("".into(), false);
    };

    let templated_subshell_bootstrap_success_block_output_bytes = ASSETS
        .get(subshell_bootstrap_success_block_path)
        .unwrap_or_else(|_| {
            panic!("Failed to retrieve {subshell_bootstrap_success_block_path} from assets.")
        })
        .to_vec();

    let rc_file_paths = shell_type.rc_file_paths(os);
    let mut is_executable = true;
    let commands: Vec<Vec<u8>> = rc_file_paths
        .iter()
        .map(|rc_file_path| {
            let rc_file_path = rc_file_path.to_str();
            is_executable &= rc_file_path.is_some();
            replace_template_chars_with_arguments(
                templated_subshell_bootstrap_success_block_output_bytes
                    .trim_ascii_end()
                    .to_owned()
                    .to_vec(),
                vec![
                    rc_file_path.unwrap_or("<Your RC file>").to_owned(),
                    shell_type.name().to_owned(),
                    if disable_tmux {
                        ", \"tmux\": false"
                    } else {
                        ""
                    }
                    .to_owned(),
                ],
            )
        })
        .collect();
    (commands.concat(), is_executable)
}

/// Replaces each instance of '%' in the given `templated_bytes` vector with `String` in
/// `arguments`, in order.
///
/// The bundled block content txt files are templated using '%' as a placeholder to be dynamically
/// replaced at runtime. This is useful to cater the exact block contents to the bootstrapped
/// subshell.
fn replace_template_chars_with_arguments(
    mut templated_bytes: Vec<u8>,
    arguments: Vec<String>,
) -> Vec<u8> {
    // This was an arbitrarily chosen character.
    const TEMPLATE_CHAR: u8 = b'%';

    for argument in arguments {
        let template_i = templated_bytes.iter().position(|b| b == &TEMPLATE_CHAR);
        if let Some(template_i) = template_i {
            templated_bytes.splice(
                template_i..template_i + 1,
                argument.into_bytes().into_iter(),
            );
        } else {
            debug_assert!(false, "Number of arguments does not match number of template chars (%) in hardcoded subshell block bytes.");
        }
    }
    templated_bytes
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
