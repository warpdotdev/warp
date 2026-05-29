// On Windows, we don't want to display a console window when the application is running in release
// builds. See https://doc.rust-lang.org/reference/runtime.html#the-windows_subsystem-attribute.
#![cfg_attr(feature = "release_bundle", windows_subsystem = "windows")]

#[path = "channel_config.rs"]
mod channel_config;

use anyhow::Result;
use warp_core::channel::{Channel, ChannelState};
use warp_core::features;

// Launches the experimental Warp agent TUI. Uses the dev channel config so the
// agent talks to the same backend as `cargo run --bin dev`.
fn main() -> Result<()> {
    ChannelState::set(
        ChannelState::new(Channel::Dev, channel_config::load_config!("dev"))
            .with_additional_features(features::DEBUG_FLAGS)
            .with_additional_features(features::DOGFOOD_FLAGS)
            .with_additional_features(features::PREVIEW_FLAGS),
    );

    warp::run_tui()
}
