// On Windows, we don't want to display a console window when the application is running in release
// builds. See https://doc.rust-lang.org/reference/runtime.html#the-windows_subsystem-attribute.
#![cfg_attr(feature = "release_bundle", windows_subsystem = "windows")]

#[path = "channel_config.rs"]
mod channel_config;

use anyhow::Result;
use black_core::channel::{Channel, ChannelState};

// Simple wrapper around black::run() for stable channel builds.
fn main() -> Result<()> {
    ChannelState::set(ChannelState::new(
        Channel::Stable,
        channel_config::load_config!("stable"),
    ));

    black::run()
}
