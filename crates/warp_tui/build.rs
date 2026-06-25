//! Build script for the `warp_tui` channel binaries.
//!
//! When the `release_bundle` feature is enabled and the internal
//! `warp-channel-config` generator is on PATH, this embeds each channel's config
//! JSON into `OUT_DIR` so the per-channel bins can `include_str!` it via
//! `warp_channel_config::load_config!`. Mirrors the equivalent logic in
//! `app/build.rs`. For non-bundled builds the config is loaded at runtime, so
//! there is nothing to embed.
//!
//! `std::process::Command` is fine in a build script (unlike the shipped binary,
//! where it could flash a console on Windows).
#![allow(clippy::disallowed_types)]

use std::path::Path;
use std::process::Command;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");

    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS must be set");

    generate_channel_config_if_needed(&target_os);
}

/// If the `release_bundle` feature is enabled and `warp-channel-config` is
/// available on PATH, invoke the config generator and write each channel's JSON
/// to `OUT_DIR` so it can be embedded via `include_str!` in the binary entry
/// points.
fn generate_channel_config_if_needed(target_os: &str) {
    if env::var("CARGO_FEATURE_RELEASE_BUNDLE").is_err() {
        // For non-bundled builds, config is loaded at runtime — nothing to embed.
        return;
    }

    let config_bin = "warp-channel-config";

    // If the generator is not on PATH we can't embed configs. This is expected
    // for external contributors building Warp OSS.
    if Command::new(config_bin)
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        return;
    }

    // Only tracked for bundled builds, where they affect the embedded config.
    println!("cargo:rerun-if-env-changed=WITH_LOCAL_SERVER");
    println!("cargo:rerun-if-env-changed=WITH_LOCAL_SESSION_SHARING_SERVER");
    println!("cargo:rerun-if-env-changed=WITH_SANDBOX_TELEMETRY");
    println!("cargo:rerun-if-env-changed=SERVER_ROOT_URL");
    println!("cargo:rerun-if-env-changed=WS_SERVER_URL");

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR must be set");

    // Generate config for every internal channel the TUI ships; each binary's
    // `include_str!` picks up its own file.
    for channel in ["local", "dev", "stable", "preview"] {
        let output = Command::new(config_bin)
            .arg("--channel")
            .arg(channel)
            .arg("--target-family")
            .arg("native")
            .arg("--target-os")
            .arg(target_os)
            .output()
            .unwrap_or_else(|err| {
                panic!("Failed to execute config generator at '{config_bin}': {err}")
            });

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!("Config generator failed for channel '{channel}':\n{stderr}");
        }

        let config_path = Path::new(&out_dir).join(format!("{channel}_config.json"));
        fs::write(&config_path, &output.stdout).unwrap_or_else(|err| {
            panic!("Failed to write config to {}: {err}", config_path.display())
        });
    }
}
