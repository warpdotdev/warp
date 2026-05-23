use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationContext {
    InsideWarp,
    OutsideWarp,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmptyParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionGetParams {
    pub action: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionListParams {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppActiveParams {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInspectParams {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockGetParams {
    pub block_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveObjectType {
    Workflow,
    Notebook,
    Environment,
    Prompt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_type: Option<DriveObjectType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveGetParams {
    pub object_type: DriveObjectType,
    pub id: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowCreateParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowFocusParams {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowCloseParams {
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabCreateParams {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabActivationTarget {
    Previous,
    Next,
    Last,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabActivateParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative: Option<TabActivationTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HorizontalDirection {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabMoveParams {
    pub direction: HorizontalDirection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabRenameParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabCloseScope {
    Target,
    Others,
    Right,
}

impl Default for TabCloseScope {
    fn default() -> Self {
        Self::Target
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabCloseParams {
    #[serde(default)]
    pub scope: TabCloseScope,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneSplitParams {
    pub direction: PaneDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneFocusParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneNavigateParams {
    pub direction: PaneDirection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneCloseParams {
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneMaximizeParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneResizeParams {
    pub direction: PaneDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputInsertParams {
    pub text: String,
    #[serde(default)]
    pub replace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputReplaceParams {
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputClearParams {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    Terminal,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputModeSetParams {
    pub mode: InputMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRunParams {
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeSetParams {
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppearanceSetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_system_theme: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light_theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark_theme: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeAdjustment {
    Increase,
    Decrease,
    Reset,
    Set,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppearanceFontSizeParams {
    pub adjustment: SizeAdjustment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppearanceZoomParams {
    pub adjustment: SizeAdjustment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingSetParams {
    pub key: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingToggleParams {
    pub key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppFocusParams {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSurfaceParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileOpenParams {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default)]
    pub new_window: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileWriteParams {
    pub path: String,
    pub contents: String,
    #[serde(default)]
    pub create: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileDeleteParams {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveCreateParams {
    pub object_type: DriveObjectType,
    pub name: String,
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveUpdateParams {
    pub object_type: DriveObjectType,
    pub id: String,
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveDeleteParams {
    pub object_type: DriveObjectType,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveRunParams {
    pub object_type: DriveObjectType,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveInsertParams {
    pub object_type: DriveObjectType,
    pub id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileListParams {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputGetParams {}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectActiveParams {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectListParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingGetParams {
    pub key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingListParams {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionListResult {
    pub actions: Vec<ActionMetadata>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionGetResult {
    pub action: ActionMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveTargetChain {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppVersionResult {
    pub protocol_version: u32,
    pub channel: String,
    pub app_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppInspectResult {
    pub version: AppVersionResult,
    pub active: ActiveTargetChain,
    pub actions: Vec<ActionMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSummary {
    pub window_id: String,
    pub is_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowListResult {
    pub windows: Vec<WindowSummary>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabListResult {
    pub tabs: Vec<TabSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabSummary {
    pub tab_id: String,
    pub window_id: String,
    pub index: u32,
    pub is_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneSummary {
    pub pane_id: String,
    pub tab_id: String,
    pub index: u32,
    pub is_active: bool,
    pub has_terminal_session: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneListResult {
    pub panes: Vec<PaneSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub pane_id: String,
    pub is_active: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionListResult {
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockSummary {
    pub block_id: String,
    pub session_id: String,
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockListResult {
    pub blocks: Vec<BlockSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockGetResult {
    pub block: BlockSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputStateResult {
    pub session_id: String,
    pub text: String,
    pub cursor_offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntrySummary {
    pub entry_id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryListResult {
    pub entries: Vec<HistoryEntrySummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeSummary {
    pub name: String,
    pub is_current: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeListResult {
    pub themes: Vec<ThemeSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearanceStateResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    pub follow_system_theme: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light_theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark_theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_zoom_percent: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingSummary {
    pub key: String,
    pub value: serde_json::Value,
    pub value_type: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingListResult {
    pub settings: Vec<SettingSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingGetResult {
    pub setting: SettingSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSummary {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileListResult {
    pub files: Vec<FileSummary>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub path: String,
    pub is_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened_at: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectListResult {
    pub projects: Vec<ProjectSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectActiveResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveObjectSummary {
    pub object_type: DriveObjectType,
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveListResult {
    pub objects: Vec<DriveObjectSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriveGetResult {
    pub object: DriveObjectSummary,
    pub content: serde_json::Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowMutationResult {
    pub window_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabMutationResult {
    pub tab_id: String,
    pub window_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneMutationResult {
    pub pane_id: String,
    pub tab_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMutationResult {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearanceMutationResult {
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingMutationResult {
    pub setting: SettingSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMutationResult {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriveMutationResult {
    pub object: DriveObjectSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionContextProof {
    VerifiedWarpTerminal { proof_id: String },
    ExternalClient,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    ReadOnlyMetadata,
    ReadOnlyTerminalData,
    MutatingNonDestructive,
    MutatingDestructiveOrExecution,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateDataCategory {
    MetadataRead,
    UnderlyingDataRead,
    AppStateMutation,
    MetadataConfigurationMutation,
    UnderlyingDataMutation,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionCategory {
    ReadMetadata,
    ReadUnderlyingData,
    MutateAppState,
    MutateMetadataConfiguration,
    MutateUnderlyingData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticatedUserRequirement {
    pub required: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetScope {
    Instance,
    Action,
    Window,
    Tab,
    Pane,
    Session,
    Block,
    Input,
    History,
    Settings,
    Appearance,
    File,
    Project,
    Drive,
    Surface,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionImplementationStatus {
    Implemented,
    Stub,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionMetadata {
    pub kind: ActionKind,
    pub name: String,
    pub implementation_status: ActionImplementationStatus,
    pub risk_tier: RiskTier,
    pub state_data_category: StateDataCategory,
    pub requires_authenticated_user: bool,
    pub authenticated_user: AuthenticatedUserRequirement,
    pub allowed_invocation_contexts: Vec<InvocationContext>,
    pub permission_category: PermissionCategory,
    pub target_scope: TargetScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WindowSelector(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TabSelector(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaneSelector(pub String);
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionSelector(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockSelector(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileSelector(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DriveObjectSelector(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TargetSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<PaneTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<BlockTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive: Option<DriveTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WindowTarget {
    Active,
    Id { id: WindowSelector },
    Index { index: u32 },
    Title { title: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TabTarget {
    Active,
    Id { id: TabSelector },
    Index { index: u32 },
    Title { title: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneTarget {
    Active,
    Id { id: PaneSelector },
    Index { index: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionTarget {
    Active,
    Id { id: SessionSelector },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlockTarget {
    Active,
    Id { id: BlockSelector },
    Index { index: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileTarget {
    Path { path: String },
    Id { id: FileSelector },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DriveTarget {
    Id {
        object_type: DriveObjectType,
        id: DriveObjectSelector,
    },
    Name {
        object_type: DriveObjectType,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub protocol_version: u32,
    pub request_id: Uuid,
    #[serde(default)]
    pub target: TargetSelector,
    pub action: Action,
}

impl RequestEnvelope {
    pub fn new(action: Action) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            target: TargetSelector::default(),
            action,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub kind: ActionKind,
    #[serde(default)]
    pub params: serde_json::Value,
}

impl Action {
    pub fn new(kind: ActionKind) -> Self {
        Self {
            kind,
            params: serde_json::Value::Object(Default::default()),
        }
    }

    pub fn with_params<T: Serialize>(kind: ActionKind, params: T) -> Result<Self, ControlError> {
        Ok(Self {
            kind,
            params: serde_json::to_value(params).map_err(|err| {
                ControlError::with_details(
                    ErrorCode::InvalidParams,
                    format!("failed to serialize {} parameters", kind.as_str()),
                    err.to_string(),
                )
            })?,
        })
    }

    pub fn params_as<T: DeserializeOwned>(&self) -> Result<T, ControlError> {
        serde_json::from_value(self.params.clone()).map_err(|err| {
            ControlError::with_details(
                ErrorCode::InvalidParams,
                format!("failed to decode {} parameters", self.kind.as_str()),
                err.to_string(),
            )
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionKind {
    #[serde(rename = "instance.list")]
    InstanceList,
    #[serde(rename = "app.ping")]
    AppPing,
    #[serde(rename = "app.inspect")]
    AppInspect,
    #[serde(rename = "app.version")]
    AppVersion,
    #[serde(rename = "app.active")]
    AppActive,
    #[serde(rename = "action.list")]
    ActionList,
    #[serde(rename = "action.get")]
    ActionGet,
    #[serde(rename = "app.focus")]
    AppFocus,
    #[serde(rename = "app.settings.open")]
    AppSettingsOpen,
    #[serde(rename = "app.command_palette.open")]
    AppCommandPaletteOpen,
    #[serde(rename = "app.command_search.open")]
    AppCommandSearchOpen,
    #[serde(rename = "app.warp_drive.open")]
    AppWarpDriveOpen,
    #[serde(rename = "app.warp_drive.toggle")]
    AppWarpDriveToggle,
    #[serde(rename = "app.resource_center.toggle")]
    AppResourceCenterToggle,
    #[serde(rename = "app.ai_assistant.toggle")]
    AppAiAssistantToggle,
    #[serde(rename = "app.code_review.toggle")]
    AppCodeReviewToggle,
    #[serde(rename = "app.vertical_tabs.toggle")]
    AppVerticalTabsToggle,
    #[serde(rename = "window.list")]
    WindowList,
    #[serde(rename = "window.create")]
    WindowCreate,
    #[serde(rename = "window.focus")]
    WindowFocus,
    #[serde(rename = "window.close")]
    WindowClose,
    #[serde(rename = "tab.list")]
    TabList,
    #[serde(rename = "tab.create")]
    TabCreate,
    #[serde(rename = "tab.activate")]
    TabActivate,
    #[serde(rename = "tab.move")]
    TabMove,
    #[serde(rename = "tab.rename")]
    TabRename,
    #[serde(rename = "tab.close")]
    TabClose,
    #[serde(rename = "pane.list")]
    PaneList,
    #[serde(rename = "pane.split")]
    PaneSplit,
    #[serde(rename = "pane.focus")]
    PaneFocus,
    #[serde(rename = "pane.navigate")]
    PaneNavigate,
    #[serde(rename = "pane.close")]
    PaneClose,
    #[serde(rename = "pane.maximize")]
    PaneMaximize,
    #[serde(rename = "pane.resize")]
    PaneResize,
    #[serde(rename = "pane.session.previous")]
    PaneSessionPrevious,
    #[serde(rename = "pane.session.next")]
    PaneSessionNext,
    #[serde(rename = "session.list")]
    SessionList,
    #[serde(rename = "block.list")]
    BlockList,
    #[serde(rename = "block.get")]
    BlockGet,
    #[serde(rename = "input.get")]
    InputGet,
    #[serde(rename = "input.insert")]
    InputInsert,
    #[serde(rename = "input.replace")]
    InputReplace,
    #[serde(rename = "input.clear")]
    InputClear,
    #[serde(rename = "input.mode.set")]
    InputModeSet,
    #[serde(rename = "input.run")]
    InputRun,
    #[serde(rename = "history.list")]
    HistoryList,
    #[serde(rename = "theme.list")]
    ThemeList,
    #[serde(rename = "theme.set")]
    ThemeSet,
    #[serde(rename = "appearance.get")]
    AppearanceGet,
    #[serde(rename = "appearance.set")]
    AppearanceSet,
    #[serde(rename = "appearance.font_size")]
    AppearanceFontSize,
    #[serde(rename = "appearance.zoom")]
    AppearanceZoom,
    #[serde(rename = "setting.get")]
    SettingGet,
    #[serde(rename = "setting.list")]
    SettingList,
    #[serde(rename = "setting.set")]
    SettingSet,
    #[serde(rename = "setting.toggle")]
    SettingToggle,
    #[serde(rename = "file.list")]
    FileList,
    #[serde(rename = "file.open")]
    FileOpen,
    #[serde(rename = "file.write")]
    FileWrite,
    #[serde(rename = "file.delete")]
    FileDelete,
    #[serde(rename = "project.active")]
    ProjectActive,
    #[serde(rename = "project.list")]
    ProjectList,
    #[serde(rename = "drive.list")]
    DriveList,
    #[serde(rename = "drive.get")]
    DriveGet,
    #[serde(rename = "drive.create")]
    DriveCreate,
    #[serde(rename = "drive.update")]
    DriveUpdate,
    #[serde(rename = "drive.delete")]
    DriveDelete,
    #[serde(rename = "drive.run")]
    DriveRun,
    #[serde(rename = "drive.insert")]
    DriveInsert,
}

impl ActionKind {
    pub const ALL: &[Self] = &[
        Self::InstanceList,
        Self::AppPing,
        Self::AppInspect,
        Self::AppVersion,
        Self::AppActive,
        Self::ActionList,
        Self::ActionGet,
        Self::AppFocus,
        Self::AppSettingsOpen,
        Self::AppCommandPaletteOpen,
        Self::AppCommandSearchOpen,
        Self::AppWarpDriveOpen,
        Self::AppWarpDriveToggle,
        Self::AppResourceCenterToggle,
        Self::AppAiAssistantToggle,
        Self::AppCodeReviewToggle,
        Self::AppVerticalTabsToggle,
        Self::WindowList,
        Self::WindowCreate,
        Self::WindowFocus,
        Self::WindowClose,
        Self::TabList,
        Self::TabCreate,
        Self::TabActivate,
        Self::TabMove,
        Self::TabRename,
        Self::TabClose,
        Self::PaneList,
        Self::PaneSplit,
        Self::PaneFocus,
        Self::PaneNavigate,
        Self::PaneClose,
        Self::PaneMaximize,
        Self::PaneResize,
        Self::PaneSessionPrevious,
        Self::PaneSessionNext,
        Self::SessionList,
        Self::BlockList,
        Self::BlockGet,
        Self::InputGet,
        Self::InputInsert,
        Self::InputReplace,
        Self::InputClear,
        Self::InputModeSet,
        Self::InputRun,
        Self::HistoryList,
        Self::ThemeList,
        Self::ThemeSet,
        Self::AppearanceGet,
        Self::AppearanceSet,
        Self::AppearanceFontSize,
        Self::AppearanceZoom,
        Self::SettingGet,
        Self::SettingList,
        Self::SettingSet,
        Self::SettingToggle,
        Self::FileList,
        Self::FileOpen,
        Self::FileWrite,
        Self::FileDelete,
        Self::ProjectActive,
        Self::ProjectList,
        Self::DriveList,
        Self::DriveGet,
        Self::DriveCreate,
        Self::DriveUpdate,
        Self::DriveDelete,
        Self::DriveRun,
        Self::DriveInsert,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InstanceList => "instance.list",
            Self::AppPing => "app.ping",
            Self::AppInspect => "app.inspect",
            Self::AppVersion => "app.version",
            Self::AppActive => "app.active",
            Self::ActionList => "action.list",
            Self::ActionGet => "action.get",
            Self::AppFocus => "app.focus",
            Self::AppSettingsOpen => "app.settings.open",
            Self::AppCommandPaletteOpen => "app.command_palette.open",
            Self::AppCommandSearchOpen => "app.command_search.open",
            Self::AppWarpDriveOpen => "app.warp_drive.open",
            Self::AppWarpDriveToggle => "app.warp_drive.toggle",
            Self::AppResourceCenterToggle => "app.resource_center.toggle",
            Self::AppAiAssistantToggle => "app.ai_assistant.toggle",
            Self::AppCodeReviewToggle => "app.code_review.toggle",
            Self::AppVerticalTabsToggle => "app.vertical_tabs.toggle",
            Self::WindowList => "window.list",
            Self::WindowCreate => "window.create",
            Self::WindowFocus => "window.focus",
            Self::WindowClose => "window.close",
            Self::TabList => "tab.list",
            Self::TabCreate => "tab.create",
            Self::TabActivate => "tab.activate",
            Self::TabMove => "tab.move",
            Self::TabRename => "tab.rename",
            Self::TabClose => "tab.close",
            Self::PaneList => "pane.list",
            Self::PaneSplit => "pane.split",
            Self::PaneFocus => "pane.focus",
            Self::PaneNavigate => "pane.navigate",
            Self::PaneClose => "pane.close",
            Self::PaneMaximize => "pane.maximize",
            Self::PaneResize => "pane.resize",
            Self::PaneSessionPrevious => "pane.session.previous",
            Self::PaneSessionNext => "pane.session.next",
            Self::SessionList => "session.list",
            Self::BlockList => "block.list",
            Self::BlockGet => "block.get",
            Self::InputGet => "input.get",
            Self::InputInsert => "input.insert",
            Self::InputReplace => "input.replace",
            Self::InputClear => "input.clear",
            Self::InputModeSet => "input.mode.set",
            Self::InputRun => "input.run",
            Self::HistoryList => "history.list",
            Self::ThemeList => "theme.list",
            Self::ThemeSet => "theme.set",
            Self::AppearanceGet => "appearance.get",
            Self::AppearanceSet => "appearance.set",
            Self::AppearanceFontSize => "appearance.font_size",
            Self::AppearanceZoom => "appearance.zoom",
            Self::SettingGet => "setting.get",
            Self::SettingList => "setting.list",
            Self::SettingSet => "setting.set",
            Self::SettingToggle => "setting.toggle",
            Self::FileList => "file.list",
            Self::FileOpen => "file.open",
            Self::FileWrite => "file.write",
            Self::FileDelete => "file.delete",
            Self::ProjectActive => "project.active",
            Self::ProjectList => "project.list",
            Self::DriveList => "drive.list",
            Self::DriveGet => "drive.get",
            Self::DriveCreate => "drive.create",
            Self::DriveUpdate => "drive.update",
            Self::DriveDelete => "drive.delete",
            Self::DriveRun => "drive.run",
            Self::DriveInsert => "drive.insert",
        }
    }

    pub fn metadata(self) -> ActionMetadata {
        let implementation_status = match self {
            Self::InstanceList
            | Self::AppPing
            | Self::AppInspect
            | Self::AppVersion
            | Self::AppActive
            | Self::ActionList
            | Self::ActionGet
            | Self::AppFocus
            | Self::AppSettingsOpen
            | Self::AppCommandPaletteOpen
            | Self::AppCommandSearchOpen
            | Self::AppWarpDriveOpen
            | Self::AppWarpDriveToggle
            | Self::AppResourceCenterToggle
            | Self::AppAiAssistantToggle
            | Self::AppCodeReviewToggle
            | Self::AppVerticalTabsToggle
            | Self::WindowList
            | Self::WindowCreate
            | Self::WindowFocus
            | Self::WindowClose
            | Self::TabList
            | Self::TabCreate
            | Self::TabActivate
            | Self::TabMove
            | Self::TabRename
            | Self::TabClose
            | Self::PaneList
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneClose
            | Self::PaneMaximize
            | Self::PaneResize
            | Self::SessionList
            | Self::BlockList
            | Self::BlockGet
            | Self::InputGet
            | Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::InputRun
            | Self::HistoryList
            | Self::ThemeList
            | Self::ThemeSet
            | Self::AppearanceGet
            | Self::AppearanceSet
            | Self::AppearanceFontSize
            | Self::AppearanceZoom
            | Self::SettingGet
            | Self::SettingList
            | Self::SettingSet
            | Self::SettingToggle
            | Self::FileList
            | Self::FileWrite
            | Self::FileDelete
            | Self::ProjectActive
            | Self::ProjectList
            | Self::DriveList
            | Self::DriveGet
            | Self::PaneSessionPrevious
            | Self::PaneSessionNext
            | Self::DriveCreate
            | Self::DriveUpdate
            | Self::DriveDelete
            | Self::DriveRun
            | Self::DriveInsert => ActionImplementationStatus::Implemented,
            _ => ActionImplementationStatus::Stub,
        };
        let requires_authenticated_user = self.default_requires_authenticated_user();
        ActionMetadata {
            kind: self,
            name: self.as_str().to_owned(),
            implementation_status,
            risk_tier: self.default_risk_tier(),
            state_data_category: self.default_state_data_category(),
            requires_authenticated_user,
            authenticated_user: AuthenticatedUserRequirement {
                required: requires_authenticated_user,
            },
            allowed_invocation_contexts: self.default_allowed_invocation_contexts(),
            permission_category: self.default_permission_category(),
            target_scope: self.default_target_scope(),
        }
    }

    pub fn implemented_metadata() -> Vec<ActionMetadata> {
        Self::ALL
            .iter()
            .copied()
            .map(Self::metadata)
            .filter(|metadata| {
                metadata.implementation_status == ActionImplementationStatus::Implemented
            })
            .collect()
    }

    pub fn is_implemented(self) -> bool {
        self.metadata().implementation_status == ActionImplementationStatus::Implemented
    }

    fn default_risk_tier(self) -> RiskTier {
        match self {
            Self::InstanceList
            | Self::AppPing
            | Self::AppInspect
            | Self::AppVersion
            | Self::AppActive
            | Self::ActionList
            | Self::ActionGet
            | Self::WindowList
            | Self::TabList
            | Self::PaneList
            | Self::SessionList
            | Self::ThemeList
            | Self::AppearanceGet
            | Self::SettingGet
            | Self::SettingList
            | Self::FileList
            | Self::ProjectActive
            | Self::ProjectList
            | Self::DriveList => RiskTier::ReadOnlyMetadata,
            Self::BlockList
            | Self::BlockGet
            | Self::InputGet
            | Self::HistoryList
            | Self::DriveGet => RiskTier::ReadOnlyTerminalData,
            Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::InputRun
            | Self::FileWrite
            | Self::FileDelete
            | Self::DriveCreate
            | Self::DriveUpdate
            | Self::DriveDelete
            | Self::DriveRun
            | Self::DriveInsert
            | Self::WindowClose
            | Self::TabClose
            | Self::PaneClose => RiskTier::MutatingDestructiveOrExecution,
            Self::AppFocus
            | Self::AppSettingsOpen
            | Self::AppCommandPaletteOpen
            | Self::AppCommandSearchOpen
            | Self::AppWarpDriveOpen
            | Self::AppWarpDriveToggle
            | Self::AppResourceCenterToggle
            | Self::AppAiAssistantToggle
            | Self::AppCodeReviewToggle
            | Self::AppVerticalTabsToggle
            | Self::WindowCreate
            | Self::WindowFocus
            | Self::TabCreate
            | Self::TabActivate
            | Self::TabMove
            | Self::TabRename
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneMaximize
            | Self::PaneResize
            | Self::PaneSessionPrevious
            | Self::PaneSessionNext
            | Self::ThemeSet
            | Self::AppearanceSet
            | Self::AppearanceFontSize
            | Self::AppearanceZoom
            | Self::SettingSet
            | Self::SettingToggle
            | Self::FileOpen => RiskTier::MutatingNonDestructive,
        }
    }

    fn default_state_data_category(self) -> StateDataCategory {
        match self {
            Self::InstanceList
            | Self::AppPing
            | Self::AppInspect
            | Self::AppVersion
            | Self::AppActive
            | Self::ActionList
            | Self::ActionGet
            | Self::WindowList
            | Self::TabList
            | Self::PaneList
            | Self::SessionList
            | Self::ThemeList
            | Self::AppearanceGet
            | Self::SettingGet
            | Self::SettingList
            | Self::FileList
            | Self::ProjectActive
            | Self::ProjectList
            | Self::DriveList => StateDataCategory::MetadataRead,
            Self::BlockList
            | Self::BlockGet
            | Self::InputGet
            | Self::HistoryList
            | Self::DriveGet => StateDataCategory::UnderlyingDataRead,
            Self::SettingSet
            | Self::SettingToggle
            | Self::ThemeSet
            | Self::AppearanceSet
            | Self::AppearanceFontSize
            | Self::AppearanceZoom
            | Self::TabRename => StateDataCategory::MetadataConfigurationMutation,
            Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::InputRun
            | Self::FileWrite
            | Self::FileDelete
            | Self::DriveCreate
            | Self::DriveUpdate
            | Self::DriveDelete
            | Self::DriveRun
            | Self::DriveInsert => StateDataCategory::UnderlyingDataMutation,
            Self::AppFocus
            | Self::AppSettingsOpen
            | Self::AppCommandPaletteOpen
            | Self::AppCommandSearchOpen
            | Self::AppWarpDriveOpen
            | Self::AppWarpDriveToggle
            | Self::AppResourceCenterToggle
            | Self::AppAiAssistantToggle
            | Self::AppCodeReviewToggle
            | Self::AppVerticalTabsToggle
            | Self::WindowCreate
            | Self::WindowFocus
            | Self::WindowClose
            | Self::TabCreate
            | Self::TabActivate
            | Self::TabMove
            | Self::TabClose
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneClose
            | Self::PaneMaximize
            | Self::PaneResize
            | Self::PaneSessionPrevious
            | Self::PaneSessionNext
            | Self::FileOpen => StateDataCategory::AppStateMutation,
        }
    }

    fn default_permission_category(self) -> PermissionCategory {
        match self.default_state_data_category() {
            StateDataCategory::MetadataRead => PermissionCategory::ReadMetadata,
            StateDataCategory::UnderlyingDataRead => PermissionCategory::ReadUnderlyingData,
            StateDataCategory::AppStateMutation => PermissionCategory::MutateAppState,
            StateDataCategory::MetadataConfigurationMutation => {
                PermissionCategory::MutateMetadataConfiguration
            }
            StateDataCategory::UnderlyingDataMutation => PermissionCategory::MutateUnderlyingData,
        }
    }

    fn default_requires_authenticated_user(self) -> bool {
        matches!(
            self,
            Self::BlockList
                | Self::BlockGet
                | Self::InputGet
                | Self::HistoryList
                | Self::DriveList
                | Self::DriveGet
                | Self::AppFocus
                | Self::AppSettingsOpen
                | Self::AppCommandPaletteOpen
                | Self::AppCommandSearchOpen
                | Self::AppWarpDriveOpen
                | Self::AppWarpDriveToggle
                | Self::AppResourceCenterToggle
                | Self::AppAiAssistantToggle
                | Self::AppCodeReviewToggle
                | Self::AppVerticalTabsToggle
                | Self::WindowCreate
                | Self::WindowFocus
                | Self::WindowClose
                | Self::TabActivate
                | Self::TabMove
                | Self::TabRename
                | Self::TabClose
                | Self::PaneSplit
                | Self::PaneFocus
                | Self::PaneNavigate
                | Self::PaneClose
                | Self::PaneMaximize
                | Self::PaneResize
                | Self::PaneSessionPrevious
                | Self::PaneSessionNext
                | Self::InputInsert
                | Self::InputReplace
                | Self::InputClear
                | Self::InputModeSet
                | Self::InputRun
                | Self::ThemeSet
                | Self::AppearanceSet
                | Self::AppearanceFontSize
                | Self::AppearanceZoom
                | Self::SettingSet
                | Self::SettingToggle
                | Self::FileOpen
                | Self::FileWrite
                | Self::FileDelete
                | Self::DriveCreate
                | Self::DriveUpdate
                | Self::DriveDelete
                | Self::DriveRun
                | Self::DriveInsert
        )
    }

    fn default_allowed_invocation_contexts(self) -> Vec<InvocationContext> {
        if matches!(
            self.default_risk_tier(),
            RiskTier::ReadOnlyMetadata | RiskTier::ReadOnlyTerminalData
        ) || matches!(
            self,
            Self::AppFocus
                | Self::AppSettingsOpen
                | Self::AppCommandPaletteOpen
                | Self::AppCommandSearchOpen
                | Self::AppWarpDriveOpen
                | Self::AppWarpDriveToggle
                | Self::AppResourceCenterToggle
                | Self::AppAiAssistantToggle
                | Self::AppCodeReviewToggle
                | Self::AppVerticalTabsToggle
                | Self::WindowCreate
                | Self::WindowFocus
                | Self::WindowClose
                | Self::TabCreate
                | Self::TabActivate
                | Self::TabMove
                | Self::TabRename
                | Self::TabClose
                | Self::PaneSplit
                | Self::PaneFocus
                | Self::PaneNavigate
                | Self::PaneClose
                | Self::PaneMaximize
                | Self::PaneResize
                | Self::InputRun
                | Self::DriveCreate
                | Self::DriveUpdate
                | Self::DriveDelete
                | Self::DriveRun
                | Self::DriveInsert
                | Self::FileWrite
                | Self::FileDelete
                | Self::ThemeSet
                | Self::AppearanceSet
                | Self::AppearanceFontSize
                | Self::AppearanceZoom
                | Self::SettingSet
                | Self::SettingToggle
        ) {
            return vec![
                InvocationContext::InsideWarp,
                InvocationContext::OutsideWarp,
            ];
        }
        if matches!(
            self,
            Self::InputInsert
                | Self::InputReplace
                | Self::InputClear
                | Self::InputModeSet
                | Self::PaneSessionPrevious
                | Self::PaneSessionNext
        ) {
            return vec![InvocationContext::InsideWarp];
        }
        Vec::new()
    }
    fn default_target_scope(self) -> TargetScope {
        match self {
            Self::WindowList | Self::WindowCreate | Self::WindowFocus | Self::WindowClose => {
                TargetScope::Window
            }
            Self::TabList
            | Self::TabCreate
            | Self::TabActivate
            | Self::TabMove
            | Self::TabRename
            | Self::TabClose => TargetScope::Tab,
            Self::PaneList
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneClose
            | Self::PaneMaximize
            | Self::PaneResize
            | Self::PaneSessionPrevious
            | Self::PaneSessionNext => TargetScope::Pane,
            Self::SessionList
            | Self::InputGet
            | Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::InputRun => TargetScope::Session,
            Self::BlockList | Self::BlockGet => TargetScope::Block,
            Self::HistoryList => TargetScope::History,
            Self::ThemeList
            | Self::ThemeSet
            | Self::AppearanceGet
            | Self::AppearanceSet
            | Self::AppearanceFontSize
            | Self::AppearanceZoom => TargetScope::Appearance,
            Self::SettingGet | Self::SettingList | Self::SettingSet | Self::SettingToggle => {
                TargetScope::Settings
            }
            Self::ActionList | Self::ActionGet => TargetScope::Action,
            Self::FileList | Self::FileOpen | Self::FileWrite | Self::FileDelete => {
                TargetScope::File
            }
            Self::ProjectActive | Self::ProjectList => TargetScope::Project,
            Self::DriveList
            | Self::DriveGet
            | Self::DriveCreate
            | Self::DriveUpdate
            | Self::DriveDelete
            | Self::DriveRun
            | Self::DriveInsert => TargetScope::Drive,
            Self::AppSettingsOpen
            | Self::AppCommandPaletteOpen
            | Self::AppCommandSearchOpen
            | Self::AppWarpDriveOpen
            | Self::AppWarpDriveToggle
            | Self::AppResourceCenterToggle
            | Self::AppAiAssistantToggle
            | Self::AppCodeReviewToggle
            | Self::AppVerticalTabsToggle => TargetScope::Surface,
            Self::InstanceList
            | Self::AppPing
            | Self::AppInspect
            | Self::AppVersion
            | Self::AppActive
            | Self::AppFocus => TargetScope::Instance,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    pub protocol_version: u32,
    pub request_id: Uuid,
    pub response: ControlResponse,
}

impl ResponseEnvelope {
    pub fn ok(request_id: Uuid, data: serde_json::Value) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            response: ControlResponse::Ok { data },
        }
    }

    pub fn error(request_id: Uuid, error: ControlError) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            response: ControlResponse::Error { error },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ControlResponse {
    Ok { data: serde_json::Value },
    Error { error: ControlError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponseEnvelope {
    pub protocol_version: u32,
    pub error: ControlError,
}

impl ErrorResponseEnvelope {
    pub fn new(error: ControlError) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ControlError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl ControlError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(
        code: ErrorCode,
        message: impl Into<String>,
        details: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            details: Some(details.into()),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    LocalControlDisabled,
    UnauthorizedLocalClient,
    InsufficientPermissions,
    AuthenticatedUserRequired,
    AuthenticatedUserUnavailable,
    ExecutionContextNotAllowed,
    ProtocolVersionUnsupported,
    InvalidRequest,
    InvalidSelector,
    InvalidParams,
    NoInstance,
    AmbiguousInstance,
    StaleTarget,
    TargetStateConflict,
    MissingTarget,
    TransportUnavailable,
    BridgeUnavailable,
    UnsupportedAction,
    NotAllowlisted,
    Internal,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = serde_json::to_value(self).map_err(|_| std::fmt::Error)?;
        let Some(value) = value.as_str() else {
            return Err(std::fmt::Error);
        };
        f.write_str(value)
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
