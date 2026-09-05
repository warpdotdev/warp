use anyhow::Result;
use objc2_foundation::{NSBundle, NSOperatingSystemVersion, NSProcessInfo};

/// Apple Developer Team ID used for code signing and validation.
pub const APPLE_TEAM_ID: &str = "2BBY89MBSN";

/// Get the path to the macOS `.app` bundle.
pub fn get_bundle_path() -> Result<String> {
    let bundle = NSBundle::mainBundle();
    let path = bundle.bundlePath();
    Ok(path.to_string())
}

/// macOS 13.0 (Ventura), the version at which System Preferences was replaced by System
/// Settings. Several `x-apple.systempreferences:` deep-link pane identifiers changed at this
/// boundary (e.g. notifications moved from `com.apple.preference.notifications` to
/// `com.apple.Notifications-Settings.extension`).
const SYSTEM_SETTINGS_MIN_VERSION: NSOperatingSystemVersion = NSOperatingSystemVersion {
    majorVersion: 13,
    minorVersion: 0,
    patchVersion: 0,
};

/// Returns whether the running macOS version uses "System Settings" (Ventura/13.0 and later)
/// rather than the older "System Preferences" (pre-Ventura, back through the project's
/// minimum-supported 10.14).
pub fn is_system_settings_era() -> bool {
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(SYSTEM_SETTINGS_MIN_VERSION)
}
