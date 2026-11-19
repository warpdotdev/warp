//! Settings for the Worktrees tools panel view and for where Warp creates git worktrees.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

/// Directory under which Warp creates new git worktrees for a repository.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Deserialize,
    Eq,
    PartialEq,
    schemars::JsonSchema,
    Serialize,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Where new git worktrees are created.",
    rename_all = "snake_case"
)]
pub enum WorktreeLocation {
    /// `<warp data dir>/worktrees/<repo>/<branch>`.
    #[default]
    #[schemars(description = "Inside Warp's data directory.")]
    WarpDataDirectory,
    /// `<repo parent>/<repo>-worktrees/<branch>`.
    #[schemars(description = "In a sibling directory next to the repository.")]
    NextToRepository,
    /// `<repo>/.worktrees/<branch>`.
    #[schemars(description = "In a .worktrees directory inside the repository.")]
    InsideRepository,
    /// `<custom directory>/<repo>/<branch>`.
    #[schemars(description = "In a custom directory.")]
    CustomDirectory,
}

impl WorktreeLocation {
    pub const ALL: [Self; 4] = [
        Self::WarpDataDirectory,
        Self::NextToRepository,
        Self::InsideRepository,
        Self::CustomDirectory,
    ];

    pub fn as_dropdown_label(self) -> &'static str {
        match self {
            Self::WarpDataDirectory => "Warp data directory",
            Self::NextToRepository => "Next to the repository",
            Self::InsideRepository => "Inside the repository (.worktrees)",
            Self::CustomDirectory => "Custom directory",
        }
    }
}

define_settings_group!(WorktreeSettings, settings: [
    worktree_panel_visible: WorktreePanelVisible {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "worktrees.panel.visible",
        description: "Whether the Worktrees view is shown in the tools panel.",
    },
    worktree_location: WorktreeLocationSetting {
        type: WorktreeLocation,
        default: WorktreeLocation::WarpDataDirectory,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "worktrees.location",
        description: "Where new git worktrees are created.",
    },
    custom_worktree_directory: CustomWorktreeDirectory {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "worktrees.custom_directory",
        description: "Directory used for new worktrees when the location is set to a custom directory. Supports a leading ~.",
    },
    fetch_before_creating_worktree: FetchBeforeCreatingWorktree {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "worktrees.fetch_before_create",
        description: "Whether new worktrees are created from the latest remote version of their base branch, fetched from origin.",
    },
    delete_worktree_on_pr_merge: DeleteWorktreeOnPrMerge {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "worktrees.delete_on_pr_merge",
        description: "Whether a worktree is removed once the pull request for its branch is merged. Worktrees with uncommitted changes are never removed.",
    },
]);

impl WorktreeSettings {
    /// Directory that holds all worktrees Warp creates for the repository at `repo_path`.
    pub fn worktree_repo_dir(&self, repo_path: &Path) -> PathBuf {
        worktree_repo_dir(
            *self.worktree_location,
            self.custom_worktree_directory.as_str(),
            repo_path,
        )
    }
}

/// See [`WorktreeSettings::worktree_repo_dir`]. Falls back to the Warp data directory when a
/// custom location is selected but no directory has been entered.
pub fn worktree_repo_dir(
    location: WorktreeLocation,
    custom_directory: &str,
    repo_path: &Path,
) -> PathBuf {
    let repo_name = repo_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("untitled");
    let warp_data_dir = || {
        warp_core::paths::data_dir()
            .join("worktrees")
            .join(repo_name)
    };
    match location {
        WorktreeLocation::WarpDataDirectory => warp_data_dir(),
        WorktreeLocation::NextToRepository => match repo_path.parent() {
            Some(parent) => parent.join(format!("{repo_name}-worktrees")),
            None => warp_data_dir(),
        },
        WorktreeLocation::InsideRepository => repo_path.join(".worktrees"),
        WorktreeLocation::CustomDirectory => {
            let custom_directory = custom_directory.trim();
            if custom_directory.is_empty() {
                warp_data_dir()
            } else {
                expand_home(custom_directory).join(repo_name)
            }
        }
    }
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match dirs::home_dir() {
            Some(home) => home.join(rest.trim_start_matches('/')),
            None => PathBuf::from(path),
        },
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
#[path = "worktree_tests.rs"]
mod tests;
