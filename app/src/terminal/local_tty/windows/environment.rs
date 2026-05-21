use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::{collections::BTreeMap, ffi::OsString};

use crate::terminal::cli_agent_sessions::event::current_protocol_version;
use crate::terminal::local_tty::shell::{extra_path_entries, ssh_socket_dir};
use itertools::Itertools;
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use winreg::types::FromRegValue;
use winreg::{
    enums::{RegType, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
    RegKey, RegValue,
};

use crate::safe_info;
use crate::terminal::local_tty::{shell::ShellStarter, PtyOptions};

const HONOR_PS1_NAME: &str = "WARP_HONOR_PS1";
const INITIAL_WORKING_DIR_NAME: &str = "WARP_INITIAL_WORKING_DIR";
const USE_SSH_WRAPPER_NAME: &str = "WARP_USE_SSH_WRAPPER";
const SHELL_DEBUG_MODE_NAME: &str = "WARP_SHELL_DEBUG_MODE";
const TERM_PROGRAM_NAME: &str = "TERM_PROGRAM";
const IS_LOCAL_SESSION_NAME: &str = "WARP_IS_LOCAL_SHELL_SESSION";
const SSH_SOCKET_DIR: &str = "SSH_SOCKET_DIR";
const PATH_APPEND_NAME: &str = "WARP_PATH_APPEND";
const CLIENT_VERSION_NAME: &str = "WARP_CLIENT_VERSION";
const CLI_AGENT_PROTOCOL_VERSION_NAME: &str = "WARP_CLI_AGENT_PROTOCOL_VERSION";
const WSLENV: &str = "WSLENV";
const HISTIGNORE: &str = "HISTIGNORE";

/// Wraps the value of an env var plus its key with preferred casing.
#[derive(Clone, Debug)]
struct EnvEntry {
    preferred_key: OsString,
    value: OsString,
}

pub(super) fn get_shell_environment_variables(options: &PtyOptions) -> Vec<u16> {
    // We map based on lowercase keys to account for case-insensitivity of env vars on Windows.
    let mut env: BTreeMap<OsString, EnvEntry> = std::env::vars_os()
        .map(|(key, value)| {
            (
                map_key(key.clone()),
                EnvEntry {
                    preferred_key: key,
                    value,
                },
            )
        })
        .collect();

    add_local_machine_env(&mut env);
    add_user_env(&mut env);

    env.insert(
        map_key(HONOR_PS1_NAME.into()),
        EnvEntry {
            preferred_key: HONOR_PS1_NAME.into(),
            value: (options.honor_ps1 as usize).to_string().into(),
        },
    );

    if let Some(start_dir) = &options.start_dir {
        env.insert(
            map_key(INITIAL_WORKING_DIR_NAME.into()),
            EnvEntry {
                preferred_key: INITIAL_WORKING_DIR_NAME.into(),
                value: start_dir.as_os_str().to_owned(),
            },
        );
    }
    env.insert(
        map_key(USE_SSH_WRAPPER_NAME.into()),
        EnvEntry {
            preferred_key: USE_SSH_WRAPPER_NAME.into(),
            value: (options.enable_ssh_wrapper as usize).to_string().into(),
        },
    );
    env.insert(
        map_key(SHELL_DEBUG_MODE_NAME.into()),
        EnvEntry {
            preferred_key: SHELL_DEBUG_MODE_NAME.into(),
            value: (options.shell_debug_mode as usize).to_string().into(),
        },
    );

    env.insert(
        map_key(TERM_PROGRAM_NAME.into()),
        EnvEntry {
            preferred_key: TERM_PROGRAM_NAME.into(),
            value: "WarpTerminal".into(),
        },
    );

    env.insert(
        map_key(IS_LOCAL_SESSION_NAME.into()),
        EnvEntry {
            preferred_key: IS_LOCAL_SESSION_NAME.into(),
            value: "1".into(),
        },
    );

    let client_version = ChannelState::app_version().unwrap_or("local");
    env.insert(
        map_key(CLIENT_VERSION_NAME.into()),
        EnvEntry {
            preferred_key: CLIENT_VERSION_NAME.into(),
            value: client_version.into(),
        },
    );

    if FeatureFlag::HOANotifications.is_enabled() {
        env.insert(
            map_key(CLI_AGENT_PROTOCOL_VERSION_NAME.into()),
            EnvEntry {
                preferred_key: CLI_AGENT_PROTOCOL_VERSION_NAME.into(),
                value: current_protocol_version().to_string().into(),
            },
        );
    }

    let ssh_socket_dir = ssh_socket_dir();
    env.insert(
        map_key(SSH_SOCKET_DIR.into()),
        EnvEntry {
            preferred_key: SSH_SOCKET_DIR.into(),
            value: ssh_socket_dir.into(),
        },
    );

    // Set WARP_PATH_APPEND with additional PATH entries to append
    let path_append = extra_path_entries()
        .map(|p| p.to_string_lossy().into_owned())
        .join(";");
    env.insert(
        map_key(PATH_APPEND_NAME.into()),
        EnvEntry {
            preferred_key: PATH_APPEND_NAME.into(),
            value: path_append.into(),
        },
    );

    match &options.shell_starter {
        ShellStarter::MSYS2(_) => {
            // Prevent all commands run before bootstrap from entering history.
            // The bootstrap script for bash unsets this variable.
            env.insert(
                map_key(HISTIGNORE.into()),
                EnvEntry {
                    preferred_key: HISTIGNORE.into(),
                    value: "*".into(),
                },
            );
        }
        ShellStarter::Wsl(_) => {
            // TODO(CORE-3107): Hook this up to a new setting "Working directory for new sessions" setting for WSL.
            let mut wslenv = wsl_env_allowlist(options.start_dir.is_some());
            if let Some(user_val) = env.get(&map_key(WSLENV.into())) {
                wslenv.push(":");
                wslenv.push(&user_val.value);
            }
            env.insert(
                map_key(WSLENV.into()),
                EnvEntry {
                    preferred_key: WSLENV.into(),
                    value: wslenv,
                },
            );
        }
        _ => {}
    }

    // Apply any caller-provided overrides last, so they win.
    for (key, value) in &options.env_vars {
        env.insert(
            map_key(key.clone()),
            EnvEntry {
                preferred_key: key.clone(),
                value: value.clone(),
            },
        );
    }

    environment_block(env.into_iter())
}

/// Build the WSLENV allowlist for variables that Windows should forward into WSL.
///
/// See https://devblogs.microsoft.com/commandline/share-environment-vars-between-wsl-and-windows/
/// for more on how WSLENV should be formatted.
fn wsl_env_allowlist(include_initial_working_dir: bool) -> OsString {
    let mut entries = vec![
        format!("{HONOR_PS1_NAME}/u"),
        format!("{USE_SSH_WRAPPER_NAME}/u"),
        format!("{SHELL_DEBUG_MODE_NAME}/u"),
        format!("{TERM_PROGRAM_NAME}/u"),
        format!("{IS_LOCAL_SESSION_NAME}/u"),
        format!("{SSH_SOCKET_DIR}/u"),
        format!("{CLIENT_VERSION_NAME}/u"),
    ];

    if FeatureFlag::HOANotifications.is_enabled() {
        entries.push(format!("{CLI_AGENT_PROTOCOL_VERSION_NAME}/u"));
    }

    if include_initial_working_dir {
        entries.push(format!("{INITIAL_WORKING_DIR_NAME}/pu"));
    }

    OsString::from(entries.join(":"))
}

/// Merges the local machine and user env var scopes
pub fn get_user_and_system_env_variable(key: &str) -> Option<OsString> {
    let mut env: BTreeMap<OsString, EnvEntry> = BTreeMap::new();
    add_local_machine_env(&mut env);
    add_user_env(&mut env);
    env.get(&map_key(key.into()))
        .map(|entry| entry.value.clone())
}

fn add_local_machine_env(env: &mut BTreeMap<OsString, EnvEntry>) {
    let Ok(sys_env) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("System\\CurrentControlSet\\Control\\Session Manager\\Environment")
    else {
        log::warn!("Unable to fetch SYS env");
        return;
    };

    for (name, value) in sys_env
        .enum_values()
        .filter_map(Result::ok)
        // https://github.com/wez/wezterm/blob/4906789a6d61da58f73b95f89b59c41af60e0f3b/pty/src/cmdbuilder.rs#L143-L145
        .filter(|(name, _)| !name.eq_ignore_ascii_case("username"))
    {
        let Ok(value) = reg_value_to_string(&value, &name) else {
            safe_info!(
                safe: ("Unable to convert value for key {name:?}"),
                full: ("Unable to convert value for key {name:?}: {:?}", value.bytes)
            );
            continue;
        };
        log::trace!("adding SYS env: {name:?} = {value:?}");
        env.insert(
            map_key(name.clone().into()),
            EnvEntry {
                preferred_key: name.into(),
                value,
            },
        );
    }
}

fn add_user_env(env: &mut BTreeMap<OsString, EnvEntry>) {
    let Ok(sys_env) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Environment") else {
        log::warn!("Unable to fetch USER env");
        return;
    };

    for (name, value) in sys_env.enum_values().filter_map(Result::ok) {
        let Ok(value) = reg_value_to_string(&value, &name) else {
            safe_info!(
                safe: ("Unable to convert value for key {name:?}"),
                full: ("Unable to convert value for key {name:?}: {:?}", value.bytes)
            );
            continue;
        };
        // Merge the user path into system instead of overwriting it.
        let value = if name.eq_ignore_ascii_case("path") {
            match env.get(&map_key(name.clone().into())) {
                Some(sys_path) => {
                    let mut result = OsString::new();
                    result.push(&sys_path.value);
                    result.push(";");
                    result.push(&value);
                    result
                }
                None => value,
            }
        } else {
            value
        };

        log::trace!("adding USER env: {name:?} = {value:?}");
        env.insert(
            map_key(name.clone().into()),
            EnvEntry {
                preferred_key: name.into(),
                value,
            },
        );
    }
}

fn reg_value_to_string(value: &RegValue, key: &str) -> anyhow::Result<OsString> {
    let key_lower = key.to_ascii_lowercase();
    // RegType::REG_EXPAND_SZ requires expansion of nested env vars, e.g. %USERPROFILE%\AppData to
    // C:\Users\andy\AppData
    // We also special-case some env vars which must always be expanded, see here:
    // https://github.com/microsoft/terminal/blob/06f736bebe84eda0c34b935a875eebe031a899b7/src/inc/til/env.h#L237-L244
    let should_expand = value.vtype == RegType::REG_EXPAND_SZ
        || key_lower == "path"
        || key_lower == "libpath"
        || key_lower == "os2libpath";
    let os_str = if should_expand {
        let value_str = OsString::from_reg_value(value)?;
        let value_hstr = HSTRING::from(&value_str);

        // We pass None for the output buffer at first b/c we need to allocate it with a certain
        // size. But, we don't know the size we need yet. The length of the expanded string
        // gets returned by ExpandEnvironmentStringsW. So, we call it once to get the size.
        let size = unsafe { ExpandEnvironmentStringsW(PCWSTR(value_hstr.as_ptr()), None) };
        if size == 0 {
            anyhow::bail!("Failed to expand environment string.");
        }

        // Now that we have the size, we call it again with a non-None output buffer to
        // actually get the expanded path.
        let mut out_buffer = vec![0; size as usize];
        unsafe { ExpandEnvironmentStringsW(PCWSTR(value_hstr.as_ptr()), Some(&mut out_buffer)) };

        Ok(OsString::from_wide(&out_buffer))
    } else {
        Ok(OsString::from_reg_value(value)?)
    };

    // These are null-terminated, but we don't want the terminator here. We add it back later.
    os_str.map(|v| v.to_string_lossy().trim_end_matches('\0').into())
}

/// Best-effort lowercase transformation of an OsString.
fn map_key(k: OsString) -> OsString {
    match k.to_str() {
        Some(s) => s.to_lowercase().into(),
        None => k,
    }
}

/// Serialize environment variables into a single string as wide characters.
fn environment_block(env: impl Iterator<Item = (OsString, EnvEntry)>) -> Vec<u16> {
    let mut block = vec![];

    for (_, entry) in env {
        // Environment variable names cannot contain an "=".
        if entry.preferred_key.is_empty() || entry.preferred_key.to_string_lossy().contains('=') {
            log::warn!(
                "Environment variable {:?} was invalid. Not adding to shell process environment block",
                entry.preferred_key
            );
            continue;
        }
        block.extend(entry.preferred_key.encode_wide());
        block.push(b'=' as u16);
        block.extend(entry.value.encode_wide());
        // Each entry is null-terminated.
        block.push(0);
    }
    // The final terminator for CreateProcessW.
    block.push(0);

    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_env_allowlist_includes_client_version_without_notifications_flag() {
        let _guard = FeatureFlag::HOANotifications.override_enabled(false);

        let wslenv = wsl_env_allowlist(false).to_string_lossy().into_owned();

        assert_eq!(
            wslenv.split(':').collect::<Vec<_>>(),
            vec![
                format!("{HONOR_PS1_NAME}/u"),
                format!("{USE_SSH_WRAPPER_NAME}/u"),
                format!("{SHELL_DEBUG_MODE_NAME}/u"),
                format!("{TERM_PROGRAM_NAME}/u"),
                format!("{IS_LOCAL_SESSION_NAME}/u"),
                format!("{SSH_SOCKET_DIR}/u"),
                format!("{CLIENT_VERSION_NAME}/u"),
            ],
        );
    }

    #[test]
    fn wsl_env_allowlist_includes_cli_agent_protocol_when_notifications_flag_is_enabled() {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);

        let wslenv = wsl_env_allowlist(true).to_string_lossy().into_owned();

        assert_eq!(
            wslenv.split(':').collect::<Vec<_>>(),
            vec![
                format!("{HONOR_PS1_NAME}/u"),
                format!("{USE_SSH_WRAPPER_NAME}/u"),
                format!("{SHELL_DEBUG_MODE_NAME}/u"),
                format!("{TERM_PROGRAM_NAME}/u"),
                format!("{IS_LOCAL_SESSION_NAME}/u"),
                format!("{SSH_SOCKET_DIR}/u"),
                format!("{CLIENT_VERSION_NAME}/u"),
                format!("{CLI_AGENT_PROTOCOL_VERSION_NAME}/u"),
                format!("{INITIAL_WORKING_DIR_NAME}/pu"),
            ],
        );
    }
}
