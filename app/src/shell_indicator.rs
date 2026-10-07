use std::convert::TryFrom;

use crate::terminal::ShellLaunchData;
use crate::terminal::shell::ShellType;
use crate::ui_components::icons::Icon;

#[derive(Clone, Copy)]
pub enum ShellIndicatorType {
    Powershell,
    GitBash,
    Ubuntu,
    Debian,
    Kali,
    Arch,
    Linux,
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
            // Docker sandbox containers are Linux regardless of host.
            ShellLaunchData::DockerSandbox { .. } => Ok(Self::Linux),
        }
    }
}
