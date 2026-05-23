use std::path::PathBuf;

/// Shared CLI args for loading command configuration from a file.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct ConfigFileArgs {
    /// Path to a YAML or JSON configuration file.
    #[arg(
        short = 'f',
        long = "file",
        value_name = "PATH",
        env = "WARP_AGENT_CONFIG_FILE"
    )]
    pub file: Option<PathBuf>,
}
