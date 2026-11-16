//! Integration-testing helpers for the Preview config directory migration.
//!
//! The production entry point (`migrate_preview_config_dir_if_needed`) checks
//! `ChannelState::channel() == Channel::Preview` before doing anything, so it
//! cannot be used directly in integration tests (which run under
//! `Channel::Integration`). This helper exposes the inner path-based migration
//! so tests can drive it with explicit directories.

use std::path::Path;

/// Runs the core symlink-based migration from `old_dir` into `new_dir`.
///
/// Thin wrapper around the internal
/// [`crate::preview_config_migration::migrate_config_dir_via_symlinks`].
pub fn run_config_dir_symlink_migration(old_dir: &Path, new_dir: &Path) {
    crate::preview_config_migration::migrate_config_dir_via_symlinks(old_dir, new_dir);
}
