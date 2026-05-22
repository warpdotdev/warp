use clap::Parser as _;

fn main() -> anyhow::Result<()> {
    let args = warp_cli::local_control::ControlArgs::parse();
    warp_cli::local_control::run(args)
}
