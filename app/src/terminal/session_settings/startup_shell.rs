use serde::{Deserialize, Deserializer, Serialize};

/// A user setting for the shell to start new terminal sessions with.
///
/// Users choose between their login shell, the default versions of zsh/bash/fish
/// (if installed, the first matching executable on their `$PATH`), and a
/// custom path or command.
#[derive(Debug, Clone, Default, PartialEq, Eq, schemars::JsonSchema)]
#[schemars(
    with = "Option<String>",
    description = "Shell to start terminal sessions with. Use null for the system default, or one of \"bash\", \"zsh\", \"fish\", \"pwsh\", or a custom shell command/path."
)]
pub enum StartupShell {
    #[default]
    Default,
    Bash,
    Fish,
    Zsh,
    PowerShell,
    Custom(String),
}

impl StartupShell {
    /// Returns the command for this startup shell.
    pub fn shell_command(&self) -> Option<&str> {
        match self {
            StartupShell::Default => None,
            StartupShell::Bash => Some("bash"),
            StartupShell::Fish => Some("fish"),
            StartupShell::Zsh => Some("zsh"),
            StartupShell::PowerShell => Some("pwsh"),
            StartupShell::Custom(shell) => Some(shell),
        }
    }
}

impl Serialize for StartupShell {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.shell_command().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StartupShell {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: Option<String> = Option::deserialize(deserializer)?;
        Ok(value.into())
    }
}

impl settings_value::SettingsValue for StartupShell {}

impl From<Option<String>> for StartupShell {
    fn from(value: Option<String>) -> Self {
        match value {
            None => StartupShell::Default,
            Some(shell) if shell == "bash" => StartupShell::Bash,
            Some(shell) if shell == "zsh" => StartupShell::Zsh,
            Some(shell) if shell == "fish" => StartupShell::Fish,
            Some(shell) if shell == "pwsh" => StartupShell::PowerShell,
            Some(shell) => StartupShell::Custom(shell),
        }
    }
}
