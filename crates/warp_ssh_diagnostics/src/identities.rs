//! Enumerate visible SSH identities in `~/.ssh`.
//!
//! Surfaces *which* keys ssh can see — used by the diagnostics chip
//! to show the user the set of keys that could potentially be
//! offered, and (in a follow-up phase) to flag mismatches between
//! `IdentityFile` config and what's actually on disk.
//!
//! Deliberately does NOT read the private key file (would require
//! passphrase handling). The check is a `stat`-level "the matching
//! private path exists" — enough to detect "public key without a
//! private key on this machine" which is the most common misconfig.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One identity file pair (`.pub` + matching private key) visible
/// in the SSH dir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshIdentity {
    /// Display name — file stem of the public key (e.g. `id_ed25519`).
    pub name: String,
    /// Absolute path to the public key file.
    pub public_key_path: PathBuf,
    /// `true` when the matching private key file (same stem,
    /// without `.pub`) also exists on disk. UI uses this to flag
    /// "public key only" entries.
    pub has_private_key: bool,
}

/// Read `$HOME/.ssh` for visible identities. Empty list when
/// `$HOME` is unset, the directory doesn't exist, or no `.pub`
/// files are present.
pub fn list_ssh_identities() -> Vec<SshIdentity> {
    let Some(ssh_dir) = ssh_dir_from_env() else {
        return Vec::new();
    };
    list_ssh_identities_in(&ssh_dir)
}

/// Test seam: read identities from a specific directory. Production
/// callers use [`list_ssh_identities`] which delegates here with
/// `$HOME/.ssh`.
pub fn list_ssh_identities_in(ssh_dir: &Path) -> Vec<SshIdentity> {
    let Ok(entries) = std::fs::read_dir(ssh_dir) else {
        return Vec::new();
    };

    let mut identities: Vec<SshIdentity> = entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("pub") {
                return None;
            }
            let stem = path.file_stem()?.to_str()?.to_string();
            if stem.is_empty() {
                return None;
            }
            let private = path.with_extension("");
            Some(SshIdentity {
                name: stem,
                public_key_path: path,
                has_private_key: private.is_file(),
            })
        })
        .collect();

    // Stable order so the UI doesn't reshuffle between refreshes.
    identities.sort_by(|a, b| a.name.cmp(&b.name));
    identities
}

#[cfg(not(target_family = "wasm"))]
fn ssh_dir_from_env() -> Option<PathBuf> {
    // Use $HOME explicitly rather than `dirs::home_dir()` so the
    // test seam below can override it cleanly. The fallback to
    // dirs::home_dir() is unnecessary on the platforms we ship.
    let home = std::env::var("HOME").ok().filter(|s| !s.is_empty())?;
    Some(PathBuf::from(home).join(".ssh"))
}

#[cfg(target_family = "wasm")]
fn ssh_dir_from_env() -> Option<PathBuf> {
    None
}

#[cfg(test)]
#[path = "identities_tests.rs"]
mod tests;
