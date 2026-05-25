//! Thin binary wrapper for the standalone `warpctrl` executable bundled with Warp.
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = warp_cli::local_control::ControlArgs::from_env();
    warp_cli::local_control::run(args)
}
