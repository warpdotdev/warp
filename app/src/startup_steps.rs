//! The startup steps that decide whether this process shows up in the macOS
//! Dock and whether it may mutate the installed application bundle.
//!
//! These live behind one seam instead of as ad-hoc `if` conditions at the call
//! sites so that the guard and the effect it protects cannot drift apart: the
//! effect is only reachable by handing it to the matching `with_*` function,
//! and that function is the only place that decides whether to run it. Tests
//! then assert the *observable* startup behavior — which effects actually run
//! for a given [`LaunchMode`] — rather than the value of a boolean predicate.
//!
//! This matters because the Dock bounce in APP-2946 survived one round of
//! "fixed": the bundled `oz` / `oz-<channel>` CLI wrapper `exec`s the GUI
//! executable from inside `Warp.app`, so a headless launch that performs
//! Dock-visible setup or mutates the app bundle re-introduces the bug.

use anyhow::Result;

use crate::LaunchMode;

/// A platform-visible startup effect governed by this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupEffect {
    /// Mark the process background-only so macOS gives it no Dock tile.
    MarkProcessBackgroundOnly,
    /// Configure the macOS Dock icon, Dock menu, and menu bar.
    ConfigureDockAndMenus,
    /// Delete the executable a previous autoupdate left behind *inside* the
    /// installed application bundle.
    RemoveOldExecutable,
}

/// Whether `effect` must run for `launch_mode`. The single source of truth for
/// all three guards.
fn should_run(effect: StartupEffect, launch_mode: &LaunchMode, autoupdate_enabled: bool) -> bool {
    match effect {
        // Headless launches share the GUI bundle's dockable identity, so they
        // have to opt out of it explicitly.
        StartupEffect::MarkProcessBackgroundOnly => launch_mode.is_headless(),
        // A headless launch has no Dock presence to configure.
        StartupEffect::ConfigureDockAndMenus => !launch_mode.is_headless(),
        // Cleaning up after an autoupdate is GUI-app maintenance: only the
        // launch mode that owns the installed bundle may mutate it.
        StartupEffect::RemoveOldExecutable => autoupdate_enabled && !launch_mode.is_headless(),
    }
}

/// Runs `effect_fn` only when `effect` applies to `launch_mode`. Returns
/// whether the effect ran.
fn run_step(
    effect: StartupEffect,
    launch_mode: &LaunchMode,
    autoupdate_enabled: bool,
    effect_fn: impl FnOnce() -> Result<()>,
) -> Result<bool> {
    if !should_run(effect, launch_mode, autoupdate_enabled) {
        return Ok(false);
    }
    effect_fn()?;
    Ok(true)
}

/// Claims a background-only process type for headless launches, so macOS never
/// gives the process a Dock tile (and never bounces one). Must be called before
/// anything can touch AppKit / Launch Services.
///
/// Returns whether the effect ran.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn with_background_process_setup(
    launch_mode: &LaunchMode,
    mark_background_only: impl FnOnce() -> Result<()>,
) -> Result<bool> {
    run_step(
        StartupEffect::MarkProcessBackgroundOnly,
        launch_mode,
        false,
        mark_background_only,
    )
}

/// Configures the macOS Dock icon, Dock menu, and menu bar for launch modes
/// that have a Dock presence.
///
/// Returns whether the effect ran.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn with_dock_and_menu_setup(
    launch_mode: &LaunchMode,
    configure: impl FnOnce() -> Result<()>,
) -> Result<bool> {
    run_step(
        StartupEffect::ConfigureDockAndMenus,
        launch_mode,
        false,
        configure,
    )
}

/// Removes the executable a previous autoupdate left inside the installed
/// application bundle, for launch modes that own that bundle.
///
/// Returns whether the effect ran.
pub(crate) fn with_old_executable_cleanup(
    launch_mode: &LaunchMode,
    autoupdate_enabled: bool,
    remove_old_executable: impl FnOnce() -> Result<()>,
) -> Result<bool> {
    run_step(
        StartupEffect::RemoveOldExecutable,
        launch_mode,
        autoupdate_enabled,
        remove_old_executable,
    )
}

#[cfg(test)]
#[path = "startup_steps_tests.rs"]
mod tests;
