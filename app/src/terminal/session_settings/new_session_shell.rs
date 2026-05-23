use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use warp_util::path::ShellFamily;
use warpui::platform::OperatingSystem;

#[derive(
    Debug,
    Default,
    Clone,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Shell to use when opening new sessions.",
    rename_all = "snake_case"
)]
pub enum NewSessionShell {
    #[default]
    #[schemars(description = "Use the operating system's default shell.")]
    SystemDefault,
    #[schemars(description = "A shell executable path.")]
    Executable(String),
    #[schemars(description = "An MSYS2 shell environment.")]
    MSYS2(String),
    #[schemars(description = "A Windows Subsystem for Linux distribution.")]
    WSL(String),
    #[schemars(description = "A custom shell command.")]
    Custom(String),
}

impl NewSessionShell {
    pub fn shell_family(&self) -> ShellFamily {
        let shell = match self {
            NewSessionShell::SystemDefault => return OperatingSystem::get().default_shell_family(),
            NewSessionShell::WSL(_) => return ShellFamily::Posix,
            NewSessionShell::Executable(shell) => shell,
            NewSessionShell::MSYS2(shell) => shell,
            NewSessionShell::Custom(shell) => shell,
        };

        let path = PathBuf::from(shell);
        if let Some(file_stem) = path
            .file_stem()
            .and_then(|s| s.to_str().map(|s| s.to_lowercase()))
        {
            if file_stem.contains("powershell") || file_stem.contains("pwsh") {
                return ShellFamily::PowerShell;
            }
        }
        ShellFamily::Posix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These regression tests pin the contract that
    // `AvailableShells::user_preferred_shell_family` relies on: a user who
    // has explicitly picked PowerShell (or a `pwsh`/`powershell` executable
    // anywhere — pwsh on macOS, pwsh on Linux) must get
    // `ShellFamily::PowerShell`, not the OS default. Without this, the
    // worktree-path shell-escape in #11144 would silently use the wrong
    // family on cross-OS shell setups (Andy Carlson's review on #11144).

    #[test]
    fn powershell_executable_picks_powershell_family_regardless_of_os() {
        let cases = [
            "pwsh",
            "pwsh.exe",
            "powershell",
            "powershell.exe",
            "/usr/local/bin/pwsh",
            "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
        ];
        for case in cases {
            let setting = NewSessionShell::Executable(case.to_string());
            assert_eq!(
                setting.shell_family(),
                ShellFamily::PowerShell,
                "{case} should resolve to PowerShell family"
            );
        }
    }

    #[test]
    fn non_powershell_executable_picks_posix_family() {
        let cases = ["bash", "zsh", "fish", "/bin/sh", "/opt/homebrew/bin/zsh"];
        for case in cases {
            let setting = NewSessionShell::Executable(case.to_string());
            assert_eq!(
                setting.shell_family(),
                ShellFamily::Posix,
                "{case} should resolve to Posix family"
            );
        }
    }

    #[test]
    fn wsl_is_always_posix() {
        let setting = NewSessionShell::WSL("Ubuntu-22.04".to_string());
        assert_eq!(setting.shell_family(), ShellFamily::Posix);
    }

    #[test]
    fn custom_pwsh_is_powershell_family() {
        let setting = NewSessionShell::Custom("/opt/microsoft/pwsh/7.4.0/pwsh".to_string());
        assert_eq!(setting.shell_family(), ShellFamily::PowerShell);
    }

    #[test]
    fn msys2_pwsh_is_powershell_family() {
        // MSYS2 lets users wrap a PowerShell binary too; the family pick
        // must still respect the executable name.
        let setting = NewSessionShell::MSYS2("pwsh.exe".to_string());
        assert_eq!(setting.shell_family(), ShellFamily::PowerShell);
    }
}
