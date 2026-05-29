use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

// Git 相关设置。目前仅控制 Git Graph 面板的显隐；后续 git 相关参数（如默认分支过滤、
// 连线样式等）也归到这一组，对应设置里独立的 "Git" 分页。
define_settings_group!(GitSettings, settings: [
    // 用户偏好：在 FeatureFlag::GitGraph 对当前渠道开启的前提下，控制 Git Graph 面板是否
    // 出现在左侧 tools panel（与 show_project_explorer / show_global_search 同类的 tab 显隐开关）。
    show_git_graph: ShowGitGraph {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "git.show_graph_panel",
        description: "Whether the Git Graph panel is shown in the tools panel.",
    },
    // Git Graph 在工作目录下探测仓库的层数：0=只看工作目录自身所属仓库；1（默认）=再看每个
    // 第一层子目录是否为仓库；2/3 依次向下。用于"一个目录下挂着多个独立 git 项目"的场景，
    // 发现多个仓库时面板顶部出现仓库下拉以供切换。
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
