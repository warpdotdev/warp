//! Sanitize dynamic-loader environment variables before Warp spawns a user
//! shell, captures the interactive shell environment, or runs a generator
//! command.
//!
//! On Linux, Warp is launched through a wrapper (see `flake.nix` and the
//! release RPM/deb/AppImage launchers) that prepends Warp's own bundled shared
//! libraries to `LD_LIBRARY_PATH`. Those bundled libraries (libstdc++,
//! libgcc_s, libssl/libcrypto, zlib, libcurl, ...) are matched to Warp's binary
//! and are not ABI-compatible with arbitrary programs. Because a shell we spawn
//! inherits our process environment, that `LD_LIBRARY_PATH` leaks into the
//! shell and into every descendant process. A descendant that dynamically links
//! the same libraries — notably a .NET/`pwsh` process spawned by VS Code's
//! "resolve shell environment" step — then loads Warp's incompatible copies
//! instead of the system ones, corrupts its runtime at startup, and aborts with
//! `SIGABRT`. See warp#12228.
//!
//! We fix this by removing Warp's own entries from the loader variables for the
//! shells we spawn, while preserving any values the user set themselves. Two
//! mechanisms are supported, in priority order:
//!
//! 1. **Launcher backup (preferred, packaging-agnostic).** If the launcher
//!    recorded the user's original value in `WARP_ORIGINAL_<VAR>` before
//!    prepending Warp's directory, we restore exactly that value (or unset the
//!    variable when the original was empty) and drop the backup variable so it
//!    never leaks into children.
//! 2. **Install-directory strip (fallback).** Absent a backup, we remove only
//!    the entries that live inside Warp's own install directory (derived from
//!    the running executable), preserving the user's other entries. This covers
//!    the current RPM/deb/AppImage launchers, which bundle their libraries under
//!    the install prefix, without requiring any launcher change.
//!
//! macOS locates Warp's bundled frameworks via rpath baked into the binary
//! rather than an inherited environment variable, so it is unaffected; on macOS
//! these variables are simply absent and every function below is a no-op.
//
// The only callers live behind the `local_tty` feature; when it is disabled
// these helpers are unused in non-test builds, so silence dead_code there
// (the unit tests below still exercise them under default features).
#![cfg_attr(not(feature = "local_tty"), allow(dead_code))]

#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::path::{Path, PathBuf};

/// The dynamic-loader variables we sanitize, each paired with the backup
/// variable a cooperating launcher may use to record the user's original value.
#[cfg(unix)]
const LOADER_VARS: &[(&str, &str)] = &[
    ("LD_LIBRARY_PATH", "WARP_ORIGINAL_LD_LIBRARY_PATH"),
    ("LD_PRELOAD", "WARP_ORIGINAL_LD_PRELOAD"),
];

/// A change to apply to a spawned command's environment for one variable.
#[cfg(unix)]
pub(crate) enum LoaderEnvMutation {
    /// Set the variable to this value.
    Set(String),
    /// Remove the variable entirely (unset it and don't inherit it).
    Remove,
}

/// Compute the loader-env mutations for a shell we are about to spawn, reading
/// the current process environment and Warp's own install directory.
#[cfg(unix)]
pub(crate) fn loader_env_mutations() -> Vec<(&'static str, LoaderEnvMutation)> {
    loader_env_mutations_from(|name| std::env::var_os(name), warp_install_dir().as_deref())
}

/// Testable core of [`loader_env_mutations`]: given an environment lookup and
/// Warp's install directory, compute the mutations to apply.
#[cfg(unix)]
fn loader_env_mutations_from(
    get: impl Fn(&str) -> Option<OsString>,
    warp_dir: Option<&Path>,
) -> Vec<(&'static str, LoaderEnvMutation)> {
    let mut mutations = Vec::new();
    for &(var, backup) in LOADER_VARS {
        if let Some(original) = get(backup) {
            // A launcher recorded the user's original value: restore it exactly
            // (removing the variable when the original was empty/unset) so we
            // drop Warp's prepended entries without having to guess them.
            let original = original.to_string_lossy().into_owned();
            mutations.push((
                var,
                if original.is_empty() {
                    LoaderEnvMutation::Remove
                } else {
                    LoaderEnvMutation::Set(original)
                },
            ));
            // Never leak the backup variable itself into children.
            mutations.push((backup, LoaderEnvMutation::Remove));
            continue;
        }

        // No launcher backup: strip entries inside Warp's install directory,
        // preserving any the user set themselves.
        let Some(current) = get(var) else {
            continue;
        };
        let current = current.to_string_lossy().into_owned();
        let sanitized = strip_warp_entries(var, &current, warp_dir);
        if sanitized != current {
            mutations.push((
                var,
                if sanitized.is_empty() {
                    LoaderEnvMutation::Remove
                } else {
                    LoaderEnvMutation::Set(sanitized)
                },
            ));
        }
    }
    mutations
}

/// Remove entries that live inside Warp's install directory from a loader
/// variable value, preserving order and the user's own entries.
#[cfg(unix)]
fn strip_warp_entries(var: &str, value: &str, warp_dir: Option<&Path>) -> String {
    // `LD_LIBRARY_PATH` is colon-separated; `LD_PRELOAD` accepts colon- or
    // whitespace-separated entries.
    let split_on_whitespace = var == "LD_PRELOAD";
    let components: Vec<&str> = value
        .split(|c: char| c == ':' || (split_on_whitespace && c.is_whitespace()))
        .collect();

    // Only rewrite the value when at least one component is actually Warp-owned.
    // Leaving it untouched otherwise preserves the user's value verbatim —
    // including empty components, which in `LD_LIBRARY_PATH` denote the current
    // directory (`:/foo`, `/foo:`, `::`) and must not be silently dropped.
    if !components
        .iter()
        .any(|entry| entry_is_within_warp(entry, warp_dir))
    {
        return value.to_owned();
    }

    // Drop only the Warp-owned components, keeping every other component
    // (including empty ones) in place. Rejoin with ':', valid for both vars.
    components
        .into_iter()
        .filter(|entry| !entry_is_within_warp(entry, warp_dir))
        .collect::<Vec<_>>()
        .join(":")
}

/// Whether a loader entry points inside Warp's own install directory.
#[cfg(unix)]
fn entry_is_within_warp(entry: &str, warp_dir: Option<&Path>) -> bool {
    let Some(warp_dir) = warp_dir else {
        return false;
    };
    let path = Path::new(entry);
    if path.starts_with(warp_dir) {
        return true;
    }
    // Resolve symlinks / relative entries before comparing, in case the launcher
    // used a non-canonical path. A lookup failure just means "not ours".
    std::fs::canonicalize(path)
        .map(|resolved| resolved.starts_with(warp_dir))
        .unwrap_or(false)
}

/// Warp's install directory: the canonicalized directory containing the running
/// executable, whose bundled libraries the Linux launcher prepends to
/// `LD_LIBRARY_PATH`.
#[cfg(unix)]
fn warp_install_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    exe.parent().map(Path::to_path_buf)
}

#[cfg(all(test, unix))]
#[path = "loader_env_tests.rs"]
mod tests;
