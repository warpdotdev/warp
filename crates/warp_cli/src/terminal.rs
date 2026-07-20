use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::share::ShareArgs;

/// Commands for working with headless Warp terminal sessions.
#[derive(Debug, Clone, Subcommand)]
pub enum TerminalCommand {
    /// Start a headless terminal session, share it, and print the join link.
    ///
    /// The command starts a shared terminal session, prints the resulting
    /// session share (join) link to stdout, and blocks until the underlying
    /// session exits.
    Share(TerminalShareArgs),
}

impl TerminalCommand {
    /// Returns the command path used to identify this invocation in tracing.
    pub fn as_str_for_tracing(&self) -> &'static str {
        match self {
            TerminalCommand::Share(_) => "terminal share",
        }
    }
}

/// Arguments for `warp terminal share`.
#[derive(Debug, Clone, Args)]
pub struct TerminalShareArgs {
    /// Who to share the session with (reuses the standard `--share` syntax:
    /// `team[:view|:edit]`, `public[:view|:edit]`, `<user@email.com>[:view|:edit]`).
    #[clap(flatten)]
    pub share: ShareArgs,

    /// Working directory for the shared shell session. Defaults to the current
    /// directory.
    #[arg(long = "working-dir", value_name = "PATH")]
    pub working_dir: Option<PathBuf>,
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
