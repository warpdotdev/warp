use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use channel_versions::{ChannelVersion, ChannelVersions, overrides};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about)]
struct Args {
    /// Name of the operating system to parse the version for.
    #[arg(long, value_enum)]
    target_os: channel_versions::overrides::TargetOS,

    /// The file containing a JSON-serialied [`ChannelVersions`] struct to
    /// apply overrides for.
    file: PathBuf,
}

/// Reads in a JSON-serialized [`ChannelVersions`] from a file, applies any
/// defined overrides that match a given target OS, and prints out the updated
/// JSON (omitting changelogs).
fn main() -> Result<()> {
    let args = Args::parse();

    let contents = fs::read_to_string(&args.file)?;

    // Deserialize the JSON data into the expected format.
    let versions: ChannelVersions = serde_json::from_str(contents.as_str())?;

    let context = overrides::Context {
        target_os: Some(args.target_os),
    };

    let dev_version_info = versions.dev.version_info_for_execution_context(&context);
    let preview_version_info = versions
        .preview
        .version_info_for_execution_context(&context);
    let stable_version_info = versions.stable.version_info_for_execution_context(&context);

    let transformed_versions = ChannelVersions {
        dev: ChannelVersion::new(dev_version_info),
        preview: ChannelVersion::new(preview_version_info),
        stable: ChannelVersion::new(stable_version_info),
        changelogs: None,
    };

    // Print out the transformed version info.
    println!("{}", serde_json::to_string_pretty(&transformed_versions)?);

    Ok(())
}
