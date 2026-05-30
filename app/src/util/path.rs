#[cfg(not(target_family = "wasm"))]
use std::{
    borrow::Cow,
    env,
    ffi::OsStr,
    path::{self, Path, PathBuf},
};

#[cfg(not(target_family = "wasm"))]
use is_executable::IsExecutable as _;
#[cfg(not(target_family = "wasm"))]
use itertools::Itertools as _;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, SingletonEntity};

use crate::remote_server::manager::RemoteServerManager;

/// Fallback label used when a `RemotePath`'s host is not currently tracked.
/// Matches the fallback in `terminal::writeable_pty::remote_server_controller::connection_label_from_user_and_host`.
const UNKNOWN_HOST_LABEL: &str = "Remote host";

/// Returns the display name of a local or remote path, prefixed with the
/// host label for remote paths.
pub fn display_name_with_host(path: &LocalOrRemotePath, ctx: &AppContext) -> String {
    let name = path.display_name();
    match path {
        LocalOrRemotePath::Local(_) => name.to_string(),
        LocalOrRemotePath::Remote(remote) => {
            let host_label = RemoteServerManager::as_ref(ctx)
                .host_label(&remote.host_id)
                .unwrap_or(UNKNOWN_HOST_LABEL);
            format!("{host_label}:{name}")
        }
    }
}

/// Returns the display path of a local or remote path,
/// prefixed with the host label for remote paths.
///
/// When `abbreviate_home` is true, local paths under the user's home directory
/// are abbreviated with a `~/` prefix. The flag is ignored for remote paths,
/// whose home directory lives on a different machine.
pub fn display_path_with_host(
    path: &LocalOrRemotePath,
    abbreviate_home: bool,
    ctx: &AppContext,
) -> String {
    match path {
        LocalOrRemotePath::Local(local_path) => {
            if abbreviate_home {
                dirs::home_dir()
                    .and_then(|home| local_path.strip_prefix(&home).ok())
                    .map(|relative| format!("~/{}", relative.display()))
                    .unwrap_or_else(|| local_path.display().to_string())
            } else {
                path.display_path()
            }
        }
        LocalOrRemotePath::Remote(remote) => {
            let host_label = RemoteServerManager::as_ref(ctx)
                .host_label(&remote.host_id)
                .unwrap_or(UNKNOWN_HOST_LABEL);
            format!("{host_label}:{}", path.display_path())
        }
    }
}

#[cfg(not(target_family = "wasm"))]
pub fn file_exists_and_is_executable(path: &Path) -> bool {
    // We need to check that the file exists, as the `is_executable` crate doesn't validate this on
    // Windows.
    path.is_file() && path.is_executable()
}

/// Resolves `command` into an executable path, matching the shell's search behavior.
/// If the command contains a path separator, it should resolve to an executable
/// file. Otherwise, it should exist in the process's `PATH`.
///
/// Callers that need to resolve against a different PATH (e.g. one
/// captured from the user's interactive login shell) should use
/// [`resolve_executable_in_path`] directly.
#[cfg(not(target_family = "wasm"))]
pub fn resolve_executable(command: &str) -> Option<Cow<'_, Path>> {
    let path_var = env::var_os("PATH").unwrap_or_default();
    resolve_executable_in_path(command, &path_var)
}

/// Like [`resolve_executable`], but resolves PATH-based lookups against
/// the given `path_env` instead of the process's own `PATH`.
///
/// Intended for callers that have a specific PATH to search (e.g. one
/// captured from the user's interactive login shell, matching how
/// MCP/LSP find binaries). Callers that want the process's PATH should
/// use [`resolve_executable`] instead.
#[cfg(not(target_family = "wasm"))]
pub fn resolve_executable_in_path<'a>(command: &'a str, path_env: &OsStr) -> Option<Cow<'a, Path>> {
    if command.contains(path::MAIN_SEPARATOR) {
        let path = Path::new(command);
        return file_exists_and_is_executable(path).then_some(Cow::Borrowed(path));
    }
    for path_dir in env::split_paths(path_env).unique() {
        if let Some(resolved) = resolve_executable_in_dir(&path_dir, command) {
            return Some(Cow::Owned(resolved));
        }
    }
    None
}

#[cfg(not(target_family = "wasm"))]
fn resolve_executable_in_dir(path_dir: &Path, command: &str) -> Option<PathBuf> {
    let resolved = path_dir.join(command);
    if file_exists_and_is_executable(&resolved) {
        return Some(resolved);
    }

    #[cfg(windows)]
    if Path::new(command).extension().is_none() {
        for ext in windows_path_extensions() {
            let resolved = path_dir.join(format!("{command}{ext}"));
            if file_exists_and_is_executable(&resolved) {
                return Some(resolved);
            }
        }
    }

    None
}

#[cfg(windows)]
fn windows_path_extensions() -> impl Iterator<Item = String> {
    env::var_os("PATHEXT")
        .unwrap_or_default()
        .to_string_lossy()
        .split(';')
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .into_iter()
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "path_tests.rs"]
mod tests;
