use std::collections::HashMap;
use std::path::Path;

use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};
use warp_core::ui::theme::AnsiColorIdentifier;

use super::project_priorities::ProjectPriorities;

#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Where new tabs are placed in the tab bar.",
    rename_all = "snake_case"
)]
pub enum NewTabPlacement {
    #[default]
    AfterCurrentTab,
    AfterAllTabs,
}

settings::macros::implement_setting_for_enum!(
    NewTabPlacement,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Never,
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "general.new_tab_placement",
    description: "Where new tabs are placed in the tab bar.",
);

#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Position of the close button on tabs.",
    rename_all = "snake_case"
)]
pub enum TabCloseButtonPosition {
    #[default]
    Right,
    Left,
}

settings::macros::implement_setting_for_enum!(
    TabCloseButtonPosition,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.tabs.tab_close_button_position",
    description: "Position of the close button on tabs.",
);

/// Visibility options for workspace decorations like the tab bar.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "When workspace decorations such as the tab bar are visible.",
    rename_all = "snake_case"
)]
pub enum WorkspaceDecorationVisibility {
    /// Always show workspace decorations.
    AlwaysShow,
    /// Hide workspace decorations if fullscreen.
    #[default]
    HideFullscreen,
    /// Only show workspace decorations on hover.
    OnHover,
}

settings::macros::implement_setting_for_enum!(
    WorkspaceDecorationVisibility,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.tabs.workspace_decoration_visibility",
    description: "When workspace decorations such as the tab bar are visible.",
);

impl WorkspaceDecorationVisibility {
    /// Choose a visibility setting that's logically opposite from this one.
    pub fn toggled(self) -> Self {
        // If we add other variants, there should still be logical opposites for each. For example,
        // toggling from any form of hidden workspace decorations should re-enable them.
        match self {
            WorkspaceDecorationVisibility::AlwaysShow => WorkspaceDecorationVisibility::OnHover,
            WorkspaceDecorationVisibility::OnHover => WorkspaceDecorationVisibility::HideFullscreen,
            WorkspaceDecorationVisibility::HideFullscreen => WorkspaceDecorationVisibility::OnHover,
        }
    }

    /// True if this is a setting where workspace decorations are hidden by default.
    pub fn hides_decorations_by_default(self) -> bool {
        matches!(self, WorkspaceDecorationVisibility::OnHover,)
    }

    /// True if *window* decorations should be shown.
    pub fn show_window_decorations(self) -> bool {
        !matches!(self, WorkspaceDecorationVisibility::OnHover)
    }
}

/// Represents the color state for a directory entry in the tab-color settings.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Color assignment state for a directory's tab.",
    rename_all = "snake_case"
)]
pub enum DirectoryTabColor {
    /// User explicitly removed this directory. Retained for backwards compatibility with settings files written by older versions.
    #[schemars(description = "The directory was explicitly removed from tab coloring.")]
    Suppressed,
    /// Directory is tracked but has no assigned color.
    #[schemars(description = "The directory is tracked but has no assigned color.")]
    Unassigned,
    /// Directory is tracked with a specific color.
    #[schemars(description = "The directory is assigned a specific color.")]
    Color(AnsiColorIdentifier),
}

impl DirectoryTabColor {
    pub(crate) fn ansi_color(self) -> Option<AnsiColorIdentifier> {
        match self {
            DirectoryTabColor::Color(c) => Some(c),
            DirectoryTabColor::Suppressed | DirectoryTabColor::Unassigned => None,
        }
    }
}

/// User-configured directory→color mappings for tab coloring.
///
/// Keys are directory paths (as strings). Values indicate the color state:
/// - `Suppressed`: directory was explicitly removed by the user via the per-row X button.
///   Retained so `color_for_directory` can shadow broader prefix matches, and for
///   backwards compatibility with settings files written by older versions.
/// - `Unassigned`: directory is tracked but has no specific color.
/// - `Color(c)`: directory is tracked with the given color.
#[derive(
    Default,
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Mapping of directory paths to their tab color assignments.")]
pub struct DirectoryTabColors(pub(crate) HashMap<String, DirectoryTabColor>);

settings::macros::implement_setting_for_enum!(
    DirectoryTabColors,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Never,
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.tabs.directory_tab_colors",
    max_table_depth: 0,
    description: "Mapping of directory paths to their tab color assignments.",
    feature_flag: warp_core::features::FeatureFlag::DirectoryTabColors,
);

impl DirectoryTabColors {
    /// Returns the configured tab color for a directory using longest-prefix matching.
    /// Returns `None` if no configured directory is a prefix of `dir`.
    pub fn color_for_directory(&self, canonical_dir: &Path) -> Option<DirectoryTabColor> {
        self.0
            .iter()
            .filter_map(|(configured_path, color)| {
                let configured = Path::new(configured_path);
                match color {
                    DirectoryTabColor::Suppressed => None,
                    _ => canonical_dir
                        .starts_with(configured)
                        .then_some((configured, *color)),
                }
            })
            .max_by_key(|(configured, _)| configured.as_os_str().len())
            .map(|(_, color)| color)
    }

    /// Returns a new value with the given directory's color updated.
    pub fn with_color(&self, path: &Path, color: DirectoryTabColor) -> Self {
        let mut map = self.0.clone();
        map.insert(canonical_directory_key(path), color);
        Self(map)
    }
}

settings::macros::implement_setting_for_enum!(
    ProjectPriorities,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Never,
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.project_layout.project_priorities",
    description: "Ordered list of prioritized projects, highest priority first.",
    feature_flag: warp_core::features::FeatureFlag::Projects,
);

/// Canonicalizes `path` into the string key used in [`DirectoryTabColors`].
pub fn canonical_directory_key(path: &Path) -> String {
    dunce::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

#[derive(
    Clone,
    Debug,
    Default,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Configuration for the header toolbar chips in the vertical tab panel header.",
    rename_all = "snake_case"
)]
pub enum HeaderToolbarChipSelection {
    #[default]
    Default,
    Custom {
        left: Vec<super::header_toolbar_item::HeaderToolbarItemKind>,
        right: Vec<super::header_toolbar_item::HeaderToolbarItemKind>,
    },
}

impl HeaderToolbarChipSelection {
    pub fn left_items(&self) -> Vec<super::header_toolbar_item::HeaderToolbarItemKind> {
        use super::header_toolbar_item::HeaderToolbarItemKind;
        match self {
            Self::Default => HeaderToolbarItemKind::default_left(),
            Self::Custom { left, .. } => left.clone(),
        }
    }

    pub fn right_items(&self) -> Vec<super::header_toolbar_item::HeaderToolbarItemKind> {
        use super::header_toolbar_item::HeaderToolbarItemKind;
        match self {
            Self::Default => HeaderToolbarItemKind::default_right(),
            Self::Custom { right, .. } => right.clone(),
        }
    }

    pub fn contains_item(&self, item: &super::header_toolbar_item::HeaderToolbarItemKind) -> bool {
        self.left_items().contains(item) || self.right_items().contains(item)
    }
}

settings::macros::implement_setting_for_enum!(
    HeaderToolbarChipSelection,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.tabs.header_toolbar_chip_selection",
    description: "Configuration for the header toolbar chips in the vertical tab panel header.",
);

#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Display mode for the vertical tab bar.",
    rename_all = "snake_case"
)]
pub enum VerticalTabsViewMode {
    #[default]
    Compact,
    Expanded,
}

settings::macros::implement_setting_for_enum!(
    VerticalTabsViewMode,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.vertical_tabs.view_mode",
    description: "Display mode for the vertical tab bar.",
);

#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Granularity of rows displayed in the vertical tabs panel.",
    rename_all = "snake_case"
)]
pub enum VerticalTabsDisplayGranularity {
    #[default]
    Panes,
    Tabs,
}

settings::macros::implement_setting_for_enum!(
    VerticalTabsDisplayGranularity,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.vertical_tabs.display_granularity",
    description: "Granularity of rows displayed in the vertical tabs panel.",
);

#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Tab item display mode in vertical tabs.",
    rename_all = "snake_case"
)]
pub enum VerticalTabsTabItemMode {
    #[default]
    FocusedSession,
    Summary,
}

settings::macros::implement_setting_for_enum!(
    VerticalTabsTabItemMode,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.vertical_tabs.tab_item_mode",
    description: "Tab item display mode in vertical tabs.",
);

#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Primary information displayed on vertical tabs.",
    rename_all = "snake_case"
)]
pub enum VerticalTabsPrimaryInfo {
    #[default]
    Command,
    WorkingDirectory,
    Branch,
}

/// How many lines of information a horizontal tab shows.
///
/// Mirrors `VerticalTabsViewMode` for the horizontal bar. `SingleLine` is the
/// historical look; `TwoLine` adds a smaller secondary line so a tab can show
/// both what the agent is called and what it is doing.
#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "How many lines of information tabs show.",
    rename_all = "snake_case"
)]
pub enum TabLineCount {
    #[default]
    SingleLine,
    TwoLine,
}

settings::macros::implement_setting_for_enum!(
    TabLineCount,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.tabs.line_count",
    description: "How many lines of information tabs show.",
);

/// The primary (first line) information shown on a tab.
///
/// The horizontal counterpart of `VerticalTabsPrimaryInfo`, with the extra
/// `AgentSession` option: the name of the agent running in the tab, such as a
/// renamed Claude Code session.
#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Primary information displayed on tabs.",
    rename_all = "snake_case"
)]
pub enum TabPrimaryInfo {
    #[default]
    AgentSession,
    /// The latest instruction the user gave the agent.
    UserInstruction,
    Command,
    WorkingDirectory,
    Branch,
}

settings::macros::implement_setting_for_enum!(
    TabPrimaryInfo,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.tabs.primary_info",
    description: "The primary information displayed on tabs.",
);

/// What each task row in the project rail shows.
///
/// Deliberately the same vocabulary as the tab-line settings so the rail is a
/// third view of one idea rather than a competing one; it resolves through the
/// same `tab_info_text` helper.
#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Information shown on each task row in the project rail.",
    rename_all = "snake_case"
)]
pub enum RailTaskInfo {
    #[default]
    AgentSession,
    UserInstruction,
    Command,
    WorkingDirectory,
    Branch,
}

settings::macros::implement_setting_for_enum!(
    RailTaskInfo,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.tabs.rail_task_info",
    description: "Information shown on each task row in the project rail.",
);

/// The secondary (second line) information shown on a tab, used only when
/// [`TabLineCount::TwoLine`] is active.
#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Secondary information displayed on two-line tabs.",
    rename_all = "snake_case"
)]
pub enum TabSecondaryInfo {
    #[default]
    Command,
    /// The latest instruction the user gave the agent. Pairs with an
    /// `AgentSession` primary to show "what it's called" over "what I asked".
    UserInstruction,
    WorkingDirectory,
    Branch,
    AgentSession,
}

settings::macros::implement_setting_for_enum!(
    TabSecondaryInfo,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.tabs.secondary_info",
    description: "The secondary information displayed on two-line tabs.",
);

impl TabSecondaryInfo {
    /// The secondary line to actually use, given the primary line.
    ///
    /// Showing the same value twice is never useful, so a secondary choice that
    /// collides with the primary falls back to a sensible alternative — the
    /// same conflict-avoidance the vertical tabs apply via
    /// `resolve_compact_subtitle`.
    pub fn resolved_for(self, primary: TabPrimaryInfo) -> Self {
        let conflicts = matches!(
            (primary, self),
            (TabPrimaryInfo::AgentSession, Self::AgentSession)
                | (TabPrimaryInfo::UserInstruction, Self::UserInstruction)
                | (TabPrimaryInfo::Command, Self::Command)
                | (TabPrimaryInfo::WorkingDirectory, Self::WorkingDirectory)
                | (TabPrimaryInfo::Branch, Self::Branch)
        );
        if !conflicts {
            return self;
        }
        match primary {
            TabPrimaryInfo::AgentSession => Self::UserInstruction,
            TabPrimaryInfo::UserInstruction => Self::Command,
            TabPrimaryInfo::Command => Self::WorkingDirectory,
            TabPrimaryInfo::WorkingDirectory => Self::Command,
            TabPrimaryInfo::Branch => Self::Command,
        }
    }
}

settings::macros::implement_setting_for_enum!(
    VerticalTabsPrimaryInfo,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.vertical_tabs.primary_info",
    description: "The primary information displayed on vertical tabs.",
);

#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Subtitle shown on compact vertical tabs.",
    rename_all = "snake_case"
)]
pub enum VerticalTabsCompactSubtitle {
    #[default]
    Branch,
    WorkingDirectory,
    Command,
}

settings::macros::implement_setting_for_enum!(
    VerticalTabsCompactSubtitle,
    TabSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "appearance.vertical_tabs.compact_subtitle",
    description: "Subtitle shown on compact vertical tabs.",
);

define_settings_group!(TabSettings, settings: [
    show_indicators: ShowIndicatorsButton {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.tabs.show_indicators_button",
        description: "Whether to show activity indicators on tabs.",
    },
    show_code_review_button: ShowCodeReviewButton {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "code.editor.show_code_review_button",
        description: "Whether to show the code review button on tabs.",
    },
    show_code_review_diff_stats: ShowCodeReviewDiffStats {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "code.editor.show_code_review_diff_stats",
        description: "Whether to show lines added/removed counts on the code review button.",
    },
    preserve_active_tab_color: PreserveActiveTabColor {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.tabs.preserve_active_tab_color",
        description: "Whether to preserve the active tab's color when switching tabs.",
    },
    use_vertical_tabs: UseVerticalTabs {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.vertical_tabs.enabled",
        description: "Whether to display tabs vertically instead of horizontally.",
    },
    rail_show_tasks: RailShowTasks {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.tabs.rail_show_tasks",
        description: "List each project's tasks under it in the project rail, each with its own status.",
        feature_flag: warp_core::features::FeatureFlag::Projects,
    },
    rail_hide_shells_without_agents: RailHideShellsWithoutAgents {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.tabs.rail_hide_shells_without_agents",
        description: "Hide project rail rows for tabs that are plain shells, with no agent session now or before. The active tab always stays visible.",
        feature_flag: warp_core::features::FeatureFlag::Projects,
    },
    use_project_layout: UseProjectLayout {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.project_layout.enabled",
        description: "Group sessions by project: a project rail on the left, with that project's tasks as tabs along the top.",
        feature_flag: warp_core::features::FeatureFlag::Projects,
    },
    show_vertical_tab_panel_in_restored_windows: ShowVerticalTabPanelInRestoredWindows {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.vertical_tabs.show_panel_in_restored_windows",
        description: "When restoring a window, open the vertical tabs panel even if it was closed when the session was saved.",
    },
    hide_title_bar_search_bar_in_vertical_tabs: HideTitleBarSearchBarInVerticalTabs {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.vertical_tabs.hide_title_bar_search_bar",
        description: "When using the vertical tab layout, hide the search bar in the title bar. Search stays available via the command palette and keyboard shortcuts.",
    },
    use_latest_user_prompt_as_conversation_title_in_tab_names: UseLatestUserPromptAsConversationTitleInTabNames {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.vertical_tabs.use_latest_prompt_as_title",
        description: "Whether vertical tab names for agent conversations use the latest user prompt.",
    },
    vertical_tabs_display_granularity: VerticalTabsDisplayGranularity,
    vertical_tabs_tab_item_mode: VerticalTabsTabItemMode,
    vertical_tabs_view_mode: VerticalTabsViewMode,
    vertical_tabs_primary_info: VerticalTabsPrimaryInfo,
    vertical_tabs_compact_subtitle: VerticalTabsCompactSubtitle,
    vertical_tabs_show_pr_link: VerticalTabsShowPrLink {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.vertical_tabs.show_pr_link",
        description: "Whether to show PR links on vertical tabs.",
    },
    vertical_tabs_show_diff_stats: VerticalTabsShowDiffStats {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.vertical_tabs.show_diff_stats",
        description: "Whether to show diff stats on vertical tabs.",
    },
    vertical_tabs_show_details_on_hover: VerticalTabsShowDetailsOnHover {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.vertical_tabs.show_details_on_hover",
        description: "Whether to show a details sidecar when hovering over a vertical tab.",
    },
    rail_task_info: RailTaskInfo,
    tab_line_count: TabLineCount,
    tab_primary_info: TabPrimaryInfo,
    tab_secondary_info: TabSecondaryInfo,
    header_toolbar_chip_selection: HeaderToolbarChipSelection,
    new_tab_placement: NewTabPlacement,
    workspace_decoration_visibility: WorkspaceDecorationVisibility,
    close_button_position: TabCloseButtonPosition,
    directory_tab_colors: DirectoryTabColors,
    project_priorities: ProjectPriorities,
]);

/// Whether the vertical-tabs sidebar layout is active.
///
/// Single source of truth for the vertical-vs-horizontal tab layout decision,
/// so no render site can disagree with another. In project mode
/// (`FeatureFlag::Projects`, the Herdr-style Projects × Tasks layout) the left
/// rail is owned by the project list and tasks live on the horizontal top tab
/// bar, so the vertical tabs sidebar is suppressed.
pub fn vertical_tabs_layout_active(ctx: &warpui::AppContext) -> bool {
    use warp_core::features::FeatureFlag;
    use warpui::SingletonEntity as _;

    FeatureFlag::VerticalTabs.is_enabled()
        && *TabSettings::as_ref(ctx).use_vertical_tabs
        && !project_layout_active(ctx)
}

/// Whether the Projects × Tasks layout is active: a project rail on the left,
/// with the selected project's tasks on the horizontal top tab bar.
///
/// Single source of truth for the feature's gating — the flag plus the user
/// setting — so the rail, the tab projection, and every navigation path agree.
pub fn project_layout_active(ctx: &warpui::AppContext) -> bool {
    use warp_core::features::FeatureFlag;
    use warpui::SingletonEntity as _;

    FeatureFlag::Projects.is_enabled() && *TabSettings::as_ref(ctx).use_project_layout
}

#[cfg(test)]
#[path = "tab_settings_tests.rs"]
mod tests;
