//! Entry point for `warp-tui`, the headless terminal-UI front-end. It is a
//! console application (no GUI window, no app bundle), so unlike the GUI bins it
//! sets no `windows_subsystem` attribute and embeds no `Info.plist`.

use anyhow::Result;
use warp_core::channel::{Channel, ChannelConfig, ChannelState};
use warp_core::{features, AppId};

#[path = "channel_config.rs"]
mod channel_config;

/// Builds the TUI channel config with dev backend endpoints and TUI-local app state.
fn tui_channel_config() -> ChannelConfig {
    let mut config = channel_config::load_config!("dev");
    config.app_id = AppId::new("dev", "warp", "WarpTui");
    config.logfile_name = "warp-tui.log".into();
    if let Some(telemetry_config) = config.telemetry_config.as_mut() {
        telemetry_config.telemetry_file_name = "warp_tui.telemetry".into();
    }
    config
}
/// Parses an optional prompt passed to `warp-tui`.
fn prompt_arg() -> Option<String> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        return None;
    }
    if matches!(args.first().map(String::as_str), Some("--prompt" | "-p")) {
        args.remove(0);
    }
    (!args.is_empty()).then(|| args.join(" "))
}

fn main() -> Result<()> {
    ChannelState::set(
        ChannelState::new(Channel::Dev, tui_channel_config())
            .with_additional_features(features::DEBUG_FLAGS)
            .with_additional_features(features::DOGFOOD_FLAGS)
            .with_additional_features(features::PREVIEW_FLAGS),
    );

    warp::run_tui(prompt_arg())
}
