use clap::{Args, Subcommand};

/// Model-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum ModelCommand {
    /// List available models.
    List,
}

/// Shared CLI args for selecting a base model.
#[derive(Debug, Clone, Args, Default)]
pub struct ModelArgs {
    /// Override the base model used by this command. Use `warp model list` to see available models.
    #[arg(long = "model", value_name = "MODEL_ID")]
    pub model: Option<String>,
}
