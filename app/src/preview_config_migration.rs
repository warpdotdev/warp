//! One-time migration that gives the Preview channel its own config
//! directory (`~/.warp-preview`) on macOS.
//!
//! Historically, Stable and Preview shared `~/.warp` on macOS. To give
//! Preview its own directory without breaking existing users, this migration
//! symlinks each top-level entry from `~/.warp` into `~/.warp-preview` on
//! first launch, so existing configuration (keybindings, themes, workflows,
//! etc.) remains available to Preview.
//!
//! See `specs/QUALITY-408/` for full context.

use std::path::Path;

use warp_core::channel::{Channel, ChannelState};
use warp_core::paths::{WARP_CONFIG_DIR, data_dir};

/// Files that should not be symlinked during the Preview config directory
/// migration. These are intentionally kept separate between Stable and
/// Preview so each channel has independent settings.
const MIGRATION_EXCLUDED_FILES: &[&str] = &["settings.toml"];

/// Migrates Preview's config directory from the shared `.warp` location to
/// `.warp-preview` by creating symlinks from each top-level entry in `.warp`
/// into the new directory.
///
/// This runs once — on the first launch after the Preview channel is given
/// its own config directory. It is a no-op if:
/// - The channel is not Preview.
/// - `~/.warp-preview` already exists.
/// - `~/.warp` does not exist.
pub(crate) fn migrate_preview_config_dir_if_needed() {
    if ChannelState::channel() != Channel::Preview {
        return;
    }

    let Some(home) = dirs::home_dir() else {
        return;
    };

    let old_dir = home.join(WARP_CONFIG_DIR);
    // `data_dir()` is already channel-aware; for Preview it resolves to
    // `~/.warp-preview`.
    let new_dir = data_dir();

    migrate_config_dir_via_symlinks(&old_dir, &new_dir);
}

/// Core migration logic: creates `new_dir` and populates it with symlinks
/// pointing to each top-level entry in `old_dir`.
///
/// This is a no-op if `new_dir` already exists or `old_dir` does not exist.
/// macOS metadata files (`.DS_Store`, `._*`) and files in
/// [`MIGRATION_EXCLUDED_FILES`] are skipped.
pub(crate) fn migrate_config_dir_via_symlinks(old_dir: &Path, new_dir: &Path) {
    use std::os::unix::fs::symlink;

    // The existence of new_dir is the migration marker — no separate marker
    // file is needed. Once this directory exists (whether created by the
    // migration itself or by ensure_warp_watch_roots_exist on a subsequent
    // launch), this function is a no-op.
    if new_dir.exists() || !old_dir.exists() {
        return;
    }

    // Create the new directory. If this fails because it already exists
    // (race with another process), that's fine.
    if let Err(err) = std::fs::create_dir(new_dir)
        && err.kind() != std::io::ErrorKind::AlreadyExists
    {
        log::warn!(
            "Failed to create config directory {}: {err}",
            new_dir.display()
        );
        return;
    }

    let entries = match std::fs::read_dir(old_dir) {
        Ok(entries) => entries,
        Err(err) => {
            log::warn!(
                "Failed to read old config directory {}: {err}",
                old_dir.display()
            );
            return;
        }
    };

    let mut migrated = 0u32;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip macOS metadata files.
        if name_str == ".DS_Store" || name_str.starts_with("._") {
            continue;
        }

        // Skip files that should remain independent between channels.
        if MIGRATION_EXCLUDED_FILES.contains(&name_str.as_ref()) {
            continue;
        }

        let target = old_dir.join(&name);
        let link = new_dir.join(&name);

        if let Err(err) = symlink(&target, &link) {
            log::warn!(
                "Failed to symlink {} -> {}: {err}",
                link.display(),
                target.display()
            );
        } else {
            migrated += 1;
        }
    }

    log::info!(
        "Migrated config directory: created {migrated} symlinks in {}",
        new_dir.display()
    );
}

#[cfg(test)]
#[path = "preview_config_migration_tests.rs"]
mod tests;
