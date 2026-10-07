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
            && (file_stem.contains("powershell") || file_stem.contains("pwsh"))
        {
            return ShellFamily::PowerShell;
        }
        ShellFamily::Posix
    }
}
