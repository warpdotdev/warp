use std::path::PathBuf;

use clap::{Args, ValueEnum};

/// Options for serving computer-use tools to an external agent harness over MCP.
#[derive(Debug, Clone, Args)]
pub struct ComputerUseMcpArgs {
    /// The model family driving the server, which determines the default screenshot size limits
    /// and how ambiguous key names are interpreted.
    #[arg(long, value_enum, default_value_t = ComputerUseModelFamily::Anthropic)]
    pub model_family: ComputerUseModelFamily,

    /// Overrides the maximum length of a screenshot's longer edge, in pixels.
    #[arg(long)]
    pub max_long_edge_px: Option<u32>,

    /// Overrides the maximum number of pixels in a screenshot.
    #[arg(long)]
    pub max_total_px: Option<u32>,

    /// How long to wait after an action before capturing its screenshot, in milliseconds.
    #[arg(long, default_value_t = 500)]
    pub settle_ms: u64,

    /// The Oz run to attach screen recordings to. Without one, recordings are saved locally.
    #[arg(long)]
    pub run_id: Option<String>,

    /// Where to save screen recordings that aren't attached to an Oz run. Defaults to a directory
    /// under the system's temporary directory.
    #[arg(long)]
    pub recordings_dir: Option<PathBuf>,

    /// Disables the screen recording tools.
    #[arg(long)]
    pub no_recording: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum ComputerUseModelFamily {
    Anthropic,
    #[value(name = "openai")]
    OpenAi,
}
