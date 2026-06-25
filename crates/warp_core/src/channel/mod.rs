mod config;
mod state;

use std::fmt;

pub use config::*;
pub use state::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// The official/first-party stable release.
    Stable,
    /// The official/first-party feature preview release.
    Preview,

    /// The internal-only nightly build.
    Dev,
    /// The internal-only HEAD build.
    Local,

    /// The open-source Zerp build.
    Oss,

    /// The integration test build.
    Integration,
}

impl Channel {
    /// Whether or not this channel is for internal use only
    pub fn is_dogfood(&self) -> bool {
        match self {
            Channel::Dev | Channel::Local => true,
            Channel::Stable | Channel::Preview | Channel::Integration | Channel::Oss => false,
        }
    }

    /// Whether this channel honors the `--server-root-url` / `--ws-server-url` /
    /// `--session-sharing-server-url` flags (and their `WARP_*` env-var equivalents).
    ///
    /// Release channels (`Stable`, `Preview`, `Oss`) ignore these overrides so shipped
    /// builds can't be redirected away from their baked-in server URLs. Internal-only channels
    /// (`Dev`, `Local`, `Integration`) continue to honor them for local development and testing.
    pub fn allows_server_url_overrides(&self) -> bool {
        match self {
            Channel::Dev | Channel::Local | Channel::Integration => true,
            Channel::Stable | Channel::Preview | Channel::Oss => false,
        }
    }

    /// Returns the CLI command name corresponding to this channel.
    pub fn cli_command_name(&self) -> &'static str {
        match self {
            Channel::Stable => "zerp-cli",
            Channel::Dev => "zerp-cli-dev",
            Channel::Preview => "zerp-cli-preview",
            Channel::Local => "zerp-cli-local",
            Channel::Integration => "zerp-cli-integration",
            Channel::Oss => "zerp-cli",
        }
    }

    /// Returns the Warp Control CLI command name corresponding to this channel.
    pub fn warpctrl_command_name(&self) -> &'static str {
        match self {
            Channel::Stable => "zerpctrl",
            Channel::Dev => "zerpctrl-dev",
            Channel::Preview => "zerpctrl-preview",
            Channel::Local => "zerpctrl-local",
            Channel::Integration => "zerpctrl-integration",
            Channel::Oss => "zerpctrl",
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Channel::Stable => "stable",
            Channel::Preview => "preview",
            Channel::Dev => "dev",
            Channel::Integration => "integration",
            Channel::Local => "local",
            Channel::Oss => "zerp",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Channel;

    #[test]
    fn cli_command_names_use_zerp_cli_branding() {
        assert_eq!(Channel::Stable.cli_command_name(), "zerp-cli");
        assert_eq!(Channel::Dev.cli_command_name(), "zerp-cli-dev");
        assert_eq!(Channel::Preview.cli_command_name(), "zerp-cli-preview");
        assert_eq!(Channel::Local.cli_command_name(), "zerp-cli-local");
        assert_eq!(
            Channel::Integration.cli_command_name(),
            "zerp-cli-integration"
        );
        assert_eq!(Channel::Oss.cli_command_name(), "zerp-cli");
    }
}
