#[path = "channel_config.rs"]
mod channel_config;

use anyhow::Result;
use warp_core::channel::{Channel, ChannelState};
use warp_core::features;

fn main() -> Result<()> {
    let config = channel_config::load_config!("local");

    let mut state = ChannelState::new(Channel::Local, config)
        .with_additional_features(features::DEBUG_FLAGS)
        .with_additional_features(features::DOGFOOD_FLAGS)
        .with_additional_features(features::PREVIEW_FLAGS)
        .with_additional_features(features::LOCAL_FLAGS);

    // Enable sandbox telemetry feature flag if the env var is set.
    if std::env::var("WITH_SANDBOX_TELEMETRY").is_ok() {
        state = state.with_additional_features(&[features::FeatureFlag::WithSandboxTelemetry]);
    }

    ChannelState::set(state);

    warp::run()
}

// If we're not using an external plist, embed the following as the Info.plist.
#[cfg(all(not(feature = "extern_plist"), target_os = "macos"))]
embed_plist::embed_info_plist_bytes!(r#"
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple Computer//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>English</string>
    <key>CFBundleDisplayName</key>
    <string>WarpLocal</string>
    <key>CFBundleExecutable</key>
    <string>warp</string>
    <key>CFBundleIdentifier</key>
    <string>dev.warp.Warp-Local</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>WarpLocal</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>UIDesignRequiresCompatibility</key>
    <true/>
    <key>CFBundleURLTypes</key>
    <array><dict><key>CFBundleURLName</key><string>Custom App</string><key>CFBundleURLSchemes</key><array><string>warplocal</string></array></dict></array>
    <key>NSHumanReadableCopyright</key>
    <string>© 2026, Denver Technologies, Inc</string>
    </dict>
    </plist>
"#.as_bytes());
