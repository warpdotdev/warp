use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use itertools::Itertools;
use serde::{Deserialize, Serialize};
use warp_cli::{ExternalCommandSpec, ExternalSessionArgs};
#[cfg(feature = "local_tty")]
use warpui::AppContext;
#[cfg(feature = "local_tty")]
use warpui::SingletonEntity as _;
#[cfg(feature = "local_tty")]
use warpui::platform::OperatingSystem;

#[cfg(feature = "local_tty")]
use crate::pane_group::NewTerminalOptions;
#[cfg(feature = "local_tty")]
use crate::root_view::{NewWorkspaceSource, open_new_with_workspace_source};
#[cfg(feature = "local_tty")]
use crate::terminal::ShellLaunchData;
#[cfg(feature = "local_tty")]
use crate::terminal::available_shells::{AvailableShell, AvailableShells};
use crate::terminal::shell::{ShellType, shell_quote_arg};
#[cfg(feature = "local_tty")]
use crate::terminal::view::TerminalView;
#[cfg(feature = "local_tty")]
use crate::uri::get_primary_window;
#[cfg(feature = "local_tty")]
use crate::workspace::{WorkspaceRegistry, active_terminal_in_window};

/// Serialized request to open a Warp terminal session and optionally execute a command inside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalSessionLaunchRequest {
    pub working_directory: Option<PathBuf>,
    pub command: Option<ExternalCommandSpec>,
    pub env_vars: Vec<(OsString, OsString)>,
    pub title: Option<String>,
    pub new_window: bool,
    pub wait_after_command: Option<bool>,
}

impl ExternalSessionLaunchRequest {
    /// Builds a launch request from parsed CLI options and the caller process's initial environment.
    pub fn from_cli_args(args: &ExternalSessionArgs, caller_env: &[(OsString, OsString)]) -> Self {
        let current_dir = std::env::current_dir().ok();
        let working_directory = resolve_launch_working_directory(
            args.working_directory.as_deref(),
            current_dir.as_deref(),
            cfg!(target_os = "macos"),
        );

        Self {
            working_directory,
            command: args.resolved_command_spec(),
            env_vars: sanitize_caller_env_vars(caller_env),
            title: args.title.clone(),
            new_window: args.new_window,
            wait_after_command: args.wait_after_command,
        }
    }
}

pub(crate) fn resolve_launch_working_directory(
    explicit_dir: Option<&Path>,
    current_dir: Option<&Path>,
    is_macos: bool,
) -> Option<PathBuf> {
    if let Some(raw) = explicit_dir {
        let expanded = match raw.to_str() {
            Some(text) => PathBuf::from(shellexpand::tilde(text).into_owned()),
            None => raw.to_path_buf(),
        };

        if expanded.is_relative()
            && let Some(base) = current_dir
        {
            return Some(base.join(expanded));
        }

        return Some(expanded);
    }

    // macOS GUI launches inherit `/` as CWD when spawned without a directory; omit it so Warp uses
    // its default starting directory instead of opening at filesystem root.
    if is_macos && current_dir == Some(Path::new("/")) {
        return None;
    }

    current_dir.map(Path::to_path_buf)
}

/// Returns `false` for ambient terminal or Warp-internal variables that would clobber Warp's own
/// PTY bootstrap (`TERM_PROGRAM`, `WARP_*`, `ZDOTDIR`, `PWD`, etc.) when applied on top of
/// shell defaults.
pub fn should_forward_caller_env_var(key: &OsStr) -> bool {
    let key_str = key.to_string_lossy();
    if key_str.is_empty() {
        return false;
    }

    let upper = key_str.to_ascii_uppercase();
    if upper.starts_with("WARP_") {
        return false;
    }

    !matches!(
        upper.as_str(),
        "TERM"
            | "TERM_PROGRAM"
            | "TERM_PROGRAM_VERSION"
            | "COLORTERM"
            | "TERMINFO"
            | "WINDOWID"
            | "SHELL"
            | "SHLVL"
            | "PWD"
            | "OLDPWD"
            | "_"
            | "ZDOTDIR"
            | "__CFBUNDLEIDENTIFIER"
            | "__CF_USER_TEXT_ENCODING"
            | "XPC_SERVICE_NAME"
            | "XPC_FLAGS"
            | "DESKTOP_STARTUP_ID"
            | "GIO_LAUNCHED_DESKTOP_FILE"
            | "GIO_LAUNCHED_DESKTOP_FILE_PID"
    )
}

/// Filters a caller's captured environment down to variables safe to inject into a new Warp shell.
pub fn sanitize_caller_env_vars(vars: &[(OsString, OsString)]) -> Vec<(OsString, OsString)> {
    vars.iter()
        .filter(|(key, _)| should_forward_caller_env_var(key))
        .cloned()
        .collect()
}

/// Formats an external command specification so it executes safely in `shell_type`.
pub fn format_external_command_for_shell(
    spec: &ExternalCommandSpec,
    shell_type: ShellType,
    wait_after_command: Option<bool>,
) -> Option<String> {
    let formatted = match spec {
        ExternalCommandSpec::Argv(argv) => {
            if argv.is_empty() {
                return None;
            }
            // VS Code on Linux wraps `bash -c` scripts in an extra pair of single quotes (`''...''`);
            // strip them before shell-quoting so bash executes the script rather than treating it as a
            // literal filename.
            let unwrapped_bash_command = (argv.len() == 3
                && (argv[0] == "bash" || argv[0].ends_with("/bash"))
                && argv[1] == "-c"
                && argv[2].len() >= 4
                && argv[2].starts_with("''")
                && argv[2].ends_with("''"))
            .then(|| &argv[2][2..argv[2].len() - 2]);

            let quoted = argv
                .iter()
                .enumerate()
                .map(|(index, arg)| {
                    let effective = if index == 2 {
                        unwrapped_bash_command.unwrap_or(arg.as_str())
                    } else {
                        arg.as_str()
                    };
                    shell_quote_arg(effective, shell_type)
                })
                .join(" ");
            match shell_type {
                ShellType::PowerShell => Some(format!("& {quoted}")),
                ShellType::Bash | ShellType::Zsh | ShellType::Fish => Some(quoted),
            }
        }
        ExternalCommandSpec::WindowsCmdLine(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }
            Some(format!(
                "cmd.exe /c {}",
                shell_quote_arg(trimmed, shell_type)
            ))
        }
    };

    formatted.map(|cmd| {
        if wait_after_command == Some(false) {
            format!("{cmd}; exit")
        } else {
            cmd
        }
    })
}

#[cfg(feature = "local_tty")]
fn resolve_target_shell_type(term: &TerminalView, ctx: &AppContext) -> ShellType {
    if let Some(shell_type) = term.active_session_shell_type(ctx) {
        return shell_type;
    }
    if let Some(shell_type) = term.model.lock().pending_shell_type() {
        return shell_type;
    }
    if let Some(launch_data) = AvailableShells::as_ref(ctx)
        .get_user_preferred_shell(ctx)
        .get_valid_shell_path_and_type()
    {
        match launch_data {
            ShellLaunchData::Executable { shell_type, .. }
            | ShellLaunchData::MSYS2 { shell_type, .. } => return shell_type,
            ShellLaunchData::WSL { .. } | ShellLaunchData::DockerSandbox { .. } => {
                return ShellType::Bash;
            }
        }
    }
    if OperatingSystem::get().is_windows() {
        ShellType::PowerShell
    } else {
        std::env::var("SHELL")
            .ok()
            .as_deref()
            .and_then(ShellType::from_name)
            .unwrap_or(ShellType::Zsh)
    }
}

/// Reuses an idle external-session tab in the primary window (or opens a new tab/window when none
/// can be reused or `request.new_window` is set), applies the caller's working directory and
/// environment variables, and queues any command for execution once shell bootstrap completes.
#[cfg(feature = "local_tty")]
pub(crate) fn open_external_session(request: ExternalSessionLaunchRequest, ctx: &mut AppContext) {
    let initial_directory = request.working_directory.filter(|path| path.is_dir());
    let env_vars: HashMap<OsString, OsString> = request.env_vars.into_iter().collect();
    let is_windows_cmdline = matches!(
        request.command.as_ref(),
        Some(ExternalCommandSpec::WindowsCmdLine(_))
    );
    // `WindowsCmdLine` invokes `cmd.exe /c`, which cannot execute inside WSL/MSYS2/non-PowerShell
    // user-preferred shells; fall back to the Windows system default shell instead.
    let shell = if OperatingSystem::get().is_windows() && is_windows_cmdline {
        AvailableShells::as_ref(ctx)
            .get_user_preferred_shell(ctx)
            .get_valid_shell_path_and_type()
            .filter(|launch_data| {
                !matches!(
                    launch_data,
                    ShellLaunchData::Executable {
                        shell_type: ShellType::PowerShell,
                        ..
                    }
                )
            })
            .map(|_| AvailableShell::default())
    } else {
        None
    };

    let options = NewTerminalOptions {
        shell,
        initial_directory,
        env_vars,
        hide_homepage: true,
        ..Default::default()
    };

    let existing_window_id = if request.new_window {
        None
    } else {
        get_primary_window(ctx.windows().frontmost_window_id(), ctx).filter(|window_id| {
            WorkspaceRegistry::as_ref(ctx)
                .get(*window_id, ctx)
                .is_some()
        })
    };

    let target_window_id = if let Some(window_id) = existing_window_id
        && let Some(workspace) = WorkspaceRegistry::as_ref(ctx).get(window_id, ctx)
    {
        workspace.update(ctx, |workspace, ctx| {
            workspace.open_or_reuse_external_session_tab(options, request.title.clone(), ctx);
        });
        window_id
    } else {
        let (window_id, _) = open_new_with_workspace_source(
            NewWorkspaceSource::Session {
                options: Box::new(options),
                initial_team_uid: None,
            },
            ctx,
        );
        if let Some(workspace) = WorkspaceRegistry::as_ref(ctx).get(window_id, ctx) {
            workspace.update(ctx, |workspace, ctx| {
                workspace.mark_active_tab_as_external_session();
                if let Some(title) = request.title.as_deref() {
                    workspace.set_active_tab_name(title, ctx);
                }
            });
        }
        window_id
    };

    ctx.windows().show_window_and_focus_app(target_window_id);

    if let Some(command_spec) = request.command {
        active_terminal_in_window(target_window_id, ctx, |term, ctx| {
            let shell_type = resolve_target_shell_type(term, ctx);
            if let Some(command) = format_external_command_for_shell(
                &command_spec,
                shell_type,
                request.wait_after_command,
            ) {
                term.set_pending_command(&command, ctx);
            }
        });
    }
}

#[cfg(not(feature = "local_tty"))]
pub(crate) fn open_external_session(
    request: ExternalSessionLaunchRequest,
    ctx: &mut warpui::AppContext,
) {
    let _ = (request, ctx);
    log::warn!("External session launch requires local_tty support");
}

#[cfg(test)]
#[path = "external_session_tests.rs"]
mod tests;
