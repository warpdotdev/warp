use std::convert::TryFrom;

use crate::terminal::shell::ShellType;
use crate::terminal::ShellLaunchData;
use crate::ui_components::icons::Icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellIndicatorType {
    Powershell,
    GitBash,
    Ubuntu,
    Debian,
    Kali,
    Arch,
    Linux,
    DevContainer,
}

impl ShellIndicatorType {
    pub fn to_icon(self) -> Icon {
        match self {
            Self::Powershell => Icon::Powershell,
            Self::GitBash => Icon::GitBash,
            Self::Ubuntu => Icon::Ubuntu,
            Self::Debian => Icon::Debian,
            Self::Kali => Icon::Kali,
            Self::Arch => Icon::Arch,
            Self::Linux => Icon::Linux,
            Self::DevContainer => Icon::Docker,
        }
    }
}

impl TryFrom<&ShellLaunchData> for ShellIndicatorType {
    type Error = ();

    fn try_from(shell_launch_data: &ShellLaunchData) -> Result<Self, Self::Error> {
        match shell_launch_data {
            ShellLaunchData::Executable { shell_type, .. } => match shell_type {
                ShellType::PowerShell => Ok(Self::Powershell),
                _ => Err(()),
            },
            ShellLaunchData::MSYS2 { .. } => Ok(Self::GitBash),
            ShellLaunchData::WSL { distro } => match distro.as_str() {
                s if s.contains("Ubuntu") => Ok(Self::Ubuntu),
                s if s.contains("Debian") => Ok(Self::Debian),
                s if s.contains("kali") => Ok(Self::Kali),
                s if s.contains("arch") => Ok(Self::Arch),
                _ => Ok(Self::Linux),
            },
            // Docker sandbox sessions are Linux regardless of host.
            ShellLaunchData::DockerSandbox { .. } => Ok(Self::Linux),
            ShellLaunchData::DevContainer { .. } => Ok(Self::DevContainer),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn dev_container_uses_docker_indicator() {
        let launch_data = ShellLaunchData::DevContainer {
            devcontainer_cli_path: PathBuf::from("/usr/local/bin/devcontainer"),
            workspace_folder: PathBuf::from("/workspace/project"),
            config_path: PathBuf::from("/workspace/project/.devcontainer/devcontainer.json"),
        };

        let indicator = ShellIndicatorType::try_from(&launch_data).unwrap();
        assert_eq!(indicator, ShellIndicatorType::DevContainer);
        assert_eq!(indicator.to_icon(), Icon::Docker);
    }
}
