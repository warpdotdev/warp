//! Helper functions for retrieving base paths for storing config/data files.
//!
//! This file should not be directly exposed to or used in integration tests;
//! any paths computed using these functions should be exposed to integration
//! tests through use-case-specific helper functions.
//!
//! `_local_dir` variants of functions are for storing non-portable data, where
//! "portable" refers to the ability to copy that file to another machine.
//! Some examples of non-portable data include things that reference local
//! paths (which may not exist on a different machine), such as paths to shell
//! binaries or user-added theme files.
//!
//! TODO(vorporeal): In general, we should be returning Option<PathBuf> or
//! Result<PathBuf> when we can't compute the home directory instead of
//! returning a relative path.

use std::path::{Path, PathBuf};

use cfg_if::cfg_if;
use directories::BaseDirs;

use crate::{
    channel::{Channel, ChannelState},
    AppId,
};

/// The name of the directory in which to put non-global Warp-specific files.
///
/// This should be used, for example, as the base directory under which
/// repository workflows would be stored (in "./.warp/workflows").
pub const WARP_CONFIG_DIR: &str = ".warp";

/// The name of the folder that stores Warp execution logs and network logs.
/// This is currently only used on Windows to maintain backwards compatibility.
pub const WARP_LOGS_DIR: &str = "logs";

fn base_warp_config_dir_name() -> String {
    match ChannelState::channel() {
        // Preview shares the same directory as Stable for backward
        // compatibility — existing users already have config in `.warp`.
        Channel::Stable | Channel::Preview => WARP_CONFIG_DIR.to_owned(),
        Channel::Oss => format!("{WARP_CONFIG_DIR}-oss"),
        Channel::Dev => format!("{WARP_CONFIG_DIR}-dev"),
        Channel::Integration => format!("{WARP_CONFIG_DIR}-integration"),
        Channel::Local => format!("{WARP_CONFIG_DIR}-local"),
    }
}
/// Returns the home-relative Warp config directory name for the current channel and data profile.
///
/// This preserves the historical `.warp*` directory shape while still isolating dev, local,
/// integration, oss, and optional development profiles.
pub fn warp_home_config_dir_name() -> String {
    let base_dir_name = base_warp_config_dir_name();

    if let Some(data_profile) = ChannelState::data_profile() {
        format!("{base_dir_name}-{data_profile}")
    } else {
        base_dir_name
    }
}

/// Returns the home-relative Warp config directory for the current channel and data profile.
///
/// Unlike [`data_dir`] and [`config_local_dir`] on non-macOS platforms, this intentionally keeps
/// Warp-authored, user-facing config under a `.warp*` directory in the home directory instead of
/// using the platform XDG/AppData project directories.
pub fn warp_home_config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home_dir| home_dir.join(warp_home_config_dir_name()))
}

pub fn warp_home_skills_dir() -> Option<PathBuf> {
    warp_home_config_dir().map(|warp_config_dir| warp_config_dir.join("skills"))
}

pub fn warp_home_mcp_config_file_path() -> Option<PathBuf> {
    warp_home_config_dir().map(|warp_config_dir| warp_config_dir.join(".mcp.json"))
}

/// Returns the macOS config directory name for the current channel.
///
/// Stable uses `.warp`, while other channels include a channel suffix
/// (e.g., `.warp-dev`, `.warp-local`).
///
/// These suffixes are persisted on disk as directory names and must not be
/// changed once established, or existing user data will be orphaned.
#[cfg(target_os = "macos")]
fn macos_config_dir_name() -> String {
    match ChannelState::channel() {
        Channel::Stable => WARP_CONFIG_DIR.to_owned(),
        Channel::Preview => format!("{WARP_CONFIG_DIR}-preview"),
        Channel::Oss => format!("{WARP_CONFIG_DIR}-oss"),
        Channel::Dev => format!("{WARP_CONFIG_DIR}-dev"),
        Channel::Integration => format!("{WARP_CONFIG_DIR}-integration"),
        Channel::Local => format!("{WARP_CONFIG_DIR}-local"),
    }
}

/// Returns the path to the directory where portable user data should be
/// stored.
///
/// This is the appropriate home for things like custom themes and workflows.
pub fn data_dir() -> PathBuf {
    cfg_if! {
        if #[cfg(target_os = "macos")] {
            // TODO(vorporeal): We should do something better than return a
            // relative path.
            dirs::home_dir().unwrap_or_default().join(macos_config_dir_name())
        } else {
            project_dirs().map(|dirs| dirs.data_dir().to_owned()).unwrap_or_default()
        }
    }
}

/// Returns the path to the directory where non-portable configuration files
/// should be stored.
pub fn config_local_dir() -> PathBuf {
    cfg_if! {
        if #[cfg(target_os = "macos")] {
            // TODO(vorporeal): We should do something better than return a
            // relative path.
            dirs::home_dir().unwrap_or_default().join(macos_config_dir_name())
        } else {
            project_dirs()
                .map(|dirs| dirs.config_local_dir().to_owned())
                .unwrap_or_default()
        }
    }
}

/// Returns the base directory for general config files. Useful for accessing the config files for
/// other programs.
pub fn base_config_dir() -> PathBuf {
    BaseDirs::new()
        .map(|dirs| dirs.config_dir().to_owned())
        .unwrap_or_default()
}

/// Returns the path to the directory where non-portable application state data
/// should be stored.
///
/// This is the appropriate home for files like our sqlite database, which
/// contains durable but non-critical and non-portable data like what windows
/// the user had open and cached state of known Warp Drive objects.
pub fn state_dir() -> PathBuf {
    let Some(project_dirs) = project_dirs() else {
        return PathBuf::new();
    };
    // For platforms that don't have a notion of a "state" directory (e.g.:
    // macOS and Windows), fall back to using the data directory.
    project_dirs
        .state_dir()
        .unwrap_or_else(|| project_dirs.data_local_dir())
        .to_owned()
}

/// Returns the path to the secure directory for non-portable application state data.
///
/// Prefer this over [`state_dir`] where possible.
///
/// On macOS, this will use the App Group container directory if available.
pub fn secure_state_dir() -> Option<PathBuf> {
    // Do not use the secure state directory in integration tests, which have a temporary home directory instead.
    if ChannelState::channel() == Channel::Integration {
        return None;
    }

    #[cfg(target_os = "macos")]
    if let Some(app_group_root) = app_group_container_path() {
        // The macOS project_path is the bundle ID (i.e. `dev.warp.Warp-Stable`).
        let project_dirs = project_dirs()?;
        return Some(
            app_group_root
                .join("Library/Application Support")
                .join(project_dirs.project_path()),
        );
    }

    None
}

/// Returns the path to the directory containing the user's custom themes.
pub fn themes_dir() -> PathBuf {
    data_dir().join("themes")
}

/// Returns the path to the directory where files can be stored for caching
/// purposes.
///
/// This is a good place to store things like user profile pictures, which
/// we don't want to fetch on every launch of the app but can be safely
/// deleted by the OS.
pub fn cache_dir() -> PathBuf {
    let Some(project_dirs) = project_dirs() else {
        return PathBuf::new();
    };
    cfg_if! {
        if #[cfg(target_os = "macos")] {
            // TODO(vorporeal): Given that this is just cache data; do we want
            // change the path we use on macOS?
            project_dirs.data_dir().to_owned()
        } else {
            project_dirs.cache_dir().to_owned()
        }
    }
}

/// Returns a display-ready version of the path that is formatted in a
/// home-dir-relative manner, if appropriate.
pub fn home_relative_path(path: &Path) -> String {
    #[cfg(unix)]
    if let Some(base_dirs) = directories::BaseDirs::new() {
        if let Ok(relative_path) = path.strip_prefix(base_dirs.home_dir()) {
            return format!("~/{}", relative_path.display());
        }
    };

    path.display().to_string()
}

/// Returns a [`directories::ProjectDirs`] configured based on the current app ID
/// and the current data profile, if one is set.
///
/// This returns [`None`] if the user's home directory could not be determined.
fn project_dirs() -> Option<directories::ProjectDirs> {
    project_dirs_for_app_id(
        ChannelState::app_id(),
        ChannelState::data_profile().as_deref(),
    )
}

/// Returns a [`directories::ProjectDirs`] configured based on the given app ID
/// and data profile.
///
/// This returns [`None`] if the user's home directory could not be determined.
fn project_dirs_for_app_id(
    app_id: AppId,
    data_profile: Option<&str>,
) -> Option<directories::ProjectDirs> {
    cfg_if::cfg_if! {
        if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            // Adjust the base application name so that we end up with
            // directories like "warp-terminal" and "warp-terminal-dev", to
            // match our Linux package name.
            let base_app_name = match app_id.application_name() {
                "Warp" => "Warp-Terminal".to_owned(),
                "WarpOss" => "Warp-Oss".to_owned(),
                other if other.starts_with("Warp") => other.replace("Warp", "Warp-Terminal-"),
                _ => app_id.application_name().to_owned(),
            };
        } else {
            let base_app_name = app_id.application_name().to_owned();
        }
    }
    let app_name = if let Some(data_profile) = data_profile {
        format!("{base_app_name}-{data_profile}")
    } else {
        base_app_name
    };
    directories::ProjectDirs::from(app_id.qualifier(), app_id.organization(), &app_name)
}

/// Returns the path to the app's secure group container on macOS.
///
/// Returns `None` if the container URL cannot be resolved or converted.
///
/// See:
/// * [Configuring app groups](https://developer.apple.com/documentation/Xcode/configuring-app-groups)
/// * The [App Groups entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.application-groups?language=objc)
/// * [`containerURLForSecurityApplicationGroupIdentifier`](https://developer.apple.com/documentation/foundation/filemanager/containerurl(forsecurityapplicationgroupidentifier:)?language=objc)
#[cfg(target_os = "macos")]
pub fn app_group_container_path() -> Option<PathBuf> {
    use std::sync::LazyLock;
    static CONTAINER_PATH: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
        use objc2_foundation::{NSFileManager, NSString};

        let fm = NSFileManager::defaultManager();
        // Keep in sync with Entitlements.plist
        let group_id = format!("{}.dev.warp", crate::macos::APPLE_TEAM_ID);
        let group_id = NSString::from_str(&group_id);
        // containerURLForSecurityApplicationGroupIdentifier always returns a value on macOS (unlike iOS).
        // We have to double-check that the path points to a directory we can actually use. In addition to
        // macOS returning a path that may not exist, processes may list the container directory without
        // having permissions to read to or write from it.
        if let Some(url) = fm.containerURLForSecurityApplicationGroupIdentifier(&group_id) {
            if let Some(ns_path) = url.path() {
                let path = PathBuf::from(ns_path.to_string());
                if tempfile::tempfile_in(&path).is_ok() {
                    return Some(path);
                }
            }
        }

        None
    });
    LazyLock::force(&CONTAINER_PATH).clone()
}

/// Returns the path to resources included in the Warp distribution.
///
/// Unlike [`warpui::AssetProvider`] assets, which are generally embedded in the binary, these are
/// stored on the filesystem alongside the rest of Warp.
///
/// ## macOS
/// The resources directory is `$APP_DIR/Contents/Resources` (e.g. `/Applications/Warp.app/Contents/Resources`).
///
/// ## Linux
/// The resources directory is `$INSTALL_DIR/resources`, where `$INSTALL_DIR` depends on the
/// specific package manager. For example, on Ubuntu this might be `/opt/warpdotdev/warp-terminal/resources`.
///
/// ## Windows
/// The resources directory is `$INSTALL_DIR/resources`, where `$INSTALL_DIR` is the directory
/// containing the Warp executable (e.g. `C:\Program Files\WarpDev\resources`).
pub fn bundled_resources_dir() -> Option<PathBuf> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            crate::macos::get_bundle_path().ok()
                .map(|bundle_path| {
                    PathBuf::from(bundle_path)
                        .join("Contents")
                        .join("Resources")
                })
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            std::env::current_exe()
                .ok()
                .and_then(|executable| std::fs::canonicalize(executable).ok())
                .and_then(|executable| executable.parent().map(|parent| parent.join("resources")))
        } else if #[cfg(target_os = "windows")] {
            std::env::current_exe()
                .ok()
                .and_then(|executable| std::fs::canonicalize(executable).ok())
                .and_then(|executable| executable.parent().map(|parent| parent.join("resources")))
        } else {
            None
        }
    }
}

#[cfg(all(test, feature = "local_fs"))]
#[path = "paths_tests.rs"]
mod tests;
