use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use command::blocking::Command;
use warp_core::channel::ChannelState;
use warp_util::path::ShellFamily;

/// Compute the target path where the Oz CLI symlink should be installed, based on channel
fn oz_install_target_path() -> PathBuf {
    PathBuf::from("/usr/local/bin").join(ChannelState::channel().cli_command_name())
}

/// Compute the target path where the Warp Control symlink should be installed, based on channel
fn warpctrl_install_target_path() -> PathBuf {
    PathBuf::from("/usr/local/bin").join(ChannelState::channel().warpctrl_command_name())
}

/// Compute the source path of the warpctrl wrapper inside the current app bundle.
///
/// Oz commands are part of the shared executable's normal argument parser, so
/// Oz can symlink directly to the current executable. Warp Control has a
/// separate parser selected by the hidden `--warpctrl` flag, so its installed
/// symlink must target the bundled wrapper that injects that flag. Without it,
/// Warp Control subcommands such as `tab` would reach the normal parser and be
/// rejected as unknown.
fn warpctrl_bundle_source_path() -> Result<PathBuf> {
    let current_binary =
        std::env::current_exe().context("Failed to get current executable path")?;
    let bundle_root = current_binary
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .ok_or_else(|| anyhow!("Current executable is not inside a bundled app"))?;
    Ok(bundle_root
        .join("Contents/Resources/bin")
        .join(ChannelState::channel().warpctrl_command_name()))
}
fn path_resolves_to(path: &Path, expected_path: &Path) -> bool {
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    let Ok(expected_path) = expected_path.canonicalize() else {
        return false;
    };
    path == expected_path
}

/// Whether the installed Warp Control command resolves to this app bundle's wrapper.
pub fn is_warpctrl_installed() -> bool {
    let Ok(source) = warpctrl_bundle_source_path() else {
        return false;
    };
    path_resolves_to(&warpctrl_install_target_path(), &source)
}

/// Create a symlink with elevated privileges using osascript
///
/// This function uses macOS's osascript to prompt for administrator privileges
/// and create a symlink
fn create_symlink_with_admin(source: &Path, target: &Path) -> Result<()> {
    let source_str = source
        .to_str()
        .ok_or_else(|| anyhow!("Source path contains invalid UTF-8: {source:?}"))?;
    let target_str = target
        .to_str()
        .ok_or_else(|| anyhow!("Target path contains invalid UTF-8: {target:?}"))?;

    let escaped_source = ShellFamily::Posix.shell_escape(source_str);
    let escaped_target = ShellFamily::Posix.shell_escape(target_str);

    // Use osascript to run the ln command with admin privileges, with a custom prompt
    let script = format!(
        "do shell script \"ln -sf {escaped_source} {escaped_target}\" with prompt \"Warp needs administrator privileges to install the command in /usr/local/bin.\" with administrator privileges"
    );

    log::debug!("Creating symlink with admin privileges");

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("Failed to execute osascript for admin privileges")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("User canceled") || stderr.contains("cancelled") {
            return Err(anyhow!("Installation cancelled by user."));
        }
        return Err(anyhow!(
            "Failed to create symlink with admin privileges: {stderr}"
        ));
    }

    Ok(())
}

/// Remove a file with elevated privileges using osascript
///
/// This function uses macOS's osascript to prompt for administrator privileges
/// and remove a file, used for CLI uninstallation.
fn remove_file_with_admin(target: &Path) -> Result<()> {
    let target_str = target
        .to_str()
        .ok_or_else(|| anyhow!("Target path contains invalid UTF-8: {target:?}"))?;

    let escaped_target = ShellFamily::Posix.shell_escape(target_str);

    let script = format!(
        "do shell script \"rm {escaped_target}\" with prompt \"Warp needs administrator privileges to uninstall the command from /usr/local/bin.\" with administrator privileges"
    );

    log::debug!("Removing file with admin privileges");

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("Failed to execute osascript for admin privileges")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("User canceled") || stderr.contains("cancelled") {
            return Err(anyhow!("Uninstallation cancelled by user."));
        }
        return Err(anyhow!(
            "Failed to remove file with admin privileges: {stderr}"
        ));
    }

    Ok(())
}

/// Install a channel-specific CLI symlink.
///
/// The target must either be absent or already be a symlink. Installation first
/// attempts to create the symlink without elevated privileges, then falls back
/// to prompting for administrator privileges.
fn install_symlink(source: &Path, target: &Path, command_name: &str) -> Result<()> {
    if target.exists() && !target.is_symlink() {
        return Err(anyhow!(
            "Cannot install {command_name}: {:?} exists but is not a symlink. Please remove it manually first.",
            target
        ));
    }

    match symlink(source, target) {
        Ok(_) => {
            log::debug!(
                "{command_name} installed successfully without admin privileges: {:?} -> {}",
                target,
                source.display()
            );
        }
        Err(_) => {
            log::debug!("{command_name} symlink creation failed, trying with admin privileges");
            create_symlink_with_admin(source, target)
                .context("Failed to create symlink even with admin privileges")?;
            log::debug!("{command_name} installed successfully with admin privileges");
        }
    }

    Ok(())
}

/// Uninstall a channel-specific CLI symlink.
///
/// The target must be a symlink so uninstalling cannot remove an unrelated
/// file. Removal first runs without elevated privileges, then falls back to
/// prompting for administrator privileges.
fn uninstall_symlink(target: &Path, command_name: &str) -> Result<()> {
    if !target.exists() {
        return Err(anyhow!("{command_name} is not currently installed."));
    }

    if !target.is_symlink() {
        return Err(anyhow!(
            "Cannot uninstall {command_name}: {:?} exists but is not a symlink. Please remove it manually.",
            target
        ));
    }

    match fs::remove_file(target) {
        Ok(_) => {
            log::debug!("{command_name} uninstalled successfully without admin privileges");
        }
        Err(_) => {
            log::debug!("{command_name} file removal failed, trying with admin privileges");
            remove_file_with_admin(target)
                .context("Failed to remove symlink even with admin privileges")?;
            log::debug!("{command_name} uninstalled successfully with admin privileges");
        }
    }

    Ok(())
}

/// Install the Oz CLI by symlinking the shared Warp executable into /usr/local/bin.
///
/// The normal argument parser dispatches Oz subcommands directly. It also uses
/// the `oz`-prefixed invocation name to print CLI help rather than launch the
/// GUI when no subcommand is provided.
pub fn install_oz() -> Result<()> {
    let oz_path = oz_install_target_path();
    let current_binary =
        std::env::current_exe().context("Failed to get current executable path")?;
    install_symlink(&current_binary, &oz_path, "Oz CLI")
}

/// Uninstall the Oz CLI by removing the symlink from /usr/local/bin
pub fn uninstall_oz() -> Result<()> {
    uninstall_symlink(&oz_install_target_path(), "Oz command")
}

/// Install Warp Control by symlinking its bundled wrapper into /usr/local/bin.
///
/// The wrapper contains no control implementation. It resolves this installed
/// symlink back into the app bundle, launches the shared Warp executable, and
/// injects `--warpctrl` so startup selects the separate Warp Control parser
/// before normal parsing or GUI startup.
pub fn install_warpctrl() -> Result<()> {
    let warpctrl_path = warpctrl_install_target_path();
    let warpctrl_source = warpctrl_bundle_source_path()?;

    if !warpctrl_source.exists() {
        return Err(anyhow!(
            "Cannot install Warp Control CLI: bundled wrapper not found at {}",
            warpctrl_source.display()
        ));
    }

    install_symlink(&warpctrl_source, &warpctrl_path, "Warp Control CLI")
}

/// Uninstall the Warp Control CLI by removing the symlink from /usr/local/bin
pub fn uninstall_warpctrl() -> Result<()> {
    uninstall_symlink(&warpctrl_install_target_path(), "Warp Control command")
}

#[cfg(test)]
#[path = "cli_install_tests.rs"]
mod tests;
