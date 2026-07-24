use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::{env, path};

use anyhow::{Result, anyhow};
use warpui::{AppContext, SingletonEntity};

use crate::system::SystemInfo;
use crate::util::path::{file_exists_and_is_executable, resolve_executable};

const KASPERSKY_PROCESS_NAME: &str = "avp";
const PWSH_EXE: &str = "pwsh.exe";
const POWERSHELL_EXE: &str = "powershell.exe";
const WSL_EXE: &str = "wsl.exe";

static POWERSHELL_7_PATH: LazyLock<Option<PathBuf>> = LazyLock::new(find_powershell_7_path);
static POWERSHELL_5_PATH: LazyLock<Option<PathBuf>> = LazyLock::new(find_powershell_5_path);
static WSL_PATH: LazyLock<Option<PathBuf>> = LazyLock::new(find_wsl_path);

/// Returns the location which Warp was installed to.
#[cfg(feature = "local_fs")]
pub fn install_dir() -> Result<path::PathBuf> {
    let current_exe = env::current_exe()?;
    current_exe
        .parent()
        .map(ToOwned::to_owned)
        .ok_or(anyhow!("Unable to get install dir"))
}

/// Returns the path to the PowerShell 7 executable on the user's machine, if we
/// were able to find one.
pub fn powershell_7_path() -> Option<&'static PathBuf> {
    POWERSHELL_7_PATH.as_ref()
}

/// Returns the path to the PowerShell 5 executable on the user's machine, if we
/// were able to find one.
pub fn powershell_5_path() -> Option<&'static PathBuf> {
    POWERSHELL_5_PATH.as_ref()
}

/// Returns the path to the a PowerShell 7 or PowerShell 5 executable on the
/// user's machine, if we were able to find one. Prefers PowerShell 7.
pub fn any_powershell_path() -> Option<&'static PathBuf> {
    if let Some(path) = POWERSHELL_7_PATH.as_ref() {
        return Some(path);
    }
    POWERSHELL_5_PATH.as_ref()
}

/// Returns the path to the WSL executable on the user's machine, if we were able
/// to find one.
pub fn wsl_path() -> Option<&'static PathBuf> {
    WSL_PATH.as_ref()
}

/// Searches the user's system for a PowerShell 7 executable and returns the
/// full path to the executable.
fn find_powershell_7_path() -> Option<PathBuf> {
    for install_path in powershell_7_install_paths() {
        let exe_path = install_path.join(PWSH_EXE);
        if file_exists_and_is_executable(&exe_path) {
            return Some(exe_path);
        }
    }

    // Check if the executable is in the PATH.
    let resolved_executable = resolve_executable(PWSH_EXE).map(|path| path.into_owned());
    if resolved_executable.is_some() {
        return resolved_executable;
    }

    log::warn!("Failed to find pwsh.exe on system");
    None
}

/// Searches the user's system for a PowerShell 5 executable and returns the
/// full path to the executable.
fn find_powershell_5_path() -> Option<PathBuf> {
    // Check the default install location.
    let exe_path = powershell_5_install_path().join(POWERSHELL_EXE);
    if file_exists_and_is_executable(&exe_path) {
        return Some(exe_path);
    }

    // Check if the executable is in the PATH.
    let resolved_executable = resolve_executable(POWERSHELL_EXE).map(|path| path.into_owned());
    if resolved_executable.is_some() {
        return resolved_executable;
    }

    log::warn!("Failed to find powershell.exe on system");
    None
}

fn find_wsl_path() -> Option<PathBuf> {
    // Check the default install location.
    let system_root = std::env::var("SYSTEMROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new("C:").join("Windows"));
    let wsl_path = system_root.join("System32").join(WSL_EXE);
    if file_exists_and_is_executable(&wsl_path) {
        return Some(wsl_path);
    }

    // Check if the executable is in the PATH.
    let resolved_executable = resolve_executable(WSL_EXE).map(|path| path.into_owned());
    if resolved_executable.is_some() {
        return resolved_executable;
    }

    None
}

/// Returns the default location where the PowerShell 5 executable
/// is usually installed.
pub fn powershell_5_install_path() -> PathBuf {
    let system_root = std::env::var("SYSTEMROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new("C:").join("Windows"));
    system_root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
}

/// Returns the default locations where the PowerShell 7 executable
/// is usually installed.
///
/// The locations to search have been adapted from Windows Terminal:
/// https://github.com/microsoft/terminal/blob/e1be2f4c73b8a8d55e07a9499a72d7b943ac3fe7/src/cascadia/TerminalSettingsModel/PowershellCoreProfileGenerator.cpp#L264-L286
///
/// Adapted under the MIT License, Copyright (c) Microsoft Corporation.  See app/assets/windows/LICENSE-WINDOWS-TERMINAL.
pub fn powershell_7_install_paths() -> impl Iterator<Item = PathBuf> {
    powershell_7_program_files_paths()
        .chain(dotnet_tools_path())
        .chain(scoop_shims_path())
        .chain(microsoft_store_app_path())
}

fn powershell_7_program_files_paths() -> impl Iterator<Item = PathBuf> {
    let program_files = env::var("PROGRAMFILES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new("C:").join("Program Files"));
    let program_files_paths = find_powershell_7_program_files_paths(program_files);

    // On 64 bit systems, check the "Program Files (x86)" directory.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    let program_files_paths = {
        let program_files_x86 = env::var("PROGRAMFILES(X86)")
            .map(PathBuf::from)
            .unwrap_or_else(|_| Path::new("C:").join("Program Files (x86)"));
        program_files_paths.chain(find_powershell_7_program_files_paths(program_files_x86))
    };

    // On ARM64 systems, check the "Program Files (Arm)" directory.
    #[cfg(target_arch = "aarch64")]
    let program_files_paths = {
        let program_files_arm = env::var("PROGRAMFILES(ARM)")
            .map(PathBuf::from)
            .unwrap_or_else(|_| Path::new("C:").join("Program Files (Arm)"));
        program_files_paths.chain(find_powershell_7_program_files_paths(program_files_arm))
    };

    program_files_paths
}

/// Given a Program Files directory, return the possible install locations for
/// the PowerShell 7 executable within that directory.
fn find_powershell_7_program_files_paths(program_files: PathBuf) -> impl Iterator<Item = PathBuf> {
    // We could be more robust in our search by iterating over all subdirectories
    // of `{directory}/PowerShell`, but this simplifies the logic and it's highly
    // unlikely that users would have versions other than the hardcoded ones here.
    ["7", "7-preview"]
        .into_iter()
        .map(move |version| program_files.join("PowerShell").join(version))
}

fn dotnet_tools_path() -> Option<PathBuf> {
    env::var("USERPROFILE")
        .map(PathBuf::from)
        .map(|user_profile| user_profile.join(".dotnet").join("tools"))
        .ok()
}

fn scoop_shims_path() -> Option<PathBuf> {
    env::var("USERPROFILE")
        .map(PathBuf::from)
        .map(|user_profile| user_profile.join("scoop").join("shims"))
        .ok()
}

fn microsoft_store_app_path() -> Option<PathBuf> {
    let windows_base_dirs = directories::BaseDirs::new()?;
    let mut microsoft_store_app_path = PathBuf::from(windows_base_dirs.data_local_dir());
    microsoft_store_app_path.push("Microsoft");
    microsoft_store_app_path.push("WindowsApps");
    Some(microsoft_store_app_path)
}

/// Determines if Kaspersky is currently running by checking if there is a
/// process with the name "avp" running.
pub fn is_kaspersky_running(ctx: &mut AppContext) -> bool {
    SystemInfo::handle(ctx).update(ctx, |system_info, _| {
        system_info.refresh_all_processes();
        system_info
            .processes_by_name(KASPERSKY_PROCESS_NAME)
            .next()
            .is_some()
    })
}
