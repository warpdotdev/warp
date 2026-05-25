//! Binary entry point for the standalone `warpctrl` CLI.
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = warp_cli::local_control::ControlArgs::from_env();
    warp_cli::local_control::run(args)
}
