use itertools::Itertools;
use warp_core::session_id::SessionId;
use warp_terminal::bootstrap::SESSION_ID_PLACEHOLDER;
pub use warp_terminal::bootstrap::{
    generate_session_id, init_shell_script_for_shell, load_and_escape_script, script_for_shell,
};
use warpui::{AppContext, AssetProvider, SingletonEntity};

#[cfg(feature = "local_fs")]
use super::{
    model::session::{BootstrapSessionType, SessionInfo},
    warpify::settings::{PIPENV_SUBSHELL_COMMAND_REGEX, POETRY_SUBSHELL_COMMAND_REGEX},
};
use crate::env_vars::{EnvVar, EnvVarExt};
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::shell::ShellType;

#[cfg(feature = "local_fs")]
pub fn is_container_subshell(session_info: &SessionInfo) -> bool {
    session_info.subshell_info.as_ref().is_some_and(|info| {
        let first_token = info
            .spawning_command
            .split_ascii_whitespace()
            .next()
            .unwrap_or("");
        first_token == "docker" || first_token == "podman"
    })
}

/// Returns `true` if Warp should use an RC-file based bootstrap (e.g. dump the bootstrap script to
/// a temp file and `source` it) for a newly spawned session with the given `shell_type`, and
/// associated `session_type` and `subshell_initialization_info`.
///
/// This returns `true` for local Fish/Pwsh shells and local subshells spawned via `poetry shell`.
///
/// We use RC-file based bootstrap for local Fish shells because there is a long-standing bug which
/// causes an explosion of formatting output when a command is longer than a screen height. (See
/// https://github.com/fish-shell/fish-shell/issues/7296 for more) This multiplication of output
/// makes our bootstrap take a long time, as we need to process all of that output (even though
/// most of it is irrelevant). To avoid the impact on bootstrap time, we write the script to a
/// temporary file and then source that file (which avoids writing the long script to the shell
/// itself).
///
/// We use RC-file based bootstrap for PowerShell because chars written to the PTY get randomly
/// ignored. See PLAT-757 in Linear.
///
/// We use RC-file based bootstrap for `poetry shell` subshells because the underlying library used
/// to spawn a subshell by `poetry shell` uses blocking PTY reads and writes, which results in a
/// deadlock when attempting to write the whole bootstrap script to the PTY; RC file-based
/// bootstrap is the only known way to bootstrap such subshells successfully.
///
/// We use RC-file based bootstrap for MSYS2 because it has slow PTY throughput.
#[cfg(feature = "local_fs")]
pub fn should_use_rc_file_bootstrap_method(
    shell_type: ShellType,
    session_info: &SessionInfo,
) -> bool {
    use super::ShellLaunchData;

    // Container subshells cannot access host temp files, so the RC-file
    // method is never viable for them.
    if is_container_subshell(session_info) {
        return false;
    }

    let session_type = &session_info.session_type;
    match session_type {
        BootstrapSessionType::Local => {
            let subshell_initialization_info = session_info.subshell_info.as_ref();
            let is_poetry_subshell = subshell_initialization_info
                .as_ref()
                .map(|info| POETRY_SUBSHELL_COMMAND_REGEX.is_match(info.spawning_command.as_str()))
                .unwrap_or(false);
            let is_pipenv_subshell = subshell_initialization_info
                .as_ref()
                .map(|info| PIPENV_SUBSHELL_COMMAND_REGEX.is_match(info.spawning_command.as_str()))
                .unwrap_or(false);
            let is_msys2 = session_info
                .launch_data
                .as_ref()
                .is_some_and(|data| matches!(data, ShellLaunchData::MSYS2 { .. }));
            shell_type == ShellType::Fish
                || shell_type == ShellType::PowerShell
                || is_poetry_subshell
                || ((is_pipenv_subshell
                    || (subshell_initialization_info.is_some() && cfg!(windows)))
                    && shell_type == ShellType::Zsh)
                || is_msys2
        }
        BootstrapSessionType::WarpifiedRemote => false,
    }
}

/// Returns the command to be used to emit the InitShell hook for a new subshell session.
///
/// If `shell_type` is `Some()`, returns a shell type-specific command (e.g. valid command for
/// bash, fish, or zsh). Otherwise, returns a shell type-agnostic command that emits the right
/// `InitShell` hook based on the shell it is evaluated in.
pub fn init_subshell_command(
    shell_type: Option<ShellType>,
    vars: &[EnvVar],
    session_id: SessionId,
    ctx: &AppContext,
) -> String {
    match shell_type {
        Some(shell_type) => {
            let subshell_script =
                init_subshell_script_for_shell(shell_type, &crate::ASSETS, vars, session_id, ctx);
            format!(r#" [ -z $WARP_BOOTSTRAPPED ] && eval '{subshell_script}'"#)
        }
        None => init_subshell_script_for_unknown_shell(&crate::ASSETS, session_id),
    }
}

/// Returns the init subshell script for the given `shell_type` (e.g. the script that emits the
/// subshell version of the InitShell DCS hook).
///
/// The returned script is one line and has escaped single-quotes for the purposes of being passed
/// as a single-quoted argument to 'eval'.
fn init_subshell_script_for_shell(
    shell_type: ShellType,
    assets: &dyn AssetProvider,
    env_vars: &[EnvVar],
    session_id: SessionId,
    ctx: &AppContext,
) -> String {
    let honor_ps1 = *SessionSettings::as_ref(ctx).honor_ps1;
    let honor_ps1_env_var_value = if honor_ps1 { "1" } else { "0" };

    // Prepend environment variable settings to the script
    let env_setup_script = format!(
        "export WARP_HONOR_PS1={}; {}",
        honor_ps1_env_var_value,
        env_vars
            .iter()
            .map(|var| var.get_initialization_string(shell_type))
            .collect_vec()
            .join(" ")
    );

    // Load and escape the shell-specific init script
    let shell_init_script = match shell_type {
        ShellType::Zsh => load_and_escape_script("bundled/bootstrap/zsh_init_subshell.sh", assets),
        ShellType::Bash => {
            load_and_escape_script("bundled/bootstrap/bash_init_subshell.sh", assets)
        }
        ShellType::Fish => {
            load_and_escape_script("bundled/bootstrap/fish_init_subshell.sh", assets)
        }
        // TODO(PLAT-750)
        ShellType::PowerShell => todo!(),
    };
    let shell_init_script =
        shell_init_script.replace(SESSION_ID_PLACEHOLDER, &session_id.as_u64().to_string());

    // Combine the environment setup script with the shell-specific init script
    format!("{env_setup_script} {shell_init_script}")
}

/// Returns the init subshell script for an unknown shell which detects the shell type.
///
/// The returned script is one line and has escaped single-quotes for the purposes of being passed
/// as a single-quoted argument to 'eval'.
fn init_subshell_script_for_unknown_shell(
    assets: &dyn AssetProvider,
    session_id: SessionId,
) -> String {
    // Load and escape the shell-specific init script
    load_and_escape_script("bundled/bootstrap/unknown_init_subshell.sh", assets)
        .replace("HOOK_NAME", "InitSubshell")
        .replace(SESSION_ID_PLACEHOLDER, &session_id.as_u64().to_string())
}
