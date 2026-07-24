use std::process::exit;

use anyhow::Result;
use channel_versions::ParsedVersion;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about)]
struct Args {
    #[arg(long)]
    version_to_roll_out: String,

    #[arg(long)]
    current_version: String,
}

/// Compares two versions and exits with a non-zero exit code if the version to rollout is older than the current version.
/// Used within the `channel-versions` repo to ensure that we always specify the `is_rollback` field when rolling back.
fn main() -> Result<()> {
    let args = Args::parse();

    let version_to_roll_out = args.version_to_roll_out;
    let current_version = args.current_version;

    let parsed_version_to_roll_out = ParsedVersion::try_from(version_to_roll_out.as_str())?;
    let parsed_current_version = ParsedVersion::try_from(current_version.as_str())?;

    match parsed_version_to_roll_out.cmp(&parsed_current_version) {
        std::cmp::Ordering::Less => {
            println!(
                "Current version ({current_version}) is newer than the version to roll out ({version_to_roll_out})"
            );
            exit(1);
        }
        std::cmp::Ordering::Equal => {
            println!(
                "Version to rollout ({version_to_roll_out}) is equal to the current version ({current_version})"
            );
        }
        std::cmp::Ordering::Greater => {
            println!(
                "Version to rollout ({version_to_roll_out}) is newer than the current version ({current_version})"
            );
        }
    }

    Ok(())
}
