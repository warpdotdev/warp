//! Loads a per-channel [`ChannelConfig`] for Warp's channel binaries.
//!
//! For non-bundled builds the internal `warp-channel-config` generator is
//! invoked at runtime; for `release_bundle` builds the config is embedded at
//! compile time via the consuming crate's build script. Shared by the GUI app
//! binaries and the `warp_tui` binaries so the loading logic lives in one place.
//!
//! The `release_bundle` cfg inside [`load_config!`] is evaluated in the
//! *consuming* crate, so each binary crate opts into embedding by defining its
//! own `release_bundle` feature (and generating `<channel>_config.json` into its
//! `OUT_DIR` from a build script).
use warp_core::channel::ChannelConfig;

/// The name of the config generator binary, expected to be on PATH.
const CONFIG_BIN_NAME: &str = "warp-channel-config";

#[macro_export]
#[cfg(windows)]
macro_rules! path_concat {
    ($path:expr, $file:expr) => {
        concat!($path, "\\", $file)
    };
}
#[macro_export]
#[cfg(not(windows))]
macro_rules! path_concat {
    ($path:expr, $file:expr) => {
        concat!($path, "/", $file)
    };
}

/// Loads the [`ChannelConfig`] for the given channel name.
///
/// In `release_bundle` builds the config is embedded at compile time (the
/// consuming crate's build script must generate `<channel>_config.json` into
/// `OUT_DIR`); otherwise the `warp-channel-config` generator is invoked at
/// runtime.
#[macro_export]
macro_rules! load_config {
    ($channel:expr) => {{
        #[cfg(feature = "release_bundle")]
        {
            $crate::load_config_from_embedded(include_str!($crate::path_concat!(
                env!("OUT_DIR"),
                concat!($channel, "_config.json")
            )))
        }

        #[cfg(not(feature = "release_bundle"))]
        {
            $crate::load_config_from_generator($channel)
        }
    }};
}

/// Invokes the config generator binary at runtime and deserializes its JSON
/// output into a [`ChannelConfig`].
pub fn load_config_from_generator(channel: &str) -> ChannelConfig {
    let target_family = if cfg!(target_family = "wasm") {
        "wasm"
    } else {
        "native"
    };

    let target_os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };

    let output = command::blocking::Command::new(CONFIG_BIN_NAME)
        .arg("--channel")
        .arg(channel)
        .arg("--target-family")
        .arg(target_family)
        .arg("--target-os")
        .arg(target_os)
        .output()
        .unwrap_or_else(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                panic!(
                    "\n\n'{CONFIG_BIN_NAME}' was not found on PATH.\n\n\
                     To build internal channels, run:\n\
                     \n\
                     \x20 ./script/install_channel_config\n\n"
                )
            }
            panic!("Failed to execute '{CONFIG_BIN_NAME}': {err}")
        });

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("Config generator failed for channel '{channel}':\n{stderr}");
    }

    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "Failed to parse config generator output for channel '{channel}': {err}\nOutput:\n{stdout}"
        )
    })
}

/// Deserializes a [`ChannelConfig`] from a JSON string embedded at compile time.
///
/// Used to load channel configuration in release bundles, where configuration
/// is embedded at compile time instead of being generated at runtime.
pub fn load_config_from_embedded(json: &str) -> ChannelConfig {
    serde_json::from_str(json)
        .unwrap_or_else(|err| panic!("Failed to parse embedded channel config: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards against the pinned `warp-channel-config` revision (see
    /// `script/install_channel_config`) regressing to one that no longer emits
    /// `oz_config.platform_root_url` (APP-5583). `platform_root_url` is
    /// serde-defaulted to `None` for backwards compatibility with older
    /// generator output, so a stale pin fails silently at runtime instead of at
    /// compile time: cloud-run links would fall back to Oz for every viewer
    /// without any test or build failure to flag it.
    ///
    /// Skips (rather than fails) when `warp-channel-config` is not on PATH: it is only
    /// installed via `./script/install_channel_config`, which requires SSH access to a private
    /// repo that is unavailable on the public mirror, fork PRs, and OSS contributor machines.
    /// `.github/actions/prepare_environment`'s macOS and Linux install steps now fail outright
    /// in `warpdotdev/warp-internal` when the SSH key needed for that access is supplied, so a
    /// broken or inaccessible pin still fails a required CI job there instead of this test
    /// silently no-op'ing.
    #[test]
    fn generator_emits_platform_root_url_for_dev_and_stable() {
        if command::blocking::Command::new(CONFIG_BIN_NAME)
            .arg("--help")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_err()
        {
            eprintln!(
                "Skipping: '{CONFIG_BIN_NAME}' not on PATH. Run ./script/install_channel_config \
                 to exercise this test."
            );
            return;
        }

        for channel in ["dev", "stable"] {
            let config = load_config_from_generator(channel);
            assert!(
                config.oz_config.platform_root_url.is_some(),
                "expected '{channel}' channel config to have a platform_root_url, got: {:?}",
                config.oz_config.platform_root_url
            );
        }
    }
}
