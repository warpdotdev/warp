use std::io;

use clap_complete::aot::{Shell, generate};
use warp_core::channel::ChannelState;

use crate::{Args, binary_name};

/// Generate shell completions for the Warp CLI and write them to stdout.
pub fn generate_to_stdout(shell: Option<Shell>) -> anyhow::Result<()> {
    let shell = match shell.or_else(Shell::from_env) {
        Some(s) => s,
        None => anyhow::bail!(
            "Could not determine shell from environment. Please provide a shell argument."
        ),
    };

    let mut cmd = Args::clap_command();
    let bin_name =
        binary_name().unwrap_or_else(|| ChannelState::channel().cli_command_name().to_string());

    generate(shell, &mut cmd, bin_name, &mut io::stdout());
    Ok(())
}
