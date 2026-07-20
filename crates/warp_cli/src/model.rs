use clap::{Args, Subcommand};

/// Model-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum ModelCommand {
    /// List available models.
    List,
}

impl ModelCommand {
    pub(crate) fn as_str_for_tracing(&self) -> &'static str {
        match self {
            ModelCommand::List => "model list",
        }
    }
}

/// Shared CLI args for selecting a base model.
#[derive(Debug, Clone, Args, Default)]
pub struct ModelArgs {
    /// Override the base model used by this command.
    ///
    /// For the default Oz harness, use `oz model list` to see available model IDs.
    ///
    /// For third-party harnesses (`--harness claude` or `--harness codex`), this
    /// sets the harness-specific model. The value is passed directly to the harness
    /// and validated server-side.
    ///
    /// Accepted values for `--harness claude` (Claude Code):
    ///   Aliases: best, fable, opus, sonnet, haiku, opus[1m], sonnet[1m], opusplan
    ///   Pinned:  claude-fable-5, claude-opus-4-8, claude-sonnet-4-6, claude-haiku-4-5
    ///
    /// Accepted values for `--harness codex`:
    ///   gpt-5.5, gpt-5.6-sol, gpt-5.6-terra, gpt-5.6-luna,
    ///   gpt-5.4, gpt-5.4-mini, gpt-5.3-codex, gpt-5.2
    #[arg(long = "model", value_name = "MODEL_ID")]
    pub model: Option<String>,
}
