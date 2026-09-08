use std::path::Path;

use command::blocking::Command;
use warp_core::safe_warn;

/// Whether opening a local file manager is useful for this process.
///
/// SSH sessions should remain path-only: the file manager would otherwise be
/// opened on the local machine rather than the remote host whose logs were
/// bundled.
pub(crate) fn should_attempt_reveal() -> bool {
    !["SSH_CONNECTION", "SSH_TTY"]
        .iter()
        .any(|name| std::env::var(name).is_ok_and(|value| !value.is_empty()))
}

/// Builds the platform-native file-manager command used to reveal a path.
#[cfg(target_os = "macos")]
pub(crate) fn build_reveal_command(path: &Path) -> Option<Command> {
    let mut command = Command::new("open");
    command.args(["-R"]).arg(path);
    Some(command)
}

/// Builds the platform-native file-manager command used to reveal a path.
#[cfg(target_os = "windows")]
pub(crate) fn build_reveal_command(path: &Path) -> Option<Command> {
    let mut command = Command::new("explorer");
    command.arg(format!("/select,{}", path.display()));
    Some(command)
}

/// Builds the platform-native file-manager command used to reveal a path.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(crate) fn build_reveal_command(path: &Path) -> Option<Command> {
    let parent = path.parent()?;
    let mut command = Command::new("xdg-open");
    command.arg(parent);
    Some(command)
}

/// Returns no reveal command on platforms without a supported file manager.
#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd"
)))]
pub(crate) fn build_reveal_command(_path: &Path) -> Option<Command> {
    None
}

/// Best-effort reveal of a generated bundle in the platform file manager.
pub(crate) fn reveal_path_in_file_manager(path: &Path) {
    if !should_attempt_reveal() {
        return;
    }

    let Some(mut command) = build_reveal_command(path) else {
        return;
    };
    if let Err(error) = command.spawn() {
        safe_warn!(
            safe: ("Failed to reveal generated log bundle in the file manager: {error}"),
            full: ("Failed to reveal generated log bundle {} in the file manager: {error}", path.display())
        );
    }
}

#[cfg(test)]
#[path = "platform_tests.rs"]
mod tests;
