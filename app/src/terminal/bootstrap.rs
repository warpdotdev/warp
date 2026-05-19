use std::borrow::Cow;

use itertools::Itertools;
use lazy_static::lazy_static;
use memo_map::MemoMap;
use warpui::{AppContext, AssetProvider, SingletonEntity};

use crate::{
    env_vars::EnvVar,
    terminal::{session_settings::SessionSettings, shell::ShellType},
};

#[cfg(feature = "local_fs")]
use super::{
    model::session::{BootstrapSessionType, SessionInfo},
    warpify::settings::{PIPENV_SUBSHELL_COMMAND_REGEX, POETRY_SUBSHELL_COMMAND_REGEX},
};

lazy_static! {
    /// A memoized cache of the fully-interpolated bootstrap script for each
    /// shell.  We store the full version here as an optimization so that we
    /// don't have to regenerate it every time we spawn a shell.
    static ref BOOTSTRAP_CACHE: MemoMap<ShellType, Vec<u8>> = Default::default();
}

/// This can sometimes appear in the beginning of files. If it gets written into the PTY, it causes
/// errors
const BYTE_ORDER_MARK: &str = "\u{FEFF}";

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
                || shell_type == ShellType::Nushell
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

/// Returns the bootstrap script that should be used when initializing a shell
/// of the given type.
///
/// This supports a very basic form of interpolation:
///
/// ```shell
/// #include bundled/bootstrap/zsh_body.sh
/// ```
///
/// The directive above instructs this function to replace that line with the
/// contents of the file in our asset cache with the path `bundled/bootstrap/zsh_body.sh`.
///
/// At the moment, this interpolation is only performed for the top-level file,
/// and is not performed recursively, but it would be useful to add such support
/// in the future.
pub fn script_for_shell(shell_type: ShellType, assets: &dyn AssetProvider) -> Cow<'static, [u8]> {
    let file = match shell_type {
        ShellType::Bash => "bash.sh",
        ShellType::Zsh => "zsh.sh",
        ShellType::Fish => "fish.sh",
        ShellType::Nushell => "nu.nu",
        ShellType::PowerShell => "pwsh.ps1",
    };

    BOOTSTRAP_CACHE
        .get_or_insert(&shell_type, || {
            let file_path = format!("bundled/bootstrap/{file}");
            let bootstrap = assets
                .get(&file_path)
                .unwrap_or_else(|_| panic!("failed to retrieve {file_path} from assets"));

            // Interpret the file as UTF-8.  We do this in an unchecked way
            // for performance, expecting that any issues here will be caught by
            // unit tests.
            let bootstrap = unsafe { String::from_utf8_unchecked(bootstrap.to_vec()) };

            let additional_files = memo_map::MemoMap::new();

            // Parse through the file, looking for any lines which start with
            // "#include", and replacing that line with the contents of the file
            // located at the path specified.
            //
            // We trim most leading and all trailing whitespace from lines, and
            // drop all empty lines and lines that only contain a comment.  We
            // keep a single leading space on each line, if one exists, to
            // avoid interfering with histignorespace behavior.
            //
            // This minimizes the number of bytes we send over the pty during the
            // bootstrap process.
            fn trim_and_borrow_line(mut line: &str) -> Cow<'_, str> {
                let len = line.len();
                let trimmed_len = line.trim_start().len();
                if trimmed_len < len {
                    let trimmed_chars = len - trimmed_len;
                    line = &line[trimmed_chars - 1..];
                }
                Cow::Borrowed(line.trim_end())
            }
            let mut script = bootstrap
                .trim_start_matches(BYTE_ORDER_MARK)
                .split('\n')
                .map(trim_and_borrow_line)
                .flat_map(|line| {
                    if let Some(path) = line.strip_prefix("#include ") {
                        additional_files
                            .get_or_insert(path, || {
                                let data = assets.get(path).unwrap_or_else(|_| {
                                    panic!("failed to retrieve {path} from assets")
                                });
                                let data_string =
                                    unsafe { String::from_utf8_unchecked(data.to_vec()) };
                                data_string.replace(
                                    "@@USING_CON_PTY_BOOLEAN@@",
                                    &(cfg!(windows).to_string()),
                                )
                            })
                            .split('\n')
                            .map(trim_and_borrow_line)
                            .collect_vec()
                    } else {
                        vec![line]
                    }
                })
                // Filter out empty lines and comments, to minimize the amount
                // of data we send over the pty during the bootstrap process.
                .filter(|line| {
                    let line = line.trim_start();
                    !(line.is_empty()
                        || line.starts_with('#')
                        || shell_type == ShellType::PowerShell
                            && line
                                .starts_with("[Diagnostics.CodeAnalysis.SuppressMessageAttribute"))
                })
                .join("\n");

            // Make sure there's a newline at the end of the bootstrap script,
            // otherwise we'll never submit the final line to the shell.
            script.push('\n');
            script.into_bytes()
        })
        .into()
}

/// Returns the init shell script for the given `shell_type` (e.g. the script that emits the
/// InitShell DCS hook).
///
/// The returned script is one line and, for shells that need it, has escaped single-quotes for the
/// purposes of being passed as a single-quoted argument to 'eval'.
pub fn init_shell_script_for_shell(shell_type: ShellType, assets: &dyn AssetProvider) -> String {
    match shell_type {
        ShellType::Zsh => load_and_escape_script("bundled/bootstrap/zsh_init_shell.sh", assets),
        ShellType::Bash => load_and_escape_script("bundled/bootstrap/bash_init_shell.sh", assets),
        ShellType::Fish => load_and_escape_script("bundled/bootstrap/fish_init_shell.sh", assets),
        ShellType::Nushell => load_script("bundled/bootstrap/nu_init_shell.nu", assets)
            .replace("@@USING_CON_PTY_BOOLEAN@@", &(cfg!(windows).to_string())),
        ShellType::PowerShell => load_script("bundled/bootstrap/pwsh_init_shell.ps1", assets),
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
    ctx: &AppContext,
) -> String {
    match shell_type {
        Some(shell_type) => {
            let subshell_script =
                init_subshell_script_for_shell(shell_type, &crate::ASSETS, vars, ctx);
            format!(r#" [ -z $WARP_BOOTSTRAPPED ] && eval '{subshell_script}'"#)
        }
        None => init_subshell_script_for_unknown_shell(&crate::ASSETS),
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
        // TODO: Add Nushell subshell bootstrap support.
        ShellType::Nushell => todo!(),
        // TODO(PLAT-750)
        ShellType::PowerShell => todo!(),
    };

    // Combine the environment setup script with the shell-specific init script
    format!("{env_setup_script} {shell_init_script}")
}

/// Returns the init subshell script for an unknown shell which detects the shell type.
///
/// The returned script is one line and has escaped single-quotes for the purposes of being passed
/// as a single-quoted argument to 'eval'.
fn init_subshell_script_for_unknown_shell(assets: &dyn AssetProvider) -> String {
    // Load and escape the shell-specific init script
    load_and_escape_script("bundled/bootstrap/unknown_init_subshell.sh", assets)
        .replace("HOOK_NAME", "InitSubshell")
}

/// Returns the raw init shell script for the given `shell_type`, without
/// single-quote escaping. Suitable for passing as an environment variable
/// where the caller controls the eval context (e.g. Docker sandbox init).
///
/// Gated on `unix` because the sole caller today is the Unix Docker
/// sandbox spawn path (`local_tty::unix::prepare_docker_sandbox`); on
/// Windows/wasm the function is dead code.
#[cfg(unix)]
pub fn raw_init_shell_script_for_shell(
    shell_type: ShellType,
    assets: &dyn AssetProvider,
) -> String {
    let file = match shell_type {
        ShellType::Bash => "bundled/bootstrap/bash_init_shell.sh",
        ShellType::Zsh => "bundled/bootstrap/zsh_init_shell.sh",
        ShellType::Fish => "bundled/bootstrap/fish_init_shell.sh",
        ShellType::Nushell => "bundled/bootstrap/nu_init_shell.nu",
        ShellType::PowerShell => "bundled/bootstrap/pwsh_init_shell.ps1",
    };
    load_script(file, assets).replace("@@USING_CON_PTY_BOOLEAN@@", &(cfg!(windows).to_string()))
}

/// Returns the script in the file at `file_path` to be passed as a single-quoted argument in the
/// shell (e.g. as a single quoted argument to `eval`).
///
/// The script is transformed in two ways:
///   * Newlines are stripped and replaced with semi-colons
///   * Single quotes are escaped (' is replaced with '"'"')
///   * Lines starting with '#' are removed -- this enables use of comments in scripts. Note,
///   however, that you still cannot use a 'partial line' comment, since this logic only considers
///   whole lines.
fn load_and_escape_script(file_path: &str, assets: &dyn AssetProvider) -> String {
    load_script(file_path, assets)
        .replace('\'', r#"'"'"'"#)
        .replace("@@USING_CON_PTY_BOOLEAN@@", &(cfg!(windows).to_string()))
}

fn load_script(file_path: &str, assets: &dyn AssetProvider) -> String {
    let script_bytes = assets
        .get(file_path)
        .unwrap_or_else(|_| panic!("Failed to retrieve {file_path} from assets"));

    std::str::from_utf8(&script_bytes)
        .expect("InitShell script should be utf8 encoded.")
        .trim_start_matches(BYTE_ORDER_MARK)
        .lines()
        .filter(|line| !line.trim_start().starts_with('#') && !line.trim().is_empty())
        .join(";")
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
