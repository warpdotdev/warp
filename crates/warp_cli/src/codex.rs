use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Debug, Clone, Args)]
pub struct CodexArgs {
    /// Override the Codex executable used to launch `codex app-server`.
    #[arg(long, global = true, env = "WARP_CODEX_PATH")]
    pub codex_path: Option<PathBuf>,

    #[command(subcommand)]
    pub command: CodexCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum CodexCommand {
    /// Sign in to Codex with a ChatGPT account.
    Login {
        /// Use the device-code flow instead of opening a browser callback flow.
        #[arg(long)]
        device_code: bool,
    },

    /// Show the active Codex account without exposing credentials.
    Status {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,

        /// Ask Codex to refresh the account token before reporting status.
        #[arg(long)]
        refresh: bool,
    },

    /// Sign out of the active Codex account.
    Logout,

    /// Run Codex as a Warp-aware custom coding agent through app-server.
    Chat {
        /// Submit one prompt and exit. Without this flag, an interactive session starts.
        #[arg(long, short = 'p')]
        prompt: Option<String>,

        /// Continue interactively after the initial prompt.
        #[arg(long, requires = "prompt")]
        interactive: bool,

        /// Override the Codex model for this thread.
        #[arg(long)]
        model: Option<String>,

        /// Working directory exposed to the Codex thread.
        #[arg(long)]
        cwd: Option<PathBuf>,

        /// Prevent Codex from modifying files.
        #[arg(long, conflicts_with = "dangerously_bypass_approvals_and_sandbox")]
        read_only: bool,

        /// Disable approvals and grant unrestricted filesystem/network access.
        #[arg(long)]
        dangerously_bypass_approvals_and_sandbox: bool,
    },
}
