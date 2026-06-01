use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

// Git-related settings. For now this only controls the visibility of the Git Graph panel;
// future git-related options (such as default branch filtering or connector style) belong in
// this group too, surfaced through the dedicated "Git" tab in settings.
define_settings_group!(GitSettings, settings: [
    // User preference: when FeatureFlag::GitGraph is enabled for the current channel, controls
    // whether the Git Graph panel appears in the left tools panel (a tab visibility toggle of the
    // same kind as show_project_explorer / show_global_search).
    show_git_graph: ShowGitGraph {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "git.show_graph_panel",
        description: "Whether the Git Graph panel is shown in the tools panel.",
    },
    // How many directory levels below the working directory the Git Graph probes for repositories:
    // 0 = only the repository the working directory itself belongs to; 1 (default) = also check
    // whether each first-level subdirectory is a repository; 2/3 descend further. Useful when a
    // single directory hosts several independent git projects; when multiple repositories are found,
    // a repository dropdown appears at the top of the panel to switch between them.
    git_graph_scan_depth: GitGraphScanDepth {
        type: u32,
        default: 1,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "git.graph_scan_depth",
        description: "How many directory levels below the working directory the Git Graph scans for repositories (0 = working directory only).",
    },
]);
