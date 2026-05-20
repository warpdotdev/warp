use std::collections::HashSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value;
use session_sharing_protocol::common::ParticipantId;
use session_sharing_protocol::common::Role;
use session_sharing_protocol::common::SessionId as SharedSessionId;
use session_sharing_protocol::sharer::SessionEndedReason;
use strum_macros::EnumDiscriminants;
use strum_macros::EnumIter;
use warp_completer::completer::MatchType;
use warp_core::command::ExitCode;
use warp_core::telemetry::EnablementState;
use warp_core::telemetry::TelemetryEvent as TelemetryEventTrait;
use warp_core::telemetry::TelemetryEventDesc;
use warpui::keymap::Keystroke;
use warpui::notification::{NotificationSendError, RequestPermissionsOutcome};
use warpui::rendering::ThinStrokes;

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::AIAgentActionId;
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::agent::AIAgentInput as FullAIAgentInput;
use crate::ai::agent::AIIdentifiers;
use crate::ai::agent::EntrypointType;
use crate::ai::agent::PassiveSuggestionTrigger;
use crate::ai::agent::ServerOutputId;
use crate::ai::agent::SuggestedLoggingId;
use crate::ai::agent_management::notifications::NotificationSourceAgent;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::AIBlockResponseRating;
use crate::ai::blocklist::CommandExecutionPermissionAllowedReason;
use crate::ai::blocklist::InputType;
use crate::ai::execution_profiles::AskUserQuestionPermission;
use crate::ai::mcp::TemplateVariable;
use crate::ai::predict::generate_ai_input_suggestions::GenerateAIInputSuggestionsRequest;
use crate::ai::predict::generate_ai_input_suggestions::GenerateAIInputSuggestionsResponseV2;
use crate::ai::predict::next_command_model::HistoryBasedAutosuggestionState;
use crate::auth::auth_manager::LoginGatedFeature;
use crate::channel::Channel;
use crate::cloud_object::{
    model::generic_string_model::GenericStringObjectId, GenericStringObjectFormat, ObjectType,
    Space,
};
#[cfg(feature = "local_fs")]
use crate::code::editor_management::CodeSource;
use crate::drive::CloudObjectTypeAndId;
use crate::drive::DriveSortOrder;
use crate::features::FeatureFlag;
use crate::launch_configs::save_modal::SaveState;
use crate::notebooks::telemetry::NotebookTelemetryAction;
use crate::notebooks::NotebookId;
use crate::notebooks::NotebookLocation;
use crate::palette::PaletteMode;
use crate::pane_group::PaneDragDropLocation;
use crate::prompt::editor_modal::OpenSource as PromptEditorOpenSource;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::QueryFilter;
use crate::server::block::DisplaySetting;
use crate::server::ids::ObjectUid;
use crate::server::ids::ServerId;
use crate::settings::import::config::ParsedTerminalSetting;
use crate::settings::import::config::SettingType;
use crate::settings::import::model::TerminalType;
use crate::settings::AgentModeCodingPermissionsType;
use crate::settings_view::TeamsInviteOption;
use crate::tab::TabTelemetryAction;
use crate::terminal::block_list_viewport::InputMode;
use crate::terminal::cli_agent_sessions::CLIAgentInputEntrypoint;
use crate::terminal::cli_agent_sessions::CLIAgentRichInputCloseReason;
use crate::terminal::input::TelemetryInputSuggestionsMode;
use crate::terminal::model::ansi::WarpificationUnavailableReason;
use crate::terminal::model::block::BlockId;
use crate::terminal::model::session::SessionId;
use crate::terminal::model::terminal_model::BlockSelectionCardinality;
use crate::terminal::model::terminal_model::TmuxInstallationState;
use crate::terminal::settings::AltScreenPaddingMode;
use crate::terminal::shared_session::SharedSessionActionSource;
use crate::terminal::shell::ShellType;
use crate::terminal::ssh::ssh_detection::SshInteractiveSessionDetected;
use crate::terminal::view::block_onboarding::onboarding_agentic_suggestions_block::OnboardingChipType;
use crate::terminal::view::inline_banner::ZeroStatePromptSuggestionTriggeredFrom;
use crate::terminal::view::inline_banner::ZeroStatePromptSuggestionType;
use crate::terminal::view::BlockEntity;
use crate::terminal::view::BlockSelectionDetails;
use crate::terminal::view::ContextMenuInfo;
use crate::terminal::view::GridHighlightedLink;
use crate::terminal::view::PromptPart;
use crate::terminal::view::{
    NotificationsDiscoveryBannerAction, NotificationsErrorBannerAction, NotificationsTrigger,
};
use crate::terminal::ShareBlockType;
use crate::tips::WelcomeTipFeature;
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::settings::EditorLayout;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::FileTarget;
use crate::workflows::WorkflowId;
use crate::workflows::WorkflowSelectionSource;
use crate::workflows::WorkflowSource;
use crate::workspace::tab_settings::TabCloseButtonPosition;
use crate::workspace::tab_settings::WorkspaceDecorationVisibility;
use crate::workspace::TabMovement;
use session_sharing_protocol::sharer::SessionSourceType;
use warp_core::interval_timer::TimingDataPoint;

#[derive(Clone, Serialize, Deserialize)]
pub struct BootstrappingInfo {
    pub shell: &'static str,
    pub is_ssh: bool,
    pub is_subshell: bool,
    pub is_wsl: bool,
    pub is_msys2: bool,
    /// `true` if the bootstrapping process was triggered by an RC file snippet.
    ///
    /// This should only be true if `is_subshell` is true.
    pub was_triggered_by_rc_file: bool,
    /// The total time it took to bootstrap the shell, in seconds.
    pub bootstrap_duration_seconds: Option<f64>,
    /// The time it took to source the user's rcfiles, in seconds.  May be None
    /// if we weren't able to get that information from the shell.
    pub rcfiles_duration_seconds: Option<f64>,
    /// The difference between the total bootstrap time and the rcfile sourcing
    /// time, which roughly equals the time cost of running our bootstrap
    /// script.  Will be None if `bootstrap_duration_seconds` or
    /// `rcfiles_duration_seconds` is None.
    pub warp_attributed_bootstrap_duration_seconds: Option<f64>,
    pub shell_version: Option<String>,
    pub terminal_session_id: Option<SessionId>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SlowBootstrapInfo {
    pub shell: &'static str,
    pub is_ssh: bool,
    pub is_subshell: bool,
    pub is_wsl: bool,
    pub is_msys2: bool,
    /// Contents of the bootstrap block when the slow bootstrap was detected.
    /// This includes both command and output content from the block.
    pub bootstrap_block_contents: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AppStartupInfo {
    pub is_session_restoration_on: bool,
    /// Whether or not a screen reader is enabled at the time the app is
    /// launched.  Should be set to None if we do not know for sure.
    pub is_screen_reader_enabled: Option<bool>,
    pub from_relaunch: bool,
    pub is_crash_reporting_enabled: bool,
    pub timing_data: Vec<TimingDataPoint>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum DownloadSource {
    Website,
    Homebrew,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BlockLatencyInfo {
    pub command: &'static str,
    pub shell: &'static str,
    pub is_ssh: bool,
    pub execution_ms: u64,
}

// For use when recording what type of cloud object a particular telemetry is for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TelemetryCloudObjectType {
    Workflow,
    Notebook,
    Folder,
    GenericStringObject(GenericStringObjectFormat),
}

impl From<&CloudObjectTypeAndId> for TelemetryCloudObjectType {
    fn from(cloud_object_type_and_id: &CloudObjectTypeAndId) -> Self {
        match cloud_object_type_and_id {
            CloudObjectTypeAndId::Notebook(_) => Self::Notebook,
            CloudObjectTypeAndId::Workflow(_) => Self::Workflow,
            CloudObjectTypeAndId::Folder(_) => Self::Folder,
            CloudObjectTypeAndId::GenericStringObject { object_type, .. } => {
                Self::GenericStringObject(*object_type)
            }
        }
    }
}

/// For use when recording how a user has access to a cloud object.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum TelemetrySpace {
    /// The object is owned by the current user.
    Personal,
    /// The object is owned by a team the user is on.
    Team,
    /// The object was shared with the user.
    Shared,
}

impl From<Space> for TelemetrySpace {
    fn from(space: Space) -> Self {
        match space {
            Space::Personal => Self::Personal,
            Space::Team { .. } => Self::Team,
            Space::Shared => Self::Shared,
        }
    }
}

/// Common metadata to include in all Warp Drive telemetry events that act on a specific object.
/// Events that only apply to a single object type may use specific metadata like [`WorkflowTelemetryMetadata`],
/// [`NotebookTelemetryMetadata`], or [`EnvVarTelemetryMetadata`] instead.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloudObjectTelemetryMetadata {
    pub object_type: TelemetryCloudObjectType,
    /// The server UID of the object. This only exists for objects that have been synced to the
    /// server.
    pub object_uid: Option<ServerId>,
    /// The space through which the user has access to the object.
    pub space: Option<TelemetrySpace>,
    /// If the object is owned by a team, this is the owning team's UID. For shared objects, the
    /// user might not be on the team.
    pub team_uid: Option<ServerId>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WorkflowTelemetryMetadata {
    pub workflow_categories: Option<Vec<String>>,
    pub workflow_source: WorkflowSource,
    pub workflow_space: Option<TelemetrySpace>,
    pub workflow_selection_source: WorkflowSelectionSource,
    // This field is only populated for cloud workflows that have been synced to the server
    pub workflow_id: Option<WorkflowId>,
    // Any referenced workflow enums that have been synced to the cloud
    pub enum_ids: Vec<GenericStringObjectId>,
}

/// Metadata to include in all notebook telemetry events.
///
/// There are 4 expected configurations:
/// * Personal cloud notebooks: `notebook_id` is `Some`, `team_uid` is `None`, and location is `PersonalCloud`
/// * Team cloud notebooks: `notebook_id` is `Some`, `team_uid` is `Some`, and location is `Team`
/// * Local file-based notebooks: `notebook_id` and `team_uid` are `None`, and location is `LocalFile`
/// * Remote file-based notebooks: `notebook_id` and `team_uid` are `None`, and location is `RemoteFile`
///
/// This representation allows for invalid combinations, but makes querying the data easier (for
/// example, to find all notebook events for a given team).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct NotebookTelemetryMetadata {
    /// The notebook ID, only available for cloud notebooks that have been synced to the server.
    pub notebook_id: Option<NotebookId>,
    /// The team UID, only available for cloud notebooks in a shared team.
    pub team_uid: Option<ServerId>,
    pub space: Option<TelemetrySpace>,
    /// Where the notebook is canonically located.
    pub location: NotebookLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown_table_count: Option<usize>,
}

impl NotebookTelemetryMetadata {
    pub fn new(
        notebook_id: impl Into<Option<NotebookId>>,
        team_uid: impl Into<Option<ServerId>>,
        location: impl Into<NotebookLocation>,
        space: Option<TelemetrySpace>,
    ) -> Self {
        Self {
            notebook_id: notebook_id.into(),
            team_uid: team_uid.into(),
            location: location.into(),
            space,
            markdown_table_count: None,
        }
    }

    pub fn with_markdown_table_count(mut self, markdown_table_count: usize) -> Self {
        self.markdown_table_count = Some(markdown_table_count);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotebookActionEvent {
    #[serde(flatten)]
    pub action: NotebookTelemetryAction,
    #[serde(flatten)]
    pub metadata: NotebookTelemetryMetadata,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct EnvVarTelemetryMetadata {
    /// The object ID, only available for cloud env vars that have been synced to the server.
    pub object_id: Option<GenericStringObjectId>,
    /// The team UID, only available for cloud env vars in a shared team.
    pub team_uid: Option<ServerId>,
    pub space: TelemetrySpace,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MCPServerTelemetryMetadata {
    pub object_id: GenericStringObjectId,
    pub name: String,
    pub transport_type: MCPServerTelemetryTransportType,
    /// The MCP server string extracted from '@modelcontextprotocol/<...>'.
    pub mcp_server: Option<String>,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub enum MCPTemplateCreationSource {
    #[serde(rename = "json")]
    Json,
    #[serde(rename = "conversion")]
    Conversion,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub enum MCPTemplateInstallationSource {
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "shared")]
    Shared,
    #[serde(rename = "gallery")]
    Gallery,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub enum MCPServerModel {
    #[serde(rename = "legacy")]
    Legacy,
    #[serde(rename = "templatable")]
    Templatable,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum MCPServerTelemetryTransportType {
    CLIServer,
    ServerSentEvents,
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum MCPServerTelemetryError {
    Initialization(String),
    RequestCancelled,
    ResponseError(String),
    SerializationError(String),
    CapabilityUnsupported(String),
    InternalError(String),
    TransportError(String),
}

#[cfg(not(target_family = "wasm"))]
impl From<rmcp::RmcpError> for MCPServerTelemetryError {
    fn from(err: rmcp::RmcpError) -> Self {
        match err {
            rmcp::RmcpError::ClientInitialize(err) => Self::Initialization(err.to_string()),
            rmcp::RmcpError::ServerInitialize(err) => Self::Initialization(err.to_string()),
            rmcp::RmcpError::TransportCreation { error, .. } => {
                Self::TransportError(error.to_string())
            }
            rmcp::RmcpError::Runtime(err) => Self::InternalError(err.to_string()),
            rmcp::RmcpError::Service(err) => match err {
                rmcp::ServiceError::McpError(_) => Self::ResponseError(err.to_string()),
                rmcp::ServiceError::TransportSend(_) => Self::TransportError(err.to_string()),
                rmcp::ServiceError::TransportClosed => Self::TransportError(err.to_string()),
                rmcp::ServiceError::UnexpectedResponse => Self::ResponseError(err.to_string()),
                rmcp::ServiceError::Cancelled { .. } => Self::InternalError(err.to_string()),
                rmcp::ServiceError::Timeout { .. } => Self::TransportError(err.to_string()),
                // The enum is marked as non-exhaustive, so we need a catch-all.
                _ => Self::InternalError(err.to_string()),
            },
            // The enum is marked as non-exhaustive, so we need a catch-all.
            _ => Self::InternalError(err.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenedSharingDialogEvent {
    pub source: SharingDialogSource,

    /// Metadata for the object being shared, if it's a Warp Drive object.
    #[serde(flatten)]
    pub object_metadata: Option<CloudObjectTelemetryMetadata>,

    /// Metadata for the session being shared, if there is one.
    pub session_id: Option<SharedSessionId>,
}

/// How the user opened the Warp Drive sharing dialog.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum SharingDialogSource {
    /// The sharing button in the pane header.
    PaneHeader,
    /// The per-pane command palette entry (includes keybindings).
    CommandPalette,
    /// The Warp Drive index context menu.
    DriveIndex,
    /// The sharing dialog was auto-opened from shared session creation.
    StartedSessionShare,
    /// The user intented into Warp with an email address to invite.
    InviteeRequest,
    /// The user jumped from an inherited ACL to its definition on a parent object.
    InheritedPermission,
    /// The onboarding block shown after users create new personal objects.
    OnboardingBlock,
    /// The conversation list overflow menu.
    ConversationList,
    /// The AI block context menu.
    AIBlockContextMenu,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum TabRenameEvent {
    OpenedEditor,
    CustomNameSet,
    CustomNameCleared,
}

/// The possible sources notifications can turned on from.
#[derive(Clone, Serialize, Deserialize)]
pub enum NotificationsTurnedOnSource {
    Settings,
    Banner,
}

/// The possible types of toggles in the find bar
#[derive(Clone, Serialize, Deserialize)]
pub enum FindOption {
    CaseSensitive,
    FindInBlock,
    Regex,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum LinkOpenMethod {
    CmdClick,
    ToolTip,
    MiddleClick,
}

/// The possible ways to trigger command x-ray
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandXRayTrigger {
    Hover,
    Keystroke,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
pub enum PaletteSource {
    PrefixChange,
    Keybinding,
    CtrlTab { shift_pressed_initially: bool },
    WarpDrive,
    QuitModal,
    LogOutModal,
    IntegrationTest,
    ConversationManager,
    ContextChip,
    PaneHeader,
    AgentTip,
    TitleBarSearchBar,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum FileTreeSource {
    /// Opened from the pane header toolbelt button.
    PaneHeader,
    Keybinding,
    LeftPanelToolbelt,
    ForceOpened,
    /// Opened from the CLI agent view footer (e.g., Claude Code).
    CLIAgentView,
}

#[cfg(feature = "local_fs")]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodePanelsFileOpenEntrypoint {
    CodeReview,
    ProjectExplorer,
    GlobalSearch,
}

/// The CLI agent being used (for telemetry purposes).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum CLIAgentType {
    Claude,
    Gemini,
    Codex,
    Amp,
    Droid,
    OpenCode,
    Copilot,
    Pi,
    Auggie,
    Cursor,
    Goose,
    Hermes,
    Vibe,
    Unknown,
}

/// The kind of plugin chip shown or dismissed (for telemetry purposes).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginChipTelemetryKind {
    Install,
    Update,
}

/// Identifies the agent variant that triggered a notification (for telemetry purposes).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationAgentVariant {
    /// Warp's built-in agent (Oz).
    Oz,
    /// A CLI agent (e.g., Claude Code, Gemini CLI, etc.).
    CLIAgent(CLIAgentType),
}

impl From<NotificationSourceAgent> for NotificationAgentVariant {
    fn from(agent: NotificationSourceAgent) -> Self {
        match agent {
            NotificationSourceAgent::Oz { .. } => Self::Oz,
            NotificationSourceAgent::CLI { agent, .. } => Self::CLIAgent(agent.into()),
        }
    }
}

/// The action taken on a plugin chip (for telemetry purposes).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginChipTelemetryAction {
    /// User clicked the auto-install button.
    Install,
    /// User clicked the auto-update button.
    Update,
    /// User clicked the manual install instructions button.
    InstallInstructions,
    /// User clicked the manual update instructions button.
    UpdateInstructions,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum WarpDriveSource {
    Legacy,
    LeftPanelToolbelt,
    ForceOpened,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum CommandCorrectionAcceptedType {
    /// TODO: We don't use the Autosuggestion variant yet. We need to wire through
    /// when an autosuggestion is accepted to be able to check this.
    Autosuggestion,
    Banner,
    Keybinding,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum CommandCorrectionEvent {
    Proposed {
        rule: &'static str,
    },
    Accepted {
        via: CommandCorrectionAcceptedType,
        rule: &'static str,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub enum CommandSearchResultType {
    History,
    Workflow,
    OpenWarpAI,
    TranslateUsingWarpAI,
    Notebook,
    EnvVarCollection,
    ViewInWarpDrive,
    AIQuery,
    Project,
}

impl From<&CommandSearchItemAction> for CommandSearchResultType {
    fn from(action: &CommandSearchItemAction) -> Self {
        use crate::search::command_search::searcher::CommandSearchItemAction::*;
        match action {
            AcceptHistory(_) | ExecuteHistory(_) => Self::History,
            AcceptWorkflow(_) => Self::Workflow,
            AcceptNotebook(_) => Self::Notebook,
            AcceptEnvVarCollection(_) => Self::EnvVarCollection,
            OpenWarpAI => Self::OpenWarpAI,
            TranslateUsingWarpAI => Self::TranslateUsingWarpAI,
            AcceptAIQuery(_) | RunAIQuery(_) => Self::AIQuery,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum CloseTarget {
    App,
    Window,
    Tab,
    Pane,
    EditorTab,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum PtySpawnMode {
    /// The pty was spawned using the terminal server.
    TerminalServer,
    /// We tried to spawn the pty using the terminal server, but something went
    /// wrong so we fell back to spawning it directly.
    FallbackToDirect,
    /// The terminal server is not in use, and we spawned the pty directly
    /// (in tests, for example).
    Direct,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum OpenedWarpAISource {
    GlobalEntryButton,
    HelpWithBlock,
    HelpWithTextSelection,
    FromAICommandSearch,
    WarmWelcome,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum WarpAIRequestResult {
    Succeeded { latency_ms: i64, truncated: bool },
    OutOfRequests,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum WarpAIActionType {
    CopyTranscript,
    Restart,
    CopyAnswer,
    CopyCode,
    InsertIntoInput,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum SaveAsWorkflowModalSource {
    Block,
    Input,
    WarpAIWorkflowCard,
    WarpAIPanel,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum LaunchConfigUiLocation {
    CommandPalette,
    AppMenu,
    TabMenu,
    Uri,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum AICommandSearchEntrypoint {
    ShortHandTrigger,
    Keybinding,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum SecretInteraction {
    RevealSecret,
    HideSecret,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum AnonymousUserSignupEntrypoint {
    HitDriveObjectLimit,
    LoginGatedFeature,
    SignUpButton,
    RenotificationBlock,
    SignUpAIPrompt,
    NextCommandSuggestionsUpgradeBanner,
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum UndoCloseItemType {
    Window,
    Tab,
    Pane,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PromptChoice {
    PS1,
    Default,
    Custom { builtin_chips: Vec<String> },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ToggleBlockFilterSource {
    /// This includes the keybinding and the command palette items.
    Binding,
    ContextMenu,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TierLimitHitEvent {
    pub team_uid: ServerId,
    pub feature: String,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub enum KnowledgePaneEntrypoint {
    /// Triggered by either the command palette or the mac menus
    #[serde(rename = "global")]
    Global,

    #[serde(rename = "settings")]
    Settings,

    #[serde(rename = "warp_drive")]
    WarpDrive,

    #[serde(rename = "ai_blocklist")]
    AIBlocklist,

    #[serde(rename = "slash_command")]
    SlashCommand,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub enum MCPServerCollectionPaneEntrypoint {
    /// Triggered by either the command palette or the mac menus
    #[serde(rename = "global")]
    Global,

    #[serde(rename = "settings")]
    Settings,

    #[serde(rename = "warp_drive")]
    WarpDrive,

    #[serde(rename = "slash_command")]
    SlashCommand,

    #[serde(rename = "mcp_settings_tab")]
    MCPSettingsTab,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentModeEntrypointSelectionType {
    /// User entered Agent Mode by taking action on a blocklist text selection.
    Text,

    /// User entered Agent Mode by taking action on a block selection.
    Block,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentModeEntrypoint {
    /// The stars icon button in the tab bar.
    #[serde(rename = "tab_bar")]
    TabBar,

    /// This corresponds to _both_ triggering from the command palette and via keybinding.
    ///
    /// Unfortunately due to the way the command palette automatically surfaces any editable
    /// keybinding as an action, we don't have enough information to discern if the binding was
    /// triggered by the palette or keyboard.
    #[serde(rename = "new_pane_binding")]
    NewPaneBinding,

    /// The stars button in the hoverable block "toolbelt".
    #[serde(rename = "block_toolbelt")]
    BlockToolbelt,

    /// The "Ask Agent Mode" option from AI command search.
    #[serde(rename = "ai_command_search")]
    AICommandSearch,

    /// Context menu item(s) that attach a blocklist selection as context to an Agent Mode query.
    #[serde(rename = "context_menu")]
    ContextMenu {
        selection_type: AgentModeEntrypointSelectionType,
    },

    /// The Agent Mode chip in the prompt.
    #[serde(rename = "prompt_chip")]
    PromptChip,

    /// The Agent Management popup, where you can see all the most recent tasks for each terminal
    /// pane across all windows/tabs/panes.
    #[serde(rename = "agent_management_popup")]
    AgentManagementPopup,

    /// User manually switched between terminal and AI input modes in UDI interface
    #[serde(rename = "udi_terminal_input_switcher")]
    UDITerminalInputSwitcher,

    /// The agent management view, where you can see both local interactive and ambient agent tasks
    #[serde(rename = "agent_management_view")]
    AgentManagementView,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AutonomySettingToggleSource {
    Speedbump,
    SettingsPage,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ToggleCodeSuggestionsSettingSource {
    Speedbump,
    Settings,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum InteractionSource {
    Button,
    Keybinding,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum PromptSuggestionViewType {
    TerminalView,
    AgentView,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentModeAttachContextMethod {
    #[serde(rename = "keyboard")]
    Keyboard,

    #[serde(rename = "mouse")]
    Mouse,
}

/// The entrypoint from which the rewind dialog was opened.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum AgentModeRewindEntrypoint {
    /// The rewind button in the AI block header.
    Button,
    /// The context menu item "Rewind to before here".
    ContextMenu,
    /// The /rewind slash command.
    SlashCommand,
}

/// Reasons why we fell back to a prompt suggestion from a suggested code diff.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum PromptSuggestionFallbackReason {
    /// Code file had too many lines, hence we stopped triggering the suggested code diff.
    #[serde(rename = "file_too_many_lines")]
    FileTooManyLines,
    /// Code file had too many bytes, hence we stopped triggering the suggested code diff.
    #[serde(rename = "file_too_many_bytes")]
    FileTooManyBytes,
    /// Missing file, when looking up filepaths in local file system.
    #[serde(rename = "missing_file")]
    MissingFile,
    /// Failed to retrieve file from local file system.
    #[serde(rename = "failed_to_retrieve_file")]
    FailedToRetrieveFile,
    /// In an SSH/remote session.
    #[serde(rename = "ssh_remote_session")]
    SSHRemoteSession,
    /// No read files permission.
    #[serde(rename = "no_read_files_permission")]
    NoReadFilesPermission,
    /// AI query timeout.
    #[serde(rename = "ai_query_timeout")]
    AIQueryTimeout,
    /// Failed to send AI request.
    #[serde(rename = "failed_to_send_ai_request")]
    FailedToSendAIRequest,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentModeSetupProjectScopedRulesActionType {
    #[serde(rename = "link_from_existing")]
    LinkFromExisting(String),
    #[serde(rename = "generate_warp_md")]
    GenerateWarpMd,
    #[serde(rename = "skip_rules")]
    SkipRules,
    #[serde(rename = "regenerate_warp_md")]
    RegenerateWarpMd,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentModeSetupCodebaseContextActionType {
    #[serde(rename = "index_codebase")]
    IndexCodebase,
    #[serde(rename = "skip_indexing")]
    SkipIndexing,
    #[serde(rename = "view_index_status")]
    ViewIndexStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentModeSetupCreateEnvironmentActionType {
    #[serde(rename = "create_environment")]
    CreateEnvironment,
    #[serde(rename = "skip_environment")]
    SkipEnvironment,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CpuUsageStats {
    /// The number of logical CPUs on the system.
    pub num_cpus: usize,

    /// The maximum CPU usage over the measurement interval.
    ///
    /// This number is in the range [0, num_cpus].  The CPU utilization, as a
    /// percentage, can be determined via `max_usage / num_cpus * 100`.
    pub max_usage: f32,

    /// The average CPU usage over the measurement interval.
    ///
    /// This number is in the range [0, num_cpus].  The CPU utilization, as a
    /// percentage, can be determined via `avg_usage / num_cpus * 100`.
    pub avg_usage: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryUsageStats {
    pub total_application_usage_bytes: usize,
    pub total_blocks: usize,
    pub total_lines: usize,

    /// Statistics about blocks that have been seen in the past 5 minutes.
    pub active_block_stats: BlockMemoryUsageStats,
    /// Statistics about blocks that haven't been seen since [5m, 1h).
    pub inactive_5m_stats: BlockMemoryUsageStats,
    /// Statistics about blocks that haven't been seen since [1h, 24h).
    pub inactive_1h_stats: BlockMemoryUsageStats,
    /// Statistics about blocks that haven't been seen since [24h, ..).
    pub inactive_24h_stats: BlockMemoryUsageStats,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockMemoryUsageStats {
    pub num_blocks: usize,
    pub num_lines: usize,
    pub estimated_memory_usage_bytes: usize,
}

/// Entrypoints to toggle the input auto-detection setting for Agent Mode.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum AgentModeAutoDetectionSettingOrigin {
    /// The "speed bump" banner shown that's shown to the user when input is autodetected.
    #[serde(rename = "banner")]
    Banner,

    /// The AI settings page.
    #[serde(rename = "settings_page")]
    SettingsPage,
}

/// Payload for the [`AgentModePotentialAutodetectionFalsePositive`] event.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentModeAutoDetectionFalsePositivePayload {
    /// Payload includes input text for dogfood channels.
    InternalDogfoodUsers { input_text: String },

    /// Do not include the misclassified input text in stable channels due to privacy concerns.
    ExternalUsers,
}

/// How the user triggered the [`AgentModeCodeFilesNavigated`] event.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum AgentModeCodeFileNavigationSource {
    /// User used the next/previous actions.
    NavigationCommand,
    /// User directly selected the file's tab.
    SelectedFileTab,
}

/// How the user triggered the [`AddTabWithShell`] event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum AddTabWithShellSource {
    CommandPalette,
    ShellSelectorMenu,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeContextDestination {
    Pty,
    AgentInput,
    RichInput,
}

#[derive(Clone, Debug, Serialize)]
pub enum AgentModeCitation {
    WarpDriveObject {
        object_type: ObjectType,
        uid: ObjectUid,
    },
    WarpDocs {
        page: String,
    },
    WebPage {
        // Don't serialize the URL to avoid leaking sensitive information.
        #[serde(skip_serializing)]
        url: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize)]
pub enum ImageProtocol {
    Kitty,
    ITerm,
}

#[derive(Clone, Copy, Debug, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InputUXChangeOrigin {
    #[default]
    Settings,
    ADELaunchModal,
}

#[derive(Clone, Debug, Serialize)]
pub enum AIAgentInput {
    UserQuery { query: String },
    AutoCodeDiffQuery { query: String },
    ResumeConversation,
    InitProjectRules { display_query: Option<String> },
    CreateEnvironment { display_query: Option<String> },
    TriggerSuggestPrompt { trigger: PassiveSuggestionTrigger },
    ActionResult { action_id: AIAgentActionId },
    CreateNewProject { query: String },
    CloneRepository { url: String },
    CodeReview,
    FetchReviewComments,
    SummarizeConversation,
    InvokeSkill { skill_name: String },
    StartFromAmbientRunPrompt,
    MessagesReceivedFromAgents { message_count: usize },
    EventsFromAgents { event_count: usize },
    PassiveSuggestionResult,
    OrchestrationConfigUpdate,
}

impl From<FullAIAgentInput> for AIAgentInput {
    fn from(input: FullAIAgentInput) -> Self {
        match input {
            FullAIAgentInput::UserQuery { query, .. } => Self::UserQuery { query },
            FullAIAgentInput::AutoCodeDiffQuery { query, .. } => Self::AutoCodeDiffQuery { query },
            FullAIAgentInput::ResumeConversation { .. } => Self::ResumeConversation,
            FullAIAgentInput::InitProjectRules { display_query, .. } => {
                Self::InitProjectRules { display_query }
            }
            FullAIAgentInput::CreateEnvironment { display_query, .. } => {
                Self::CreateEnvironment { display_query }
            }
            FullAIAgentInput::TriggerPassiveSuggestion { trigger, .. } => {
                Self::TriggerSuggestPrompt { trigger }
            }
            FullAIAgentInput::ActionResult { result, .. } => Self::ActionResult {
                action_id: result.id,
            },
            FullAIAgentInput::CreateNewProject { query, .. } => Self::CreateNewProject { query },
            FullAIAgentInput::CloneRepository { clone_repo_url, .. } => Self::CloneRepository {
                url: clone_repo_url.into_url(),
            },
            FullAIAgentInput::CodeReview { .. } => Self::CodeReview,
            FullAIAgentInput::FetchReviewComments { .. } => Self::FetchReviewComments,
            FullAIAgentInput::SummarizeConversation { .. } => Self::SummarizeConversation,
            FullAIAgentInput::InvokeSkill { skill, .. } => Self::InvokeSkill {
                skill_name: skill.name.clone(),
            },
            FullAIAgentInput::StartFromAmbientRunPrompt { .. } => Self::StartFromAmbientRunPrompt,
            FullAIAgentInput::MessagesReceivedFromAgents { messages } => {
                Self::MessagesReceivedFromAgents {
                    message_count: messages.len(),
                }
            }
            FullAIAgentInput::EventsFromAgents { events } => Self::EventsFromAgents {
                event_count: events.len(),
            },
            FullAIAgentInput::PassiveSuggestionResult { .. } => Self::PassiveSuggestionResult,
            FullAIAgentInput::OrchestrationConfigUpdate { .. } => Self::OrchestrationConfigUpdate,
        }
    }
}

/// The origin of an agent view entry, for telemetry purposes.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryAgentViewEntryOrigin {
    Input { was_prompt_autodetected: bool },
    ConversationSelector,
    AgentModeHomepage,
    AgentViewBlock,
    AIDocument,
    AutoFollowUp,
    RestoreExistingConversation,
    SharedSessionSelection,
    AgentRequestedNewConversation,
    AcceptedPromptSuggestion,
    AcceptedUnitTestSuggestion,
    AcceptedPassiveCodeDiff,
    InlineCodeReview,
    AmbientAgent,
    Cli,
    ImageAdded,
    SlashCommand,
    CodeReviewContext,
    ContinueConversationButton,
    ViewPassiveCodeDiffDetails,
    ResumeConversationButton,
    CodexModal,
    LongRunningCommand,
    HistoryMenu,
    InlineConversationMenu,
    PromptChip,
    OnboardingCallout,
    ConversationListView,
    Onboarding,
    Keybinding,
    SlashInit,
    CreateEnvironment,
    ProjectEntry,
    ClearBuffer,
    DefaultSessionMode,
    ChildAgent,
    LinearDeepLink,
    ThirdPartyCloudAgent,
    OrchestrationPillBar,
}

impl From<AgentViewEntryOrigin> for TelemetryAgentViewEntryOrigin {
    fn from(origin: AgentViewEntryOrigin) -> Self {
        match origin {
            AgentViewEntryOrigin::Input {
                was_prompt_autodetected,
            } => Self::Input {
                was_prompt_autodetected,
            },
            AgentViewEntryOrigin::ConversationSelector => Self::ConversationSelector,
            AgentViewEntryOrigin::AgentModeHomepage => Self::AgentModeHomepage,
            AgentViewEntryOrigin::AgentViewBlock => Self::AgentViewBlock,
            AgentViewEntryOrigin::AIDocument => Self::AIDocument,
            AgentViewEntryOrigin::AutoFollowUp => Self::AutoFollowUp,
            AgentViewEntryOrigin::RestoreExistingConversation => Self::RestoreExistingConversation,
            AgentViewEntryOrigin::SharedSessionSelection => Self::SharedSessionSelection,
            AgentViewEntryOrigin::AgentRequestedNewConversation => {
                Self::AgentRequestedNewConversation
            }
            AgentViewEntryOrigin::AcceptedPromptSuggestion => Self::AcceptedPromptSuggestion,
            AgentViewEntryOrigin::AcceptedUnitTestSuggestion => Self::AcceptedUnitTestSuggestion,
            AgentViewEntryOrigin::AcceptedPassiveCodeDiff => Self::AcceptedPassiveCodeDiff,
            AgentViewEntryOrigin::InlineCodeReview => Self::InlineCodeReview,
            AgentViewEntryOrigin::CloudAgent => Self::AmbientAgent,
            AgentViewEntryOrigin::ThirdPartyCloudAgent => Self::ThirdPartyCloudAgent,
            AgentViewEntryOrigin::Cli => Self::Cli,
            AgentViewEntryOrigin::ImageAdded => Self::ImageAdded,
            AgentViewEntryOrigin::SlashCommand { .. } => Self::SlashCommand,
            AgentViewEntryOrigin::CodeReviewContext => Self::CodeReviewContext,
            AgentViewEntryOrigin::LongRunningCommand => Self::LongRunningCommand,
            AgentViewEntryOrigin::ContinueConversationButton => Self::ContinueConversationButton,
            AgentViewEntryOrigin::ViewPassiveCodeDiffDetails => Self::ViewPassiveCodeDiffDetails,
            AgentViewEntryOrigin::ResumeConversationButton => Self::ResumeConversationButton,
            AgentViewEntryOrigin::CodexModal => Self::CodexModal,
            AgentViewEntryOrigin::InlineHistoryMenu => Self::HistoryMenu,
            AgentViewEntryOrigin::InlineConversationMenu => Self::InlineConversationMenu,
            AgentViewEntryOrigin::PromptChip => Self::PromptChip,
            AgentViewEntryOrigin::OnboardingCallout => Self::OnboardingCallout,
            AgentViewEntryOrigin::ConversationListView => Self::ConversationListView,
            AgentViewEntryOrigin::Onboarding => Self::Onboarding,
            AgentViewEntryOrigin::Keybinding => Self::Keybinding,
            AgentViewEntryOrigin::SlashInit => Self::SlashInit,
            AgentViewEntryOrigin::CreateEnvironment => Self::CreateEnvironment,
            AgentViewEntryOrigin::ProjectEntry => Self::ProjectEntry,
            AgentViewEntryOrigin::ClearBuffer => Self::ClearBuffer,
            AgentViewEntryOrigin::DefaultSessionMode => Self::DefaultSessionMode,
            AgentViewEntryOrigin::ChildAgent => Self::ChildAgent,
            AgentViewEntryOrigin::LinearDeepLink => Self::LinearDeepLink,
            AgentViewEntryOrigin::OrchestrationPillBar => Self::OrchestrationPillBar,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub enum SlashMenuSource {
    SlashButton,
    UserTyped,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginEventSource {
    OnboardingSlide,
    AuthModal,
}

/// Details about which type of slash command was accepted
#[derive(Clone, Debug, Serialize)]
pub enum SlashCommandAcceptedDetails {
    /// A built-in static command with its specific name (e.g., "/init", "/diff-review")
    StaticCommand { command_name: String },
    /// A user-created saved prompt/workflow
    SavedPrompt,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AutoReloadModalAction {
    #[serde(rename = "dismissed")]
    Dismissed,
    #[serde(rename = "enabled_auto_reload")]
    EnabledAutoReload,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OutOfCreditsBannerAction {
    #[serde(rename = "dismissed")]
    Dismissed,
    #[serde(rename = "credits_purchased")]
    CreditsPurchased,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CLISubagentControlState {
    AgentInControl,
    UserInControl,
    AgentTaggedIn,
    AgentTaggedOut,
}

#[derive(Clone, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum TelemetryEvent {
    AutosuggestionInserted {
        insertion_length: usize,
        buffer_length: usize,
    },
    BlockCompleted {
        block_finished_to_precmd_delay_ms: u64,
        honor_ps1_enabled: bool,
        num_secrets_redacted: usize,
        /// The number of lines in the block's output grid when it was
        /// finished.
        num_output_lines: u64,
        /// The number of lines of output that were truncated while the block
        /// was active and receiving output.
        num_output_lines_truncated: u64,
        terminal_session_id: Option<SessionId>,
        is_udi_enabled: bool,
        /// Whether the command was executed while in an active agent view.
        is_in_agent_view: bool,
    },
    /// This is identical to the `BlockCompleted` event, but includes extra fields for
    /// the command run / time it took the block to complete / exit code.
    /// That sort of telemetry should *NEVER* be sent in production, so
    /// DO NOT SEND THIS IN NON-DOGFOOD ENVIRONMENTS!
    BlockCompletedOnDogfoodOnly {
        block_finished_to_precmd_delay_ms: u64,
        honor_ps1_enabled: bool,
        num_secrets_redacted: usize,
        /// The number of lines in the block's output grid when it was
        /// finished.
        num_output_lines: u64,
        /// The number of lines of output that were truncated while the block
        /// was active and receiving output.
        num_output_lines_truncated: u64,
        command: String,
        duration: Duration,
        exit_code: ExitCode,
        terminal_session_id: Option<SessionId>,
    },
    /// A new block of background output was started and added to the block list.
    BackgroundBlockStarted,
    /// User-perceptible latency (i.e. from hitting enter to first frame after command finishes) for
    /// a number of commands that perform minimal work we use as a baseline.
    BaselineCommandLatency(BlockLatencyInfo),
    SessionCreation,
    Login,
    OpenSuggestionsMenu(TelemetryInputSuggestionsMode),
    ConfirmSuggestion {
        mode: TelemetryInputSuggestionsMode,
        match_type: MatchType,
    },
    OpenContextMenu {
        context_menu_info: ContextMenuInfo,
    },
    /// Copy command, output or both for some number of blocks.
    ContextMenuCopy(BlockEntity, BlockSelectionCardinality),
    ContextMenuOpenShareModal(BlockSelectionCardinality),
    ContextMenuFindWithinBlocks(BlockSelectionCardinality),
    ContextMenuCopyPrompt {
        part: PromptPart,
    },
    ContextMenuToggleGitPromptDirtyIndicator {
        enabled: bool,
    },
    ContextMenuInsertSelectedText,
    ContextMenuCopySelectedText,
    /// The user opened the prompt editor modal.
    OpenPromptEditor {
        entrypoint: PromptEditorOpenSource,
    },
    /// The user's prompt was edited via the prompt editor modal.
    PromptEdited {
        prompt: PromptChoice,
        entrypoint: String,
    },
    ReinputCommands(BlockSelectionCardinality),
    JumpToPreviousCommand,
    CopyBlockSharingLink(ShareBlockType),
    GenerateBlockSharingLink {
        share_type: ShareBlockType,
        display_setting: DisplaySetting,
        show_prompt: bool,
        redact_secrets: bool,
    },
    BlockSelection(BlockSelectionDetails),
    BootstrappingSlow(BootstrappingInfo),
    BootstrappingSlowContents(SlowBootstrapInfo),
    /// Logged when a pending session is abandoned before it hits Bootstrapped.
    SessionAbandonedBeforeBootstrap {
        pending_shell: Option<ShellType>,
        has_pending_ssh_session: bool,
        was_ever_visible: bool,
        duration_since_start: Duration,
    },
    BootstrappingSucceeded(BootstrappingInfo),
    /// The user accepted a completion suggestion when it was the only one in the suggestions menu.
    /// This event is named with 'Tab' to maintain backwards compatibility; the completion
    /// suggestions menu may be triggered with a keybinding other than tab.
    TabSingleResultAutocompletion,
    EditorUnhandledModifierKey(String),
    CopyInviteLink,
    OpenThemeChooser,
    ThemeSelection {
        theme: String,
        entrypoint: String,
    },
    AppIconSelection {
        icon: String,
    },
    CursorDisplayType {
        cursor: String,
    },
    OpenThemeCreatorModal,
    CreateCustomTheme,
    DeleteCustomTheme,
    SplitPane,
    UnableToAutoUpdateToNewVersion,
    /// An update was successfully installed, and we're attempting to relaunch the app.
    AutoupdateRelaunchAttempt {
        new_version: String,
    },
    SkipOnboardingSurvey,
    ToggleRestoreSession(bool),
    DatabaseStartUpError(String),
    DatabaseReadError(String),
    DatabaseWriteError(String),
    AppStartup(AppStartupInfo),
    /// The native app was opened while logged out. Since Warp requires login,
    /// this usually means a new user.
    LoggedOutStartup,
    /// The download source, if it can be determined. Will only be sent when
    /// the app is launched while logged out.
    DownloadSource(DownloadSource),
    /// We attempted to bootstrap an SSH session via the SSH wrapper.  The
    /// argument is the name of the remote shell.
    SSHBootstrapAttempt(String),
    SSHControlMasterError {
        has_remote_server: bool,
    },
    KeybindingChanged {
        action: String,
        keystroke: Keystroke,
    },
    KeybindingResetToDefault {
        action: String,
    },
    KeybindingRemoved {
        action: String,
    },
    FeaturesPageAction {
        action: String,
        value: String,
    },
    WorkflowExecuted(WorkflowTelemetryMetadata),
    WorkflowSelected(WorkflowTelemetryMetadata),
    OpenWorkflowSearch,
    OpenQuakeModeWindow,
    OpenWelcomeTips,
    CompleteWelcomeTipFeature {
        total_completed_count: usize,
        tip_name: WelcomeTipFeature,
    },
    DismissWelcomeTips,
    ShowNotificationsDiscoveryBanner,
    NotificationsDiscoveryBannerAction(NotificationsDiscoveryBannerAction),
    ShowNotificationsErrorBanner,
    NotificationsErrorBannerAction(NotificationsErrorBannerAction),
    NotificationPermissionsRequested {
        source: NotificationsTurnedOnSource,
        trigger: Option<NotificationsTrigger>,
    },
    NotificationsRequestPermissionsOutcome {
        outcome: RequestPermissionsOutcome,
    },
    // NotificationSent events are emitted at the app level. Thus, they encompass
    // notifications that are successfully sent _and_ those that fail at the platform level.
    NotificationSent {
        trigger: NotificationsTrigger,
        /// Identifies which agent variant produced the desktop notification, if any.
        agent_variant: Option<NotificationAgentVariant>,
    },
    NotificationFailedToSend {
        error: NotificationSendError,
    },
    NotificationClicked,
    ToggleFindOption {
        option: FindOption,
        enabled: bool,
    },
    SignUpButtonClicked,
    LoginButtonClicked {
        source: LoginEventSource,
    },
    LoginLaterButtonClicked {
        source: LoginEventSource,
    },
    LoginLaterConfirmationButtonClicked {
        source: LoginEventSource,
    },
    OpenNewSessionFromFilePath,
    OpenTeamFromURI,
    ShowedSuggestedAgentModeWorkflowChip {
        logging_id: SuggestedLoggingId,
    },
    ShowedSuggestedAgentModeWorkflowModal {
        logging_id: SuggestedLoggingId,
    },
    SelectNavigationPaletteItem,
    SelectCommandPaletteOption(String),
    PaletteSearchOpened {
        mode: PaletteMode,
        source: PaletteSource,
    },
    PaletteSearchResultAccepted {
        result_type: &'static str,
        filter: Option<QueryFilter>,
        buffer_length: usize,
    },
    PaletteSearchExited {
        filter: Option<QueryFilter>,
        buffer_length: usize,
    },
    AuthCommonQuestionClicked {
        question: &'static str,
    },
    AuthToggleFAQ {
        open: bool,
    },
    OpenAuthPrivacySettings {
        source: LoginEventSource,
    },
    TabRenamed(TabRenameEvent),
    MoveActiveTab {
        direction: TabMovement,
    },
    MoveTab {
        direction: TabMovement,
    },
    DragAndDropTab,
    TabOperations {
        action: TabTelemetryAction,
    },
    EditedInputBeforePrecmd,
    TriedToExecuteBeforePrecmd,
    ThinStrokesSettingChanged {
        new_value: ThinStrokes,
    },
    BookmarkBlockToggled {
        enable_bookmark: bool,
    },
    JumpToBookmark,
    JumpToBottomofBlockButtonClicked,
    ToggleJumpToBottomofBlockButton {
        enabled: bool,
    },
    ToggleShowBlockDividers {
        enabled: bool,
    },
    OpenLink {
        link: GridHighlightedLink,
        open_with: LinkOpenMethod,
    },
    OpenChangelogLink {
        url: String,
    },
    ShowInFileExplorer,
    CommandXRayTriggered {
        trigger: CommandXRayTrigger,
    },
    OpenLaunchConfigSaveModal,
    SaveLaunchConfig {
        state: SaveState,
    },
    OpenLaunchConfigFile,
    OpenLaunchConfig {
        ui_location: LaunchConfigUiLocation,
        open_in_active_window: bool,
    },
    TeamCreated,
    TeamJoined,
    TeamLeft,
    ToggleSettingsSync {
        is_settings_sync_enabled: bool,
    },
    TeamLinkCopied,
    RemovedUserFromTeam,
    DeletedWorkflow,
    DeletedNotebook,
    ToggleApprovalsModal,
    ChangedInviteViewOption(TeamsInviteOption),
    SendEmailInvites,
    CommandCorrection {
        event: CommandCorrectionEvent,
    },
    SetLineHeight {
        new_value: f32,
    },
    ResourceCenterOpened,
    ResourceCenterTipsCompleted,
    ResourceCenterTipsSkipped,
    KeybindingsPageOpened,
    CommandSearchOpened {
        has_initial_query: bool,
    },
    CommandSearchExited {
        query_filter: Option<QueryFilter>,
        buffer_length: usize,
    },
    CommandSearchResultAccepted {
        result_index: usize,
        result_type: CommandSearchResultType,
        query_filter: Option<QueryFilter>,
        buffer_length: usize,
        was_immediately_executed: bool,
    },
    CommandSearchFilterChanged {
        new_filter: Option<QueryFilter>,
    },
    CommandSearchAsyncQueryCompleted {
        filters: HashSet<QueryFilter>,
        error_payload: Option<Value>,
    },
    GlobalSearchOpened,
    GlobalSearchQueryStarted,
    AICommandSearchOpened {
        entrypoint: AICommandSearchEntrypoint,
    },
    OpenNotebook(NotebookTelemetryMetadata),
    EditNotebook {
        metadata: NotebookTelemetryMetadata,
        meaningful_change: bool,
    },
    NotebookAction(NotebookActionEvent),
    OpenedAltScreenFind,
    UserInitiatedClose {
        initiated_on: CloseTarget,
    },
    QuitModalShown {
        running_processes: u32,
        shared_sessions: u32,
        modal_for: CloseTarget,
    },
    QuitModalCancel {
        nav_palette: bool,
        modal_for: CloseTarget,
    },
    QuitModalDisabled,
    UserInitiatedLogOut,
    LogOutModalShown,
    LogOutModalCancel {
        nav_palette: bool,
    },
    SetOpacity {
        // Represented in percentages from 1-100.
        opacity: u8,
    },
    SetBlurRadius {
        // The radius value from 1-18.
        blur_radius: u8,
    },
    ToggleDimInactivePanes {
        enabled: bool,
    },
    InputModeChanged {
        old_mode: InputMode,
        new_mode: InputMode,
    },
    PtySpawned {
        mode: PtySpawnMode,
    },
    InitialWorkingDirectoryConfigurationChanged {
        advanced_mode_enabled: bool,
    },
    /// Opened legacy Warp AI.
    OpenedWarpAI {
        source: OpenedWarpAISource,
    },
    /// Issued legacy Warp AI request.
    WarpAIRequestIssued {
        result: WarpAIRequestResult,
    },
    WarpAIAction {
        action_type: WarpAIActionType,
    },
    /// This is purely for static prompts! Do not send user-written prompts with this event.
    UsedWarpAIPreparedPrompt {
        prompt: &'static str,
    },
    ToggleFocusPaneOnHover {
        enabled: bool,
    },
    WarpAICharacterLimitExceeded,
    OpenInputContextMenu,
    InputCutSelectedText,
    InputCopySelectedText,
    InputSelectAll,
    InputPaste,
    InputCommandSearch,
    InputAICommandSearch,
    InputAskWarpAI,
    SaveAsWorkflowModal {
        source: SaveAsWorkflowModalSource,
    },
    ExperimentTriggered {
        experiment: &'static str,
        layer: &'static str,
        group_assignment: &'static str,
    },
    ToggleSyncAllPanesInAllTabs {
        enabled: bool,
    },
    ToggleSyncAllPanesInTab {
        enabled: bool,
    },
    ToggleSameLinePrompt {
        enabled: bool,
    },
    ToggleNewWindowsAtCustomSize {
        enabled: bool,
    },
    SetNewWindowsAtCustomSize,
    DisableInputSync,
    ToggleTabIndicators {
        enabled: bool,
    },
    TogglePreserveActiveTabColor {
        enabled: bool,
    },
    ShowSubshellBanner,
    DeclineSubshellBootstrap {
        remember: bool,
    },
    TriggerSubshellBootstrap {
        triggered_by_rc_file_snippet: bool,
    },
    AddDenylistedSubshellCommand,
    RemoveDenylistedSubshellCommand,
    AddAddedSubshellCommand,
    RemoveAddedSubshellCommand,
    ReceivedSubshellRcFileDcs,
    AddDenylistedSshTmuxWrapperHost,
    RemoveDenylistedSshTmuxWrapperHost,
    /// User Setting for enabling SSH Tmux Wrapper changed.
    ToggleSshTmuxWrapper {
        enabled: bool,
    },
    ToggleSshWarpification {
        enabled: bool,
    },
    /// User changed the SSH extension install mode.
    SetSshExtensionInstallMode {
        mode: &'static str,
    },
    /// User toggled the "Don't ask me this again" checkbox on the SSH
    /// remote-server choice block.
    SshRemoteServerChoiceDoNotAskAgainToggled {
        checked: bool,
    },
    /// An ssh interactive session was detected.
    SshInteractiveSessionDetected(SshInteractiveSessionDetected),
    SshTmuxWarpifyBannerDisplayed,
    /// A SSH Warpify Block was accepted
    SshTmuxWarpifyBlockAccepted,
    /// A SSH Warpify Block was dismissed
    SshTmuxWarpifyBlockDismissed,
    WarpifyFooterShown {
        is_ssh: bool,
    },
    AgentToolbarDismissed,
    WarpifyFooterAcceptedWarpify {
        is_ssh: bool,
    },
    /// How long until the warpify process succeeded
    SshTmuxWarpificationSuccess {
        tmux_installation: Option<TmuxInstallationState>,
        duration_ms: u64,
    },
    /// An SSH Error block was displayed to the user.
    SshTmuxWarpificationErrorBlock {
        error: WarpificationUnavailableReason,
        tmux_installation: Option<TmuxInstallationState>,
    },
    /// A SSH Install Tmux Block was displayed.
    SshInstallTmuxBlockDisplayed,
    /// A SSH Install Tmux Block was accepted.
    SshInstallTmuxBlockAccepted,
    /// A SSH Install Tmux Block was dismissed.
    SshInstallTmuxBlockDismissed,
    ShowAliasExpansionBanner,
    EnableAliasExpansionFromBanner,
    DismissAliasExpansionBanner,
    ShowVimKeybindingsBanner,
    EnableVimKeybindingsFromBanner,
    DismissVimKeybindingsBanner,
    InitiateReauth,
    InitiateAnonymousUserSignup {
        entrypoint: AnonymousUserSignupEntrypoint,
    },
    AnonymousUserExpirationLockout,
    AnonymousUserLinkedFromBrowser,
    AnonymousUserAttemptLoginGatedFeature {
        feature: LoginGatedFeature,
    },
    AnonymousUserHitCloudObjectLimit,
    NeedsReauth,
    WarpDriveOpened {
        source: WarpDriveSource,
        is_code_mode_v2: bool,
    },
    // Toggled the legacy Warp AI side panel.
    ToggleWarpAI {
        opened: bool,
    },
    ToggleSecretRedaction {
        enabled: bool,
    },
    CustomSecretRegexAdded,
    ToggleObfuscateSecret {
        interaction: SecretInteraction,
    },
    CopySecret,
    AutoGenerateMetadataSuccess,
    AutoGenerateMetadataError {
        error_payload: Value,
    },
    UpdateSortingChoice {
        sorting_choice: DriveSortOrder,
    },
    UndoClose {
        item_type: UndoCloseItemType,
    },
    /// This event is used to measure PTY throughput.
    /// NOTE: this event is only meant to be used for WarpDev.
    PtyThroughput {
        /// The maximum PTY throughput in bytes/sec, aggregated over a 10 minute period.
        max_bytes_per_second: usize,
    },
    DuplicateObject(TelemetryCloudObjectType),
    ExportObject(TelemetryCloudObjectType),
    DriveSharingOnboardingBlockShown,
    CommandFileRun,
    PageUpDownInEditorPressed {
        // Key pressed when nothing is in the editor (no-op)
        is_empty_editor: bool,
        // Is PageDown. Otherwise is PageUp
        is_down: bool,
    },
    /// Emitted on start share attempt, not on success.
    StartedSharingCurrentSession {
        includes_scrollback: bool,
        source: SharedSessionActionSource,
    },
    StoppedSharingCurrentSession {
        source: SharedSessionActionSource,
        reason: SessionEndedReason,
    },
    JoinedSharedSession {
        session_id: SharedSessionId,
        source_type: SessionSourceType,
    },
    SharedSessionModalUpgradePressed,
    /// Emitted when a shared session sharer cancels granting a role
    /// (currently only applies when granting executor mode).
    SharerCancelledGrantRole {
        role: Role,
    },
    /// Emitted when a shared session sharer checks "dont show again"
    /// in confirmation modal when granting a role.
    SharerGrantModalDontShowAgain,
    JumpToSharedSessionParticipant {
        jumped_to: ParticipantId,
    },
    CopiedSharedSessionLink {
        source: SharedSessionActionSource,
    },
    WebSessionOpenedOnDesktop {
        source: SharedSessionActionSource,
    },
    WebCloudObjectOpenedOnDesktop {
        object_metadata: CloudObjectTelemetryMetadata,
    },
    UnsupportedShell {
        shell: String,
    },
    LogOut,
    SettingsImportInitiated,
    InviteTeammates {
        num_teammates: usize,
        team_uid: ServerId,
    },
    CopyObjectToClipboard(TelemetryCloudObjectType),
    OpenAndWarpifyDockerSubshell {
        /// Some variant if we support this shell type, and None otherwise.
        shell_type: Option<ShellType>,
    },
    /// Represents an update to a block filter query that goes from empty to non-empty.
    UpdateBlockFilterQuery,
    UpdateBlockFilterQueryContextLines {
        num_context_lines: u16,
    },
    ToggleBlockFilterQuery {
        enabled: bool,
        source: ToggleBlockFilterSource,
    },
    ToggleBlockFilterCaseSensitivity {
        enabled: bool,
    },
    ToggleBlockFilterRegex {
        enabled: bool,
    },
    ToggleBlockFilterInvert {
        enabled: bool,
    },
    BlockFilterToolbeltButtonClicked,
    ToggleSnackbarInActivePane {
        show_snackbar: bool,
    },
    PaneDragInitiated,
    PaneDropped {
        drop_location: PaneDragDropLocation,
    },
    ObjectLinkCopied {
        link: String,
    },
    FileTreeToggled {
        source: FileTreeSource,
        is_code_mode_v2: bool,
        /// The CLI agent type if opened from a CLI agent footer (e.g., Claude Code).
        cli_agent: Option<CLIAgentType>,
    },
    /// User attached a file or directory as context from the file tree
    FileTreeItemAttachedAsContext {
        is_directory: bool,
    },
    /// User added selected code as context from the code editor.
    CodeSelectionAddedAsContext {
        destination: CodeContextDestination,
    },
    /// User created a new file from the file tree
    FileTreeItemCreated,
    /// Conversation list view was opened
    ConversationListViewOpened,
    /// User opened a conversation from the conversation list
    ConversationListItemOpened {
        /// Whether the conversation is an ambient agent task (vs a local conversation)
        is_ambient_agent: bool,
    },
    /// User deleted a conversation from the conversation list
    ConversationListItemDeleted,
    /// User copied a conversation link from the conversation list
    ConversationListLinkCopied {
        /// Whether the conversation is an ambient agent task (vs a local conversation)
        is_ambient_agent: bool,
    },
    /// Created a blocklist AI block.
    AgentModeCreatedAIBlock {
        /// The client-generated exchange ID for the AI exchange (input + output turn) rendered in this AI block.
        client_exchange_id: String,

        /// The server-generated output ID for the output in this block.
        ///
        /// This is only populated if the some part of the response was successfully received.
        server_output_id: Option<ServerOutputId>,

        was_autodetected_ai_query: bool,

        /// Time from sending request to receiving the first token in the output.
        time_to_first_token_ms: Option<u128>,

        /// Time from sending request to receiving the last token in the output.
        time_to_last_token_ms: Option<u128>,

        /// `true` if the output resulted in a user-facing error.
        was_user_facing_error: bool,

        /// `true` if the the AI block was cancelled before receiving any output or while streaming
        /// output.
        cancelled: bool,

        /// The ID of the conversation this block belongs to.
        conversation_id: AIConversationId,

        /// Whether or not Universal Developer Input mode is enabled
        is_udi_enabled: bool,
    },
    /// Rated a blocklist AI response via thumbs up/down.
    AgentModeRatedResponse {
        /// The server-generated ID for the output corresponding to this rating.
        server_output_id: Option<ServerOutputId>,

        /// The ID of the conversation to which the rated output belongs.
        conversation_id: AIConversationId,
        rating: AIBlockResponseRating,
    },
    /// The user tried to send an Agent Mode query but they have already reached their AI request
    /// limit. Note that this limit is for all AI requests, not Agent Mode alone.
    AgentModeUserAttemptedQueryAtRequestLimit {
        /// The AI request limit for the user's current plan.
        limit: usize,
    },
    AgentModeClickedEntrypoint {
        entrypoint: AgentModeEntrypoint,
    },

    /// User clicked the continue conversation button from a block footer.
    AgentModeContinueConversationButtonClicked {
        conversation_id: AIConversationId,
    },

    /// User opened the rewind confirmation dialog.
    AgentModeRewindDialogOpened {
        entrypoint: AgentModeRewindEntrypoint,
    },

    /// User executed a conversation rewind.
    AgentModeRewindExecuted {
        /// The number of AI blocks that were reverted.
        num_blocks_reverted: usize,
    },

    /// Emitted when a user explicitly attaches a block as context to an Agent Mode query.
    ///
    /// This is only emitted for the initial attachment -- its intended to express user's intent to
    /// attach context.
    ///
    /// For example, this is emitted when a user selects a block to attach as context and has no
    /// prior blocks selected. If a user has already selected a block as context and is merely
    /// changing or adding to the existing selection, this is not emitted.
    ///
    /// Also note this is not emitted if the user clicks an entrypoint that automatically attaches
    /// context (like clicking the block toolbelt stars button, for instance).
    AgentModeAttachedBlockContext {
        method: AgentModeAttachContextMethod,
    },

    /// Emitted when the user toggles the "Input Auto-detection" setting in the AI settings page or
    /// in the auto-detection "speed bump" banner.
    AgentModeToggleAutoDetectionSetting {
        is_autodetection_enabled: bool,
        origin: AgentModeAutoDetectionSettingOrigin,
    },

    /// Emitted when the input type is changed from one type to new_input_type.
    AgentModeChangedInputType {
        input: Option<String>,
        buffer_length: usize,
        is_manually_changed: bool,
        new_input_type: InputType,
        active_block_id: BlockId,
        /// Whether or not Universal Developer Input mode is enabled
        is_udi_enabled: bool,
    },

    /// Emitted when the user manually toggles the terminal input from AI mode to shell mode when
    /// the current input text has been auto-detected as AI input -- this is likely a natural
    /// language auto-detection false-positive.
    AgentModePotentialAutoDetectionFalsePositive(AgentModeAutoDetectionFalsePositivePayload),

    /// This is a telemetry event used to help track performance of Agent Predict in Warp,
    /// by keeping track of the context given and the predictions generated.
    AgentModePrediction {
        was_suggestion_accepted: bool,
        request_duration_ms: i64,
        is_from_ai: bool,
        does_actual_command_match_prediction: bool,
        does_actual_command_match_history_prediction: bool,
        history_prediction_likelihood: f64,
        total_history_count: usize,
        // The below fields are only collected if telemetry is enabled.
        actual_next_command_run: Option<String>,
        history_based_autosuggestion_state: Option<HistoryBasedAutosuggestionState>,
        generate_ai_input_suggestions_request: Option<GenerateAIInputSuggestionsRequest>,
        generate_ai_input_suggestions_response: Option<GenerateAIInputSuggestionsResponseV2>,
    },

    /// Keeps track of number of times the user is presented with a Prompt Suggestions banner.
    PromptSuggestionShown {
        id: String,
        request_duration_ms: u64,
        block_id: Option<String>,
        view: PromptSuggestionViewType,
        /// Server-assigned request token from the `/passive-suggestion`
        /// request that generated this suggestion. Used to join client-side
        /// telemetry with server-side logs. `None` on the legacy code path.
        server_request_token: Option<String>,
    },

    /// Keeps track of number of times the user is presented with a Suggested Code Diff banner.
    SuggestedCodeDiffBannerShown {
        prompt_suggestion_id: String,
        /// Exchange ID of the conversation that produced this diff.
        /// `None` on the MAA passive-suggestion code path, which does not
        /// create an exchange.
        code_exchange_id: Option<AIAgentExchangeId>,
        block_id: Option<String>,
        request_duration_ms: u64,
        /// Server-assigned request token from the `/passive-suggestion`
        /// request. Used to join client-side telemetry with server-side logs.
        /// `None` on the legacy code path.
        server_request_token: Option<String>,
    },

    /// Keeps track of number of times the user falls back to a prompt suggestion from a suggested code diff banner.
    SuggestedCodeDiffFailed {
        prompt_suggestion_id: String,
        reason: PromptSuggestionFallbackReason,
    },

    /// Keeps track of number of times the user accepts & runs a query from the Prompt Suggestions banner.
    PromptSuggestionAccepted {
        id: String,
        view: PromptSuggestionViewType,
        interaction_source: InteractionSource,
    },

    /// Keeps track of number of times the user is presented with a Static Prompt Suggestions banner.
    StaticPromptSuggestionsBannerShown {
        id: String,
        block_id: String,
        static_prompt_suggestion_name: String,
        // The below fields are only collected if telemetry is enabled.
        query: Option<String>,
        block_command: Option<String>,
        request_duration_ms: u64,
        view: PromptSuggestionViewType,
    },

    /// Keeps track of number of times the user accepts a Static Prompt Suggestion.
    StaticPromptSuggestionAccepted {
        id: String,
        view: PromptSuggestionViewType,
        interaction_source: InteractionSource,
    },

    /// Keeps track of number of times the user uses a zero state prompt suggestion & the type of suggestion used.
    ZeroStatePromptSuggestionUsed {
        suggestion_type: ZeroStatePromptSuggestionType,
        triggered_from: ZeroStatePromptSuggestionTriggeredFrom,
    },

    UnitTestSuggestionShown {
        identifiers: AIIdentifiers,
    },

    UnitTestSuggestionAccepted {
        identifiers: AIIdentifiers,
        query: Option<String>,
        interaction_source: InteractionSource,
    },

    /// Keeps track of when the user cancels a suggested prompt.
    UnitTestSuggestionCancelled {
        identifiers: AIIdentifiers,
        interaction_source: InteractionSource,
    },

    /// Emitted when a user makes their first edit to any file in a code diff suggestion from Agent
    /// Mode.
    AgentModeCodeSuggestionEditedByUser {
        /// Server-generated unique ID associated with the AI API output that generated the
        /// suggestion. Used to join client-side telemetry with server-side logs.
        output_id: ServerOutputId,
    },

    /// Emitted when a user switches between files while viewing a code diff suggestion from Agent
    /// Mode.
    AgentModeCodeFilesNavigated {
        output_id: ServerOutputId,
        source: AgentModeCodeFileNavigationSource,
    },

    AgentModeCodeDiffHunksNavigated {
        output_id: ServerOutputId,
    },

    /// Emitted when the user toggles the "Intelligent autosuggestions" setting in the AI settings page.
    ToggleIntelligentAutosuggestionsSetting {
        is_intelligent_autosuggestions_enabled: bool,
    },

    /// Emitted when the user toggles global AI.
    ToggleGlobalAI {
        is_ai_enabled: bool,
    },

    /// Emitted when the user toggles codebase context.
    ToggleCodebaseContext {
        is_codebase_context_enabled: bool,
    },

    ToggleAutoIndexing {
        is_autoindexing_enabled: bool,
    },

    ActiveIndexedReposChanged {
        updated_number_of_codebase_indices: usize,
        hit_max_indices: bool,
    },

    /// Emitted when the user toggles active AI.
    ToggleActiveAI {
        is_active_ai_enabled: bool,
    },

    /// Emitted when the user toggles the "Prompt Suggestions" setting in the AI settings page.
    TogglePromptSuggestionsSetting {
        is_prompt_suggestions_enabled: bool,
    },

    /// Emitted when the user toggles the "Code Suggestions" setting.
    ToggleCodeSuggestionsSetting {
        source: ToggleCodeSuggestionsSettingSource,
        is_code_suggestions_enabled: bool,
    },

    /// Emitted when the user toggles the "Natural Language Autosuggestions" setting in the AI settings page.
    ToggleNaturalLanguageAutosuggestionsSetting {
        is_natural_language_autosuggestions_enabled: bool,
    },

    /// Emitted when the user toggles the "Shared Block Title Auto Generation" setting in the AI settings page.
    ToggleSharedBlockTitleGenerationSetting {
        is_shared_block_title_generation_enabled: bool,
    },

    /// Emitted when the user toggles the "Git Operations Autogen" setting in the AI settings page.
    ToggleGitOperationsAutogenSetting {
        is_git_operations_autogen_enabled: bool,
    },

    /// Emitted when the user toggles the "Voice Input" setting in the AI settings page.
    ToggleVoiceInputSetting {
        is_voice_input_enabled: bool,
    },

    /// Emitted when the user toggles the "Show Agent Tips" setting in the AI settings page.
    ToggleShowAgentTips {
        is_enabled: bool,
    },

    TierLimitHit(TierLimitHitEvent),
    SharedObjectLimitHitBannerViewPlansButtonClicked,
    ResourceUsageStats {
        cpu: CpuUsageStats,
        mem: MemoryUsageStats,
    },
    MemoryUsageStats {
        total_application_usage_bytes: usize,
        total_blocks: usize,
        total_lines: usize,

        /// Statistics about blocks that have been seen in the past 5 minutes.
        active_block_stats: BlockMemoryUsageStats,
        /// Statistics about blocks that haven't been seen since [5m, 1h).
        inactive_5m_stats: BlockMemoryUsageStats,
        /// Statistics about blocks that haven't been seen since [1h, 24h).
        inactive_1h_stats: BlockMemoryUsageStats,
        /// Statistics about blocks that haven't been seen since [24h, ..).
        inactive_24h_stats: BlockMemoryUsageStats,
    },
    MemoryUsageHigh {
        total_application_usage_bytes: u64,
        /// Platform-specific memory breakdown (JSON object with keys that
        /// vary by OS).  See `memory_footprint::memory_breakdown()`.
        memory_breakdown: serde_json::Value,
    },
    EnvVarCollectionInvoked(EnvVarTelemetryMetadata),
    EnvVarWorkflowParameterization(EnvVarTelemetryMetadata),

    /// The user imported settings from another terminal.
    CompletedSettingsImport {
        terminal_type: TerminalType,
        imported_settings: Vec<ParsedTerminalSetting>,
    },
    /// The user focused a terminal option to import settings from.
    SettingsImportConfigFocused(TerminalType),
    /// The user clicked the "Reset to defaults" button in the settings import onboarding block.
    SettingsImportResetButtonClicked,
    /// Completed parsing a terminal for its settings to import.
    SettingsImportConfigParsed {
        timing_data: Vec<TimingDataPoint>,
        terminal_type: TerminalType,
        settings_shown_to_user: Option<Vec<SettingType>>,
    },
    /// When parsing iTerm for settings it contained multiple hotkey bindings.
    ITermMultipleHotkeys,
    UserMenuUpgradeClicked,
    ToggleWorkspaceDecorationVisibility {
        previous_value: WorkspaceDecorationVisibility,
        new_value: WorkspaceDecorationVisibility,
    },
    UpdateAltScreenPaddingMode {
        new_mode: AltScreenPaddingMode,
    },
    AddTabWithShell {
        source: AddTabWithShellSource,
        shell: String,
    },
    AgentModeSurfacedCitations {
        citations: Vec<AgentModeCitation>,
        block_id: String,
        conversation_id: AIConversationId,
        server_output_id: Option<ServerOutputId>,
    },
    AgentModeOpenedCitation {
        citation: AgentModeCitation,
        block_id: String,
        conversation_id: AIConversationId,
        server_output_id: Option<ServerOutputId>,
    },
    OpenedSharingDialog(OpenedSharingDialogEvent),
    ToggleLigatureRendering {
        enabled: bool,
    },
    WorkflowAliasAdded {
        workflow_id: Option<WorkflowId>,
        workflow_space: Option<TelemetrySpace>,
    },
    WorkflowAliasRemoved {
        workflow_id: Option<WorkflowId>,
        workflow_space: Option<TelemetrySpace>,
    },
    WorkflowAliasEnvVarsAttached {
        workflow_id: Option<WorkflowId>,
        workflow_space: Option<TelemetrySpace>,
        env_vars_id: Option<GenericStringObjectId>,
        env_vars_space: Option<TelemetrySpace>,
    },
    WorkflowAliasArgumentEdited {
        workflow_id: Option<WorkflowId>,
        workflow_space: Option<TelemetrySpace>,
    },

    ToggledAgentModeAutoexecuteReadonlyCommandsSetting {
        src: AutonomySettingToggleSource,
        enabled: bool,
    },
    ChangedAgentModeCodingPermissions {
        src: AutonomySettingToggleSource,
        new: AgentModeCodingPermissionsType,
    },
    ChangedAgentModeAskUserQuestionPermission {
        src: AutonomySettingToggleSource,
        new: AskUserQuestionPermission,
    },
    FullEmbedCodebaseContextSearchSuccess {
        action_id: AIAgentActionId,
        total_search_duration: Duration,
        out_of_sync_delay: Option<Duration>,
    },
    FullEmbedCodebaseContextSearchFailed {
        action_id: AIAgentActionId,
        error: String,
    },
    RepoOutlineConstructionSuccess {
        total_parse_seconds: usize,
        file_count: usize,
    },
    RepoOutlineConstructionFailed {
        error: String,
    },
    AutoexecutedAgentModeRequestedCommand {
        reason: CommandExecutionPermissionAllowedReason,
    },
    AgenticOnboardingBlockSelected {
        block_type: OnboardingChipType,
    },
    KnowledgePaneOpened {
        entrypoint: KnowledgePaneEntrypoint,
    },
    #[cfg(feature = "local_fs")]
    CodePaneOpened {
        source: CodeSource,
        layout: EditorLayout,
        preview: bool,
    },
    #[cfg(feature = "local_fs")]
    CodePanelsFileOpened {
        entrypoint: CodePanelsFileOpenEntrypoint,
        target: FileTarget,
    },
    #[cfg(feature = "local_fs")]
    PreviewPanePromoted,
    AISuggestedRuleAdded {
        rule_id: SuggestedLoggingId,
    },
    AISuggestedRuleEdited {
        rule_id: SuggestedLoggingId,
    },
    AISuggestedRuleContentChanged {
        rule_id: SuggestedLoggingId,
        is_saved: bool,
    },
    AISuggestedAgentModeWorkflowAdded {
        logging_id: SuggestedLoggingId,
    },
    AttachedImagesToAgentModeQuery {
        num_images: usize,
        /// Whether or not Universal Developer Input mode is enabled
        is_udi_enabled: bool,
    },
    /// An error was encountered fetching available WSL distributions from the Registry.
    /// This typically means the user hasn't installed or enabled WSL.
    #[cfg(windows)]
    WSLRegistryError,
    #[cfg(windows)]
    AutoupdateUnableToCloseApplications,
    #[cfg(windows)]
    AutoupdateFileInUse,
    #[cfg(windows)]
    AutoupdateMutexTimeout,
    #[cfg(windows)]
    AutoupdateForcekillFailed {
        exit_code: i32,
    },
    #[cfg(windows)]
    AutoupdateMinidumpCleanupFailed {
        exit_code: i32,
    },
    ExecutedWarpDrivePrompt {
        id: Option<WorkflowId>,
        selection_source: WorkflowSelectionSource,
    },
    ImageReceived {
        image_protocol: ImageProtocol,
    },
    /// A file from the result of an AI Agent Action exceeded the context limit.
    FileExceededContextLimit {
        identifiers: AIIdentifiers,
    },
    AgentModeError {
        identifiers: AIIdentifiers,
        error: String,
        /// Some errors are retried internally without showing to the user.
        is_user_visible: bool,
        /// Whether a conversation resume will be attempted after this error.
        will_attempt_to_resume: bool,
    },
    /// Emitted when a MultiAgent request that initially failed is successfully completed after retries.
    AgentModeRequestRetrySucceeded {
        identifiers: AIIdentifiers,
        /// The number of retry attempts that were made before success
        retry_count: usize,
        /// The original error that was retried
        original_error: String,
    },
    GrepToolSucceeded,
    GrepToolFailed {
        queries: Option<Vec<String>>,
        path: Option<String>,
        shell_type: Option<ShellType>,
        working_directory: Option<String>,
        absolute_path: Option<String>,
        command: Option<String>,
        output: Option<String>,
        error: String,
        server_output_id: Option<ServerOutputId>,
    },
    FileGlobToolSucceeded,
    FileGlobToolFailed {
        server_output_id: Option<ServerOutputId>,
    },
    MCPServerCollectionPaneOpened {
        entrypoint: MCPServerCollectionPaneEntrypoint,
    },
    MCPServerAdded {
        metadata: MCPServerTelemetryMetadata,
    },
    MCPTemplateCreated {
        source: MCPTemplateCreationSource,
        variables: Vec<TemplateVariable>,
        name: String,
    },
    MCPTemplateInstalled {
        source: MCPTemplateInstallationSource,
    },
    MCPTemplateShared,
    MCPServerSpawned {
        transport_type: MCPServerTelemetryTransportType,
        error: Option<MCPServerTelemetryError>,
        server_model: MCPServerModel,
    },
    MCPToolCallAccepted {
        server_output_id: Option<ServerOutputId>,
        tool_call: String,
        error: Option<MCPServerTelemetryError>,
    },
    ShellTerminatedPrematurely {
        shell_type: Option<ShellType>,
        shell_path: Option<String>,
        reason: String,
        reason_details: Option<String>,
        antivirus_name: Option<String>,
        long_os_version: Option<String>,
        exit_reason: Option<String>,
    },
    SearchCodebaseRequested {
        action_id: AIAgentActionId,
        server_output_id: Option<ServerOutputId>,
        is_cross_repo: bool,
    },
    SearchCodebaseRepoUnavailable {
        action_id: AIAgentActionId,
        error: String,
    },
    /// User changed the input UX mode (e.g. Universal Developer Input, UDI, mode or Classic)
    InputUXModeChanged {
        is_udi_enabled: bool,
        origin: InputUXChangeOrigin,
    },
    /// User interacted with context chips (git branch, working directory, etc.)
    ContextChipInteracted {
        chip_type: String,
        /// "opened"
        action: String,
        /// Whether or not Universal Developer Input mode is enabled
        is_udi_enabled: bool,
    },
    /// User used voice input functionality
    VoiceInputUsed {
        action: String, // "start", "stop", "cancel"
        /// Duration of voice session in milliseconds (for stop action)
        session_duration_ms: Option<u64>,
        /// Whether or not Universal Developer Input mode is enabled
        is_udi_enabled: bool,
        /// Current input mode when voice was used
        current_input_mode: InputType,
    },
    /// User interacted with @-menu for context attachment
    AtMenuInteracted {
        /// Length of the query string
        query_length: Option<usize>,
        /// "opened", "item_selected", "cancelled"
        action: String,
        /// How many items were available in the menu
        item_count: Option<usize>,
        /// Whether or not Universal Developer Input mode is enabled
        is_udi_enabled: bool,
        /// Current input mode when @ menu was used
        current_input_mode: InputType,
    },
    TabCloseButtonPositionUpdated {
        position: TabCloseButtonPosition,
    },
    ExpandedCodeSuggestions {
        identifiers: AIIdentifiers,
    },
    AIExecutionProfileCreated,
    AIExecutionProfileDeleted,
    AIExecutionProfileSettingUpdated {
        setting_type: String,
        setting_value: String,
    },
    AIExecutionProfileAddedToAllowlist {
        list_type: String,
        value: String,
    },
    AIExecutionProfileAddedToDenylist {
        list_type: String,
        value: String,
    },
    AIExecutionProfileRemovedFromAllowlist {
        list_type: String,
        value: String,
    },
    AIExecutionProfileRemovedFromDenylist {
        list_type: String,
        value: String,
    },
    AIExecutionProfileModelSelected {
        model_type: String,
        model_value: String,
    },
    AIExecutionProfileContextWindowSelected {
        tokens: Option<u32>,
    },
    /// The AI input was not sent because there was already an in-flight request.
    AIInputNotSent {
        entrypoint: Option<EntrypointType>,
        inputs: Vec<AIAgentInput>,
        active_server_conversation_id: Option<ServerConversationToken>,
        active_client_conversation_id: Option<AIConversationId>,
    },
    OpenSlashMenu {
        source: SlashMenuSource,
        /// Whether the inline slash commands UI is enabled.
        is_inline_ui_enabled: bool,
        /// Whether the menu was opened in the agent view vs terminal mode.
        is_in_agent_view: bool,
    },
    SlashCommandAccepted {
        command_details: SlashCommandAcceptedDetails,
        /// Whether the command was accepted in the agent view vs terminal mode.
        is_in_agent_view: bool,
    },
    AgentModeSetupBannerAccepted,
    AgentModeSetupBannerDismissed,
    AgentModeSetupProjectScopedRulesAction {
        action: AgentModeSetupProjectScopedRulesActionType,
    },

    AgentModeSetupCodebaseContextAction {
        action: AgentModeSetupCodebaseContextActionType,
    },
    AgentModeSetupCreateEnvironmentAction {
        action: AgentModeSetupCreateEnvironmentActionType,
    },
    InputBufferSubmitted {
        input_type: input_classifier::InputType,
        is_locked: bool,
        was_lock_set_with_empty_buffer: bool,
    },
    /// User submitted a prompt from the create project view - metadata (non-UGC)
    CreateProjectPromptSubmitted {
        /// Whether this was a custom prompt or a predefined suggestion
        is_custom_prompt: bool,
        /// For suggested prompts, this is always collected. For custom prompts, this is None.
        suggested_prompt: Option<String>,
        /// Whether this was from the FTUX
        is_ftux: bool,
    },
    /// User submitted a custom prompt from the create project view - content (UGC)
    CreateProjectPromptSubmittedContent {
        /// The custom prompt content - only collected when UGC is enabled
        custom_prompt: String,
    },
    /// User submitted a repository URL from the clone repo view
    CloneRepoPromptSubmitted {
        is_ftux: bool,
    },
    /// From the first-time user "get started" page, skip straight to terminal without
    /// creating/opening a project/repository.
    GetStartedSkipToTerminal,

    /// User selected an item from the "Recent" list on the new tab zero state
    RecentMenuItemSelected {
        // The kind of recent menu item selected
        kind: &'static str,
    },

    /// User selected a folder to open as a repo from the "Open repository" button
    OpenRepoFolderSubmitted {
        is_ftux: bool,
    },

    /// User closed the "Out of credits" banner (dismissed or purchased credits)
    OutOfCreditsBannerClosed {
        action: OutOfCreditsBannerAction,
        selected_credits: Option<i32>,
        auto_reload_checkbox_enabled: bool,
        banner_toggle_flag_enabled: bool,
        post_purchase_modal_flag_enabled: bool,
    },

    /// User closed the auto-reload modal (either dismissed or enabled auto-reload)
    AutoReloadModalClosed {
        action: AutoReloadModalAction,
        selected_credits: Option<i32>,
        banner_toggle_flag_enabled: bool,
        post_purchase_modal_flag_enabled: bool,
    },

    /// User toggled auto-reload in Billing & Usage settings
    AutoReloadToggledFromBillingSettings {
        enabled: bool,
        banner_toggle_flag_enabled: bool,
        post_purchase_modal_flag_enabled: bool,
    },

    /// Emitted when the control state of the CLI subagent changes.
    CLISubagentControlStateChanged {
        conversation_id: Option<AIConversationId>,
        block_id: BlockId,
        control_state: CLISubagentControlState,
    },
    /// Emitted when user toggles the visibility of agent responses.
    CLISubagentResponsesToggled {
        conversation_id: AIConversationId,
        block_id: BlockId,
        is_hidden: bool,
    },
    /// Emitted when user dismisses the input in the CLI subagent.
    CLISubagentInputDismissed {
        conversation_id: AIConversationId,
        block_id: BlockId,
    },
    /// Emitted when user approves a blocked action from the CLI subagent.
    CLISubagentActionExecuted {
        conversation_id: AIConversationId,
        block_id: BlockId,
        is_autoexecuted: bool,
    },
    /// Emitted when user rejects a blocked action from the CLI subagent.
    CLISubagentActionRejected {
        conversation_id: AIConversationId,
        block_id: BlockId,
        user_took_over: bool,
    },
    /// Emitted when the user toggles the Agent Management View.
    AgentManagementViewToggled {
        is_open: bool,
    },
    /// Emitted when the user opens a session from the Agent Management View.
    AgentManagementViewOpenedSession,
    /// Emitted when the user copies a session link from the Agent Management View.
    AgentManagementViewCopiedSessionLink,
    /// Detected that Warp is running in an isolated sandbox.
    DetectedIsolationPlatform {
        platform: warp_isolation_platform::IsolationPlatformType,
    },

    AgentTipShown {
        tip: String,
    },
    AgentTipClicked {
        tip: String,
        click_target: String,
    },
    /// Emitted when an agent-requested command causes the shell to exit.
    AgentExitedShellProcess {
        command: String,
        server_output_id: Option<ServerOutputId>,
    },
    /// Emitted when the user uses voice input from the CLI agent footer.
    CLIAgentToolbarVoiceInputUsed {
        /// The CLI agent being used.
        cli_agent: CLIAgentType,
    },
    /// Emitted when the user attaches an image from the CLI agent footer.
    CLIAgentToolbarImageAttached {
        /// The CLI agent being used.
        cli_agent: CLIAgentType,
    },
    /// Emitted when the CLI agent footer is shown.
    CLIAgentToolbarShown {
        /// The CLI agent being shown.
        cli_agent: CLIAgentType,
    },
    /// Emitted when the user opens the CLI agent rich input editor.
    CLIAgentRichInputOpened {
        /// The CLI agent being used.
        cli_agent: CLIAgentType,
        /// How the editor was opened (Ctrl-G or footer button).
        entrypoint: CLIAgentInputEntrypoint,
    },
    /// Emitted when the CLI agent rich input editor is closed.
    CLIAgentRichInputClosed {
        /// The CLI agent being used.
        cli_agent: CLIAgentType,
        /// Why the editor was closed.
        reason: CLIAgentRichInputCloseReason,
    },
    /// Emitted when the user submits a prompt via the CLI agent rich input editor.
    CLIAgentRichInputSubmitted {
        /// The CLI agent being used.
        cli_agent: CLIAgentType,
        /// Length of the submitted prompt in characters.
        prompt_length: usize,
    },
    /// Emitted when the user clicks a plugin chip (install, update, or instructions).
    CLIAgentPluginChipClicked {
        /// The CLI agent being used.
        cli_agent: CLIAgentType,
        /// The specific action taken.
        action: PluginChipTelemetryAction,
    },
    /// Emitted when the user dismisses the plugin chip.
    CLIAgentPluginChipDismissed {
        /// The CLI agent being used.
        cli_agent: CLIAgentType,
        /// Whether this was the install or update chip.
        chip_kind: PluginChipTelemetryKind,
    },
    /// Emitted when auto plugin install or update succeeds.
    CLIAgentPluginOperationSucceeded {
        /// The CLI agent being used.
        cli_agent: CLIAgentType,
        /// Whether this was an install or update operation.
        operation: PluginChipTelemetryKind,
    },
    /// Emitted when auto plugin install or update fails.
    CLIAgentPluginOperationFailed {
        /// The CLI agent being used.
        cli_agent: CLIAgentType,
        /// Whether this was an install or update operation.
        operation: PluginChipTelemetryKind,
    },
    /// Emitted when a CLI agent plugin is first recognized (SessionStart event received).
    CLIAgentPluginDetected {
        /// The CLI agent whose plugin was detected.
        cli_agent: CLIAgentType,
    },
    /// Emitted when an agent notification is shown (toast or mailbox notification).
    AgentNotificationShown {
        /// Which agent variant produced the notification.
        agent_variant: NotificationAgentVariant,
    },
    /// Emitted when the user toggles the CLI agent footer setting.
    ToggleCLIAgentToolbarSetting {
        /// Whether the setting is enabled or disabled.
        is_enabled: bool,
    },
    /// Emitted when the user toggles the "Use Agent" footer setting.
    ToggleUseAgentToolbarSetting {
        /// Whether the setting is enabled or disabled.
        is_enabled: bool,
    },
    /// Emitted when the user enters the agent view.
    AgentViewEntered {
        /// The origin/entrypoint for entering the agent view.
        origin: TelemetryAgentViewEntryOrigin,
        /// Whether a request was automatically triggered upon entry (e.g., prompt was provided).
        did_auto_trigger_request: bool,
    },
    /// Emitted when the user exits the agent view.
    AgentViewExited {
        /// The origin/entrypoint that was used when entering the agent view.
        origin: TelemetryAgentViewEntryOrigin,
        /// Whether the conversation was empty (had no exchanges) when exiting.
        was_empty: bool,
    },
    /// Emitted when the inline conversation menu is opened.
    InlineConversationMenuOpened {
        /// Whether the menu was opened in the agent view vs terminal mode.
        is_in_agent_view: bool,
    },
    /// Emitted when an item is selected from the inline conversation menu.
    InlineConversationMenuItemSelected {
        /// Whether the item was selected in the agent view vs terminal mode.
        is_in_agent_view: bool,
    },
    /// Emitted when the agent shortcuts view visibility is toggled.
    AgentShortcutsViewToggled {
        /// Whether the shortcuts view is now visible.
        is_visible: bool,
    },
    /// Emitted when the Codex modal is opened.
    CodexModalOpened,
    /// Emitted when the user clicks "Use Codex" in the Codex modal.
    CodexModalUseCodexClicked,
    /// Emitted when the cloud agent capacity modal is opened.
    CloudAgentCapacityModalOpened,
    /// Emitted when the cloud agent capacity modal is dismissed.
    CloudAgentCapacityModalDismissed,
    /// Emitted when the user clicks the upgrade button in the cloud agent capacity modal.
    CloudAgentCapacityModalUpgradeClicked,
    /// Emitted when a RequestComputerUse action is approved (manually or auto-executed).
    ComputerUseApproved {
        client_conversation_id: AIConversationId,
        server_conversation_id: Option<String>,
        is_autoexecuted: bool,
        ambient_agent_task_id: Option<AmbientAgentTaskId>,
    },
    /// Emitted when a RequestComputerUse action is cancelled/rejected.
    ComputerUseCancelled {
        client_conversation_id: AIConversationId,
        server_conversation_id: Option<String>,
        ambient_agent_task_id: Option<AmbientAgentTaskId>,
    },
    /// Emitted when a warp://linear deeplink is opened.
    LinearIssueLinkOpened,
    /// Emitted when the free tier limit hit interstitial is displayed.
    FreeTierLimitHitInterstitialDisplayed,
    /// Emitted when the user clicks the "Upgrade" button in the free tier limit hit interstitial.
    FreeTierLimitHitInterstitialUpgradeButtonClicked,
    /// Emitted when the user clicks close on the free tier limit hit interstitial.
    FreeTierLimitHitInterstitialClosed,
    /// Emitted when the remote server binary check completes.
    RemoteServerBinaryCheck {
        found: bool,
        error: Option<String>,
        remote_os: Option<String>,
        remote_arch: Option<String>,
    },
    /// Emitted when the remote server binary installation completes.
    /// `error` is `None` on success, `Some(reason)` on failure.
    RemoteServerInstallation {
        error: Option<String>,
        install_source: Option<remote_server::transport::InstallSource>,
        remote_os: Option<String>,
        remote_arch: Option<String>,
    },
    /// Emitted when the remote server connection + initialization completes.
    /// `error` is `None` on success, `Some(reason)` on failure.
    RemoteServerInitialization {
        phase: remote_server::manager::RemoteServerInitPhase,
        error: Option<String>,
        remote_os: Option<String>,
        remote_arch: Option<String>,
        /// Exit code of the SSH subprocess, if available.
        /// Helps distinguish proxy crashes from transport failures.
        exit_code: Option<i32>,
        /// Whether the SSH subprocess was killed by a signal.
        signal_killed: Option<bool>,
    },
    /// Emitted when an established remote server connection drops.
    RemoteServerDisconnection {
        remote_os: Option<String>,
        remote_arch: Option<String>,
    },
    /// Emitted when a client request to the remote server fails.
    RemoteServerClientRequestError {
        operation: remote_server::manager::RemoteServerOperation,
        error_type: remote_server::manager::RemoteServerErrorKind,
        remote_os: Option<String>,
        remote_arch: Option<String>,
    },
    /// Emitted when a server message cannot be decoded (no parseable request_id).
    RemoteServerMessageDecodingError {
        remote_os: Option<String>,
        remote_arch: Option<String>,
    },
    /// Emitted when the full remote server setup flow completes successfully.
    RemoteServerSetupDuration {
        duration_ms: u64,
        installed_binary: bool,
        remote_os: Option<String>,
        remote_arch: Option<String>,
        /// Short description of the remote libc (e.g. "glibc 2.35",
        /// "musl", "unknown"). `None` when the preinstall check did
        /// not run (e.g. macOS hosts).
        remote_libc: Option<String>,
    },
    /// Emitted when the preinstall check classifies the remote host as
    /// unsupported by the prebuilt remote-server binary, so the controller
    /// silently falls back to the legacy SSH/`RemoteCommandExecutor`
    /// flow without surfacing an install prompt.
    RemoteServerHostUnsupported {
        remote_os: Option<String>,
        remote_arch: Option<String>,
        /// Typed unsupported reason. Converted into stable telemetry
        /// fields in `payload()`.
        unsupported_reason: remote_server::setup::UnsupportedReason,
        /// Detected libc on the remote host, e.g. `"glibc 2.28"`,
        /// `"musl"`, `"unknown"`.
        detected_libc: String,
    },
    /// Emitted when a reconnection attempt succeeds after a spontaneous disconnect.
    RemoteServerReconnection {
        attempt: u32,
        remote_os: Option<String>,
        remote_arch: Option<String>,
    },
    /// Emitted when all reconnection attempts are exhausted.
    RemoteServerReconnectExhausted {
        attempts: u32,
        remote_os: Option<String>,
        remote_arch: Option<String>,
        exit_code: Option<i32>,
        signal_killed: Option<bool>,
    },
}

impl TelemetryEventTrait for TelemetryEvent {
    fn name(&self) -> &'static str {
        self.name()
    }

    fn payload(&self) -> Option<Value> {
        self.payload()
    }

    fn description(&self) -> &'static str {
        let discriminant: TelemetryEventDiscriminants = self.into();
        discriminant.description()
    }

    fn contains_ugc(&self) -> bool {
        self.contains_ugc()
    }

    fn enablement_state(&self) -> EnablementState {
        self.enablement_state()
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEvent {
    pub fn name(&self) -> &'static str {
        let discriminant: TelemetryEventDiscriminants = self.into();
        discriminant.name()
    }

    pub fn enablement_state(&self) -> EnablementState {
        let discriminant: TelemetryEventDiscriminants = self.into();
        discriminant.enablement_state()
    }

    pub fn payload(&self) -> Option<Value> {
        match self {
            TelemetryEvent::ShowedSuggestedAgentModeWorkflowChip { logging_id } => Some(json!({
                "logging_id": logging_id,
            })),
            TelemetryEvent::ShowedSuggestedAgentModeWorkflowModal { logging_id } => Some(json!({
                "logging_id": logging_id,
            })),
            TelemetryEvent::AISuggestedAgentModeWorkflowAdded { logging_id } => Some(json!({
                "logging_id": logging_id,
            })),
            TelemetryEvent::AutosuggestionInserted {
                insertion_length,
                buffer_length,
            } => {
                Some(json!({"insertion_length": insertion_length, "buffer_length": buffer_length}))
            }
            TelemetryEvent::AgentModeContinueConversationButtonClicked { conversation_id } => {
                Some(json!({"conversation_id": conversation_id}))
            }
            TelemetryEvent::AgentModeRewindDialogOpened { entrypoint } => {
                Some(json!({"entrypoint": entrypoint}))
            }
            TelemetryEvent::AgentModeRewindExecuted {
                num_blocks_reverted,
            } => Some(json!({"num_blocks_reverted": num_blocks_reverted})),
            TelemetryEvent::BootstrappingSlow(info) => Some(json!(info)),
            TelemetryEvent::BootstrappingSlowContents(info) => Some(json!(info)),
            TelemetryEvent::ToggleSettingsSync {
                is_settings_sync_enabled,
            } => Some(json!({ "is_settings_sync_enabled": is_settings_sync_enabled })),
            TelemetryEvent::SessionAbandonedBeforeBootstrap {
                pending_shell,
                has_pending_ssh_session,
                was_ever_visible,
                duration_since_start,
            } => Some(json!({
                "pending_shell": pending_shell.map(|shell| shell.name()),
                "has_pending_ssh_session": has_pending_ssh_session,
                "was_ever_visible": was_ever_visible,
                "duration_since_start_secs": duration_since_start.as_secs_f32(),
            })),
            TelemetryEvent::BlockCompleted {
                block_finished_to_precmd_delay_ms,
                honor_ps1_enabled,
                num_secrets_redacted,
                num_output_lines,
                num_output_lines_truncated,
                terminal_session_id,
                is_udi_enabled,
                is_in_agent_view,
            } => Some(json!({
                "block_finished_to_precmd_delay_ms": block_finished_to_precmd_delay_ms,
                "honor_ps1_enabled": honor_ps1_enabled,
                "num_secrets_redacted": num_secrets_redacted,
                "num_output_lines": num_output_lines,
                "num_output_lines_truncated": num_output_lines_truncated,
                "terminal_session_id": terminal_session_id,
                "is_udi_enabled": is_udi_enabled,
                "is_in_agent_view": is_in_agent_view,
            })),
            TelemetryEvent::ToggleFocusPaneOnHover { enabled } => Some(json!({
                "enabled": enabled,
            })),
            TelemetryEvent::BlockCompletedOnDogfoodOnly {
                block_finished_to_precmd_delay_ms,
                honor_ps1_enabled,
                num_secrets_redacted,
                num_output_lines,
                num_output_lines_truncated,
                command,
                duration,
                exit_code,
                terminal_session_id,
            } => Some(json!({
                "block_finished_to_precmd_delay_ms": block_finished_to_precmd_delay_ms,
                "honor_ps1_enabled": honor_ps1_enabled,
                "num_secrets_redacted": num_secrets_redacted,
                "num_output_lines": num_output_lines,
                "num_output_lines_truncated": num_output_lines_truncated,
                "command": command,
                "duration": duration,
                "exit_code": exit_code,
                "terminal_session_id": terminal_session_id,
            })),
            TelemetryEvent::BootstrappingSucceeded(info) => Some(json!(info)),
            TelemetryEvent::SSHBootstrapAttempt(remote_shell) => {
                Some(json!({ "shell": remote_shell.as_str() }))
            }
            TelemetryEvent::OpenContextMenu { context_menu_info } => Some(
                json!({ "type": context_menu_info.type_for_telemetry(), "open_method": context_menu_info.open_method_for_telemetry() }),
            ),
            TelemetryEvent::ContextMenuCopy(entity, cardinality) => {
                Some(json!({ "entity": entity.as_str(), "cardinality": cardinality }))
            }
            TelemetryEvent::ContextMenuFindWithinBlocks(cardinality) => {
                Some(json!({ "cardinality": cardinality }))
            }
            TelemetryEvent::ContextMenuOpenShareModal(cardinality) => {
                Some(json!({ "cardinality": cardinality }))
            }
            TelemetryEvent::ContextMenuCopyPrompt { part } => Some(json!({ "part": part })),
            TelemetryEvent::ReinputCommands(cardinality) => {
                Some(json!({ "cardinality": cardinality }))
            }
            TelemetryEvent::ContextMenuToggleGitPromptDirtyIndicator { enabled } => {
                Some(json!({ "enabled": enabled }))
            }
            TelemetryEvent::BlockSelection(details) => Some(json!(details)),
            TelemetryEvent::OpenSuggestionsMenu(mode) => Some(json!(mode)),
            TelemetryEvent::ConfirmSuggestion { mode, match_type } => {
                Some(json!({ "mode": mode, "match_type": match_type }))
            }
            TelemetryEvent::EditorUnhandledModifierKey(normalized_keystroke) => {
                Some(json!(normalized_keystroke.as_str()))
            }
            TelemetryEvent::ThemeSelection { theme, entrypoint } => {
                Some(json!({ "theme": theme, "entrypoint": entrypoint }))
            }
            TelemetryEvent::AppIconSelection { icon } => Some(json!({"icon": icon})),
            TelemetryEvent::CursorDisplayType {
                cursor: cursor_display_type,
            } => Some(json!({"cursor": cursor_display_type})),
            TelemetryEvent::ObjectLinkCopied { link } => Some(json!({"link": link})),
            TelemetryEvent::FileTreeToggled {
                source,
                is_code_mode_v2,
                cli_agent,
            } => Some(
                json!({"source": source, "is_code_mode_v2": is_code_mode_v2, "cli_agent": cli_agent}),
            ),
            TelemetryEvent::FileTreeItemAttachedAsContext { is_directory } => {
                Some(json!({"is_directory": is_directory}))
            }
            TelemetryEvent::ToggleRestoreSession(enabled) => Some(json!({ "enabled": enabled })),
            TelemetryEvent::DatabaseStartUpError(error) => Some(json!(error)),
            TelemetryEvent::DatabaseReadError(error) => Some(json!(error)),
            TelemetryEvent::DatabaseWriteError(error) => Some(json!(error)),
            TelemetryEvent::AppStartup(info) => Some(json!(info)),
            TelemetryEvent::DownloadSource(source) => Some(json!(source)),
            TelemetryEvent::BaselineCommandLatency(info) => Some(json!(info)),
            TelemetryEvent::KeybindingChanged { action, keystroke } => {
                Some(json!({ "action": action, "keystroke": keystroke.normalized() }))
            }
            TelemetryEvent::KeybindingResetToDefault { action } => {
                Some(json!({ "action": action }))
            }
            TelemetryEvent::KeybindingRemoved { action } => Some(json!({ "action": action })),
            TelemetryEvent::FeaturesPageAction { action, value } => {
                Some(json!({"action": action, "value": value}))
            }
            TelemetryEvent::WorkflowExecuted(metadata) => Some(json!(metadata)),
            TelemetryEvent::WorkflowSelected(metadata) => Some(json!(metadata)),
            TelemetryEvent::CompleteWelcomeTipFeature {
                total_completed_count,
                tip_name,
            } => Some(
                json!({ "total_completed_count": total_completed_count, "tip_name": tip_name }),
            ),
            TelemetryEvent::NotificationsDiscoveryBannerAction(action) => {
                Some(json!({ "action": action }))
            }
            TelemetryEvent::InputModeChanged { old_mode, new_mode } => {
                Some(json!({ "old_mode": old_mode, "new_mode": new_mode }))
            }
            TelemetryEvent::NotificationsErrorBannerAction(action) => {
                Some(json!({ "action": action }))
            }
            TelemetryEvent::NotificationPermissionsRequested { source, trigger } => {
                Some(json!({ "source": source, "trigger": trigger }))
            }
            TelemetryEvent::NotificationFailedToSend { error } => Some(json!({ "error": error })),
            TelemetryEvent::NotificationSent {
                trigger,
                agent_variant,
            } => Some(json!({
                "trigger": trigger,
                "agent_variant": agent_variant,
            })),
            TelemetryEvent::NotificationsRequestPermissionsOutcome { outcome } => {
                Some(json!({ "outcome": outcome }))
            }
            TelemetryEvent::ToggleFindOption { option, enabled } => {
                Some(json!({ "option": option, "enabled": enabled }))
            }
            TelemetryEvent::SelectCommandPaletteOption(option) => Some(json!({ "option": option })),
            TelemetryEvent::PaletteSearchOpened { mode, source } => {
                Some(json!({ "mode": mode, "source": source }))
            }
            TelemetryEvent::PaletteSearchResultAccepted {
                result_type,
                filter: mode,
                buffer_length,
            } => Some(
                json!({ "result_type": result_type, "mode": mode, "buffer_length": buffer_length }),
            ),
            TelemetryEvent::PaletteSearchExited {
                filter: mode,
                buffer_length,
            } => Some(json!({ "mode": mode, "buffer_length": buffer_length })),
            TelemetryEvent::AuthCommonQuestionClicked { question } => Some(json!(question)),
            TelemetryEvent::AuthToggleFAQ { open } => {
                let payload = if *open { "open" } else { "close" };
                Some(json!(payload))
            }
            TelemetryEvent::TabRenamed(rename_event) => Some(json!(rename_event)),
            TelemetryEvent::MoveActiveTab { direction } => Some(json!({ "direction": direction })),
            TelemetryEvent::MoveTab { direction } => Some(json!({ "direction": direction })),
            TelemetryEvent::TabOperations { action } => Some(json!({ "action": action })),
            TelemetryEvent::ThinStrokesSettingChanged { new_value } => {
                Some(json!({ "new_value": new_value }))
            }
            TelemetryEvent::BookmarkBlockToggled { enable_bookmark } => {
                Some(json!({ "enable_bookmark": enable_bookmark }))
            }
            TelemetryEvent::OpenLink { link, open_with } => {
                Some(json!({"link_type": link, "open_with": open_with}))
            }
            TelemetryEvent::OpenChangelogLink { url } => Some(json!({ "url": url })),
            TelemetryEvent::CommandXRayTriggered { trigger } => Some(json!({ "trigger": trigger })),
            TelemetryEvent::SaveLaunchConfig { state } => Some(json!({ "state": state })),
            TelemetryEvent::SaveAsWorkflowModal { source } => Some(json!({ "source": source })),
            TelemetryEvent::CommandCorrection { event } => Some(json!({ "event": event })),
            TelemetryEvent::SetLineHeight { new_value } => Some(json!({ "new_value": new_value })),
            TelemetryEvent::CommandSearchOpened { has_initial_query } => {
                Some(json!({ "has_initial_query": has_initial_query }))
            }
            TelemetryEvent::CommandSearchExited {
                buffer_length,
                query_filter,
            } => Some(json!({ "buffer_length": buffer_length, "query_filter": query_filter })),
            TelemetryEvent::CommandSearchResultAccepted {
                result_index,
                result_type,
                query_filter,
                buffer_length,
                was_immediately_executed,
            } => Some(json!({
                "result_index": result_index,
                "result_type": result_type,
                "query_filter": query_filter,
                "buffer_length": buffer_length,
                "was_immediately_executed": was_immediately_executed
            })),
            TelemetryEvent::CommandSearchFilterChanged { new_filter } => {
                Some(json!({ "new_filter": new_filter }))
            }
            TelemetryEvent::CommandSearchAsyncQueryCompleted {
                filters,
                error_payload,
            } => Some(json!({ "filter": filters, "error": error_payload })),
            TelemetryEvent::AICommandSearchOpened { entrypoint } => {
                Some(json!({ "entrypoint": entrypoint }))
            }
            TelemetryEvent::OpenNotebook(metadata) => Some(json!(metadata)),
            TelemetryEvent::EditNotebook {
                metadata,
                meaningful_change,
            } => Some(json!({
                "notebook_id": metadata.notebook_id,
                "team_uid": metadata.team_uid,
                "meaningful_change": meaningful_change,
            })),
            TelemetryEvent::NotebookAction(event) => Some(json!(event)),
            TelemetryEvent::UserInitiatedClose { initiated_on } => {
                Some(json!({ "initiated_on": initiated_on }))
            }
            TelemetryEvent::QuitModalShown {
                running_processes,
                shared_sessions,
                modal_for,
            } => Some(
                json!({ "running_processes": running_processes, "shared_sessions": shared_sessions, "modal_for": modal_for }),
            ),
            TelemetryEvent::QuitModalCancel {
                nav_palette,
                modal_for,
            } => Some(json!({ "nav_palette": nav_palette, "modal_for": modal_for })),
            TelemetryEvent::LogOutModalCancel { nav_palette } => {
                Some(json!({ "nav_palette": nav_palette }))
            }
            TelemetryEvent::SetBlurRadius { blur_radius } => {
                Some(json!({ "blur_radius": blur_radius }))
            }
            TelemetryEvent::SetOpacity { opacity } => Some(json!({ "opacity": opacity })),
            TelemetryEvent::ToggleDimInactivePanes { enabled } => {
                Some(json!({ "enabled": enabled }))
            }
            TelemetryEvent::ToggleJumpToBottomofBlockButton { enabled } => {
                Some(json!({ "enabled": enabled }))
            }
            TelemetryEvent::PtySpawned { mode } => Some(json!({ "mode": mode })),
            TelemetryEvent::InitialWorkingDirectoryConfigurationChanged {
                advanced_mode_enabled,
            } => Some(json!({ "advanced_mode_enabled": advanced_mode_enabled })),
            TelemetryEvent::OpenedWarpAI { source } => Some(json!({ "source": source })),
            TelemetryEvent::WarpAIRequestIssued { result } => Some(json!({ "result": result })),
            TelemetryEvent::WarpAIAction { action_type } => {
                Some(json!({ "action_type": action_type }))
            }
            TelemetryEvent::MCPServerCollectionPaneOpened { entrypoint } => {
                Some(json!({ "entrypoint": entrypoint }))
            }
            TelemetryEvent::MCPServerAdded { metadata } => Some(json!({
                "object_id": metadata.object_id,
                "name": metadata.name,
                "transport_type": metadata.transport_type,
                "mcp_server": metadata.mcp_server,
            })),
            TelemetryEvent::MCPTemplateCreated {
                source,
                variables,
                name,
            } => Some(json!({
                "source": source,
                "variables": variables,
                "name": name,
            })),
            TelemetryEvent::MCPTemplateInstalled { source } => Some(json!({
                "source": source,
            })),
            TelemetryEvent::MCPTemplateShared => None,
            TelemetryEvent::MCPServerSpawned {
                transport_type,
                server_model,
                error,
            } => Some(
                json!({"transport_type": transport_type, "server_model": server_model, "error": error}),
            ),
            TelemetryEvent::MCPToolCallAccepted {
                server_output_id,
                tool_call,
                error,
            } => Some(json!({
                "server_output_id": server_output_id,
                "tool_call": tool_call,
                "error": error,
            })),
            TelemetryEvent::KnowledgePaneOpened { entrypoint } => {
                Some(json!({ "entrypoint": entrypoint }))
            }
            #[cfg(feature = "local_fs")]
            TelemetryEvent::CodePaneOpened {
                source,
                layout,
                preview,
            } => Some(
                json!({ "source": source.telemetry_source_name(), "layout": layout, "preview": preview }),
            ),
            #[cfg(feature = "local_fs")]
            TelemetryEvent::CodePanelsFileOpened { entrypoint, target } => {
                let (target, layout, editor) = match target {
                    FileTarget::MarkdownViewer(layout) => {
                        ("warp_markdown_viewer", Some(*layout), None)
                    }
                    FileTarget::CodeEditor(layout) => ("warp_code_editor", Some(*layout), None),
                    FileTarget::EnvEditor => ("env_editor", None, None),
                    FileTarget::SystemDefault => ("system_default", None, None),
                    FileTarget::SystemGeneric => ("system_generic", None, None),
                    FileTarget::ExternalEditor(editor) => ("external_editor", None, Some(*editor)),
                };

                Some(json!({
                    "entrypoint": entrypoint,
                    "target": target,
                    "layout": layout,
                    "editor": editor,
                }))
            }
            #[cfg(feature = "local_fs")]
            TelemetryEvent::PreviewPanePromoted => None,
            TelemetryEvent::CodeSelectionAddedAsContext { destination } => Some(json!({
                "destination": destination,
            })),
            TelemetryEvent::AISuggestedRuleAdded { rule_id } => Some(json!({ "rule_id": rule_id })),
            TelemetryEvent::AISuggestedRuleEdited { rule_id } => {
                Some(json!({ "rule_id": rule_id }))
            }
            TelemetryEvent::AISuggestedRuleContentChanged { rule_id, is_saved } => {
                Some(json!({ "rule_id": rule_id, "is_saved": is_saved }))
            }
            TelemetryEvent::UsedWarpAIPreparedPrompt { prompt } => {
                Some(json!({ "prompt": prompt }))
            }
            TelemetryEvent::ExperimentTriggered {
                experiment,
                layer,
                group_assignment,
            } => Some(
                json!({ "experiment": experiment, "layer": layer, "group_assignment": group_assignment }),
            ),
            TelemetryEvent::ToggleSyncAllPanesInAllTabs { enabled } => {
                Some(json!({ "enabled": enabled }))
            }
            TelemetryEvent::ToggleSyncAllPanesInTab { enabled } => {
                Some(json!({ "enabled": enabled }))
            }
            TelemetryEvent::ToggleTabIndicators { enabled } => Some(json!({ "enabled": enabled })),
            TelemetryEvent::TogglePreserveActiveTabColor { enabled } => {
                Some(json!({ "enabled": enabled }))
            }
            TelemetryEvent::DeclineSubshellBootstrap { remember } => {
                Some(json!({ "remember": remember }))
            }
            TelemetryEvent::AgentToolbarDismissed => None,
            TelemetryEvent::WarpifyFooterShown { is_ssh }
            | TelemetryEvent::WarpifyFooterAcceptedWarpify { is_ssh } => {
                Some(json!({ "is_ssh": is_ssh }))
            }
            TelemetryEvent::ToggleSameLinePrompt { enabled } => Some(json!({ "enabled": enabled })),
            TelemetryEvent::TriggerSubshellBootstrap {
                triggered_by_rc_file_snippet,
            } => Some(json!({
                "triggered_by_rc_file_snippet": triggered_by_rc_file_snippet
            })),
            TelemetryEvent::OpenLaunchConfig {
                ui_location,
                open_in_active_window,
            } => Some(
                json!({ "ui_location": ui_location, "open_in_active_window": open_in_active_window }),
            ),
            TelemetryEvent::ToggleWarpAI { opened } => Some(json!({ "opened": opened })),
            TelemetryEvent::ToggleSecretRedaction { enabled } => {
                Some(json!({ "enabled": enabled }))
            }
            TelemetryEvent::ToggleObfuscateSecret { interaction } => {
                Some(json!({ "interaction": interaction }))
            }
            TelemetryEvent::AutoGenerateMetadataError { error_payload } => {
                Some(json!({ "error": error_payload }))
            }
            TelemetryEvent::UpdateSortingChoice { sorting_choice } => {
                Some(json!({ "sorting_choice": sorting_choice }))
            }
            TelemetryEvent::UndoClose { item_type } => Some(json!({ "item_type": item_type })),
            TelemetryEvent::PromptEdited { prompt, entrypoint } => Some(json!({
                "prompt": prompt,
                "entrypoint": entrypoint
            })),
            TelemetryEvent::OpenPromptEditor { entrypoint } => {
                Some(json!({ "entrypoint": entrypoint }))
            }
            TelemetryEvent::PtyThroughput {
                max_bytes_per_second,
            } => Some(json!({
                "max_bytes_per_second": max_bytes_per_second,
            })),
            TelemetryEvent::DuplicateObject(object_type) => {
                Some(json!({ "object_type": object_type }))
            }
            TelemetryEvent::ExportObject(object_type) => {
                Some(json!({ "object_type": object_type }))
            }
            TelemetryEvent::GenerateBlockSharingLink {
                share_type,
                display_setting,
                show_prompt,
                redact_secrets,
            } => Some(
                json!({"share_type": share_type, "display_setting": display_setting, "show_prompt": show_prompt, "redact_secrets": redact_secrets}),
            ),
            TelemetryEvent::CopyBlockSharingLink(share_type) => {
                Some(json!({ "share_type": share_type }))
            }
            TelemetryEvent::PageUpDownInEditorPressed {
                is_empty_editor,
                is_down,
            } => Some(json!({"is_empty_editor": is_empty_editor, "is_down": is_down})),
            TelemetryEvent::StartedSharingCurrentSession {
                includes_scrollback,
                source,
            } => Some(json!({ "includes_scrollback": includes_scrollback, "source": source })),
            TelemetryEvent::StoppedSharingCurrentSession { source, reason } => {
                Some(json!({ "source": source, "reason": reason }))
            }
            TelemetryEvent::UnsupportedShell { shell } => Some(json!({ "shell": shell })),
            TelemetryEvent::CopyObjectToClipboard(object_type) => {
                Some(json!({ "object_type": object_type }))
            }
            TelemetryEvent::OpenAndWarpifyDockerSubshell { shell_type } => {
                Some(json!({ "shell_type": shell_type }))
            }
            TelemetryEvent::ToggleBlockFilterQuery { enabled, source } => {
                Some(json!({"enabled": enabled, "source": source}))
            }
            TelemetryEvent::ToggleBlockFilterRegex { enabled } => {
                Some(json!({ "enabled": enabled }))
            }
            TelemetryEvent::ToggleShowBlockDividers { enabled } => {
                Some(json!({ "enabled": enabled }))
            }
            TelemetryEvent::ToggleBlockFilterCaseSensitivity { enabled } => {
                Some(json!({ "enabled": enabled }))
            }
            TelemetryEvent::ToggleBlockFilterInvert { enabled } => {
                Some(json!({ "enabled": enabled }))
            }
            TelemetryEvent::UpdateBlockFilterQueryContextLines { num_context_lines } => {
                Some(json!({ "num_context_lines": num_context_lines }))
            }
            TelemetryEvent::ToggleNewWindowsAtCustomSize { enabled } => {
                Some(json!({"enabled": enabled}))
            }
            TelemetryEvent::ToggleSshTmuxWrapper { enabled } => Some(json!({"enabled": enabled})),
            TelemetryEvent::ToggleSshWarpification { enabled } => Some(json!({"enabled": enabled})),
            TelemetryEvent::SetSshExtensionInstallMode { mode } => Some(json!({"mode": mode})),
            TelemetryEvent::SshRemoteServerChoiceDoNotAskAgainToggled { checked } => {
                Some(json!({"checked": checked}))
            }
            TelemetryEvent::SshInteractiveSessionDetected(ssh_interactive_session_detected) => {
                Some(json!({"ssh_interactive_session": ssh_interactive_session_detected}))
            }
            TelemetryEvent::SshTmuxWarpificationSuccess {
                duration_ms,
                tmux_installation,
            } => Some(json!({
                "duration_ms": duration_ms,
                "tmux_installation": *tmux_installation,
            })),
            TelemetryEvent::SshTmuxWarpificationErrorBlock {
                error,
                tmux_installation,
            } => Some(json!({
                "error": error,
                "tmux_installation": *tmux_installation,
            })),
            TelemetryEvent::JoinedSharedSession {
                session_id,
                source_type,
            } => Some(json!({
                "session_id": session_id,
                "source_type": source_type,
            })),
            TelemetryEvent::SharerCancelledGrantRole { role } => Some(json!({ "role": role })),
            TelemetryEvent::JumpToSharedSessionParticipant { jumped_to } => {
                Some(json!({ "jumped_to": jumped_to }))
            }
            TelemetryEvent::CopiedSharedSessionLink { source } => Some(json!({ "source": source })),
            TelemetryEvent::WebSessionOpenedOnDesktop { source } => {
                Some(json!({ "source": source}))
            }
            TelemetryEvent::WebCloudObjectOpenedOnDesktop { object_metadata } => Some(json!({
                "object": object_metadata,
            })),
            TelemetryEvent::ToggleSnackbarInActivePane { show_snackbar } => {
                Some(json!({ "show_snackbar": show_snackbar }))
            }
            TelemetryEvent::PaneDropped { drop_location } => {
                Some(json!({ "location": drop_location }))
            }
            TelemetryEvent::InviteTeammates {
                num_teammates,
                team_uid,
            } => Some(json!({"num_teammates": num_teammates, "team_uid": team_uid})),
            TelemetryEvent::AgentModeCreatedAIBlock {
                client_exchange_id,
                server_output_id,
                was_autodetected_ai_query,
                time_to_first_token_ms,
                time_to_last_token_ms,
                was_user_facing_error,
                cancelled,
                conversation_id,
                is_udi_enabled,
            } => Some(json!({
                "client_exchange_id": client_exchange_id,
                "server_output_id": server_output_id,
                "was_autodetected_ai_query": was_autodetected_ai_query,
                "time_to_first_token_ms": time_to_first_token_ms,
                "time_to_last_token_ms": time_to_last_token_ms,
                "was_user_facing_error": was_user_facing_error,
                "cancelled": cancelled,
                "conversation_id": conversation_id,
                "is_udi_enabled": is_udi_enabled,
            })),
            TelemetryEvent::TierLimitHit(event) => Some(json!(event)),
            TelemetryEvent::AgentModeUserAttemptedQueryAtRequestLimit { limit } => {
                Some(json!({"limit": limit}))
            }
            TelemetryEvent::AgentModeClickedEntrypoint { entrypoint } => {
                Some(json!({"entrypoint": entrypoint}))
            }
            TelemetryEvent::AgentModeAttachedBlockContext { method } => {
                Some(json!({"method": method}))
            }
            TelemetryEvent::AgentModeToggleAutoDetectionSetting {
                is_autodetection_enabled,
                origin,
            } => Some(
                json!({"is_autodetection_enabled": is_autodetection_enabled, "origin": origin }),
            ),
            TelemetryEvent::ToggleIntelligentAutosuggestionsSetting {
                is_intelligent_autosuggestions_enabled,
            } => Some(
                json!({"is_intelligent_autosuggestions_enabled": is_intelligent_autosuggestions_enabled}),
            ),
            // Using legacy name to avoid breaking telemetry.
            TelemetryEvent::TogglePromptSuggestionsSetting {
                is_prompt_suggestions_enabled,
            } => Some(
                json!({"is_agent_mode_query_suggestions_enabled": is_prompt_suggestions_enabled}),
            ),
            TelemetryEvent::ToggleCodeSuggestionsSetting {
                source,
                is_code_suggestions_enabled,
            } => Some(
                json!({"source": source, "is_code_suggestions_enabled": is_code_suggestions_enabled}),
            ),
            TelemetryEvent::ToggleNaturalLanguageAutosuggestionsSetting {
                is_natural_language_autosuggestions_enabled,
            } => Some(
                json!({"is_natural_language_autosuggestions_enabled": is_natural_language_autosuggestions_enabled}),
            ),
            TelemetryEvent::ToggleSharedBlockTitleGenerationSetting {
                is_shared_block_title_generation_enabled,
            } => Some(
                json!({"is_shared_block_title_generation_enabled": is_shared_block_title_generation_enabled}),
            ),
            TelemetryEvent::ToggleGitOperationsAutogenSetting {
                is_git_operations_autogen_enabled,
            } => Some(
                json!({"is_git_operations_autogen_enabled": is_git_operations_autogen_enabled}),
            ),
            TelemetryEvent::ToggleVoiceInputSetting {
                is_voice_input_enabled,
            } => Some(json!({"is_voice_input_enabled": is_voice_input_enabled})),
            TelemetryEvent::AgentModePotentialAutoDetectionFalsePositive(
                AgentModeAutoDetectionFalsePositivePayload::InternalDogfoodUsers { input_text },
            ) => Some(json!({"input_text": input_text})),
            TelemetryEvent::AgentModeChangedInputType {
                input,
                buffer_length,
                is_manually_changed,
                new_input_type,
                active_block_id,
                is_udi_enabled,
            } => Some(
                json!({"input": input, "buffer_length": buffer_length, "is_manually_changed": is_manually_changed, "new_input_type": new_input_type, "active_block_id": active_block_id, "is_udi_enabled": is_udi_enabled}),
            ),
            TelemetryEvent::AgentModePrediction {
                was_suggestion_accepted,
                request_duration_ms,
                is_from_ai,
                does_actual_command_match_prediction,
                does_actual_command_match_history_prediction,
                history_prediction_likelihood,
                total_history_count,
                actual_next_command_run,
                history_based_autosuggestion_state,
                generate_ai_input_suggestions_request,
                generate_ai_input_suggestions_response,
            } => {
                let (history_command_prediction, history_command_prediction_likelihood) =
                    if let Some(state) = history_based_autosuggestion_state {
                        (
                            Some(state.history_command_prediction.clone()),
                            Some(state.history_command_prediction_likelihood),
                        )
                    } else {
                        (None, None)
                    };

                Some(json!({
                    "was_suggestion_accepted": was_suggestion_accepted,
                    "request_duration_ms": request_duration_ms,
                    "is_from_ai": is_from_ai,
                    "does_actual_command_match_prediction": does_actual_command_match_prediction,
                    "does_actual_command_match_history_prediction": does_actual_command_match_history_prediction,
                    "history_prediction_likelihood": history_prediction_likelihood,
                    "total_history_count": total_history_count,
                    "actual_next_command_run": actual_next_command_run,
                    "generate_ai_input_suggestions_request": generate_ai_input_suggestions_request,
                    "generate_ai_input_suggestions_response": generate_ai_input_suggestions_response,
                    "history_command_prediction": history_command_prediction,
                    "history_command_prediction_likelihood": history_command_prediction_likelihood,
                }))
            }
            TelemetryEvent::PromptSuggestionShown {
                id,
                request_duration_ms,
                block_id,
                view,
                server_request_token,
            } => Some(json!({
                "id": id,
                "request_duration_ms": request_duration_ms,
                "block_id": block_id,
                "view": view,
                "server_request_token": server_request_token,
            })),
            TelemetryEvent::SuggestedCodeDiffBannerShown {
                prompt_suggestion_id,
                code_exchange_id,
                block_id,
                request_duration_ms,
                server_request_token,
            } => Some(json!({
                "prompt_suggestion_id": prompt_suggestion_id,
                "code_exchange_id": code_exchange_id,
                "block_id": block_id,
                "request_duration_ms": request_duration_ms,
                "server_request_token": server_request_token,
            })),
            TelemetryEvent::SuggestedCodeDiffFailed {
                prompt_suggestion_id,
                reason,
            } => Some(json!({
                "prompt_suggestion_id": prompt_suggestion_id,
                "reason": reason,
            })),
            TelemetryEvent::PromptSuggestionAccepted {
                id,
                view,
                interaction_source,
            } => Some(json!({
                "id": id,
                "view": view,
                "interaction_source": interaction_source,
            })),
            TelemetryEvent::StaticPromptSuggestionsBannerShown {
                id,
                query,
                block_id,
                block_command,
                static_prompt_suggestion_name,
                request_duration_ms,
                view,
            } => Some(json!({
                "id": id,
                "query": query,
                "block_id": block_id,
                "block_command": block_command,
                "static_prompt_suggestion_name": static_prompt_suggestion_name,
                "request_duration_ms": request_duration_ms,
                "view": view,
            })),
            TelemetryEvent::StaticPromptSuggestionAccepted {
                id,
                view,
                interaction_source,
            } => Some(json!({
                "id": id,
                "view": view,
                "interaction_source": interaction_source,
            })),
            TelemetryEvent::ZeroStatePromptSuggestionUsed {
                suggestion_type,
                triggered_from,
            } => Some(json!({"type": suggestion_type, "triggered_from": triggered_from})),
            TelemetryEvent::UnitTestSuggestionShown { identifiers } => Some(json!({
                "server_output_id": identifiers.server_output_id,
                "exchange_id": identifiers.client_exchange_id,
                "conversation_id": identifiers.server_conversation_id,
            })),
            TelemetryEvent::UnitTestSuggestionAccepted {
                identifiers,
                query,
                interaction_source,
            } => Some(json!({
                "server_output_id": identifiers.server_output_id,
                "exchange_id": identifiers.client_exchange_id,
                "conversation_id": identifiers.server_conversation_id,
                "query": query,
                "interaction_source": interaction_source,
            })),
            TelemetryEvent::UnitTestSuggestionCancelled {
                identifiers,
                interaction_source,
            } => Some(json!({
                "server_output_id": identifiers.server_output_id,
                "exchange_id": identifiers.client_exchange_id,
                "conversation_id": identifiers.server_conversation_id,
                "interaction_source": interaction_source,
            })),
            TelemetryEvent::AgentModeCodeSuggestionEditedByUser { output_id } => {
                Some(json!({"output_id": output_id}))
            }
            TelemetryEvent::AgentModeCodeFilesNavigated { output_id, source } => {
                Some(json!({"output_id": output_id, "source": source}))
            }
            TelemetryEvent::AgentModeCodeDiffHunksNavigated { output_id } => {
                Some(json!({"output_id": output_id}))
            }
            TelemetryEvent::ResourceUsageStats { cpu, mem } => Some(json!({
                "cpu": cpu,
                "mem": {
                    // Only report the total application usage; skip sending
                    // the additional, more detailed usage information.
                    "total_application_usage_bytes": mem.total_application_usage_bytes,
                },
            })),
            TelemetryEvent::MemoryUsageStats {
                total_application_usage_bytes,
                total_blocks,
                total_lines,
                active_block_stats,
                inactive_5m_stats,
                inactive_1h_stats,
                inactive_24h_stats,
            } => Some(json!({
                "total_application_usage_bytes": total_application_usage_bytes,
                "total_blocks": total_blocks,
                "total_lines": total_lines,
                "active_block_stats": active_block_stats,
                "inactive_5m_stats": inactive_5m_stats,
                "inactive_1h_stats": inactive_1h_stats,
                "inactive_24h_stats": inactive_24h_stats
            })),
            TelemetryEvent::MemoryUsageHigh {
                total_application_usage_bytes,
                memory_breakdown,
            } => Some(json!({
                "total_application_usage_bytes": total_application_usage_bytes,
                "memory_breakdown": memory_breakdown,
            })),
            TelemetryEvent::EnvVarCollectionInvoked(metadata) => Some(json!(metadata)),
            TelemetryEvent::EnvVarWorkflowParameterization(metadata) => Some(json!(metadata)),
            TelemetryEvent::CompletedSettingsImport {
                terminal_type,
                imported_settings,
            } => Some(
                json!({ "terminal_type": terminal_type, "imported_settings": imported_settings}),
            ),
            TelemetryEvent::SettingsImportConfigParsed {
                timing_data,
                terminal_type,
                settings_shown_to_user,
            } => Some(
                json!({"timing_data": timing_data,  "terminal_type": terminal_type, "settings_shown_to_user": settings_shown_to_user}),
            ),
            TelemetryEvent::SettingsImportConfigFocused(terminal_type_and_profile) => {
                Some(json!({"terminal_and_type_profile": terminal_type_and_profile}))
            }
            TelemetryEvent::InitiateAnonymousUserSignup { entrypoint } => {
                Some(json!({"entrypoint": entrypoint}))
            }
            TelemetryEvent::AnonymousUserAttemptLoginGatedFeature { feature } => {
                Some(json!({"feature": feature}))
            }
            TelemetryEvent::ToggleWorkspaceDecorationVisibility {
                previous_value,
                new_value,
            } => Some(json!({
                "previous_value": previous_value,
                "new_value": new_value,
            })),
            TelemetryEvent::UpdateAltScreenPaddingMode { new_mode } => Some(json!({
                "new_mode": new_mode,
            })),
            TelemetryEvent::AddTabWithShell { source, shell } => {
                Some(json!({ "source": source, "shell": shell }))
            }
            TelemetryEvent::AgentModeSurfacedCitations {
                citations,
                block_id,
                conversation_id,
                server_output_id,
            } => Some(
                json!({ "citations": citations, "block_id": block_id, "conversation_id": conversation_id, "server_output_id": server_output_id }),
            ),
            TelemetryEvent::AgentModeOpenedCitation {
                citation,
                block_id,
                conversation_id,
                server_output_id,
            } => Some(
                json!({ "citation": citation, "block_id": block_id, "conversation_id": conversation_id, "server_output_id": server_output_id }),
            ),
            TelemetryEvent::OpenedSharingDialog(event) => Some(json!(event)),
            TelemetryEvent::ToggleGlobalAI { is_ai_enabled } => {
                Some(json!({"is_ai_enabled": is_ai_enabled}))
            }
            TelemetryEvent::ToggleActiveAI {
                is_active_ai_enabled,
            } => Some(json!({"is_active_ai_enabled": is_active_ai_enabled})),
            TelemetryEvent::ToggleCodebaseContext {
                is_codebase_context_enabled,
            } => Some(json!( {
                "is_codebase_context_enabled": is_codebase_context_enabled
            })),
            TelemetryEvent::ToggleAutoIndexing {
                is_autoindexing_enabled,
            } => Some(json!({
                "is_autoindexing_enabled": is_autoindexing_enabled
            })),
            TelemetryEvent::ActiveIndexedReposChanged {
                updated_number_of_codebase_indices,
                hit_max_indices,
            } => Some(json!({
                "updated_number_of_codebase_indices": updated_number_of_codebase_indices,
                "hit_max_indices": hit_max_indices
            })),
            TelemetryEvent::ToggleLigatureRendering { enabled } => {
                Some(json!({"enabled": enabled}))
            }
            TelemetryEvent::WorkflowAliasAdded {
                workflow_id,
                workflow_space,
            } => Some(json!({
                "workflow_id": workflow_id,
                "workflow_space": workflow_space,
            })),
            TelemetryEvent::WorkflowAliasRemoved {
                workflow_id,
                workflow_space,
            } => Some(json!({
                "workflow_id": workflow_id,
                "workflow_space": workflow_space,
            })),
            TelemetryEvent::WorkflowAliasArgumentEdited {
                workflow_id,
                workflow_space,
            } => Some(json!({
                "workflow_id": workflow_id,
                "workflow_space": workflow_space,
            })),
            TelemetryEvent::WorkflowAliasEnvVarsAttached {
                workflow_id,
                workflow_space,
                env_vars_id,
                env_vars_space,
            } => Some(json!({
                "workflow_id": workflow_id,
                "workflow_space": workflow_space,
                "env_vars_id": env_vars_id,
                "env_vars_space": env_vars_space,
            })),
            TelemetryEvent::AutoupdateRelaunchAttempt { new_version } => Some(json!({
                "new_version": new_version,
            })),
            TelemetryEvent::ToggledAgentModeAutoexecuteReadonlyCommandsSetting { src, enabled } => {
                Some(json!({
                    "source": src,
                    "enabled": enabled,
                }))
            }
            TelemetryEvent::ChangedAgentModeCodingPermissions { src, new } => Some(json!({
                "source": src,
                "new": new,
            })),
            TelemetryEvent::ChangedAgentModeAskUserQuestionPermission { src, new } => Some(json!({
                "source": src,
                "new": new,
            })),
            TelemetryEvent::FullEmbedCodebaseContextSearchSuccess {
                action_id,
                total_search_duration,
                out_of_sync_delay,
            } => Some(json!({
                "action_id": action_id,
                "total_search_duration": total_search_duration,
                "out_of_sync_delay": out_of_sync_delay
            })),
            TelemetryEvent::FullEmbedCodebaseContextSearchFailed { action_id, error } => {
                Some(json!({
                    "action_id": action_id,
                    "error": error
                }))
            }
            TelemetryEvent::RepoOutlineConstructionSuccess {
                total_parse_seconds,
                file_count,
            } => Some(json!({
                "total_parse_seconds": total_parse_seconds,
                "file_count": file_count,
            })),
            TelemetryEvent::RepoOutlineConstructionFailed { error } => Some(json!({
                "error": error,
            })),
            TelemetryEvent::AutoexecutedAgentModeRequestedCommand { reason } => Some(json!({
                "reason": reason,
            })),
            TelemetryEvent::AgenticOnboardingBlockSelected { block_type } => Some(json!({
                "block_type": block_type,
            })),
            TelemetryEvent::AttachedImagesToAgentModeQuery {
                num_images,
                is_udi_enabled,
            } => Some(json!({
                "num_images": num_images,
                "is_udi_enabled": is_udi_enabled,
            })),
            TelemetryEvent::AgentModeRatedResponse {
                server_output_id,
                conversation_id,
                rating,
            } => Some(json!({
                "server_output_id": server_output_id,
                "conversation_id": conversation_id,
                "rating": rating,
            })),
            TelemetryEvent::ExecutedWarpDrivePrompt {
                id,
                selection_source,
            } => Some(json!({
                "id": id,
                "selection_source": selection_source,
            })),
            TelemetryEvent::ImageReceived { image_protocol } => Some(json!({
                "image_protocol": image_protocol,
            })),
            TelemetryEvent::FileExceededContextLimit { identifiers } => Some(json!({
                "server_output_id": identifiers.server_output_id,
                "exchange_id": identifiers.client_exchange_id,
                "conversation_id": identifiers.server_conversation_id,
            })),
            TelemetryEvent::AgentModeError {
                identifiers,
                error,
                is_user_visible,
                will_attempt_to_resume,
            } => Some(json!({
                "server_output_id": identifiers.server_output_id,
                "exchange_id": identifiers.client_exchange_id,
                "conversation_id": identifiers.server_conversation_id,
                "error": error,
                "is_user_visible": is_user_visible,
                "will_attempt_to_resume": will_attempt_to_resume,
            })),
            TelemetryEvent::AgentModeRequestRetrySucceeded {
                identifiers,
                retry_count,
                original_error,
            } => Some(json!({
                "server_output_id": identifiers.server_output_id,
                "exchange_id": identifiers.client_exchange_id,
                "conversation_id": identifiers.server_conversation_id,
                "retry_count": retry_count,
                "original_error": original_error,
            })),
            TelemetryEvent::GrepToolFailed {
                queries,
                path,
                shell_type,
                working_directory,
                absolute_path,
                command,
                output,
                error,
                server_output_id,
            } => Some(json!({
                "queries": queries,
                "path": path,
                "shell_type": shell_type,
                "working_directory": working_directory,
                "absolute_path": absolute_path,
                "command": command,
                "output": output,
                "error": error,
                "server_output_id": server_output_id,
            })),
            TelemetryEvent::FileGlobToolFailed { server_output_id } => Some(json!({
                "server_output_id": server_output_id,
            })),
            TelemetryEvent::ShellTerminatedPrematurely {
                shell_type,
                shell_path,
                reason,
                reason_details,
                antivirus_name,
                long_os_version,
                exit_reason,
            } => Some(json!({
                "shell_type": shell_type,
                "shell_path": shell_path,
                "reason": reason,
                "reason_details": reason_details,
                "antivirus_name": antivirus_name,
                "long_os_version": long_os_version,
                "exit_reason": exit_reason,
            })),
            TelemetryEvent::SearchCodebaseRequested {
                action_id,
                server_output_id,
                is_cross_repo,
            } => Some(json!({
                "action_id": action_id,
                "server_output_id": server_output_id,
                "is_cross_repo": is_cross_repo,
            })),
            TelemetryEvent::SearchCodebaseRepoUnavailable { action_id, error } => Some(json!({
                "action_id": action_id,
                "error": error,
            })),
            TelemetryEvent::InputUXModeChanged {
                is_udi_enabled,
                origin,
            } => Some(json!({
                "is_udi_enabled": is_udi_enabled,
                "origin": origin,
            })),
            TelemetryEvent::ContextChipInteracted {
                chip_type,
                action,
                is_udi_enabled,
            } => Some(json!({
                "chip_type": chip_type,
                "action": action,
                "is_udi_enabled": is_udi_enabled,
            })),
            TelemetryEvent::VoiceInputUsed {
                action,
                session_duration_ms,
                is_udi_enabled,
                current_input_mode,
            } => Some(json!({
                "action": action,
                "session_duration_ms": session_duration_ms,
                "is_udi_enabled": is_udi_enabled,
                "current_input_mode": current_input_mode,
            })),
            TelemetryEvent::AtMenuInteracted {
                action,
                query_length,
                item_count,
                is_udi_enabled,
                current_input_mode,
            } => Some(json!({
                "action": action,
                "query_length": query_length,
                "item_count": item_count,
                "is_udi_enabled": is_udi_enabled,
                "current_input_mode": current_input_mode,
            })),
            TelemetryEvent::TabCloseButtonPositionUpdated { position } => Some(json!({
                "position": position,
            })),
            TelemetryEvent::ExpandedCodeSuggestions { identifiers } => Some(json!({
                "server_output_id": identifiers.server_output_id,
                "exchange_id": identifiers.client_exchange_id,
                "conversation_id": identifiers.server_conversation_id,
            })),
            TelemetryEvent::BackgroundBlockStarted
            | TelemetryEvent::SessionCreation
            | TelemetryEvent::Login
            | TelemetryEvent::ContextMenuInsertSelectedText
            | TelemetryEvent::ContextMenuCopySelectedText
            | TelemetryEvent::JumpToPreviousCommand
            | TelemetryEvent::TabSingleResultAutocompletion
            | TelemetryEvent::CopyInviteLink
            | TelemetryEvent::OpenThemeChooser
            | TelemetryEvent::OpenThemeCreatorModal
            | TelemetryEvent::CreateCustomTheme
            | TelemetryEvent::DeleteCustomTheme
            | TelemetryEvent::SplitPane
            | TelemetryEvent::UnableToAutoUpdateToNewVersion
            | TelemetryEvent::SkipOnboardingSurvey
            | TelemetryEvent::LoggedOutStartup
            | TelemetryEvent::OpenWorkflowSearch
            | TelemetryEvent::OpenQuakeModeWindow
            | TelemetryEvent::OpenWelcomeTips
            | TelemetryEvent::DismissWelcomeTips
            | TelemetryEvent::ShowNotificationsDiscoveryBanner
            | TelemetryEvent::ShowNotificationsErrorBanner
            | TelemetryEvent::NotificationClicked
            | TelemetryEvent::SignUpButtonClicked
            | TelemetryEvent::OpenNewSessionFromFilePath
            | TelemetryEvent::OpenTeamFromURI
            | TelemetryEvent::SelectNavigationPaletteItem
            | TelemetryEvent::DragAndDropTab
            | TelemetryEvent::EditedInputBeforePrecmd
            | TelemetryEvent::TriedToExecuteBeforePrecmd
            | TelemetryEvent::JumpToBookmark
            | TelemetryEvent::JumpToBottomofBlockButtonClicked
            | TelemetryEvent::ShowInFileExplorer
            | TelemetryEvent::OpenLaunchConfigSaveModal
            | TelemetryEvent::OpenLaunchConfigFile
            | TelemetryEvent::TeamCreated
            | TelemetryEvent::TeamJoined
            | TelemetryEvent::TeamLeft
            | TelemetryEvent::TeamLinkCopied
            | TelemetryEvent::RemovedUserFromTeam
            | TelemetryEvent::DeletedWorkflow
            | TelemetryEvent::DeletedNotebook
            | TelemetryEvent::ToggleApprovalsModal
            | TelemetryEvent::ChangedInviteViewOption(_)
            | TelemetryEvent::SendEmailInvites
            | TelemetryEvent::ResourceCenterOpened
            | TelemetryEvent::ResourceCenterTipsCompleted
            | TelemetryEvent::ResourceCenterTipsSkipped
            | TelemetryEvent::KeybindingsPageOpened
            | TelemetryEvent::OpenedAltScreenFind
            | TelemetryEvent::QuitModalDisabled
            | TelemetryEvent::UserInitiatedLogOut
            | TelemetryEvent::LogOutModalShown
            | TelemetryEvent::WarpAICharacterLimitExceeded
            | TelemetryEvent::OpenInputContextMenu
            | TelemetryEvent::InputCutSelectedText
            | TelemetryEvent::InputCopySelectedText
            | TelemetryEvent::InputSelectAll
            | TelemetryEvent::InputPaste
            | TelemetryEvent::InputCommandSearch
            | TelemetryEvent::InputAICommandSearch
            | TelemetryEvent::InputAskWarpAI
            | TelemetryEvent::SetNewWindowsAtCustomSize
            | TelemetryEvent::DisableInputSync
            | TelemetryEvent::ShowSubshellBanner
            | TelemetryEvent::SshTmuxWarpifyBannerDisplayed
            | TelemetryEvent::AddDenylistedSubshellCommand
            | TelemetryEvent::RemoveDenylistedSubshellCommand
            | TelemetryEvent::AddAddedSubshellCommand
            | TelemetryEvent::RemoveAddedSubshellCommand
            | TelemetryEvent::ReceivedSubshellRcFileDcs
            | TelemetryEvent::AddDenylistedSshTmuxWrapperHost
            | TelemetryEvent::RemoveDenylistedSshTmuxWrapperHost
            | TelemetryEvent::SshTmuxWarpifyBlockAccepted
            | TelemetryEvent::SshTmuxWarpifyBlockDismissed
            | TelemetryEvent::SshInstallTmuxBlockDisplayed
            | TelemetryEvent::SshInstallTmuxBlockAccepted
            | TelemetryEvent::SshInstallTmuxBlockDismissed
            | TelemetryEvent::ShowAliasExpansionBanner
            | TelemetryEvent::EnableAliasExpansionFromBanner
            | TelemetryEvent::DismissAliasExpansionBanner
            | TelemetryEvent::ShowVimKeybindingsBanner
            | TelemetryEvent::EnableVimKeybindingsFromBanner
            | TelemetryEvent::DismissVimKeybindingsBanner
            | TelemetryEvent::InitiateReauth
            | TelemetryEvent::NeedsReauth
            | TelemetryEvent::AnonymousUserExpirationLockout
            | TelemetryEvent::AnonymousUserLinkedFromBrowser
            | TelemetryEvent::AnonymousUserHitCloudObjectLimit
            | TelemetryEvent::CustomSecretRegexAdded
            | TelemetryEvent::CopySecret
            | TelemetryEvent::AutoGenerateMetadataSuccess
            | TelemetryEvent::CommandFileRun
            | TelemetryEvent::SharerGrantModalDontShowAgain
            | TelemetryEvent::LogOut
            | TelemetryEvent::UpdateBlockFilterQuery
            | TelemetryEvent::BlockFilterToolbeltButtonClicked
            | TelemetryEvent::PaneDragInitiated
            | TelemetryEvent::SharedObjectLimitHitBannerViewPlansButtonClicked
            | TelemetryEvent::SharedSessionModalUpgradePressed
            | TelemetryEvent::AgentModePotentialAutoDetectionFalsePositive(
                AgentModeAutoDetectionFalsePositivePayload::ExternalUsers,
            )
            | TelemetryEvent::SettingsImportResetButtonClicked
            | TelemetryEvent::ITermMultipleHotkeys
            | TelemetryEvent::DriveSharingOnboardingBlockShown
            | TelemetryEvent::SettingsImportInitiated
            | TelemetryEvent::GrepToolSucceeded
            | TelemetryEvent::FileGlobToolSucceeded
            | TelemetryEvent::UserMenuUpgradeClicked
            | TelemetryEvent::AIExecutionProfileCreated
            | TelemetryEvent::AIExecutionProfileDeleted
            | TelemetryEvent::FileTreeItemCreated
            | TelemetryEvent::ConversationListItemDeleted
            | TelemetryEvent::ConversationListViewOpened
            | TelemetryEvent::GlobalSearchOpened
            | TelemetryEvent::GlobalSearchQueryStarted
            | TelemetryEvent::GetStartedSkipToTerminal => None,
            TelemetryEvent::SSHControlMasterError { has_remote_server } => Some(json!({
                "has_remote_server": has_remote_server,
            })),
            TelemetryEvent::RemoteServerBinaryCheck {
                found,
                error,
                remote_os,
                remote_arch,
            } => Some(json!({
                "found": found,
                "error": error,
                "remote_os": remote_os,
                "remote_arch": remote_arch,
            })),
            TelemetryEvent::RemoteServerInstallation {
                error,
                install_source,
                remote_os,
                remote_arch,
            } => Some(json!({
                "error": error,
                "install_source": install_source,
                "remote_os": remote_os,
                "remote_arch": remote_arch,
            })),
            TelemetryEvent::RemoteServerInitialization {
                phase,
                error,
                remote_os,
                remote_arch,
                exit_code,
                signal_killed,
            } => Some(json!({
                "phase": phase,
                "error": error,
                "remote_os": remote_os,
                "remote_arch": remote_arch,
                "exit_code": exit_code,
                "signal_killed": signal_killed,
            })),
            TelemetryEvent::RemoteServerDisconnection {
                remote_os,
                remote_arch,
            } => Some(json!({
                "remote_os": remote_os,
                "remote_arch": remote_arch,
            })),
            TelemetryEvent::RemoteServerReconnection {
                attempt,
                remote_os,
                remote_arch,
            } => Some(json!({
                "attempt": attempt,
                "remote_os": remote_os,
                "remote_arch": remote_arch,
            })),
            TelemetryEvent::RemoteServerReconnectExhausted {
                attempts,
                remote_os,
                remote_arch,
                exit_code,
                signal_killed,
            } => Some(json!({
                "attempts": attempts,
                "remote_os": remote_os,
                "remote_arch": remote_arch,
                "exit_code": exit_code,
                "signal_killed": signal_killed,
            })),
            TelemetryEvent::RemoteServerClientRequestError {
                operation,
                error_type,
                remote_os,
                remote_arch,
            } => Some(json!({
                "operation": operation,
                "error_type": error_type,
                "remote_os": remote_os,
                "remote_arch": remote_arch,
            })),
            TelemetryEvent::RemoteServerMessageDecodingError {
                remote_os,
                remote_arch,
            } => Some(json!({
                "remote_os": remote_os,
                "remote_arch": remote_arch,
            })),
            TelemetryEvent::RemoteServerSetupDuration {
                duration_ms,
                installed_binary,
                remote_os,
                remote_arch,
                remote_libc,
            } => Some(json!({
                "duration_ms": duration_ms,
                "installed_binary": installed_binary,
                "remote_os": remote_os,
                "remote_arch": remote_arch,
                "remote_libc": remote_libc,
            })),
            TelemetryEvent::RemoteServerHostUnsupported {
                remote_os,
                remote_arch,
                unsupported_reason,
                detected_libc,
            } => {
                let unsupported_os = match unsupported_reason {
                    remote_server::setup::UnsupportedReason::UnsupportedOs { os } => {
                        Some(os.clone())
                    }
                    remote_server::setup::UnsupportedReason::GlibcTooOld { .. }
                    | remote_server::setup::UnsupportedReason::NonGlibc { .. }
                    | remote_server::setup::UnsupportedReason::UnsupportedArch { .. } => None,
                };
                let unsupported_arch = match unsupported_reason {
                    remote_server::setup::UnsupportedReason::UnsupportedArch { arch } => {
                        Some(arch.clone())
                    }
                    remote_server::setup::UnsupportedReason::GlibcTooOld { .. }
                    | remote_server::setup::UnsupportedReason::NonGlibc { .. }
                    | remote_server::setup::UnsupportedReason::UnsupportedOs { .. } => None,
                };
                Some(json!({
                    "remote_os": remote_os,
                    "remote_arch": remote_arch,
                    "reason": unsupported_reason.as_telemetry_reason(),
                    "detected_libc": detected_libc,
                    "unsupported_os": unsupported_os,
                    "unsupported_arch": unsupported_arch,
                }))
            }
            TelemetryEvent::ConversationListItemOpened { is_ambient_agent } => Some(json!({
                "is_ambient_agent": is_ambient_agent,
            })),
            TelemetryEvent::ConversationListLinkCopied { is_ambient_agent } => Some(json!({
                "is_ambient_agent": is_ambient_agent,
            })),
            TelemetryEvent::AIExecutionProfileSettingUpdated {
                setting_type,
                setting_value,
            } => Some(json!({
                "setting_type": setting_type,
                "setting_value": setting_value,
            })),
            TelemetryEvent::AIExecutionProfileAddedToAllowlist { list_type, value } => {
                Some(json!({
                    "list_type": list_type,
                    "value": value,
                }))
            }
            TelemetryEvent::AIExecutionProfileAddedToDenylist { list_type, value } => Some(json!({
                "list_type": list_type,
                "value": value,
            })),
            TelemetryEvent::AIExecutionProfileRemovedFromAllowlist { list_type, value } => {
                Some(json!({
                    "list_type": list_type,
                    "value": value,
                }))
            }
            TelemetryEvent::AIExecutionProfileRemovedFromDenylist { list_type, value } => {
                Some(json!({
                    "list_type": list_type,
                    "value": value,
                }))
            }
            TelemetryEvent::AIExecutionProfileModelSelected {
                model_type,
                model_value,
            } => Some(json!({
                "model_type": model_type,
                "model_value": model_value,
            })),
            TelemetryEvent::AIExecutionProfileContextWindowSelected { tokens } => Some(json!({
                "tokens": tokens,
            })),
            TelemetryEvent::AIInputNotSent {
                entrypoint,
                inputs,
                active_server_conversation_id,
                active_client_conversation_id,
            } => Some(json!({
                "entrypoint": entrypoint,
                "inputs": inputs,
                "active_server_conversation_id": active_server_conversation_id,
                "active_client_conversation_id": active_client_conversation_id,
            })),
            TelemetryEvent::OpenSlashMenu {
                source,
                is_inline_ui_enabled,
                is_in_agent_view,
            } => Some(json!({
                "source": source,
                "is_inline_ui_enabled": is_inline_ui_enabled,
                "is_in_agent_view": is_in_agent_view,
            })),
            TelemetryEvent::SlashCommandAccepted {
                command_details,
                is_in_agent_view,
            } => Some(json!({
                "command_details": command_details,
                "is_in_agent_view": is_in_agent_view,
            })),
            TelemetryEvent::AgentModeSetupBannerAccepted => None,
            TelemetryEvent::AgentModeSetupBannerDismissed => None,
            TelemetryEvent::AgentModeSetupProjectScopedRulesAction { action } => Some(json!({
                "action": action,
            })),
            TelemetryEvent::AgentModeSetupCodebaseContextAction { action } => Some(json!({
                "action": action,
            })),
            TelemetryEvent::AgentModeSetupCreateEnvironmentAction { action } => Some(json!({
                "action": action,
            })),
            #[cfg(windows)]
            TelemetryEvent::WSLRegistryError
            | TelemetryEvent::AutoupdateUnableToCloseApplications
            | TelemetryEvent::AutoupdateFileInUse
            | TelemetryEvent::AutoupdateMutexTimeout => None,
            #[cfg(windows)]
            TelemetryEvent::AutoupdateForcekillFailed { exit_code } => Some(json!({
                "exit_code": exit_code,
            })),
            #[cfg(windows)]
            TelemetryEvent::AutoupdateMinidumpCleanupFailed { exit_code } => Some(json!({
                "exit_code": exit_code,
            })),
            TelemetryEvent::InputBufferSubmitted {
                input_type,
                is_locked,
                was_lock_set_with_empty_buffer,
            } => Some(json!({
                "input_type": input_type,
                "is_locked": is_locked,
                "was_lock_set_with_empty_buffer": was_lock_set_with_empty_buffer,
            })),
            TelemetryEvent::CreateProjectPromptSubmitted {
                is_custom_prompt,
                suggested_prompt,
                is_ftux,
            } => Some(json!({
                "is_custom_prompt": is_custom_prompt,
                "suggested_prompt": suggested_prompt,
                "is_ftux": is_ftux,
            })),
            TelemetryEvent::CreateProjectPromptSubmittedContent { custom_prompt } => Some(json!({
                "custom_prompt": custom_prompt
            })),
            TelemetryEvent::CloneRepoPromptSubmitted { is_ftux } => Some(json!({
                "is_ftux": is_ftux,
            })),
            TelemetryEvent::RecentMenuItemSelected { kind } => Some(json!({
                "kind": kind,
            })),
            TelemetryEvent::OpenRepoFolderSubmitted { is_ftux } => Some(json!({
                "is_ftux": is_ftux,
            })),
            TelemetryEvent::OutOfCreditsBannerClosed {
                action,
                selected_credits,
                auto_reload_checkbox_enabled,
                banner_toggle_flag_enabled,
                post_purchase_modal_flag_enabled,
            } => Some(json!({
                "action": action,
                "selected_credits": selected_credits,
                "auto_reload_checkbox_enabled": auto_reload_checkbox_enabled,
                "banner_toggle_flag_enabled": banner_toggle_flag_enabled,
                "post_purchase_modal_flag_enabled": post_purchase_modal_flag_enabled,
            })),
            TelemetryEvent::AutoReloadModalClosed {
                action,
                selected_credits,
                banner_toggle_flag_enabled,
                post_purchase_modal_flag_enabled,
            } => Some(json!({
                "action": action,
                "selected_credits": selected_credits,
                "banner_toggle_flag_enabled": banner_toggle_flag_enabled,
                "post_purchase_modal_flag_enabled": post_purchase_modal_flag_enabled,
            })),
            TelemetryEvent::AutoReloadToggledFromBillingSettings {
                enabled,
                banner_toggle_flag_enabled,
                post_purchase_modal_flag_enabled,
            } => Some(json!({
                "enabled": enabled,
                "banner_toggle_flag_enabled": banner_toggle_flag_enabled,
                "post_purchase_modal_flag_enabled": post_purchase_modal_flag_enabled,
            })),
            TelemetryEvent::WarpDriveOpened {
                source,
                is_code_mode_v2,
            } => Some(json!({
                "source": source,
                "is_code_mode_v2": is_code_mode_v2,
            })),
            TelemetryEvent::AgentTipShown { tip } => Some(json!({
                "tip": tip,
            })),
            TelemetryEvent::AgentTipClicked { tip, click_target } => Some(json!({
                "tip": tip,
                "click_target": click_target,
            })),
            TelemetryEvent::ToggleShowAgentTips { is_enabled } => Some(json!({
                "is_enabled": is_enabled,
            })),
            TelemetryEvent::CLISubagentControlStateChanged {
                conversation_id,
                block_id,
                control_state,
            } => Some(json!({
                "conversation_id": conversation_id,
                "block_id": block_id,
                "control_state": control_state,
            })),
            TelemetryEvent::CLISubagentResponsesToggled {
                conversation_id,
                block_id,
                is_hidden,
            } => Some(json!({
                "conversation_id": conversation_id,
                "block_id": block_id,
                "is_hidden": is_hidden,
            })),
            TelemetryEvent::CLISubagentInputDismissed {
                conversation_id,
                block_id,
            } => Some(json!({
                "conversation_id": conversation_id,
                "block_id": block_id,
            })),
            TelemetryEvent::CLISubagentActionExecuted {
                conversation_id,
                block_id,
                is_autoexecuted,
            } => Some(json!({
                "conversation_id": conversation_id,
                "block_id": block_id,
                "is_autoexecuted": is_autoexecuted,
            })),
            TelemetryEvent::CLISubagentActionRejected {
                conversation_id,
                block_id,
                user_took_over,
            } => Some(json!({
                "conversation_id": conversation_id,
                "block_id": block_id,
                "user_took_over": user_took_over,
            })),
            TelemetryEvent::AgentManagementViewToggled { is_open } => Some(json!({
                "is_open": is_open,
            })),
            TelemetryEvent::AgentManagementViewOpenedSession => None,
            TelemetryEvent::AgentManagementViewCopiedSessionLink => None,
            TelemetryEvent::DetectedIsolationPlatform { platform } => Some(json!({
                "platform": platform,
            })),
            TelemetryEvent::AgentExitedShellProcess {
                command,
                server_output_id,
            } => Some(json!({
                "command": command,
                "server_output_id": server_output_id,
            })),
            TelemetryEvent::CLIAgentToolbarVoiceInputUsed { cli_agent } => Some(json!({
                "agent_name": cli_agent,
            })),
            TelemetryEvent::CLIAgentToolbarImageAttached { cli_agent } => Some(json!({
                "agent_name": cli_agent,
            })),
            TelemetryEvent::CLIAgentToolbarShown { cli_agent } => Some(json!({
                "agent_name": cli_agent,
            })),
            TelemetryEvent::CLIAgentRichInputOpened {
                cli_agent,
                entrypoint,
            } => Some(json!({
                "agent_name": cli_agent,
                "entrypoint": entrypoint,
            })),
            TelemetryEvent::CLIAgentRichInputClosed { cli_agent, reason } => Some(json!({
                "agent_name": cli_agent,
                "reason": reason,
            })),
            TelemetryEvent::CLIAgentRichInputSubmitted {
                cli_agent,
                prompt_length,
            } => Some(json!({
                "agent_name": cli_agent,
                "prompt_length": prompt_length,
            })),
            TelemetryEvent::CLIAgentPluginChipClicked { cli_agent, action } => Some(json!({
                "agent_name": cli_agent,
                "action": action,
            })),
            TelemetryEvent::CLIAgentPluginChipDismissed {
                cli_agent,
                chip_kind,
            } => Some(json!({
                "agent_name": cli_agent,
                "chip_kind": chip_kind,
            })),
            TelemetryEvent::CLIAgentPluginOperationSucceeded {
                cli_agent,
                operation,
            } => Some(json!({
                "agent_name": cli_agent,
                "operation": operation,
            })),
            TelemetryEvent::CLIAgentPluginOperationFailed {
                cli_agent,
                operation,
            } => Some(json!({
                "agent_name": cli_agent,
                "operation": operation,
            })),
            TelemetryEvent::CLIAgentPluginDetected { cli_agent } => Some(json!({
                "agent_name": cli_agent,
            })),
            TelemetryEvent::AgentNotificationShown { agent_variant } => Some(json!({
                "agent_variant": agent_variant,
            })),
            TelemetryEvent::ToggleCLIAgentToolbarSetting { is_enabled } => Some(json!({
                "is_enabled": is_enabled,
            })),
            TelemetryEvent::ToggleUseAgentToolbarSetting { is_enabled } => Some(json!({
                "is_enabled": is_enabled,
            })),
            TelemetryEvent::AgentViewEntered {
                origin,
                did_auto_trigger_request,
            } => Some(json!({
                "origin": origin,
                "did_auto_trigger_request": did_auto_trigger_request,
            })),
            TelemetryEvent::AgentViewExited { origin, was_empty } => Some(json!({
                "origin": origin,
                "was_empty": was_empty,
            })),
            TelemetryEvent::InlineConversationMenuOpened { is_in_agent_view } => Some(json!({
                "is_in_agent_view": is_in_agent_view,
            })),
            TelemetryEvent::InlineConversationMenuItemSelected { is_in_agent_view } => {
                Some(json!({
                    "is_in_agent_view": is_in_agent_view,
                }))
            }
            TelemetryEvent::AgentShortcutsViewToggled { is_visible } => Some(json!({
                "is_visible": is_visible,
            })),
            TelemetryEvent::CodexModalOpened => None,
            TelemetryEvent::CodexModalUseCodexClicked => None,
            TelemetryEvent::LinearIssueLinkOpened => None,
            TelemetryEvent::CloudAgentCapacityModalOpened => None,
            TelemetryEvent::CloudAgentCapacityModalDismissed => None,
            TelemetryEvent::CloudAgentCapacityModalUpgradeClicked => None,
            TelemetryEvent::ComputerUseApproved {
                client_conversation_id,
                server_conversation_id,
                is_autoexecuted,
                ambient_agent_task_id,
            } => Some(json!({
                "client_conversation_id": client_conversation_id,
                "server_conversation_id": server_conversation_id,
                "is_autoexecuted": is_autoexecuted,
                "ambient_agent_task_id": ambient_agent_task_id.map(|id| id.to_string()),
            })),
            TelemetryEvent::ComputerUseCancelled {
                client_conversation_id,
                server_conversation_id,
                ambient_agent_task_id,
            } => Some(json!({
                "client_conversation_id": client_conversation_id,
                "server_conversation_id": server_conversation_id,
                "ambient_agent_task_id": ambient_agent_task_id.map(|id| id.to_string()),
            })),
            TelemetryEvent::FreeTierLimitHitInterstitialDisplayed => None,
            TelemetryEvent::FreeTierLimitHitInterstitialUpgradeButtonClicked => None,
            TelemetryEvent::FreeTierLimitHitInterstitialClosed => None,
            TelemetryEvent::LoginButtonClicked { source }
            | TelemetryEvent::LoginLaterButtonClicked { source }
            | TelemetryEvent::LoginLaterConfirmationButtonClicked { source }
            | TelemetryEvent::OpenAuthPrivacySettings { source } => Some(json!({
                "source": source,
            })),
        }
    }

    /// Returns whether the event contains user generated content, indicating it should
    /// be sent to a dedicated rudderstack source.
    pub fn contains_ugc(&self) -> bool {
        match self {
            TelemetryEvent::GrepToolFailed { .. } => true,
            TelemetryEvent::BootstrappingSlowContents { .. } => true,
            TelemetryEvent::AIInputNotSent { .. } => true,
            TelemetryEvent::AgentExitedShellProcess { .. } => true,
            TelemetryEvent::CreateProjectPromptSubmitted { .. } => false,
            TelemetryEvent::CreateProjectPromptSubmittedContent { .. } => true,
            TelemetryEvent::AgentModePrediction {
                actual_next_command_run,
                history_based_autosuggestion_state,
                generate_ai_input_suggestions_request,
                generate_ai_input_suggestions_response,
                ..
            } => {
                // These fields can contain UGC, so if any are set, assume this event contains UGC.
                actual_next_command_run.is_some()
                    || history_based_autosuggestion_state.is_some()
                    || generate_ai_input_suggestions_request.is_some()
                    || generate_ai_input_suggestions_response.is_some()
            }
            TelemetryEvent::AgentModeChangedInputType { input, .. } => input.is_some(),
            TelemetryEvent::UnitTestSuggestionAccepted { query, .. } => query.is_some(),
            TelemetryEvent::AgentModePotentialAutoDetectionFalsePositive(payload) => {
                // For internal dogfood users, the payload contains UGC.
                matches!(
                    payload,
                    AgentModeAutoDetectionFalsePositivePayload::InternalDogfoodUsers { .. }
                )
            }
            TelemetryEvent::ShowedSuggestedAgentModeWorkflowModal { .. }
            | TelemetryEvent::ShowedSuggestedAgentModeWorkflowChip { .. }
            | TelemetryEvent::AISuggestedAgentModeWorkflowAdded { .. }
            | TelemetryEvent::AutosuggestionInserted { .. }
            | TelemetryEvent::BlockCompleted { .. }
            | TelemetryEvent::BlockCompletedOnDogfoodOnly { .. }
            | TelemetryEvent::BackgroundBlockStarted
            | TelemetryEvent::BaselineCommandLatency(_)
            | TelemetryEvent::SessionCreation
            | TelemetryEvent::Login
            | TelemetryEvent::AgentModeContinueConversationButtonClicked { .. }
            | TelemetryEvent::AgentModeRewindDialogOpened { .. }
            | TelemetryEvent::AgentModeRewindExecuted { .. }
            | TelemetryEvent::OpenSuggestionsMenu(_)
            | TelemetryEvent::ConfirmSuggestion { .. }
            | TelemetryEvent::OpenContextMenu { .. }
            | TelemetryEvent::ContextMenuCopy(_, _)
            | TelemetryEvent::ContextMenuOpenShareModal(_)
            | TelemetryEvent::ContextMenuFindWithinBlocks(_)
            | TelemetryEvent::ContextMenuCopyPrompt { .. }
            | TelemetryEvent::ContextMenuToggleGitPromptDirtyIndicator { .. }
            | TelemetryEvent::ContextMenuInsertSelectedText
            | TelemetryEvent::ContextMenuCopySelectedText
            | TelemetryEvent::OpenPromptEditor { .. }
            | TelemetryEvent::PromptEdited { .. }
            | TelemetryEvent::ReinputCommands(_)
            | TelemetryEvent::JumpToPreviousCommand
            | TelemetryEvent::CopyBlockSharingLink(_)
            | TelemetryEvent::GenerateBlockSharingLink { .. }
            | TelemetryEvent::BlockSelection(_)
            | TelemetryEvent::BootstrappingSlow(_)
            | TelemetryEvent::SessionAbandonedBeforeBootstrap { .. }
            | TelemetryEvent::BootstrappingSucceeded(_)
            | TelemetryEvent::TabSingleResultAutocompletion
            | TelemetryEvent::EditorUnhandledModifierKey(_)
            | TelemetryEvent::CopyInviteLink
            | TelemetryEvent::OpenThemeChooser
            | TelemetryEvent::ThemeSelection { .. }
            | TelemetryEvent::AppIconSelection { .. }
            | TelemetryEvent::CursorDisplayType { .. }
            | TelemetryEvent::OpenThemeCreatorModal
            | TelemetryEvent::CreateCustomTheme
            | TelemetryEvent::DeleteCustomTheme
            | TelemetryEvent::SplitPane
            | TelemetryEvent::UnableToAutoUpdateToNewVersion
            | TelemetryEvent::AutoupdateRelaunchAttempt { .. }
            | TelemetryEvent::SkipOnboardingSurvey
            | TelemetryEvent::ToggleRestoreSession(_)
            | TelemetryEvent::DatabaseStartUpError(_)
            | TelemetryEvent::DatabaseReadError(_)
            | TelemetryEvent::DatabaseWriteError(_)
            | TelemetryEvent::AppStartup(_)
            | TelemetryEvent::LoggedOutStartup
            | TelemetryEvent::DownloadSource(_)
            | TelemetryEvent::SSHBootstrapAttempt(_)
            | TelemetryEvent::SSHControlMasterError { .. }
            | TelemetryEvent::KeybindingChanged { .. }
            | TelemetryEvent::KeybindingResetToDefault { .. }
            | TelemetryEvent::KeybindingRemoved { .. }
            | TelemetryEvent::FeaturesPageAction { .. }
            | TelemetryEvent::WorkflowExecuted(_)
            | TelemetryEvent::WorkflowSelected(_)
            | TelemetryEvent::OpenWorkflowSearch
            | TelemetryEvent::OpenQuakeModeWindow
            | TelemetryEvent::OpenWelcomeTips
            | TelemetryEvent::CompleteWelcomeTipFeature { .. }
            | TelemetryEvent::DismissWelcomeTips
            | TelemetryEvent::ShowNotificationsDiscoveryBanner
            | TelemetryEvent::NotificationsDiscoveryBannerAction(_)
            | TelemetryEvent::ShowNotificationsErrorBanner
            | TelemetryEvent::NotificationsErrorBannerAction(_)
            | TelemetryEvent::NotificationPermissionsRequested { .. }
            | TelemetryEvent::NotificationsRequestPermissionsOutcome { .. }
            | TelemetryEvent::NotificationSent { .. }
            | TelemetryEvent::NotificationFailedToSend { .. }
            | TelemetryEvent::NotificationClicked
            | TelemetryEvent::ToggleFindOption { .. }
            | TelemetryEvent::SignUpButtonClicked
            | TelemetryEvent::LoginButtonClicked { .. }
            | TelemetryEvent::LoginLaterButtonClicked { .. }
            | TelemetryEvent::LoginLaterConfirmationButtonClicked { .. }
            | TelemetryEvent::OpenNewSessionFromFilePath
            | TelemetryEvent::OpenTeamFromURI
            | TelemetryEvent::SelectNavigationPaletteItem
            | TelemetryEvent::SelectCommandPaletteOption(_)
            | TelemetryEvent::PaletteSearchOpened { .. }
            | TelemetryEvent::PaletteSearchResultAccepted { .. }
            | TelemetryEvent::PaletteSearchExited { .. }
            | TelemetryEvent::AuthCommonQuestionClicked { .. }
            | TelemetryEvent::AuthToggleFAQ { .. }
            | TelemetryEvent::OpenAuthPrivacySettings { .. }
            | TelemetryEvent::TabRenamed(_)
            | TelemetryEvent::MoveActiveTab { .. }
            | TelemetryEvent::MoveTab { .. }
            | TelemetryEvent::DragAndDropTab
            | TelemetryEvent::TabOperations { .. }
            | TelemetryEvent::EditedInputBeforePrecmd
            | TelemetryEvent::TriedToExecuteBeforePrecmd
            | TelemetryEvent::ThinStrokesSettingChanged { .. }
            | TelemetryEvent::BookmarkBlockToggled { .. }
            | TelemetryEvent::JumpToBookmark
            | TelemetryEvent::JumpToBottomofBlockButtonClicked
            | TelemetryEvent::ToggleJumpToBottomofBlockButton { .. }
            | TelemetryEvent::ToggleShowBlockDividers { .. }
            | TelemetryEvent::OpenLink { .. }
            | TelemetryEvent::OpenChangelogLink { .. }
            | TelemetryEvent::ShowInFileExplorer
            | TelemetryEvent::CommandXRayTriggered { .. }
            | TelemetryEvent::OpenLaunchConfigSaveModal
            | TelemetryEvent::SaveLaunchConfig { .. }
            | TelemetryEvent::OpenLaunchConfigFile
            | TelemetryEvent::OpenLaunchConfig { .. }
            | TelemetryEvent::TeamCreated
            | TelemetryEvent::TeamJoined
            | TelemetryEvent::TeamLeft
            | TelemetryEvent::ToggleSettingsSync { .. }
            | TelemetryEvent::TeamLinkCopied
            | TelemetryEvent::RemovedUserFromTeam
            | TelemetryEvent::DeletedWorkflow
            | TelemetryEvent::DeletedNotebook
            | TelemetryEvent::ToggleApprovalsModal
            | TelemetryEvent::ChangedInviteViewOption(_)
            | TelemetryEvent::SendEmailInvites
            | TelemetryEvent::CommandCorrection { .. }
            | TelemetryEvent::SetLineHeight { .. }
            | TelemetryEvent::ResourceCenterOpened
            | TelemetryEvent::ResourceCenterTipsCompleted
            | TelemetryEvent::ResourceCenterTipsSkipped
            | TelemetryEvent::KeybindingsPageOpened
            | TelemetryEvent::GlobalSearchOpened
            | TelemetryEvent::GlobalSearchQueryStarted
            | TelemetryEvent::CommandSearchOpened { .. }
            | TelemetryEvent::CommandSearchExited { .. }
            | TelemetryEvent::CommandSearchResultAccepted { .. }
            | TelemetryEvent::CommandSearchFilterChanged { .. }
            | TelemetryEvent::CommandSearchAsyncQueryCompleted { .. }
            | TelemetryEvent::AICommandSearchOpened { .. }
            | TelemetryEvent::OpenNotebook(_)
            | TelemetryEvent::EditNotebook { .. }
            | TelemetryEvent::NotebookAction(_)
            | TelemetryEvent::OpenedAltScreenFind
            | TelemetryEvent::UserInitiatedClose { .. }
            | TelemetryEvent::QuitModalShown { .. }
            | TelemetryEvent::QuitModalCancel { .. }
            | TelemetryEvent::QuitModalDisabled
            | TelemetryEvent::UserInitiatedLogOut
            | TelemetryEvent::LogOutModalShown
            | TelemetryEvent::LogOutModalCancel { .. }
            | TelemetryEvent::SetOpacity { .. }
            | TelemetryEvent::SetBlurRadius { .. }
            | TelemetryEvent::ToggleDimInactivePanes { .. }
            | TelemetryEvent::InputModeChanged { .. }
            | TelemetryEvent::PtySpawned { .. }
            | TelemetryEvent::InitialWorkingDirectoryConfigurationChanged { .. }
            | TelemetryEvent::OpenedWarpAI { .. }
            | TelemetryEvent::WarpAIRequestIssued { .. }
            | TelemetryEvent::WarpAIAction { .. }
            | TelemetryEvent::UsedWarpAIPreparedPrompt { .. }
            | TelemetryEvent::ToggleFocusPaneOnHover { .. }
            | TelemetryEvent::WarpAICharacterLimitExceeded
            | TelemetryEvent::OpenInputContextMenu
            | TelemetryEvent::InputCutSelectedText
            | TelemetryEvent::InputCopySelectedText
            | TelemetryEvent::InputSelectAll
            | TelemetryEvent::InputPaste
            | TelemetryEvent::InputCommandSearch
            | TelemetryEvent::InputAICommandSearch
            | TelemetryEvent::InputAskWarpAI
            | TelemetryEvent::SaveAsWorkflowModal { .. }
            | TelemetryEvent::ExperimentTriggered { .. }
            | TelemetryEvent::ToggleSyncAllPanesInAllTabs { .. }
            | TelemetryEvent::ToggleSyncAllPanesInTab { .. }
            | TelemetryEvent::ToggleSameLinePrompt { .. }
            | TelemetryEvent::ToggleNewWindowsAtCustomSize { .. }
            | TelemetryEvent::SetNewWindowsAtCustomSize
            | TelemetryEvent::DisableInputSync
            | TelemetryEvent::ToggleTabIndicators { .. }
            | TelemetryEvent::TogglePreserveActiveTabColor { .. }
            | TelemetryEvent::ShowSubshellBanner
            | TelemetryEvent::DeclineSubshellBootstrap { .. }
            | TelemetryEvent::TriggerSubshellBootstrap { .. }
            | TelemetryEvent::AddDenylistedSubshellCommand
            | TelemetryEvent::RemoveDenylistedSubshellCommand
            | TelemetryEvent::AddAddedSubshellCommand
            | TelemetryEvent::RemoveAddedSubshellCommand
            | TelemetryEvent::ReceivedSubshellRcFileDcs
            | TelemetryEvent::AddDenylistedSshTmuxWrapperHost
            | TelemetryEvent::RemoveDenylistedSshTmuxWrapperHost
            | TelemetryEvent::ToggleSshTmuxWrapper { .. }
            | TelemetryEvent::SshInteractiveSessionDetected(_)
            | TelemetryEvent::SshTmuxWarpifyBannerDisplayed
            | TelemetryEvent::SshTmuxWarpifyBlockAccepted
            | TelemetryEvent::SshTmuxWarpifyBlockDismissed
            | TelemetryEvent::WarpifyFooterShown { .. }
            | TelemetryEvent::AgentToolbarDismissed
            | TelemetryEvent::WarpifyFooterAcceptedWarpify { .. }
            | TelemetryEvent::SshTmuxWarpificationSuccess { .. }
            | TelemetryEvent::SshTmuxWarpificationErrorBlock { .. }
            | TelemetryEvent::SshInstallTmuxBlockDisplayed
            | TelemetryEvent::SshInstallTmuxBlockAccepted
            | TelemetryEvent::SshInstallTmuxBlockDismissed
            | TelemetryEvent::ShowAliasExpansionBanner
            | TelemetryEvent::EnableAliasExpansionFromBanner
            | TelemetryEvent::DismissAliasExpansionBanner
            | TelemetryEvent::ShowVimKeybindingsBanner
            | TelemetryEvent::EnableVimKeybindingsFromBanner
            | TelemetryEvent::DismissVimKeybindingsBanner
            | TelemetryEvent::InitiateReauth
            | TelemetryEvent::InitiateAnonymousUserSignup { .. }
            | TelemetryEvent::AnonymousUserExpirationLockout
            | TelemetryEvent::AnonymousUserLinkedFromBrowser
            | TelemetryEvent::AnonymousUserAttemptLoginGatedFeature { .. }
            | TelemetryEvent::AnonymousUserHitCloudObjectLimit
            | TelemetryEvent::NeedsReauth
            | TelemetryEvent::WarpDriveOpened { .. }
            | TelemetryEvent::ToggleWarpAI { .. }
            | TelemetryEvent::ToggleSecretRedaction { .. }
            | TelemetryEvent::CustomSecretRegexAdded
            | TelemetryEvent::ToggleObfuscateSecret { .. }
            | TelemetryEvent::CopySecret
            | TelemetryEvent::AutoGenerateMetadataSuccess
            | TelemetryEvent::AutoGenerateMetadataError { .. }
            | TelemetryEvent::UpdateSortingChoice { .. }
            | TelemetryEvent::UndoClose { .. }
            | TelemetryEvent::PtyThroughput { .. }
            | TelemetryEvent::DuplicateObject(_)
            | TelemetryEvent::ExportObject(_)
            | TelemetryEvent::DriveSharingOnboardingBlockShown
            | TelemetryEvent::CommandFileRun
            | TelemetryEvent::PageUpDownInEditorPressed { .. }
            | TelemetryEvent::StartedSharingCurrentSession { .. }
            | TelemetryEvent::StoppedSharingCurrentSession { .. }
            | TelemetryEvent::JoinedSharedSession { .. }
            | TelemetryEvent::SharedSessionModalUpgradePressed
            | TelemetryEvent::SharerCancelledGrantRole { .. }
            | TelemetryEvent::SharerGrantModalDontShowAgain
            | TelemetryEvent::JumpToSharedSessionParticipant { .. }
            | TelemetryEvent::CopiedSharedSessionLink { .. }
            | TelemetryEvent::WebSessionOpenedOnDesktop { .. }
            | TelemetryEvent::WebCloudObjectOpenedOnDesktop { .. }
            | TelemetryEvent::UnsupportedShell { .. }
            | TelemetryEvent::LogOut
            | TelemetryEvent::InviteTeammates { .. }
            | TelemetryEvent::CopyObjectToClipboard(_)
            | TelemetryEvent::OpenAndWarpifyDockerSubshell { .. }
            | TelemetryEvent::UpdateBlockFilterQuery
            | TelemetryEvent::UpdateBlockFilterQueryContextLines { .. }
            | TelemetryEvent::ToggleBlockFilterQuery { .. }
            | TelemetryEvent::ToggleBlockFilterCaseSensitivity { .. }
            | TelemetryEvent::ToggleBlockFilterRegex { .. }
            | TelemetryEvent::ToggleBlockFilterInvert { .. }
            | TelemetryEvent::BlockFilterToolbeltButtonClicked
            | TelemetryEvent::ToggleSnackbarInActivePane { .. }
            | TelemetryEvent::PaneDragInitiated
            | TelemetryEvent::PaneDropped { .. }
            | TelemetryEvent::ObjectLinkCopied { .. }
            | TelemetryEvent::FileTreeToggled { .. }
            | TelemetryEvent::AgentModeUserAttemptedQueryAtRequestLimit { .. }
            | TelemetryEvent::AgentModeClickedEntrypoint { .. }
            | TelemetryEvent::AgentModeAttachedBlockContext { .. }
            | TelemetryEvent::AgentModeToggleAutoDetectionSetting { .. }
            | TelemetryEvent::PromptSuggestionShown { .. }
            | TelemetryEvent::SuggestedCodeDiffBannerShown { .. }
            | TelemetryEvent::SuggestedCodeDiffFailed { .. }
            | TelemetryEvent::PromptSuggestionAccepted { .. }
            | TelemetryEvent::ZeroStatePromptSuggestionUsed { .. }
            | TelemetryEvent::UnitTestSuggestionShown { .. }
            | TelemetryEvent::UnitTestSuggestionCancelled { .. }
            | TelemetryEvent::AgentModeCodeSuggestionEditedByUser { .. }
            | TelemetryEvent::AgentModeCodeFilesNavigated { .. }
            | TelemetryEvent::AgentModeCodeDiffHunksNavigated { .. }
            | TelemetryEvent::ToggleIntelligentAutosuggestionsSetting { .. }
            | TelemetryEvent::ToggleGlobalAI { .. }
            | TelemetryEvent::ToggleCodebaseContext { .. }
            | TelemetryEvent::ToggleAutoIndexing { .. }
            | TelemetryEvent::ToggleActiveAI { .. }
            | TelemetryEvent::TogglePromptSuggestionsSetting { .. }
            | TelemetryEvent::ToggleCodeSuggestionsSetting { .. }
            | TelemetryEvent::ToggleVoiceInputSetting { .. }
            | TelemetryEvent::TierLimitHit(_)
            | TelemetryEvent::SharedObjectLimitHitBannerViewPlansButtonClicked
            | TelemetryEvent::ResourceUsageStats { .. }
            | TelemetryEvent::MemoryUsageStats { .. }
            | TelemetryEvent::MemoryUsageHigh { .. }
            | TelemetryEvent::EnvVarCollectionInvoked(_)
            | TelemetryEvent::EnvVarWorkflowParameterization(_)
            | TelemetryEvent::CompletedSettingsImport { .. }
            | TelemetryEvent::SettingsImportConfigFocused(_)
            | TelemetryEvent::SettingsImportResetButtonClicked
            | TelemetryEvent::SettingsImportConfigParsed { .. }
            | TelemetryEvent::ITermMultipleHotkeys
            | TelemetryEvent::ToggleWorkspaceDecorationVisibility { .. }
            | TelemetryEvent::UpdateAltScreenPaddingMode { .. }
            | TelemetryEvent::AddTabWithShell { .. }
            | TelemetryEvent::AgentModeSurfacedCitations { .. }
            | TelemetryEvent::AgentModeOpenedCitation { .. }
            | TelemetryEvent::OpenedSharingDialog(_)
            | TelemetryEvent::ToggleLigatureRendering { .. }
            | TelemetryEvent::WorkflowAliasAdded { .. }
            | TelemetryEvent::WorkflowAliasRemoved { .. }
            | TelemetryEvent::WorkflowAliasEnvVarsAttached { .. }
            | TelemetryEvent::WorkflowAliasArgumentEdited { .. }
            | TelemetryEvent::ToggledAgentModeAutoexecuteReadonlyCommandsSetting { .. }
            | TelemetryEvent::ChangedAgentModeCodingPermissions { .. }
            | TelemetryEvent::ChangedAgentModeAskUserQuestionPermission { .. }
            | TelemetryEvent::RepoOutlineConstructionSuccess { .. }
            | TelemetryEvent::RepoOutlineConstructionFailed { .. }
            | TelemetryEvent::AutoexecutedAgentModeRequestedCommand { .. }
            | TelemetryEvent::AgenticOnboardingBlockSelected { .. }
            | TelemetryEvent::KnowledgePaneOpened { .. }
            | TelemetryEvent::MCPServerCollectionPaneOpened { .. }
            | TelemetryEvent::MCPServerAdded { .. }
            | TelemetryEvent::MCPTemplateCreated { .. }
            | TelemetryEvent::MCPTemplateInstalled { .. }
            | TelemetryEvent::MCPTemplateShared
            | TelemetryEvent::MCPServerSpawned { .. }
            | TelemetryEvent::MCPToolCallAccepted { .. }
            | TelemetryEvent::ExecutedWarpDrivePrompt { .. }
            | TelemetryEvent::ToggleSshWarpification { .. }
            | TelemetryEvent::SetSshExtensionInstallMode { .. }
            | TelemetryEvent::SshRemoteServerChoiceDoNotAskAgainToggled { .. }
            | TelemetryEvent::SettingsImportInitiated
            | TelemetryEvent::AgentModeCreatedAIBlock { .. }
            | TelemetryEvent::AgentModeRatedResponse { .. }
            | TelemetryEvent::StaticPromptSuggestionsBannerShown { .. }
            | TelemetryEvent::StaticPromptSuggestionAccepted { .. }
            | TelemetryEvent::AISuggestedRuleAdded { .. }
            | TelemetryEvent::AISuggestedRuleEdited { .. }
            | TelemetryEvent::AISuggestedRuleContentChanged { .. }
            | TelemetryEvent::AttachedImagesToAgentModeQuery { .. }
            | TelemetryEvent::ImageReceived { .. }
            | TelemetryEvent::FileExceededContextLimit { .. }
            | TelemetryEvent::AgentModeError { .. }
            | TelemetryEvent::AgentModeRequestRetrySucceeded { .. }
            | TelemetryEvent::ToggleNaturalLanguageAutosuggestionsSetting { .. }
            | TelemetryEvent::ToggleSharedBlockTitleGenerationSetting { .. }
            | TelemetryEvent::ToggleGitOperationsAutogenSetting { .. }
            | TelemetryEvent::GrepToolSucceeded
            | TelemetryEvent::FileGlobToolSucceeded
            | TelemetryEvent::FileGlobToolFailed { .. }
            | TelemetryEvent::ShellTerminatedPrematurely { .. }
            | TelemetryEvent::FullEmbedCodebaseContextSearchFailed { .. }
            | TelemetryEvent::FullEmbedCodebaseContextSearchSuccess { .. }
            | TelemetryEvent::SearchCodebaseRequested { .. }
            | TelemetryEvent::SearchCodebaseRepoUnavailable { .. }
            | TelemetryEvent::InputUXModeChanged { .. }
            | TelemetryEvent::ContextChipInteracted { .. }
            | TelemetryEvent::VoiceInputUsed { .. }
            | TelemetryEvent::AtMenuInteracted { .. }
            | TelemetryEvent::UserMenuUpgradeClicked
            | TelemetryEvent::ActiveIndexedReposChanged { .. }
            | TelemetryEvent::TabCloseButtonPositionUpdated { .. }
            | TelemetryEvent::ExpandedCodeSuggestions { .. }
            | TelemetryEvent::AIExecutionProfileCreated
            | TelemetryEvent::AIExecutionProfileDeleted
            | TelemetryEvent::AIExecutionProfileSettingUpdated { .. }
            | TelemetryEvent::AIExecutionProfileAddedToAllowlist { .. }
            | TelemetryEvent::AIExecutionProfileAddedToDenylist { .. }
            | TelemetryEvent::AIExecutionProfileRemovedFromAllowlist { .. }
            | TelemetryEvent::AIExecutionProfileRemovedFromDenylist { .. }
            | TelemetryEvent::AIExecutionProfileModelSelected { .. }
            | TelemetryEvent::AIExecutionProfileContextWindowSelected { .. }
            | TelemetryEvent::OpenSlashMenu { .. }
            | TelemetryEvent::SlashCommandAccepted { .. }
            | TelemetryEvent::AgentModeSetupBannerAccepted
            | TelemetryEvent::AgentModeSetupBannerDismissed
            | TelemetryEvent::AgentModeSetupProjectScopedRulesAction { .. }
            | TelemetryEvent::AgentModeSetupCodebaseContextAction { .. }
            | TelemetryEvent::AgentModeSetupCreateEnvironmentAction { .. }
            | TelemetryEvent::CloneRepoPromptSubmitted { .. }
            | TelemetryEvent::GetStartedSkipToTerminal
            | TelemetryEvent::FileTreeItemAttachedAsContext { .. }
            | TelemetryEvent::CodeSelectionAddedAsContext { .. }
            | TelemetryEvent::FileTreeItemCreated
            | TelemetryEvent::ConversationListViewOpened
            | TelemetryEvent::ConversationListItemOpened { .. }
            | TelemetryEvent::ConversationListItemDeleted
            | TelemetryEvent::ConversationListLinkCopied { .. }
            | TelemetryEvent::AgentViewEntered { .. }
            | TelemetryEvent::AgentViewExited { .. }
            | TelemetryEvent::InlineConversationMenuOpened { .. }
            | TelemetryEvent::InlineConversationMenuItemSelected { .. }
            | TelemetryEvent::AgentShortcutsViewToggled { .. }
            | TelemetryEvent::InputBufferSubmitted { .. }
            | TelemetryEvent::RecentMenuItemSelected { .. }
            | TelemetryEvent::OpenRepoFolderSubmitted { .. }
            | TelemetryEvent::OutOfCreditsBannerClosed { .. }
            | TelemetryEvent::AutoReloadModalClosed { .. }
            | TelemetryEvent::AutoReloadToggledFromBillingSettings { .. }
            | TelemetryEvent::CLISubagentControlStateChanged { .. }
            | TelemetryEvent::CLISubagentResponsesToggled { .. }
            | TelemetryEvent::CLISubagentInputDismissed { .. }
            | TelemetryEvent::CLISubagentActionExecuted { .. }
            | TelemetryEvent::CLISubagentActionRejected { .. }
            | TelemetryEvent::AgentManagementViewToggled { .. }
            | TelemetryEvent::AgentManagementViewOpenedSession
            | TelemetryEvent::AgentManagementViewCopiedSessionLink
            | TelemetryEvent::DetectedIsolationPlatform { .. }
            | TelemetryEvent::AgentTipShown { .. }
            | TelemetryEvent::AgentTipClicked { .. }
            | TelemetryEvent::ToggleShowAgentTips { .. }
            | TelemetryEvent::CLIAgentToolbarVoiceInputUsed { .. }
            | TelemetryEvent::CLIAgentToolbarImageAttached { .. }
            | TelemetryEvent::CLIAgentToolbarShown { .. }
            | TelemetryEvent::CLIAgentPluginChipClicked { .. }
            | TelemetryEvent::CLIAgentPluginChipDismissed { .. }
            | TelemetryEvent::CLIAgentPluginOperationSucceeded { .. }
            | TelemetryEvent::CLIAgentPluginOperationFailed { .. }
            | TelemetryEvent::CLIAgentPluginDetected { .. }
            | TelemetryEvent::AgentNotificationShown { .. }
            | TelemetryEvent::CLIAgentRichInputOpened { .. }
            | TelemetryEvent::CLIAgentRichInputClosed { .. }
            | TelemetryEvent::CLIAgentRichInputSubmitted { .. }
            | TelemetryEvent::ToggleCLIAgentToolbarSetting { .. }
            | TelemetryEvent::ToggleUseAgentToolbarSetting { .. }
            | TelemetryEvent::CodexModalOpened
            | TelemetryEvent::CodexModalUseCodexClicked
            | TelemetryEvent::LinearIssueLinkOpened
            | TelemetryEvent::CloudAgentCapacityModalOpened
            | TelemetryEvent::CloudAgentCapacityModalDismissed
            | TelemetryEvent::CloudAgentCapacityModalUpgradeClicked
            | TelemetryEvent::ComputerUseApproved { .. }
            | TelemetryEvent::ComputerUseCancelled { .. }
            | TelemetryEvent::FreeTierLimitHitInterstitialDisplayed
            | TelemetryEvent::FreeTierLimitHitInterstitialUpgradeButtonClicked
            | TelemetryEvent::FreeTierLimitHitInterstitialClosed
            | TelemetryEvent::RemoteServerBinaryCheck { .. }
            | TelemetryEvent::RemoteServerInstallation { .. }
            | TelemetryEvent::RemoteServerInitialization { .. }
            | TelemetryEvent::RemoteServerDisconnection { .. }
            | TelemetryEvent::RemoteServerClientRequestError { .. }
            | TelemetryEvent::RemoteServerMessageDecodingError { .. }
            | TelemetryEvent::RemoteServerSetupDuration { .. }
            | TelemetryEvent::RemoteServerHostUnsupported { .. }
            | TelemetryEvent::RemoteServerReconnection { .. }
            | TelemetryEvent::RemoteServerReconnectExhausted { .. } => false,
            #[cfg(feature = "local_fs")]
            TelemetryEvent::CodePaneOpened { .. }
            | TelemetryEvent::CodePanelsFileOpened { .. }
            | TelemetryEvent::PreviewPanePromoted => false,
            #[cfg(windows)]
            TelemetryEvent::WSLRegistryError
            | TelemetryEvent::AutoupdateUnableToCloseApplications
            | TelemetryEvent::AutoupdateFileInUse
            | TelemetryEvent::AutoupdateMutexTimeout
            | TelemetryEvent::AutoupdateForcekillFailed { .. }
            | TelemetryEvent::AutoupdateMinidumpCleanupFailed { .. } => false,
        }
    }

    /// Prints a JSON containing all telemetry events enabled for the current build.
    /// The keys are the event name and the values are the event description.
    #[cfg(not(target_family = "wasm"))]
    pub fn print_telemetry_events_json() -> anyhow::Result<()> {
        // We initialize the feature flags so that we can determine which telemetry events to print.
        crate::init_feature_flags();

        let events: serde_json::Map<String, Value> = warp_core::telemetry::all_events()
            .filter_map(|event| {
                if !event.enablement_state().is_enabled() {
                    return None;
                }

                Some((
                    event.name().to_string(),
                    Value::String(event.description().to_string()),
                ))
            })
            .collect();

        let json_pretty_print_string = serde_json::to_string_pretty(&events)?;
        println!("{json_pretty_print_string}");
        Ok(())
    }
}

impl TelemetryEventDesc for TelemetryEventDiscriminants {
    fn enablement_state(&self) -> EnablementState {
        // We disallow the wildcard statement to prevent us from accidentally ignoring any
        // variants added in the future. Going forward, we should associate all new telemetry events
        // with a feature flag when appropriate.
        #[deny(clippy::wildcard_enum_match_arm)]
        match self {
            Self::SearchCodebaseRequested { .. } | Self::SearchCodebaseRepoUnavailable { .. } => {
                EnablementState::Flag(FeatureFlag::CrossRepoContext)
            }
            Self::AISuggestedAgentModeWorkflowAdded
            | Self::ShowedSuggestedAgentModeWorkflowChip
            | Self::ShowedSuggestedAgentModeWorkflowModal => {
                EnablementState::Flag(FeatureFlag::SuggestedAgentModeWorkflows)
            }
            Self::RepoOutlineConstructionSuccess { .. } => {
                EnablementState::Flag(FeatureFlag::AgentModeAnalytics)
            }
            Self::RepoOutlineConstructionFailed { .. } => {
                EnablementState::Flag(FeatureFlag::AgentModeAnalytics)
            }
            Self::FullEmbedCodebaseContextSearchFailed { .. }
            | Self::FullEmbedCodebaseContextSearchSuccess { .. } => {
                EnablementState::Flag(FeatureFlag::FullSourceCodeEmbedding)
            }
            Self::ObjectLinkCopied => EnablementState::Always,
            Self::FileTreeToggled => EnablementState::Flag(FeatureFlag::FileTree),
            Self::FileTreeItemAttachedAsContext => EnablementState::Flag(FeatureFlag::FileTree),
            Self::CodeSelectionAddedAsContext => EnablementState::Flag(FeatureFlag::HoaCodeReview),
            Self::FileTreeItemCreated => EnablementState::Flag(FeatureFlag::FileTree),
            Self::ConversationListViewOpened
            | Self::ConversationListItemOpened
            | Self::ConversationListItemDeleted
            | Self::ConversationListLinkCopied => {
                EnablementState::Flag(FeatureFlag::AgentViewConversationListView)
            }
            Self::AgentViewEntered
            | Self::AgentViewExited
            | Self::InlineConversationMenuOpened
            | Self::InlineConversationMenuItemSelected
            | Self::AgentShortcutsViewToggled => EnablementState::Flag(FeatureFlag::AgentView),
            Self::CreateProjectPromptSubmitted => EnablementState::Flag(FeatureFlag::GetStartedTab),
            Self::CreateProjectPromptSubmittedContent => {
                EnablementState::Flag(FeatureFlag::GetStartedTab)
            }
            Self::CloneRepoPromptSubmitted => EnablementState::Flag(FeatureFlag::GetStartedTab),
            Self::GetStartedSkipToTerminal => EnablementState::Flag(FeatureFlag::GetStartedTab),
            Self::PtyThroughput => EnablementState::Flag(FeatureFlag::RecordPtyThroughput),
            Self::AgentModeCreatedAIBlock => EnablementState::Flag(FeatureFlag::AgentMode),
            Self::MCPServerCollectionPaneOpened { .. }
            | Self::MCPServerAdded { .. }
            | Self::MCPServerSpawned { .. }
            | Self::MCPToolCallAccepted { .. } => EnablementState::Flag(FeatureFlag::McpServer),
            Self::MCPTemplateCreated { .. }
            | Self::MCPTemplateInstalled { .. }
            | Self::MCPTemplateShared { .. } => EnablementState::Always,
            Self::KnowledgePaneOpened { .. } => EnablementState::Flag(FeatureFlag::AIRules),
            #[cfg(feature = "local_fs")]
            Self::CodePaneOpened { .. } => EnablementState::Always,
            #[cfg(feature = "local_fs")]
            Self::CodePanelsFileOpened { .. } => EnablementState::Always,
            #[cfg(feature = "local_fs")]
            Self::PreviewPanePromoted => EnablementState::Always,
            Self::AISuggestedRuleAdded { .. } => EnablementState::Flag(FeatureFlag::SuggestedRules),
            Self::AISuggestedRuleEdited { .. } => {
                EnablementState::Flag(FeatureFlag::SuggestedRules)
            }
            Self::AISuggestedRuleContentChanged { .. } => {
                EnablementState::Flag(FeatureFlag::SuggestedRules)
            }
            Self::ToggleFocusPaneOnHover { .. } => EnablementState::Always,
            Self::InitiateAnonymousUserSignup { .. }
            | Self::LoginLaterButtonClicked
            | Self::LoginLaterConfirmationButtonClicked
            | Self::AnonymousUserExpirationLockout
            | Self::AnonymousUserLinkedFromBrowser
            | Self::AnonymousUserAttemptLoginGatedFeature
            | Self::AnonymousUserHitCloudObjectLimit => EnablementState::Always,

            Self::AgentModeChangedInputType => EnablementState::Always,
            Self::StartedSharingCurrentSession
            | Self::StoppedSharingCurrentSession
            | Self::SharedSessionModalUpgradePressed => {
                EnablementState::Flag(FeatureFlag::CreatingSharedSessions)
            }
            Self::JoinedSharedSession => EnablementState::Flag(FeatureFlag::ViewingSharedSessions),
            Self::OpenNotebook | Self::EditNotebook | Self::NotebookAction => {
                EnablementState::Always
            }
            Self::ToggleSettingsSync { .. } => EnablementState::Always,
            Self::AgentTipShown | Self::AgentTipClicked | Self::ToggleShowAgentTips => {
                EnablementState::Flag(FeatureFlag::AgentTips)
            }
            Self::AutosuggestionInserted => EnablementState::Always,
            Self::BlockCompleted => EnablementState::Always,
            Self::BackgroundBlockStarted => EnablementState::Always,
            Self::BaselineCommandLatency => EnablementState::Always,
            Self::SessionCreation => EnablementState::Always,
            Self::Login => EnablementState::Always,
            Self::OpenSuggestionsMenu => EnablementState::Always,
            Self::ConfirmSuggestion => EnablementState::Always,
            Self::OpenContextMenu => EnablementState::Always,
            Self::ContextMenuCopy => EnablementState::Always,
            Self::ContextMenuOpenShareModal => EnablementState::Always,
            Self::ContextMenuFindWithinBlocks => EnablementState::Always,
            Self::ContextMenuCopyPrompt => EnablementState::Always,
            Self::ContextMenuToggleGitPromptDirtyIndicator => EnablementState::Always,
            Self::ContextMenuInsertSelectedText => EnablementState::Always,
            Self::ContextMenuCopySelectedText => EnablementState::Always,
            Self::OpenPromptEditor => EnablementState::Always,
            Self::PromptEdited => EnablementState::Always,
            Self::ReinputCommands => EnablementState::Always,
            Self::JumpToPreviousCommand => EnablementState::Always,
            Self::CopyBlockSharingLink => EnablementState::Always,
            Self::GenerateBlockSharingLink => EnablementState::Always,
            Self::BlockSelection => EnablementState::Always,
            Self::BootstrappingSlow => EnablementState::Always,
            Self::BootstrappingSlowContents => EnablementState::Always,
            Self::SessionAbandonedBeforeBootstrap => EnablementState::Always,
            Self::BootstrappingSucceeded => EnablementState::Always,
            Self::TabSingleResultAutocompletion => EnablementState::Always,
            Self::EditorUnhandledModifierKey => EnablementState::Always,
            Self::CopyInviteLink => EnablementState::Always,
            Self::OpenThemeChooser => EnablementState::Always,
            Self::ThemeSelection => EnablementState::Always,
            Self::AppIconSelection => EnablementState::Always,
            Self::CursorDisplayType => EnablementState::Always,
            Self::OpenThemeCreatorModal => EnablementState::Always,
            Self::CreateCustomTheme => EnablementState::Always,
            Self::DeleteCustomTheme => EnablementState::Always,
            Self::SplitPane => EnablementState::Always,
            Self::UnableToAutoUpdateToNewVersion | Self::AutoupdateRelaunchAttempt => {
                EnablementState::Always
            }
            Self::SkipOnboardingSurvey => EnablementState::Always,
            Self::ToggleRestoreSession => EnablementState::Always,
            Self::DatabaseStartUpError => EnablementState::Always,
            Self::DatabaseReadError => EnablementState::Always,
            Self::DatabaseWriteError => EnablementState::Always,
            Self::AppStartup => EnablementState::Always,
            Self::LoggedOutStartup => EnablementState::Always,
            Self::DownloadSource => EnablementState::Always,
            Self::SSHBootstrapAttempt => EnablementState::Always,
            Self::SSHControlMasterError => EnablementState::Always,
            Self::KeybindingChanged => EnablementState::Always,
            Self::KeybindingResetToDefault => EnablementState::Always,
            Self::KeybindingRemoved => EnablementState::Always,
            Self::FeaturesPageAction => EnablementState::Always,
            Self::WorkflowExecuted => EnablementState::Always,
            Self::WorkflowSelected => EnablementState::Always,
            Self::OpenWorkflowSearch => EnablementState::Always,
            Self::OpenQuakeModeWindow => EnablementState::Always,
            Self::OpenWelcomeTips => EnablementState::Always,
            Self::CompleteWelcomeTipFeature => EnablementState::Always,
            Self::DismissWelcomeTips => EnablementState::Always,
            Self::ShowNotificationsDiscoveryBanner => EnablementState::Always,
            Self::NotificationsDiscoveryBannerAction => EnablementState::Always,
            Self::ShowNotificationsErrorBanner => EnablementState::Always,
            Self::NotificationsErrorBannerAction => EnablementState::Always,
            Self::NotificationPermissionsRequested => EnablementState::Always,
            Self::NotificationsRequestPermissionsOutcome => EnablementState::Always,
            Self::NotificationSent => EnablementState::Always,
            Self::NotificationFailedToSend => EnablementState::Always,
            Self::NotificationClicked => EnablementState::Always,
            Self::ToggleFindOption => EnablementState::Always,
            Self::SignUpButtonClicked => EnablementState::Always,
            Self::LoginButtonClicked => EnablementState::Always,
            Self::OpenNewSessionFromFilePath => EnablementState::Always,
            Self::OpenTeamFromURI => EnablementState::Always,
            Self::SelectCommandPaletteOption => EnablementState::Always,
            Self::PaletteSearchOpened => EnablementState::Always,
            Self::PaletteSearchResultAccepted => EnablementState::Always,
            Self::PaletteSearchExited => EnablementState::Always,
            Self::SelectNavigationPaletteItem => EnablementState::Always,
            Self::AuthCommonQuestionClicked => EnablementState::Always,
            Self::AuthToggleFAQ => EnablementState::Always,
            Self::OpenAuthPrivacySettings => EnablementState::Always,
            Self::TabRenamed => EnablementState::Always,
            Self::MoveActiveTab => EnablementState::Always,
            Self::MoveTab => EnablementState::Always,
            Self::DragAndDropTab => EnablementState::Always,
            Self::TabOperations => EnablementState::Always,
            Self::EditedInputBeforePrecmd => EnablementState::Always,
            Self::TriedToExecuteBeforePrecmd => EnablementState::Always,
            Self::ThinStrokesSettingChanged => EnablementState::Always,
            Self::BookmarkBlockToggled => EnablementState::Always,
            Self::JumpToBookmark => EnablementState::Always,
            Self::JumpToBottomofBlockButtonClicked => EnablementState::Always,
            Self::ToggleJumpToBottomofBlockButton => EnablementState::Always,
            Self::OpenLink => EnablementState::Always,
            Self::OpenChangelogLink => EnablementState::Always,
            Self::ShowInFileExplorer => EnablementState::Always,
            Self::CommandXRayTriggered => EnablementState::Always,
            Self::OpenLaunchConfigSaveModal => EnablementState::Always,
            Self::SaveLaunchConfig => EnablementState::Always,
            Self::OpenLaunchConfigFile => EnablementState::Always,
            Self::OpenLaunchConfig => EnablementState::Always,
            Self::TeamCreated => EnablementState::Always,
            Self::TeamJoined => EnablementState::Always,
            Self::TeamLeft => EnablementState::Always,
            Self::TeamLinkCopied => EnablementState::Always,
            Self::RemovedUserFromTeam => EnablementState::Always,
            Self::DeletedWorkflow => EnablementState::Always,
            Self::DeletedNotebook => EnablementState::Always,
            Self::ToggleApprovalsModal => EnablementState::Always,
            Self::ChangedInviteViewOption => EnablementState::Always,
            Self::SendEmailInvites => EnablementState::Always,
            Self::CommandCorrection => EnablementState::Always,
            Self::SetLineHeight => EnablementState::Always,
            Self::ResourceCenterOpened => EnablementState::Always,
            Self::ResourceCenterTipsCompleted => EnablementState::Always,
            Self::ResourceCenterTipsSkipped => EnablementState::Always,
            Self::KeybindingsPageOpened => EnablementState::Always,
            Self::GlobalSearchOpened => EnablementState::Always,
            Self::GlobalSearchQueryStarted => EnablementState::Always,
            Self::CommandSearchOpened => EnablementState::Always,
            Self::CommandSearchExited => EnablementState::Always,
            Self::CommandSearchResultAccepted => EnablementState::Always,
            Self::CommandSearchFilterChanged => EnablementState::Always,
            Self::CommandSearchAsyncQueryCompleted => EnablementState::Always,
            Self::AICommandSearchOpened => EnablementState::Always,
            Self::OpenedAltScreenFind => EnablementState::Always,
            Self::UserInitiatedClose => EnablementState::Always,
            Self::QuitModalShown => EnablementState::Always,
            Self::QuitModalCancel => EnablementState::Always,
            Self::QuitModalDisabled => EnablementState::Always,
            Self::UserInitiatedLogOut => EnablementState::Always,
            Self::LogOutModalShown => EnablementState::Always,
            Self::LogOutModalCancel => EnablementState::Always,
            Self::SetOpacity => EnablementState::Always,
            Self::SetBlurRadius => EnablementState::Always,
            Self::ToggleDimInactivePanes => EnablementState::Always,
            Self::InputModeChanged => EnablementState::Always,
            Self::PtySpawned => EnablementState::Always,
            Self::InitialWorkingDirectoryConfigurationChanged => EnablementState::Always,
            Self::OpenedWarpAI => EnablementState::Always,
            Self::WarpAIRequestIssued => EnablementState::Always,
            Self::WarpAIAction => EnablementState::Always,
            Self::UsedWarpAIPreparedPrompt => EnablementState::Always,
            Self::WarpAICharacterLimitExceeded => EnablementState::Always,
            Self::OpenInputContextMenu => EnablementState::Always,
            Self::InputCutSelectedText => EnablementState::Always,
            Self::InputCopySelectedText => EnablementState::Always,
            Self::InputSelectAll => EnablementState::Always,
            Self::InputPaste => EnablementState::Always,
            Self::InputCommandSearch => EnablementState::Always,
            Self::InputAICommandSearch => EnablementState::Always,
            Self::InputAskWarpAI => EnablementState::Always,
            Self::SaveAsWorkflowModal => EnablementState::Always,
            Self::ExperimentTriggered => EnablementState::Always,
            Self::ToggleSyncAllPanesInAllTabs => EnablementState::Always,
            Self::ToggleSyncAllPanesInTab => EnablementState::Always,
            Self::ToggleSameLinePrompt => EnablementState::Always,
            Self::ToggleNewWindowsAtCustomSize => EnablementState::Always,
            Self::SetNewWindowsAtCustomSize => EnablementState::Always,
            Self::DisableInputSync => EnablementState::Always,
            Self::ToggleTabIndicators => EnablementState::Always,
            Self::TogglePreserveActiveTabColor => EnablementState::Always,
            Self::ShowSubshellBanner => EnablementState::Always,
            Self::SshTmuxWarpifyBannerDisplayed => EnablementState::Always,
            Self::DeclineSubshellBootstrap => EnablementState::Always,
            Self::TriggerSubshellBootstrap => EnablementState::Always,
            Self::AddDenylistedSubshellCommand => EnablementState::Always,
            Self::RemoveDenylistedSubshellCommand => EnablementState::Always,
            Self::ToggleSshTmuxWrapper => EnablementState::Always,
            Self::ToggleSshWarpification => EnablementState::Always,
            Self::SetSshExtensionInstallMode => EnablementState::Always,
            Self::SshRemoteServerChoiceDoNotAskAgainToggled => EnablementState::Always,
            Self::AddDenylistedSshTmuxWrapperHost => EnablementState::Always,
            Self::RemoveDenylistedSshTmuxWrapperHost => EnablementState::Always,
            Self::SshInteractiveSessionDetected => EnablementState::Always,
            Self::SshTmuxWarpifyBlockAccepted => EnablementState::Always,
            Self::SshTmuxWarpifyBlockDismissed => EnablementState::Always,
            Self::WarpifyFooterShown
            | Self::AgentToolbarDismissed
            | Self::WarpifyFooterAcceptedWarpify => EnablementState::Always,
            Self::SshTmuxWarpificationSuccess => EnablementState::Always,
            Self::SshTmuxWarpificationErrorBlock => EnablementState::Always,
            Self::SshInstallTmuxBlockDisplayed => EnablementState::Always,
            Self::SshInstallTmuxBlockAccepted => EnablementState::Always,
            Self::SshInstallTmuxBlockDismissed => EnablementState::Always,
            Self::AddAddedSubshellCommand => EnablementState::Always,
            Self::RemoveAddedSubshellCommand => EnablementState::Always,
            Self::ReceivedSubshellRcFileDcs => EnablementState::Always,
            Self::ShowAliasExpansionBanner => EnablementState::Always,
            Self::EnableAliasExpansionFromBanner => EnablementState::Always,
            Self::DismissAliasExpansionBanner => EnablementState::Always,
            Self::ShowVimKeybindingsBanner => EnablementState::Always,
            Self::EnableVimKeybindingsFromBanner => EnablementState::Always,
            Self::DismissVimKeybindingsBanner => EnablementState::Always,
            Self::InitiateReauth => EnablementState::Always,
            Self::NeedsReauth => EnablementState::Always,
            Self::WarpDriveOpened => EnablementState::Always,
            Self::ToggleWarpAI => EnablementState::Always,
            Self::ToggleSecretRedaction => EnablementState::Always,
            Self::CustomSecretRegexAdded => EnablementState::Always,
            Self::ToggleObfuscateSecret => EnablementState::Always,
            Self::CopySecret => EnablementState::Always,
            Self::AutoGenerateMetadataSuccess => EnablementState::Always,
            Self::AutoGenerateMetadataError => EnablementState::Always,
            Self::UpdateSortingChoice => EnablementState::Always,
            Self::UndoClose => EnablementState::Always,
            Self::DuplicateObject => EnablementState::Always,
            Self::ExportObject => EnablementState::Always,
            Self::CommandFileRun => EnablementState::Always,
            Self::PageUpDownInEditorPressed => EnablementState::Always,
            Self::UnsupportedShell => EnablementState::Always,
            Self::LogOut => EnablementState::Always,
            Self::SettingsImportInitiated => EnablementState::Always,
            Self::InviteTeammates => EnablementState::Always,
            Self::CopyObjectToClipboard => EnablementState::Always,
            Self::OpenAndWarpifyDockerSubshell => EnablementState::Always,
            Self::UpdateBlockFilterQuery => EnablementState::Always,
            Self::UpdateBlockFilterQueryContextLines => EnablementState::Always,
            Self::ToggleBlockFilterQuery => EnablementState::Always,
            Self::ToggleBlockFilterCaseSensitivity => EnablementState::Always,
            Self::ToggleBlockFilterRegex => EnablementState::Always,
            Self::ToggleBlockFilterInvert => EnablementState::Always,
            Self::BlockFilterToolbeltButtonClicked => EnablementState::Always,
            Self::ToggleSnackbarInActivePane => EnablementState::Always,
            Self::PaneDragInitiated => EnablementState::Always,
            Self::PaneDropped => EnablementState::Always,
            Self::TierLimitHit => EnablementState::Always,
            Self::SharerCancelledGrantRole => EnablementState::Always,
            Self::SharerGrantModalDontShowAgain => EnablementState::Always,
            Self::JumpToSharedSessionParticipant => EnablementState::Always,
            Self::CopiedSharedSessionLink => EnablementState::Always,
            Self::WebSessionOpenedOnDesktop => EnablementState::Always,
            Self::WebCloudObjectOpenedOnDesktop => EnablementState::Always,
            Self::ToggleShowBlockDividers => EnablementState::Flag(FeatureFlag::MinimalistUI),
            Self::DriveSharingOnboardingBlockShown => EnablementState::Always,
            Self::SharedObjectLimitHitBannerViewPlansButtonClicked => EnablementState::Always,
            Self::ResourceUsageStats => EnablementState::Always,
            Self::ToggleGlobalAI => EnablementState::Always,
            Self::ToggleActiveAI => EnablementState::Always,
            Self::AgenticOnboardingBlockSelected => EnablementState::Always,
            Self::MemoryUsageStats => EnablementState::ChannelSpecific {
                channels: vec![Channel::Local, Channel::Dev],
            },
            Self::MemoryUsageHigh => EnablementState::Always,
            Self::AgentModeUserAttemptedQueryAtRequestLimit
            | Self::AgentModeClickedEntrypoint
            | Self::AgentModeAttachedBlockContext
            | Self::AgentModeToggleAutoDetectionSetting
            | Self::AgentModePotentialAutoDetectionFalsePositive => {
                EnablementState::Flag(FeatureFlag::AgentMode)
            }
            Self::EnvVarCollectionInvoked | Self::EnvVarWorkflowParameterization => {
                EnablementState::Always
            }
            Self::BlockCompletedOnDogfoodOnly => EnablementState::ChannelSpecific {
                channels: vec![Channel::Local, Channel::Dev],
            },
            Self::CompletedSettingsImport
            | Self::SettingsImportConfigFocused
            | Self::SettingsImportConfigParsed
            | Self::SettingsImportResetButtonClicked
            | Self::ITermMultipleHotkeys => EnablementState::Always,
            Self::ToggleIntelligentAutosuggestionsSetting | Self::AgentModePrediction => {
                EnablementState::Always
            }
            Self::PromptSuggestionShown
            | Self::SuggestedCodeDiffBannerShown
            | Self::SuggestedCodeDiffFailed
            | Self::PromptSuggestionAccepted
            | Self::StaticPromptSuggestionsBannerShown
            | Self::StaticPromptSuggestionAccepted
            | Self::TogglePromptSuggestionsSetting
            | Self::ToggleCodeSuggestionsSetting
            | Self::UnitTestSuggestionShown { .. }
            | Self::UnitTestSuggestionAccepted { .. }
            | Self::UnitTestSuggestionCancelled { .. } => EnablementState::Always,
            Self::ToggleNaturalLanguageAutosuggestionsSetting => {
                EnablementState::Flag(FeatureFlag::PredictAMQueries)
            }
            Self::ToggleSharedBlockTitleGenerationSetting => {
                EnablementState::Flag(FeatureFlag::SharedBlockTitleGeneration)
            }
            Self::ToggleGitOperationsAutogenSetting => {
                EnablementState::Flag(FeatureFlag::GitOperationsInCodeReview)
            }
            Self::ZeroStatePromptSuggestionUsed => EnablementState::Always,
            Self::ToggleVoiceInputSetting => EnablementState::Always,
            Self::AgentModeCodeSuggestionEditedByUser
            | Self::AgentModeCodeFilesNavigated
            | Self::AgentModeCodeDiffHunksNavigated => EnablementState::Always,

            Self::ToggleWorkspaceDecorationVisibility => {
                EnablementState::Flag(FeatureFlag::FullScreenZenMode)
            }
            Self::UpdateAltScreenPaddingMode => EnablementState::Always,
            Self::AddTabWithShell => EnablementState::Flag(FeatureFlag::ShellSelector),
            Self::AgentModeSurfacedCitations | Self::AgentModeOpenedCitation => {
                EnablementState::Always
            }
            Self::OpenedSharingDialog => EnablementState::Always,
            Self::ToggleLigatureRendering => EnablementState::Flag(FeatureFlag::Ligatures),
            Self::WorkflowAliasAdded
            | Self::WorkflowAliasRemoved
            | Self::WorkflowAliasArgumentEdited
            | Self::WorkflowAliasEnvVarsAttached => {
                EnablementState::Flag(FeatureFlag::WorkflowAliases)
            }
            Self::ToggledAgentModeAutoexecuteReadonlyCommandsSetting
            | Self::ChangedAgentModeCodingPermissions
            | Self::ChangedAgentModeAskUserQuestionPermission
            | Self::AutoexecutedAgentModeRequestedCommand => EnablementState::Always,
            Self::AttachedImagesToAgentModeQuery => {
                EnablementState::Flag(FeatureFlag::ImageAsContext)
            }
            #[cfg(windows)]
            Self::WSLRegistryError
            | Self::AutoupdateUnableToCloseApplications
            | Self::AutoupdateFileInUse
            | Self::AutoupdateMutexTimeout
            | Self::AutoupdateForcekillFailed { .. }
            | Self::AutoupdateMinidumpCleanupFailed { .. } => EnablementState::Always,
            Self::ToggleCodebaseContext => EnablementState::Always,
            Self::ToggleAutoIndexing => EnablementState::Always,
            Self::AgentModeRatedResponse => {
                EnablementState::Flag(FeatureFlag::GlobalAIAnalyticsBanner)
            }
            Self::ExecutedWarpDrivePrompt => EnablementState::Flag(FeatureFlag::AgentModeWorkflows),
            Self::ImageReceived => EnablementState::Always,
            Self::FileExceededContextLimit => EnablementState::Always,
            Self::AgentModeError => EnablementState::Always,
            Self::AgentModeRequestRetrySucceeded => EnablementState::Always,
            Self::GrepToolSucceeded => EnablementState::Always,
            Self::GrepToolFailed => EnablementState::Always,
            Self::FileGlobToolSucceeded => EnablementState::Always,
            Self::FileGlobToolFailed { .. } => EnablementState::Always,
            Self::ShellTerminatedPrematurely { .. } => EnablementState::Always,
            Self::InputUXModeChanged { .. } => EnablementState::Always,
            Self::ContextChipInteracted { .. } => EnablementState::Always,
            Self::VoiceInputUsed { .. } => EnablementState::Always,
            Self::AtMenuInteracted { .. } => EnablementState::Always,
            Self::ActiveIndexedReposChanged { .. } => {
                EnablementState::Flag(FeatureFlag::FullSourceCodeEmbedding)
            }
            Self::UserMenuUpgradeClicked => EnablementState::Always,
            Self::TabCloseButtonPositionUpdated { .. } => EnablementState::Always,
            Self::ExpandedCodeSuggestions { .. } => EnablementState::Always,
            Self::AIExecutionProfileCreated
            | Self::AIExecutionProfileDeleted
            | Self::AIExecutionProfileSettingUpdated { .. }
            | Self::AIExecutionProfileAddedToAllowlist { .. }
            | Self::AIExecutionProfileAddedToDenylist { .. }
            | Self::AIExecutionProfileRemovedFromAllowlist { .. }
            | Self::AIExecutionProfileRemovedFromDenylist { .. }
            | Self::AIExecutionProfileModelSelected { .. }
            | Self::AIExecutionProfileContextWindowSelected { .. } => {
                EnablementState::Flag(FeatureFlag::MultiProfile)
            }
            Self::AIInputNotSent { .. } => EnablementState::Always,
            Self::OpenSlashMenu { .. } => EnablementState::Always,
            Self::SlashCommandAccepted { .. } => EnablementState::Always,
            Self::AgentModeSetupBannerAccepted { .. } => EnablementState::Always,
            Self::AgentModeSetupBannerDismissed => EnablementState::Always,
            Self::AgentModeSetupProjectScopedRulesAction { .. } => EnablementState::Always,
            Self::AgentModeSetupCodebaseContextAction { .. } => EnablementState::Always,
            Self::AgentModeSetupCreateEnvironmentAction { .. } => EnablementState::Always,
            Self::InputBufferSubmitted => EnablementState::Always,
            Self::AgentModeContinueConversationButtonClicked { .. } => EnablementState::Always,
            Self::AgentModeRewindDialogOpened { .. } => {
                EnablementState::Flag(FeatureFlag::RevertToCheckpoints)
            }
            Self::AgentModeRewindExecuted { .. } => {
                EnablementState::Flag(FeatureFlag::RevertToCheckpoints)
            }
            Self::RecentMenuItemSelected => EnablementState::Always,
            Self::OpenRepoFolderSubmitted => EnablementState::Always,
            Self::OutOfCreditsBannerClosed => EnablementState::Always,
            Self::AutoReloadModalClosed => EnablementState::Always,
            Self::AutoReloadToggledFromBillingSettings => EnablementState::Always,
            Self::CLISubagentControlStateChanged { .. }
            | Self::CLISubagentResponsesToggled { .. }
            | Self::CLISubagentInputDismissed { .. }
            | Self::CLISubagentActionExecuted { .. }
            | Self::CLISubagentActionRejected { .. } => EnablementState::Always,
            Self::AgentManagementViewToggled { .. }
            | Self::AgentManagementViewOpenedSession
            | Self::AgentManagementViewCopiedSessionLink => {
                EnablementState::Flag(FeatureFlag::AgentManagementView)
            }
            Self::DetectedIsolationPlatform { .. } => EnablementState::Always,
            Self::AgentExitedShellProcess { .. } => EnablementState::Always,
            Self::CLIAgentToolbarVoiceInputUsed { .. } => EnablementState::Always,
            Self::CLIAgentToolbarImageAttached { .. } => EnablementState::Always,
            Self::CLIAgentToolbarShown { .. } => EnablementState::Always,
            Self::CLIAgentPluginChipClicked { .. }
            | Self::CLIAgentPluginChipDismissed { .. }
            | Self::CLIAgentPluginOperationSucceeded { .. }
            | Self::CLIAgentPluginOperationFailed { .. } => {
                EnablementState::Flag(FeatureFlag::HOANotifications)
            }
            Self::CLIAgentPluginDetected { .. } => EnablementState::Always,
            Self::AgentNotificationShown { .. } => {
                EnablementState::Flag(FeatureFlag::HOANotifications)
            }
            Self::CLIAgentRichInputOpened { .. }
            | Self::CLIAgentRichInputClosed { .. }
            | Self::CLIAgentRichInputSubmitted { .. } => {
                EnablementState::Flag(FeatureFlag::CLIAgentRichInput)
            }
            Self::ToggleCLIAgentToolbarSetting { .. } => EnablementState::Always,
            Self::ToggleUseAgentToolbarSetting { .. } => EnablementState::Always,
            Self::CodexModalOpened | Self::CodexModalUseCodexClicked => EnablementState::Always,
            Self::LinearIssueLinkOpened => EnablementState::Always,
            Self::CloudAgentCapacityModalOpened
            | Self::CloudAgentCapacityModalDismissed
            | Self::CloudAgentCapacityModalUpgradeClicked => {
                EnablementState::Flag(FeatureFlag::CloudMode)
            }
            Self::ComputerUseApproved | Self::ComputerUseCancelled => {
                EnablementState::Flag(FeatureFlag::AgentModeComputerUse)
            }
            Self::FreeTierLimitHitInterstitialDisplayed { .. } => EnablementState::Always,
            Self::FreeTierLimitHitInterstitialUpgradeButtonClicked { .. } => {
                EnablementState::Always
            }
            Self::FreeTierLimitHitInterstitialClosed { .. } => EnablementState::Always,
            Self::RemoteServerBinaryCheck
            | Self::RemoteServerInstallation
            | Self::RemoteServerInitialization
            | Self::RemoteServerDisconnection
            | Self::RemoteServerClientRequestError
            | Self::RemoteServerMessageDecodingError
            | Self::RemoteServerSetupDuration
            | Self::RemoteServerHostUnsupported
            | Self::RemoteServerReconnection
            | Self::RemoteServerReconnectExhausted => {
                EnablementState::Flag(FeatureFlag::SshRemoteServer)
            }
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::RepoOutlineConstructionSuccess => "Repo Outline Built Successfully",
            Self::RepoOutlineConstructionFailed => "Repo Outline Construction Failed",
            Self::AutosuggestionInserted => "Autosuggestion Inserted",
            // Although this event is sent when the block completes rather than
            // when it's created, we are still naming it "Block Creation" to
            // preserve our historical telemetry data.
            Self::BlockCompleted => "Block Creation",
            Self::BlockCompletedOnDogfoodOnly => "Block Completed (dogfood only)",
            Self::BackgroundBlockStarted => "Background Block Started",
            Self::SessionCreation => "Tab Creation",
            Self::Login => "Logged in to native app",
            Self::AgentModeContinueConversationButtonClicked => {
                "Clicked Continue Conversation Button"
            }
            Self::AgentModeRewindDialogOpened { .. } => "Opened Rewind Confirmation Dialog",
            Self::AgentModeRewindExecuted { .. } => "Executed Conversation Rewind",
            Self::ReinputCommands => "Context Menu: Reinput Commands",
            Self::ToggleSettingsSync => "Toggle Settings Sync",
            Self::ToggleFocusPaneOnHover => "Toggle Focus Pane On Hover",
            Self::LoginLaterButtonClicked => "Login Later Button Clicked",
            Self::LoginLaterConfirmationButtonClicked => "Login Later Confirmation Button Clicked",
            Self::JumpToPreviousCommand => "Jumped to Previous Command",
            Self::OpenContextMenu => "Open Context Menu",
            Self::ContextMenuFindWithinBlocks => "Context Menu: Find Within Blocks",
            Self::ContextMenuOpenShareModal => "Context Menu: Initiate Block Sharing",
            Self::ContextMenuCopy => "Context Menu Copy",
            Self::CopyBlockSharingLink => "Copy Block Sharing Link",
            Self::GenerateBlockSharingLink => "Generate Block Sharing Link",
            Self::BlockSelection => "Block Selection",
            Self::BootstrappingSlow => "Bootstrapping Slow",
            Self::BootstrappingSlowContents => "Bootstrap Slow Contents",
            Self::ObjectLinkCopied => "Object Link Copied",
            Self::FileTreeToggled => "File Tree Toggled",
            Self::FileTreeItemAttachedAsContext => "FileTree.AttachedAsContext",
            Self::CodeSelectionAddedAsContext => "CodeView.SelectionAddedAsContext",
            Self::FileTreeItemCreated => "FileTree.ItemCreated",
            Self::ConversationListViewOpened => "ConversationList.Opened",
            Self::ConversationListItemOpened => "ConversationList.ItemOpened",
            Self::ConversationListItemDeleted => "ConversationList.ItemDeleted",
            Self::ConversationListLinkCopied => "ConversationList.LinkCopied",
            Self::AgentViewEntered => "AgentView.Entered",
            Self::AgentViewExited => "AgentView.Exited",
            Self::InlineConversationMenuOpened => "AgentView.InlineConversationMenuOpened",
            Self::InlineConversationMenuItemSelected => {
                "AgentView.InlineConversationMenuItemSelected"
            }
            Self::AgentShortcutsViewToggled => "AgentView.ShortcutsViewToggled",
            Self::CreateProjectPromptSubmitted => "Create Project Prompt Submitted",
            Self::CreateProjectPromptSubmittedContent => "Create Project Prompt Submitted Content",
            Self::CloneRepoPromptSubmitted => "Clone Repo Prompt Submitted",
            Self::GetStartedSkipToTerminal => "Get Started Skip to Terminal",
            Self::InitiateAnonymousUserSignup => "Anonymous User Initiated Signup",
            Self::AnonymousUserExpirationLockout => "Anonymous User Expiration Lockout",
            Self::AnonymousUserLinkedFromBrowser => "Anonymous User Linked from Browser",
            Self::AnonymousUserAttemptLoginGatedFeature => {
                "Anonymous User Attempted Login-Gated Feature"
            }
            Self::MCPServerCollectionPaneOpened { .. } => "MCP Server Collection Pane Opened",
            Self::MCPServerAdded { .. } => "MCP Server Added",
            Self::MCPTemplateCreated { .. } => "MCP Template Created",
            Self::MCPTemplateInstalled { .. } => "MCP Template Installed",
            Self::MCPTemplateShared => "MCP Template Shared",
            Self::MCPServerSpawned { .. } => "MCP Server Spawned",
            Self::MCPToolCallAccepted { .. } => "MCP Tool Call Accepted",
            Self::KnowledgePaneOpened { .. } => "Knowledge Pane Opened",
            #[cfg(feature = "local_fs")]
            Self::CodePaneOpened { .. } => "Code Pane Opened",
            #[cfg(feature = "local_fs")]
            Self::CodePanelsFileOpened { .. } => "CodePanels.FileOpened",
            #[cfg(feature = "local_fs")]
            Self::PreviewPanePromoted => "Preview Pane Promoted",
            Self::AISuggestedRuleAdded { .. } => "AI Suggested Rule Added",
            Self::AISuggestedRuleEdited { .. } => "AI Suggested Rule Edited",
            Self::AISuggestedRuleContentChanged { .. } => "AI Suggested Rule Content Changed",
            Self::AnonymousUserHitCloudObjectLimit => "Anonymous User Hit Cloud Object Limit",
            Self::BootstrappingSucceeded => "Bootstrapping Succeeded",
            Self::SessionAbandonedBeforeBootstrap => "Session Abandoned Before Bootstrap",
            Self::TabSingleResultAutocompletion => "Tab Single Result Autocompletion",
            Self::OpenSuggestionsMenu => "Open Suggestions Menu",
            Self::ConfirmSuggestion => "Confirm Suggestion",
            Self::ContextMenuInsertSelectedText => "Context Menu Insert Selected Text into Input",
            Self::ContextMenuCopySelectedText => "Context Menu Copy Selected Text",
            Self::ContextMenuCopyPrompt => "Context Menu Copy Prompt",
            Self::ContextMenuToggleGitPromptDirtyIndicator => {
                "Context Menu Toggle Git Prompt Dirty Indicator"
            }
            Self::EditorUnhandledModifierKey => "Unhandled Editor Modifier Key",
            Self::CopyInviteLink => "Copy Invite Link",
            Self::OpenThemeChooser => "Open Theme Chooser",
            Self::ThemeSelection => "Select Theme",
            Self::AppIconSelection => "Select App Icon",
            Self::CursorDisplayType => "Select Cursor Type",
            Self::OpenThemeCreatorModal => "Open Theme Creator Modal",
            Self::CreateCustomTheme => "Create Custom Theme",
            Self::DeleteCustomTheme => "Delete Custom Theme",
            Self::UnableToAutoUpdateToNewVersion => "Unable to Update To New Version",
            Self::AutoupdateRelaunchAttempt => "Attempting to Relaunch for Update",
            Self::SplitPane => "Split Pane",
            Self::SkipOnboardingSurvey => "Skip Onboarding Survey",
            Self::ToggleRestoreSession => "Toggle Restore Session",
            Self::DatabaseStartUpError => "Database Startup Error",
            Self::DatabaseWriteError => "Database Write Error",
            Self::DatabaseReadError => "Database Read Error",
            Self::AppStartup => "App Startup",
            Self::LoggedOutStartup => "Logged-out App Startup",
            Self::DownloadSource => "App Download Source",
            Self::BaselineCommandLatency => "BaselineCommand Latency",
            Self::SSHBootstrapAttempt => "SSH Bootstrap Attempt",
            Self::SSHControlMasterError => "SSH ControlMaster Error",
            Self::SetNewWindowsAtCustomSize => "Set New Windows at Custom Size",
            Self::ToggleNewWindowsAtCustomSize => "Toggle New Windows at Custom Size",
            Self::KeybindingChanged => "Keybinding Changed",
            Self::KeybindingResetToDefault => "Keybinding Reset to Default",
            Self::KeybindingRemoved => "Keybinding Removed",
            Self::OpenWorkflowSearch => "Open Workflows Search",
            Self::WorkflowExecuted => "Workflow Executed",
            Self::WorkflowSelected => "Workflow Selected",
            Self::FeaturesPageAction => "Features Page Action",
            Self::OpenQuakeModeWindow => "Open Quake Mode Window",
            Self::OpenWelcomeTips => "Open Welcome Tips",
            Self::CompleteWelcomeTipFeature => "Complete Welcome Tip",
            Self::DismissWelcomeTips => "Dismiss Welcome Tips",
            Self::ShowNotificationsDiscoveryBanner => "ShowNotificationsDiscoveryBanner",
            Self::NotificationsDiscoveryBannerAction => "Notifications Discovery Banner Action",
            Self::ShowNotificationsErrorBanner => "ShowNotificationsErrorBanner",
            Self::NotificationsErrorBannerAction => "Notifications Error Banner Action",
            Self::NotificationPermissionsRequested => "Notification Permissions Requested",
            Self::NotificationSent => "Notification Sent",
            Self::NotificationFailedToSend => "Notification Failed to Send",
            Self::NotificationClicked => "Notification Clicked",
            Self::NotificationsRequestPermissionsOutcome => {
                "Notification Request Permissions Outcome"
            }
            Self::ToggleFindOption => "Find Option Toggled",
            Self::SignUpButtonClicked => "Sign Up Button Clicked in App",
            Self::LoginButtonClicked => "Log In Button Clicked in App",
            Self::OpenNewSessionFromFilePath => "New Session From Directory",
            Self::OpenTeamFromURI => "Open Team from URI",
            Self::SelectCommandPaletteOption => "Select Command Palette Option",
            Self::PaletteSearchOpened => "Open Palette",
            Self::PaletteSearchResultAccepted => "Command Palette Search Accepted",
            Self::PaletteSearchExited => "Command Palette Search Exited",
            Self::AuthCommonQuestionClicked => "Auth Common Question Clicked in App",
            Self::AuthToggleFAQ => "Auth: Toggle Common Questions",
            Self::OpenAuthPrivacySettings => "Auth: Open Privacy Settings Overlay",
            Self::TabRenamed => "Tab Renamed",
            Self::MoveActiveTab => "Move Active Tab",
            Self::MoveTab => "Move Tab",
            Self::DragAndDropTab => "Drag and Drop Tab",
            Self::TabOperations => "Tab Operations",
            Self::EditedInputBeforePrecmd => "Edited Input Before Precmd",
            Self::TriedToExecuteBeforePrecmd => "Tried to Execute Before Precmd",
            Self::ThinStrokesSettingChanged => "Thin Strokes Setting Changed",
            Self::BookmarkBlockToggled => "Toggled Bookmark Block",
            Self::JumpToBookmark => "Jumped to Bookmark Block",
            Self::JumpToBottomofBlockButtonClicked => "Jumped to Bottom of Block Button Clicked",
            Self::OpenLink => "Opened Link",
            Self::OpenChangelogLink => "Opened Changelog Link",
            Self::ShowInFileExplorer => "Showed File in File Explorer",
            Self::CommandXRayTriggered => "Triggered Command XRay",
            Self::OpenLaunchConfigSaveModal => "Open Save Config Modal",
            Self::SaveLaunchConfig => "Save Launch Config",
            Self::OpenLaunchConfigFile => "Open Launch Config File",
            Self::OpenLaunchConfig => "Open Launch Config",
            Self::LogOut => "Log Out",
            Self::SelectNavigationPaletteItem => "Select Navigation Palette Item",
            Self::CommandCorrection => "Command Correction Event",
            Self::SetLineHeight => "Set Line Height",
            Self::ResourceCenterOpened => "Resource Center Opened",
            Self::ResourceCenterTipsCompleted => "Resource Center Tips Completed",
            Self::ResourceCenterTipsSkipped => "Resource Center Tips Skipped",
            Self::KeybindingsPageOpened => "Resource Center Keybindings Page Opened",
            Self::GlobalSearchOpened => "Global Search Opened",
            Self::GlobalSearchQueryStarted => "Global Search Query Started",
            Self::CommandSearchOpened => "Command Search Opened",
            Self::CommandSearchExited => "Command Search Exited",
            Self::CommandSearchResultAccepted => "Command Search Result Accepted",
            Self::CommandSearchFilterChanged => "Command Search Filter Changed",
            Self::CommandSearchAsyncQueryCompleted => "Command Search Async Query Completed",
            Self::AICommandSearchOpened => "AI Command Search opened",
            Self::OpenNotebook => "Notebook Opened",
            Self::EditNotebook => "Notebook Edited",
            Self::NotebookAction => "Notebook Action",
            Self::OpenedAltScreenFind => "Opened alt screen find bar",
            Self::UserInitiatedClose => "User Initiated Closing Something",
            Self::QuitModalShown => "Quit Modal Shown",
            Self::QuitModalCancel => "Quit Modal Cancel Pressed",
            Self::QuitModalDisabled => "Quit Modal Disabled",
            Self::UserInitiatedLogOut => "User Initiated Log Out",
            Self::LogOutModalShown => "Log Out Modal Shown",
            Self::LogOutModalCancel => "Log Out Modal Cancel Pressed",
            Self::SetBlurRadius => "Set Window Blur Radius",
            Self::SetOpacity => "Set Window Opacity",
            Self::ToggleDimInactivePanes => "Toggle Dim Inactive Panes",
            Self::ToggleJumpToBottomofBlockButton => "Toggle Jump to Bottom of Block Button",
            Self::ToggleShowBlockDividers => "Toggle Show Block Dividers",
            Self::PtySpawned => "Pty Spawned",
            Self::InitialWorkingDirectoryConfigurationChanged => {
                "InitialWorkingDirectoryConfigurationChanged"
            }
            Self::InputModeChanged => "Input Mode Changed",
            Self::OpenedWarpAI => "Opened Warp AI",
            Self::WarpAIRequestIssued => "Warp AI Request Issued",
            Self::WarpAIAction => "Warp AI Action",
            Self::UsedWarpAIPreparedPrompt => "Used Warp AI Prepared Prompt",
            Self::WarpAICharacterLimitExceeded => "Warp AI Character Limit Exceeded",
            Self::OpenInputContextMenu => "OpenInputBoxContextMenu",
            Self::InputCutSelectedText => "InputBoxCutSelectedText",
            Self::InputCopySelectedText => "InputBoxCutSelectedText",
            Self::InputSelectAll => "InputBoxSelectAll",
            Self::InputPaste => "InputBoxPaste",
            Self::InputCommandSearch => "InputBoxCommandSearch",
            Self::InputAICommandSearch => "InputBoxAICommandSearch",
            Self::InputAskWarpAI => "InputBoxAskWarpAI",
            Self::SaveAsWorkflowModal => "Opened Save As Workflow Modal",
            Self::ExperimentTriggered => "experiments.client.enroll_client",
            Self::ToggleSyncAllPanesInAllTabs => "Toggle Sync Inputs Across All Panes in All Tabs",
            Self::ToggleSyncAllPanesInTab => "Toggle Sync Inputs Across All Panes in Current Tab",
            Self::ToggleSameLinePrompt => "Toggle Same Line Prompt",
            Self::DisableInputSync => "Disable Input Sync Inputs",
            Self::ToggleTabIndicators => "Toggle Tab Indicators",
            Self::TogglePreserveActiveTabColor => "Toggle Preserve Active Tab Color",
            Self::ShowSubshellBanner => "Show Subshell Banner",
            Self::SshTmuxWarpifyBannerDisplayed => "Show Warpify SSH Banner",
            Self::DeclineSubshellBootstrap => "Decline Subshell Bootstrap",
            Self::TriggerSubshellBootstrap => "Trigger Subshell Bootstrap",
            Self::AddDenylistedSubshellCommand => "Add Denylisted Subshell Command",
            Self::RemoveDenylistedSubshellCommand => "Remove Denylisted Subshell Command",
            Self::AddAddedSubshellCommand => "Add Added Subshell Command",
            Self::RemoveAddedSubshellCommand => "Remove Added Subshell Command",
            Self::ReceivedSubshellRcFileDcs => "Received Subshell RC File DCS",
            Self::ToggleSshTmuxWrapper => "Toggle SSH Tmux Wrapper",
            Self::ToggleSshWarpification => "Toggle SSH Warpification",
            Self::SetSshExtensionInstallMode => "Set SSH Extension Install Mode",
            Self::SshRemoteServerChoiceDoNotAskAgainToggled => {
                "SSH Remote Server Choice Do Not Ask Again Toggled"
            }
            Self::AddDenylistedSshTmuxWrapperHost => "Add Denylisted SSH Tmux Wrapper Host",
            Self::RemoveDenylistedSshTmuxWrapperHost => "Remove Denylisted SSH Tmux Wrapper Host",
            Self::SshInteractiveSessionDetected => "SSH Interactive Session Detected",
            Self::SshTmuxWarpifyBlockAccepted => "SSH Tmux Warpify Block Accepted",
            Self::SshTmuxWarpifyBlockDismissed => "SSH Tmux Warpify Block Dismissed",
            Self::WarpifyFooterShown => "Warpify Footer Shown",
            Self::AgentToolbarDismissed => "Agent Toolbar Dismissed",
            Self::WarpifyFooterAcceptedWarpify => "Warpify Footer Accepted Warpify",
            Self::SshTmuxWarpificationSuccess => "SSH Tmux Warpification Succeeded",
            Self::SshTmuxWarpificationErrorBlock => "SSH Tmux Warpification Error Block",
            Self::SshInstallTmuxBlockDisplayed => "SSH Install Tmux Block Displayed",
            Self::SshInstallTmuxBlockAccepted => "SSH Install Tmux Block Accepted",
            Self::SshInstallTmuxBlockDismissed => "SSH Install Tmux Block Dismissed",
            Self::ShowAliasExpansionBanner => "Show Alias Expansion Banner",
            Self::DismissAliasExpansionBanner => "Dismiss Alias Expansion Banner",
            Self::EnableAliasExpansionFromBanner => "Enable Alias Expansion From Banner",
            Self::InitiateReauth => "Initiate Reauth",
            Self::NeedsReauth => "Needs Reauth",
            Self::WarpDriveOpened => "Warp Drive Opened",
            Self::ToggleWarpAI => "Toggle Warp AI",
            Self::ToggleSecretRedaction => "Toggle Secret Redaction",
            Self::CustomSecretRegexAdded => "Custom Secret Regex Added",
            Self::ToggleObfuscateSecret => "Toggle Obfuscate Secret",
            Self::CopySecret => "Copy Obfuscated Secret",
            Self::AutoGenerateMetadataSuccess => "Generate Metadata For Workflow Success",
            Self::AutoGenerateMetadataError => "Generate Metadata For Workflow Error",
            Self::UpdateSortingChoice => "Updated Sorting Choice",
            Self::UndoClose => "Undo Close",
            Self::OpenPromptEditor => "Prompt Editor Opened",
            Self::PromptEdited => "Prompt Edited",
            Self::PtyThroughput => "PTY Throughput",
            Self::DuplicateObject => "Duplicate Object",
            Self::ExportObject => "Export Object",
            Self::CommandFileRun => "Command File Run",
            Self::PageUpDownInEditorPressed => "Page Up/Down In Editor Pressed",
            Self::StartedSharingCurrentSession => "Started Sharing Current Session",
            Self::StoppedSharingCurrentSession => "Stopped Sharing Current Session",
            Self::JoinedSharedSession => "Joined Shared Session",
            Self::SharedSessionModalUpgradePressed => "Shared Session Modal Upgrade Pressed",
            Self::SharerCancelledGrantRole => "Sharer Cancelled Grant Role",
            Self::SharerGrantModalDontShowAgain => "Don't Show Sharer Grant Modal Again",
            Self::JumpToSharedSessionParticipant { .. } => "Jumped to Shared Session Participant",
            Self::CopiedSharedSessionLink { .. } => "Copied Shared Session Link",
            Self::WebSessionOpenedOnDesktop { .. } => "Web session opened on desktop",
            Self::WebCloudObjectOpenedOnDesktop { .. } => "Warp Drive object opened on desktop",
            Self::DriveSharingOnboardingBlockShown => "Warp Drive Sharing onboarding block shown",
            Self::UnsupportedShell => "Unsupported Shell",
            Self::SettingsImportInitiated => "Settings Import Initiated",
            Self::InviteTeammates => "Invited Teammates",
            Self::CopyObjectToClipboard => "Copy Object To Clipboard",
            Self::OpenAndWarpifyDockerSubshell => "OpenAndWarpifyDockerSubshell",
            Self::UpdateBlockFilterQuery => "Update Block Filter Query",
            Self::ToggleBlockFilterQuery => "Toggle Block Filter Query",
            Self::ToggleBlockFilterCaseSensitivity => "Toggle Block Filter Case Sensitivity",
            Self::ToggleBlockFilterRegex => "Toggle Block Filter Regex",
            Self::ToggleBlockFilterInvert => "Toggle Block Filter Invert",
            Self::BlockFilterToolbeltButtonClicked => "Block Filter Toolbelt Button Clicked",
            Self::ShowVimKeybindingsBanner => "Vim Keybindings Banner Displayed",
            Self::EnableVimKeybindingsFromBanner => "Vim Keybindings Enabled from Banner",
            Self::DismissVimKeybindingsBanner => "Vim Keybindings Banner Dismissed",
            Self::UpdateBlockFilterQueryContextLines => {
                "Update Block Filter Query With Context Lines"
            }
            Self::ToggleSnackbarInActivePane => "Toggle Sticky Command Header in Active Pane",
            Self::PaneDragInitiated => "Pane Drag Inititiated",
            Self::PaneDropped => "Pane Drag Ended",
            Self::AgentModeCreatedAIBlock => "AgentMode.CreatedAIBlock",
            Self::TeamCreated => "Team Created",
            Self::TeamJoined => "Team Joined",
            Self::TeamLeft => "Team Left",
            Self::TeamLinkCopied => "Team Link Copied",
            Self::RemovedUserFromTeam => "Removed user from team",
            Self::DeletedWorkflow => "Deleted Workflow",
            Self::DeletedNotebook => "Deleted Notebook",
            Self::ToggleApprovalsModal => "Toggle Approvals Modal",
            Self::ChangedInviteViewOption => "Changed invite view option",
            Self::SendEmailInvites => "Sent email invites",
            Self::TierLimitHit => "Tier Limit Hit",
            Self::SharedObjectLimitHitBannerViewPlansButtonClicked => {
                "Shared Object Limit Hit Banner View Plans Button Clicked"
            }
            Self::AgentModeUserAttemptedQueryAtRequestLimit => "AgentMode.QueryAttemptAtLImit",
            Self::AgentModeClickedEntrypoint => "AgentMode.ClickedEntrypoint",
            Self::AgentModeAttachedBlockContext => "AgentMode.AttachedContext",
            Self::ResourceUsageStats => "perf_metrics.resource_usage",
            Self::MemoryUsageStats => "perf_metrics.memory_usage",
            Self::MemoryUsageHigh => "perf_metrics.memory_usage_high",
            Self::AgentModeToggleAutoDetectionSetting => "AgentMode.ToggleAutoDetectionSetting",
            Self::AgentModePotentialAutoDetectionFalsePositive => {
                "AgentMode.PotentialAutoDetectionFalsePositive"
            }
            Self::AgentModeChangedInputType => "AgentMode.ChangedInputType",
            Self::AgentModePrediction => "Agent Predict",
            // Agent Mode Query Suggestions is the legacy name for Prompt Suggestions - we avoid renaming
            // the event to avoid breaking historical telemetry data.
            Self::PromptSuggestionShown => "Agent Mode Query Suggestions Banner Shown",
            Self::SuggestedCodeDiffBannerShown => "Suggested Code Diff Banner Shown",
            Self::SuggestedCodeDiffFailed => "Suggested Code Diff Failed",
            Self::PromptSuggestionAccepted => "Agent Mode Query Suggestion Accepted",
            Self::StaticPromptSuggestionsBannerShown => "Static Prompt Suggestions Banner Shown",
            Self::StaticPromptSuggestionAccepted => "Static Prompt Suggestion Accepted",
            Self::ZeroStatePromptSuggestionUsed => "Zero State Prompt Suggestion Used",
            Self::TogglePromptSuggestionsSetting => "Toggle Agent Mode Query Suggestions Setting",
            Self::UnitTestSuggestionShown { .. } => "Suggested Prompt Shown",
            Self::UnitTestSuggestionAccepted { .. } => "Suggested Prompt Accepted",
            Self::UnitTestSuggestionCancelled { .. } => "Suggested Prompt Cancelled",
            Self::ToggleCodeSuggestionsSetting => "Toggle Code Suggestions Setting",
            Self::ToggleNaturalLanguageAutosuggestionsSetting => {
                "Toggle Natural Language Autosuggestions Setting"
            }
            Self::ToggleSharedBlockTitleGenerationSetting => "Toggle SharedBlock Title Generation",
            Self::ToggleGitOperationsAutogenSetting => "Toggle Git Operations Autogen Setting",
            Self::AgentModeCodeSuggestionEditedByUser => "AgentMode.Code.SuggestedCodeEditedByUser",
            Self::AgentModeCodeFilesNavigated => "AgentMode.Code.FilesNavigated",
            Self::AgentModeCodeDiffHunksNavigated => "AgentMode.Code.DiffHunksNavigated",
            Self::ToggleIntelligentAutosuggestionsSetting => {
                "Toggle Intelligent Autosuggestions Setting"
            }
            Self::ToggleVoiceInputSetting => "Toggle Voice Input Setting",
            Self::EnvVarCollectionInvoked => "Invoked Environment Variables",
            Self::EnvVarWorkflowParameterization => {
                "Parameterized Workflow With Environment Variables"
            }
            Self::CompletedSettingsImport => "Completed Settings Import",
            Self::SettingsImportConfigFocused => "Focused Config in Settings Import",
            Self::SettingsImportConfigParsed => "Parsed Config in Settings Import",
            Self::SettingsImportResetButtonClicked => {
                "Clicked Reset to Defaults Button in Settings Import"
            }
            Self::ITermMultipleHotkeys => "ITerm Profile has Multiple Hotkeys",
            Self::ToggleWorkspaceDecorationVisibility => "Toggled Tab Bar Visibility",
            Self::UpdateAltScreenPaddingMode => "Updated Alt Screen Padding Mode",
            Self::AddTabWithShell => "Add Tab With Shell",
            Self::AgentModeSurfacedCitations => "AgentMode.SurfacedCitations",
            Self::AgentModeOpenedCitation => "AgentMode.OpenedCitation",
            Self::OpenedSharingDialog => "Opened Sharing Dialog",
            Self::ToggleGlobalAI => "Toggle Global AI Enablement",
            Self::ToggleActiveAI => "Toggle Active AI Enablement",
            Self::ToggleLigatureRendering => "Toggle Ligature Rendering",
            Self::WorkflowAliasAdded => "Added Workflow Alias",
            Self::WorkflowAliasRemoved => "Removed Workflow Alias",
            Self::WorkflowAliasArgumentEdited => "Edited Workflow Alias Argument",
            Self::WorkflowAliasEnvVarsAttached => "Attached Workflow Alias Environment Variables",

            Self::ToggledAgentModeAutoexecuteReadonlyCommandsSetting => {
                "AIAutonomy.ToggledAutoexecuteReadonlyCommandsSetting"
            }
            Self::ChangedAgentModeCodingPermissions => {
                "AIAutonomy.ChangedAgentModeCodingPermissions"
            }
            Self::ChangedAgentModeAskUserQuestionPermission => {
                "AIAutonomy.ChangedAgentModeAskUserQuestionPermission"
            }
            Self::AutoexecutedAgentModeRequestedCommand => {
                "AIAutonomy.AutoexecutedRequestedCommand"
            }
            Self::AgenticOnboardingBlockSelected => "AgenticOnboarding.BlockSelected",
            Self::RemoteServerBinaryCheck => "RemoteServer.BinaryCheck",
            Self::RemoteServerInstallation => "RemoteServer.Installation",
            Self::RemoteServerInitialization => "RemoteServer.Initialization",
            Self::RemoteServerDisconnection => "RemoteServer.Disconnection",
            Self::RemoteServerClientRequestError => "RemoteServer.ClientRequestError",
            Self::RemoteServerMessageDecodingError => "RemoteServer.MessageDecodingError",
            Self::RemoteServerSetupDuration => "RemoteServer.SetupDuration",
            Self::RemoteServerHostUnsupported => "RemoteServer.HostUnsupported",
            Self::RemoteServerReconnection => "RemoteServer.Reconnection",
            Self::RemoteServerReconnectExhausted => "RemoteServer.ReconnectExhausted",
            #[cfg(windows)]
            Self::WSLRegistryError => "WSL Distribution Registry Error",
            #[cfg(windows)]
            Self::AutoupdateUnableToCloseApplications => {
                "Windows Autoupdate: Setup Unable to Close Applications"
            }
            #[cfg(windows)]
            Self::AutoupdateFileInUse => "Windows Autoupdate: File In Use Error",
            #[cfg(windows)]
            Self::AutoupdateMutexTimeout => "Windows Autoupdate: Mutex Timeout",
            #[cfg(windows)]
            Self::AutoupdateForcekillFailed { .. } => "Windows Autoupdate: Forcekill Failed",
            #[cfg(windows)]
            Self::AutoupdateMinidumpCleanupFailed { .. } => {
                "Windows Autoupdate: Minidump Cleanup Failed"
            }
            Self::ToggleCodebaseContext => "Toggle Agent Mode Codebase Context",
            Self::ToggleAutoIndexing => "Toggle Codebase Context Autoindexing",
            Self::ActiveIndexedReposChanged => "Active Indexed Repos Changed",
            Self::AttachedImagesToAgentModeQuery => "AgentMode.AttachedImages",
            Self::AgentModeRatedResponse => "AgentMode.RatedResponse",
            Self::ExecutedWarpDrivePrompt => "AgentMode.ExecutedWarpDrivePrompt",
            Self::ImageReceived => "Image Received",
            Self::FileExceededContextLimit => "AgentMode.Code.FileExceededContextLimit",
            Self::AgentModeError => "AgentMode.Error",
            Self::AgentModeRequestRetrySucceeded => "AgentMode.RequestRetrySucceeded",
            Self::GrepToolSucceeded => "AgentMode.Grep.Succeeded",
            Self::GrepToolFailed => "AgentMode.Grep.Failed",
            Self::FileGlobToolSucceeded => "AgentMode.FileGlob.Succeeded",
            Self::FileGlobToolFailed { .. } => "AgentMode.FileGlob.Failed",
            Self::ShellTerminatedPrematurely { .. } => "Shell Terminated Prematurely",
            Self::FullEmbedCodebaseContextSearchSuccess { .. } => {
                "AgentMode.FullEmbedCodebaseContextSearch.Success"
            }
            Self::FullEmbedCodebaseContextSearchFailed { .. } => {
                "AgentMode.FullEmbedCodebaseContextSearch.Failed"
            }
            Self::ShowedSuggestedAgentModeWorkflowChip => "AgentMode.ShowedSuggestedWorkflowChip",
            Self::AISuggestedAgentModeWorkflowAdded => {
                "AgentMode.AISuggestedAgentModeWorkflowAdded"
            }
            Self::ShowedSuggestedAgentModeWorkflowModal => {
                "AgentMode.ShowedSuggestedAgentModeWorkflowModal"
            }
            Self::SearchCodebaseRequested { .. } => "AgentMode.SearchCodebase.Requested",
            Self::SearchCodebaseRepoUnavailable { .. } => {
                "AgentMode.SearchCodebase.RepoUnavailable"
            }
            Self::InputUXModeChanged { .. } => "Input.InputUXModeChanged",
            Self::ContextChipInteracted { .. } => "Input.ContextChipInteracted",
            Self::VoiceInputUsed { .. } => "Input.VoiceInputUsed",
            Self::AtMenuInteracted { .. } => "Input.AtMenuInteracted",
            Self::UserMenuUpgradeClicked => "User Menu Upgrade Clicked",
            Self::TabCloseButtonPositionUpdated { .. } => "Update Tab Close Button Position",
            Self::ExpandedCodeSuggestions { .. } => "Expanded Code Suggestion",
            Self::AIExecutionProfileCreated => "AI Execution Profile Created",
            Self::AIExecutionProfileDeleted => "AI Execution Profile Deleted",
            Self::AIExecutionProfileSettingUpdated { .. } => {
                "AI Execution Profile: Setting Updated"
            }
            Self::AIExecutionProfileAddedToAllowlist { .. } => {
                "AI Execution Profile: Added To Allowlist"
            }
            Self::AIExecutionProfileAddedToDenylist { .. } => {
                "AI Execution Profile: Added To Denylist"
            }
            Self::AIExecutionProfileRemovedFromAllowlist { .. } => {
                "AI Execution Profile: Removed From Allowlist"
            }
            Self::AIExecutionProfileRemovedFromDenylist { .. } => {
                "AI Execution Profile: Removed From Denylist"
            }
            Self::AIExecutionProfileModelSelected { .. } => "AI Execution Profile: Model Selected",
            Self::AIExecutionProfileContextWindowSelected { .. } => {
                "AI Execution Profile: Context Window Selected"
            }
            Self::AIInputNotSent { .. } => "AI Input Not Sent",
            Self::OpenSlashMenu { .. } => "Open Slash Menu",
            Self::SlashCommandAccepted { .. } => "Slash Command Accepted",
            Self::AgentModeSetupBannerAccepted => "Agent Mode Setup Banner Accepted",
            Self::AgentModeSetupBannerDismissed => "Agent Mode Setup Banner Dismissed",
            Self::AgentModeSetupProjectScopedRulesAction { .. } => {
                "Agent Mode Setup Project Scoped Rules Action"
            }
            Self::AgentModeSetupCodebaseContextAction { .. } => {
                "Agent Mode.Setup Codebase Context Action"
            }
            Self::AgentModeSetupCreateEnvironmentAction { .. } => {
                "AgentMode.SetupCreateEnvironmentAction"
            }
            Self::InputBufferSubmitted => "AgentMode.NaturalLanguageDetection.InputBufferSubmitted",
            Self::RecentMenuItemSelected { .. } => "Recent Menu Item Selected",
            Self::OpenRepoFolderSubmitted { .. } => "Open Repo Folder Submitted",
            Self::OutOfCreditsBannerClosed => "revenue.OutOfCreditsBannerClosed",
            Self::AutoReloadModalClosed => "revenue.AutoReloadModalClosed",
            Self::AutoReloadToggledFromBillingSettings => {
                "revenue.AutoReloadToggledFromBillingSettings"
            }
            Self::CLISubagentControlStateChanged { .. } => "CLI Subagent Control State Changed",
            Self::CLISubagentResponsesToggled { .. } => "CLI Subagent Responses Toggled",
            Self::CLISubagentInputDismissed { .. } => "CLI Subagent Input Dismissed",
            Self::CLISubagentActionExecuted { .. } => "CLI Subagent Action Executed",
            Self::CLISubagentActionRejected { .. } => "CLI Subagent Action Rejected",
            Self::AgentManagementViewToggled { .. } => "Agent Management View Toggled",
            Self::AgentManagementViewOpenedSession => "Agent Management View Opened Session",
            Self::AgentManagementViewCopiedSessionLink => {
                "Agent Management View Copied Session Link"
            }
            Self::DetectedIsolationPlatform { .. } => "Isolation.DetectedIsolationPlatform",
            Self::AgentTipShown => "AgentTip Shown",
            Self::AgentTipClicked => "AgentTip Clicked",
            Self::ToggleShowAgentTips => "Toggle Show Agent Tips",
            Self::AgentExitedShellProcess => "AgentMode.ExitedShellProcess",
            Self::CLIAgentToolbarVoiceInputUsed { .. } => "CLIAgentFooter.VoiceInputUsed",
            Self::CLIAgentToolbarImageAttached { .. } => "CLIAgentFooter.ImageAttached",
            Self::CLIAgentToolbarShown { .. } => "CLIAgentFooter.Shown",
            Self::CLIAgentPluginChipClicked { .. } => "CLIAgentPlugin.ChipClicked",
            Self::CLIAgentPluginChipDismissed { .. } => "CLIAgentPlugin.ChipDismissed",
            Self::CLIAgentPluginOperationSucceeded { .. } => "CLIAgentPlugin.OperationSucceeded",
            Self::CLIAgentPluginOperationFailed { .. } => "CLIAgentPlugin.OperationFailed",
            Self::CLIAgentPluginDetected { .. } => "CLIAgentPlugin.Detected",
            Self::AgentNotificationShown { .. } => "AgentNotification.Shown",
            Self::CLIAgentRichInputOpened { .. } => "CLIAgentRichInput.Opened",
            Self::CLIAgentRichInputClosed { .. } => "CLIAgentRichInput.Closed",
            Self::CLIAgentRichInputSubmitted { .. } => "CLIAgentRichInput.Submitted",
            Self::ToggleCLIAgentToolbarSetting { .. } => "CLIAgentFooter.SettingToggled",
            Self::ToggleUseAgentToolbarSetting { .. } => "UseAgentToolbar.SettingToggled",
            Self::CodexModalOpened => "CodexModal.Opened",
            Self::CodexModalUseCodexClicked => "CodexModal.UseCodexClicked",
            Self::LinearIssueLinkOpened => "Linear.IssueLinkOpened",
            Self::CloudAgentCapacityModalOpened => "AmbientAgent.ConcurrencyModal.Opened",
            Self::CloudAgentCapacityModalDismissed => "AmbientAgent.ConcurrencyModal.Dismissed",
            Self::CloudAgentCapacityModalUpgradeClicked => {
                "AmbientAgent.ConcurrencyModal.UpgradeClicked"
            }
            Self::ComputerUseApproved => "ComputerUse.Approved",
            Self::ComputerUseCancelled => "ComputerUse.Cancelled",
            Self::FreeTierLimitHitInterstitialDisplayed { .. } => {
                "FreeTierLimitHitInterstitial.Displayed"
            }
            Self::FreeTierLimitHitInterstitialUpgradeButtonClicked { .. } => {
                "FreeTierLimitHitInterstitial.UpgradeButtonClicked"
            }
            Self::FreeTierLimitHitInterstitialClosed { .. } => {
                "FreeTierLimitHitInterstitial.Closed"
            }
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::AIExecutionProfileContextWindowSelected => {
                "Selected a context window limit for an execution profile's base model"
            }
            Self::AISuggestedAgentModeWorkflowAdded => {
                "User created an AI suggested Agent Mode workflow"
            }
            Self::ShowedSuggestedAgentModeWorkflowModal => {
                "Showed the suggested Agent Mode workflow modal to the user"
            }
            Self::RepoOutlineConstructionSuccess => {
                "Repository outline built successfully for providing codebase context"
            }
            Self::RepoOutlineConstructionFailed => "Repository outline built failed",
            Self::AutosuggestionInserted => "Accepted autosuggestion",
            Self::BlockCompleted => "Created Block",
            Self::AgentModeContinueConversationButtonClicked => {
                "User clicked the Continue Conversation button in a block footer"
            }
            Self::AgentModeRewindDialogOpened { .. } => {
                "User opened the rewind confirmation dialog"
            }
            Self::AgentModeRewindExecuted { .. } => {
                "User executed a rewind to a previous conversation state"
            }
            Self::BlockCompletedOnDogfoodOnly => {
                "Completed a block, with extra information for dogfood only"
            }
            Self::InitiateAnonymousUserSignup => "An anonymous user initiated the sign up flow",
            Self::AnonymousUserExpirationLockout => {
                "An anonymous user opened Warp after their conversion deadline and was locked out"
            }
            Self::AnonymousUserLinkedFromBrowser => {
                "Received an auth payload from anonymous user after linking in browser"
            }
            Self::AnonymousUserAttemptLoginGatedFeature => {
                "Anonymous user attempted to access a login-gated feature"
            }
            Self::AnonymousUserHitCloudObjectLimit => {
                "Anonymous user attempted to create a cloud object past their personal object limit"
            }
            Self::BackgroundBlockStarted => {
                "Warp created a background-output Block (whenever a processes has been backgrounded and yields some output)"
            }
            Self::BaselineCommandLatency => "Command execution time",
            Self::SessionCreation => "Created a tab",
            Self::MCPServerCollectionPaneOpened { .. } => "MCP Server Collection Pane Opened",
            Self::MCPServerAdded { .. } => "MCP Server Added",
            Self::MCPTemplateCreated { .. } => "MCP Template Created",
            Self::MCPTemplateInstalled { .. } => "MCP Template Installed",
            Self::MCPTemplateShared => "MCP Template Shared",
            Self::MCPServerSpawned { .. } => "MCP Server Spawned",
            Self::MCPToolCallAccepted { .. } => "MCP Tool Call Accepted",
            Self::KnowledgePaneOpened { .. } => "Knowledge Pane Opened",
            #[cfg(feature = "local_fs")]
            Self::CodePaneOpened { .. } => "Opened the code editor pane from various sources",
            #[cfg(feature = "local_fs")]
            Self::CodePanelsFileOpened { .. } => {
                "Opened a file from code review, project explorer, or global search"
            }
            #[cfg(feature = "local_fs")]
            Self::PreviewPanePromoted => "Promoted a preview code tab to a normal tab",
            Self::AISuggestedRuleAdded { .. } => {
                "Clicked the Add Suggested Rule button in the AI blocklist"
            }
            Self::AISuggestedRuleEdited { .. } => {
                "Clicked the Edit Suggested Rule button in the AI blocklist"
            }
            Self::AISuggestedRuleContentChanged { .. } => {
                "Content changed by the user in the suggested rule dialog"
            }
            Self::ToggleSettingsSync => "Toggle Settings Sync",
            Self::Login => "Login is successful",
            Self::LoginLaterButtonClicked => "Clicked \"Login later\" button",
            Self::LoginLaterConfirmationButtonClicked => {
                "Clicked \"Yes, skip login\" confirmation button"
            }
            Self::OpenSuggestionsMenu => "Opened a suggestion menus, such as with up arrow or tab",
            Self::ConfirmSuggestion => "Accepted tab completion suggestion",
            Self::OpenContextMenu => {
                "Opened context menu (such as right clicking, clicking on ellipses in the top right of a Block, etc.)"
            }
            Self::ContextMenuCopy => "Clicked \"Copy\" in context menu",
            Self::ContextMenuOpenShareModal => "Opened \"Share\" modal via context menu",
            Self::ContextMenuFindWithinBlocks => "Clicked \"find within blocks\" in context menu",
            Self::ContextMenuCopyPrompt => "Clicked  \"Copy Prompt\" in context menu",
            Self::ContextMenuToggleGitPromptDirtyIndicator => {
                "Toggled indicator of dirty git prompt"
            }
            Self::ContextMenuInsertSelectedText => "Clicked \"insert into input\" in context menu",
            Self::ContextMenuCopySelectedText => "Clicked \"Copy selected text\" in context menu",
            Self::OpenPromptEditor => "Opened the prompt editor",
            Self::PromptEdited => "Edited the prompt using the built-in prompt editor",
            Self::ReinputCommands => "Clicked \"reinput commands\" in context menu",
            Self::JumpToPreviousCommand => "Jumped to a previous command",
            Self::CopyBlockSharingLink => "Clicked \"Share block...\" in context menu",
            Self::GenerateBlockSharingLink => "Generated Block sharing link",
            Self::BlockSelection => "Selected Block",
            Self::BootstrappingSlow => "Slow bootstrap on session startup",
            Self::BootstrappingSlowContents => {
                "Contents of the bootstrap block if bootstrapping is slow"
            }
            Self::SessionAbandonedBeforeBootstrap => {
                "Abandoned session before the bootstrapping completes"
            }
            Self::BootstrappingSucceeded => "Successful bootstrap for session",
            Self::TabSingleResultAutocompletion => {
                "Accepted tab completion and inserted into Input Editor"
            }
            Self::EditorUnhandledModifierKey => {
                "Used modifier keybinding keystroke which is not currently supported"
            }
            Self::CopyInviteLink => "Clicked \"Copy Link\" on Referral Modal",
            Self::OpenThemeChooser => {
                "Opened theme chooser (list of different themes and visualizations of those themes)"
            }
            Self::ThemeSelection => "Selected theme",
            Self::AppIconSelection => "Selected app icon",
            Self::CursorDisplayType => "Selected cursor type",
            Self::OpenThemeCreatorModal => {
                "Opened theme creator modal (modal to create a new theme)"
            }
            Self::CreateCustomTheme => "Created a custom theme using the built-in theme creator",
            Self::DeleteCustomTheme => "Deleted a custom theme using the built-in theme creator",
            Self::SplitPane => "Split tab into multiple panes",
            Self::UnableToAutoUpdateToNewVersion => {
                "Update available but not authorized to install"
            }
            Self::AutoupdateRelaunchAttempt => {
                "Attempted to relaunch the app after installing an update"
            }
            Self::SkipOnboardingSurvey => "Skipped onboarding survey as a whole",
            Self::ToggleRestoreSession => {
                "Toggled session restoration (\"Restore windows, tabs, panes, on startup\")"
            }
            Self::DatabaseStartUpError => "Failed to initialize sqlite upon startup",
            Self::DatabaseReadError => {
                "Database read error when trying to get app state for session restoration"
            }
            Self::DatabaseWriteError => {
                "Database write error when trying to write app state for session restoration"
            }
            Self::AppStartup => "App is launched",
            Self::LoggedOutStartup => "Started Warp in the logged-out / signed-out state",
            Self::DownloadSource => {
                "Whether the Warp was installed from the home page or through homebrew"
            }
            Self::SSHBootstrapAttempt => "Attempted bootstrapping for an SSH session",
            Self::SSHControlMasterError => {
                "Encountered a ControlMaster error during an SSH session"
            }
            Self::KeybindingChanged => "Edited a custom keybinding",
            Self::KeybindingResetToDefault => "Reset a custom keybinding to its default",
            Self::KeybindingRemoved => "Removed / cleared a keybinding",
            Self::FeaturesPageAction => "Changed settings in Features Page",
            Self::WorkflowExecuted => "Executed workflow",
            Self::WorkflowSelected => "Selected workflow and populated into the Input Editor",
            Self::OpenWorkflowSearch => "Opened workflows search in command search pane",
            Self::OpenQuakeModeWindow => {
                "Toggled quake mode window when previously hidden or closed"
            }
            Self::OpenWelcomeTips => "Opened welcome tips in app",
            Self::CompleteWelcomeTipFeature => "Completed all welcome tips items",
            Self::DismissWelcomeTips => "Dismissed Welcome tips",
            Self::ShowNotificationsDiscoveryBanner => {
                "Showed notifications discovery banner in the block list"
            }
            Self::NotificationsDiscoveryBannerAction => {
                "Showed banner introducing the notifications feature"
            }
            Self::ShowNotificationsErrorBanner => "Showed error banner for notifications feature",
            Self::NotificationsErrorBannerAction => "Showed error banner for notifications feature",
            Self::NotificationPermissionsRequested => {
                "Requested permission for desktop notification permissions"
            }
            Self::NotificationsRequestPermissionsOutcome => {
                "Recorded outcome of attempting to request desktop notification permissions"
            }
            Self::NotificationSent => "Sent desktop notification",
            Self::NotificationFailedToSend => "Failed to send desktop notification",
            Self::NotificationClicked => "Clicked desktop notification sent from Warp",
            Self::ToggleShowAgentTips => "Toggled the Show Agent Tips setting in AI settings",
            Self::ToggleFindOption => "Changed settings in Find Toggle",
            Self::SignUpButtonClicked => "Clicked \"Sign Up\" button",
            Self::LoginButtonClicked => "Clicked on \"Log in\" button",
            Self::OpenNewSessionFromFilePath => {
                "Dragged a file, folder, etc. into Warp to start a session"
            }
            Self::OpenTeamFromURI => {
                "Showed settings view of their newly joined team within the app"
            }
            Self::SelectCommandPaletteOption => "Selected option from command palette (i.e. CMD-P)",
            Self::PaletteSearchOpened => "Opened the palette",
            Self::PaletteSearchResultAccepted => "Accepted a command palette search result",
            Self::PaletteSearchExited => "Exited command palette search without accepting a result",
            Self::SelectNavigationPaletteItem => {
                "Selected session from the Session Navigation Palette (search across panes, tabs, and windows)"
            }
            Self::AuthCommonQuestionClicked => "Clicked on \"Common Question\" when logging in",
            Self::AuthToggleFAQ => "Toggled FAQ Page when logging in",
            Self::OpenAuthPrivacySettings => "Privacy settings are open during sign-in",
            Self::TabRenamed => "Changed tab title",
            Self::MoveActiveTab => "Move active tab left or right",
            Self::MoveTab => "Move tab left or right",
            Self::DragAndDropTab => "Tab dragged and dropped",
            Self::TabOperations => {
                "Took operation on a tab: change color, close tab, close adjacent tabs, etc."
            }
            Self::EditedInputBeforePrecmd => "Input edited before precmd hook completes",
            Self::TriedToExecuteBeforePrecmd => {
                "Attempted to execute command before precmd, a shell stage that has metadata on a command such as ssh, prompt info, etc."
            }
            Self::ThinStrokesSettingChanged => {
                "Changed thin strokes setting in settings -> Appearance"
            }
            Self::BookmarkBlockToggled => "Bookmarked or unbookmarked Block",
            Self::JumpToBookmark => "Jumped to bookmarked Block",
            Self::JumpToBottomofBlockButtonClicked => {
                "Used the button to jump to the bottom of a Block"
            }
            Self::ToggleJumpToBottomofBlockButton => {
                "Enabled or disabled the Jump to Bottom of Block Button"
            }
            Self::ToggleShowBlockDividers => "Enabled or disabled the Show Block Dividers Button",
            Self::OpenLink => "Opened a highlighted link within input or output",
            Self::OpenChangelogLink => "Opened the changelog link within the App",
            Self::ShowInFileExplorer => "Opened a file in Finder by using \"Show in Finder\"",
            Self::CommandXRayTriggered => {
                "Triggered Command X-Ray (hovering over a command for explanation)"
            }
            Self::OpenLaunchConfigSaveModal => "Opened save launch configuration modal",
            Self::SaveLaunchConfig => {
                "Saved current launch configuration of windows, tabs, and panes"
            }
            Self::OpenLaunchConfigFile => {
                "Opened the launch config YAML file from modal once saved successfully"
            }
            Self::OpenLaunchConfig => "Opened launch config for a session",
            Self::TeamCreated => "Created a Warp Drive team",
            Self::TeamJoined => "Joined a Warp Drive team",
            Self::TeamLeft => "Left a Warp Drive team",
            Self::TeamLinkCopied => "Copied a Warp Drive team link",
            Self::RemovedUserFromTeam => "Remove user from Warp Drive team",
            Self::DeletedWorkflow => "Deleted workflow from Warp Drive team",
            Self::DeletedNotebook => "Deleted notebook from Warp Drive team",
            Self::ToggleApprovalsModal => "Opened or closed teams modal",
            Self::ChangedInviteViewOption => "Toggled between link and invite for invite",
            Self::SendEmailInvites => "Sent email invites for Warp Drive team",
            Self::CommandCorrection => "Accepted command correction",
            Self::SetLineHeight => "Set line height through Settings -> Appearance",
            Self::ResourceCenterOpened => "Opened Resource Center pane",
            Self::ResourceCenterTipsCompleted => "Completed resource center tips",
            Self::ResourceCenterTipsSkipped => "Skipped welcome tips for new users",
            Self::KeybindingsPageOpened => "Opened the keybinding page within the resource center",
            Self::CommandSearchOpened => "Opened command search (universal search panel to search)",
            Self::CommandSearchExited => {
                "Exited command search (universal search panel to search) without accepting a result"
            }
            Self::CommandSearchResultAccepted => "Accepted command search result",
            Self::CommandSearchFilterChanged => "Changed command search filter",
            Self::CommandSearchAsyncQueryCompleted => {
                "Finished searching for a command in the background"
            }
            Self::AICommandSearchOpened => {
                "Opened the modal for AI Command Search, where you can use natural language to search for commands"
            }
            Self::OpenNotebook => "Opened a notebook",
            Self::EditNotebook => "Edited a notebook",
            Self::NotebookAction => {
                "Took an action on a notebook: edit, delete, modified font size, etc."
            }
            Self::OpenedAltScreenFind => "Opened the Find bar in the Alt Screen",
            Self::UserInitiatedClose => "Attempted to either quit the app or close a window",
            Self::QuitModalShown => {
                "Showed an alert modal to warn the user about closing the app/window with a running process"
            }
            Self::QuitModalCancel => "`Cancel` button on the alert modal was pressed",
            Self::QuitModalDisabled => {
                "The quit modal dialog has been disabled and will not popup when a user closes Warp while a session is running"
            }
            Self::UserInitiatedLogOut => {
                "Confirms a user has explicitly logged out of the application"
            }
            Self::LogOutModalShown => "When the log out modal is displayed",
            Self::LogOutModalCancel => "Escaped the log out flow by canceling the log out modal",
            Self::SetOpacity => {
                "Changed the opacity (window transparency) from the `Settings -> Appearance` dialog"
            }
            Self::SetBlurRadius => {
                "Changed the blur radius from the `Settings -> Appearance` dialog"
            }
            Self::ToggleDimInactivePanes => {
                "Whether the dim inactive panes feature has been toggled"
            }
            Self::InputModeChanged => {
                "Changed the Input Editor Mode (Pinned to Bottom, Pinned to Top, Classic / Waterfall Mode)"
            }
            Self::PtySpawned => {
                "Tracks the manner by which we create a new shell process (new codepath vs. old codepath).  Used to ensure nothing breaks as we change parts of our infrastructure."
            }
            Self::InitialWorkingDirectoryConfigurationChanged => {
                "Replaced the default working directory with a different path"
            }
            Self::OpenedWarpAI => "Activated Warp AI",
            Self::WarpAIRequestIssued => "Issued a question to Warp AI",
            Self::WarpAIAction => "Executed a Warp AI action: Restart, Copy, Insert into terminal",
            Self::UsedWarpAIPreparedPrompt => {
                "Used one of the Warp-provided prompts, like \"Show examples\""
            }
            Self::WarpAICharacterLimitExceeded => {
                "Attempted to ask a question longer than 1k chars to Warp AI"
            }
            Self::OpenInputContextMenu => "Opened the Input Editor's context menu",
            Self::InputCutSelectedText => {
                "Cut the highlighted text via the Input Editor's context menu (right clicking the buffer)"
            }
            Self::InputCopySelectedText => "Copied selected text from Input Editor",
            Self::InputSelectAll => {
                "Selected all the text in the Input Editor via its context menu (right clicking the buffer)"
            }
            Self::InputPaste => {
                "Pasted text into the Input Editor's via its context menu (right clicking the buffer)"
            }
            Self::InputCommandSearch => {
                "Opened Command Search via the Input Editor's context menu (right clicking the buffer)"
            }
            Self::InputAICommandSearch => {
                "Opened AI Command Search via the Input Editor's context menu (right clicking the buffer)"
            }
            Self::InputAskWarpAI => "Clicked \"Ask Warp AI\" from the Input Editor's context menu",
            Self::SaveAsWorkflowModal => {
                "Opened the modal to create a new workflow using a Block's context--command, etc."
            }
            Self::ExperimentTriggered => "Client assigned to A/B test",
            Self::ToggleSyncAllPanesInAllTabs => {
                "Enable the synchronization of the Input Editor's buffer to all the panes in all the tabs"
            }
            Self::ToggleSyncAllPanesInTab => {
                "Enable the synchronization of the Input Editor's buffer to all the panes in the current tab"
            }
            Self::ToggleSameLinePrompt => "Toggled on/off same line prompt",
            Self::ToggleNewWindowsAtCustomSize => {
                "Whether the new windows at custom size feature has been toggled"
            }
            Self::ToggleFocusPaneOnHover => {
                "Toggled on/off focus pane on hover feature, which causes panes to automatically focus when hovering over them"
            }
            Self::SetNewWindowsAtCustomSize => {
                "Set new windows at custom size through Settings -> Appearance"
            }
            Self::DisableInputSync => {
                "Disabled / turn off the Input Synchronization (across editors)"
            }
            Self::ToggleTabIndicators => {
                "Enabled or disabled the tab indicators (failed command, etc.)"
            }
            Self::TogglePreserveActiveTabColor => {
                "Enabled or disabled preserving the active tab color"
            }
            Self::ShowSubshellBanner => {
                "Displayed the banner asking whether Warp should Warpify the current session via Warp's subshell wrapper"
            }
            Self::SshTmuxWarpifyBannerDisplayed => {
                "Displayed the banner asking whether Warp should Warpify the current SSH session via Warp's SSH Wrapper"
            }
            Self::DeclineSubshellBootstrap => {
                "Developer declined the Warp banner to Warpify the current session"
            }
            Self::TriggerSubshellBootstrap => {
                "Attempted to Warpify the current session via Warp's subshell wrapper"
            }
            Self::AddDenylistedSubshellCommand => {
                "Explicitly prevent a command from being Warpified via Warp's subshell wrapper"
            }
            Self::RemoveDenylistedSubshellCommand => {
                "Removed a command from the list of commands to IGNORE when trying to Warpify via Warp's subshell wrapper"
            }
            Self::AddAddedSubshellCommand => {
                "Added a command to be automatically Warpified via Warp's subshell wrapper"
            }
            Self::RemoveAddedSubshellCommand => {
                "Removed a command from the list of commands to automatically Warpify via Warp's subshell wrapper"
            }
            Self::ReceivedSubshellRcFileDcs => "Spawned a subshell to be automatically Warpified",
            Self::ToggleSshTmuxWrapper => {
                "Changed the setting for SSH sessions to prompt for Tmux Wrapper"
            }
            Self::ToggleSshWarpification => "Changed the setting for SSH sessions to be warified",
            Self::SetSshExtensionInstallMode => {
                "Changed the SSH extension install mode (always ask / always allow / always skip)"
            }
            Self::SshRemoteServerChoiceDoNotAskAgainToggled => {
                "Toggled the 'Don't ask me this again' checkbox on the SSH remote-server choice block"
            }
            Self::AddDenylistedSshTmuxWrapperHost => {
                "Added a SSH host to the denylist for prompting for Tmux Wrapper"
            }
            Self::RemoveDenylistedSshTmuxWrapperHost => {
                "Removed an SSH host from the denylist from prompting for Tmux Wrapper"
            }
            Self::AgentModeRatedResponse => "User rated an Agent Mode response",
            Self::SshInteractiveSessionDetected => "An interactive SSH session was detected",
            Self::SshTmuxWarpifyBlockAccepted => "User accepted an ssh tmux warpify block",
            Self::SshTmuxWarpifyBlockDismissed => "User dismissed an ssh tmux warpify block",
            Self::WarpifyFooterShown => {
                "Displayed the warpify footer for a detected subshell or SSH session"
            }
            Self::AgentToolbarDismissed => "User dismissed the use-agent toolbar",
            Self::WarpifyFooterAcceptedWarpify => "User clicked Warpify in the warpify footer",
            Self::SshTmuxWarpificationSuccess => "Ssh tmux warpification succeeded",
            Self::SshTmuxWarpificationErrorBlock => "Ssh tmux warpification errored out",
            Self::SshInstallTmuxBlockDisplayed => "Displayed an ssh install tmux block",
            Self::SshInstallTmuxBlockAccepted => "User accepted an ssh install tmux block",
            Self::SshInstallTmuxBlockDismissed => "User dismissed an ssh install tmux block",
            Self::ShowAliasExpansionBanner => {
                "Displayed the banner asking whether Warp should automatically expand aliases within the Input Editor"
            }
            Self::EnableAliasExpansionFromBanner => {
                "Enabled automatic alias expansion within the Input Editor from the banner"
            }
            Self::DismissAliasExpansionBanner => {
                "Dismissed the banner to enable automatic alias expansion within the Input Editor"
            }
            Self::ShowVimKeybindingsBanner => {
                "Displayed the banner asking whether Warp should enable Vim keybindings in the Input Editor"
            }
            Self::EnableVimKeybindingsFromBanner => {
                "Enabled Vim keybindings in the Input Editor from the banner"
            }
            Self::DismissVimKeybindingsBanner => {
                "Dismissed the banner to enable Vim keybindings in the Input Editor"
            }
            Self::InitiateReauth => "Started the flow to re-authenticate the client",
            Self::NeedsReauth => "User needs to re-authenticate",
            Self::WarpDriveOpened => "Opened Warp Drive panel",
            Self::ToggleWarpAI => {
                "Toggled Warp AI--an AI assistant to help you debug errors, look up forgotten commands and more"
            }
            Self::ToggleSecretRedaction => {
                "Toggled on/off the setting for Secret Redaction - attempts to redact secrets and sensitive information"
            }
            Self::CustomSecretRegexAdded => "Custom Secret Regex Added",
            Self::ToggleObfuscateSecret => "Revealed or hid a secret",
            Self::CopySecret => "Copied a secret's obfuscated contents to clipboard",
            Self::AutoGenerateMetadataSuccess => {
                "Successfully generated metadata for a workflow using Warp AI"
            }
            Self::AutoGenerateMetadataError => {
                "Failed to generate metadata for a workflow using Warp AI"
            }
            Self::UpdateSortingChoice => "Modified the sorting scheme for Warp Drive objects",
            Self::UndoClose => "Re-opened a closed tab or window (undo closing a tab or window)",
            Self::PtyThroughput => "A sample of the max PTY throughput in bytes/sec",
            Self::DuplicateObject => "Cloned a Warp Drive object",
            Self::ExportObject => "Exported a Warp Drive object",
            Self::CommandFileRun => {
                "Opened a .cmd or unix executable file and ran it directly in Warp"
            }
            Self::PageUpDownInEditorPressed => {
                "Pressed `PAGE-UP` or `PAGE-DOWN` within the Input Editor"
            }
            Self::StartedSharingCurrentSession => "Started sharing the current session",
            Self::StoppedSharingCurrentSession => "Halted sharing the current session",
            Self::JoinedSharedSession => {
                "When you join another instance of Warp using shared sessions"
            }
            Self::SharedSessionModalUpgradePressed => {
                "Pressed upgrade after reaching max session sharing limit"
            }
            Self::SharerCancelledGrantRole => {
                "When you cancel granting a role to a shared session participant"
            }
            Self::SharerGrantModalDontShowAgain => {
                "When you check don't show again on the confirmation modal for granting a role"
            }
            Self::JumpToSharedSessionParticipant => {
                "Clicked on a shared session participant avatar to jump to their location in the session"
            }
            Self::CopiedSharedSessionLink => "Copied a shared session link",
            Self::WebSessionOpenedOnDesktop => {
                "Shared session viewed on the web was opened on the desktop"
            }
            Self::WebCloudObjectOpenedOnDesktop => {
                "Warp Drive object on the web was opened on the desktop"
            }
            Self::DriveSharingOnboardingBlockShown => {
                "Showed onboarding block for Warp Drive sharing"
            }
            Self::UnsupportedShell => "Booted Warp with a shell that isn't supported",
            Self::LogOut => "Logged out of the Warp client",
            Self::SettingsImportInitiated => "Started the import settings flow for new users",
            Self::InviteTeammates => "Sent emails to invite teammates to join Warp Drive team",
            Self::CopyObjectToClipboard => "Copied an object to the user's keyboard",
            Self::OpenAndWarpifyDockerSubshell => {
                "Warpifying a docker subshell from using the docker extension"
            }
            Self::UpdateBlockFilterQuery => "When a new filter is applied to a block",
            Self::UpdateBlockFilterQueryContextLines => {
                "When the number of context lines for a block filter query is updated"
            }
            Self::ToggleBlockFilterQuery => "Toggled on/off a block filter query",
            Self::ToggleBlockFilterCaseSensitivity => {
                "Toggled on/off case sensitivity within the block filter editor"
            }
            Self::ToggleBlockFilterRegex => "Toggled on/off regex within the block filter editor",
            Self::ToggleBlockFilterInvert => "Toggled on/off invert within the block filter editor",
            Self::BlockFilterToolbeltButtonClicked => {
                "Clicked the block filter icon in the top-right of a block"
            }
            Self::ToggleSnackbarInActivePane => {
                "Expanded or collapsed the sticky command header in the active pane"
            }
            Self::PaneDragInitiated => "Initiated dragging a pane via the header",
            Self::PaneDropped => "Ended dragging a pane via the pane header",
            Self::AgentModeCreatedAIBlock => "Created an AI block in agent mode",
            Self::TierLimitHit => "User hit the tier limit for a feature",
            Self::SharedObjectLimitHitBannerViewPlansButtonClicked => {
                "Clicked the 'View Plans' button on the persistent drive banner"
            }
            Self::AgentModeUserAttemptedQueryAtRequestLimit => {
                "Tried to send an Agent Mode query but they already reached the query limit"
            }
            Self::AgentModeClickedEntrypoint => "Clicked on an Agent Mode entrypoint",
            Self::AgentModeAttachedBlockContext => {
                "Attached block as context to an Agent Mode query"
            }
            Self::ResourceUsageStats => "Periodic report on application resource usage statistics",
            Self::MemoryUsageStats => "Periodic report on application memory usage statistics",
            Self::MemoryUsageHigh => {
                "Total application memory usage exceeded a significant threshold"
            }
            Self::AgentModeToggleAutoDetectionSetting => {
                "Toggled the setting that enables or disables natural language auto-detection in the input. "
            }
            Self::AgentModePotentialAutoDetectionFalsePositive => {
                "Manually toggled input to shell mode after input was auto-detected as natural language."
            }
            Self::AgentModeChangedInputType => {
                "The input type was changed from shell -> AI or AI -> shell"
            }
            Self::AgentModePrediction => "Completed an Agent Predict prediction",
            Self::ToggleIntelligentAutosuggestionsSetting => {
                "Toggled on/off the intelligent autosuggestions setting"
            }
            Self::TogglePromptSuggestionsSetting => "Toggled on/off the prompt suggestions setting",
            Self::ToggleCodeSuggestionsSetting => "Toggled on/off the code suggestions setting",
            Self::ToggleNaturalLanguageAutosuggestionsSetting => {
                "Toggled on/off the natural language autosuggestions setting"
            }
            Self::ToggleSharedBlockTitleGenerationSetting => {
                "Toggled on/off the shared block title generation setting"
            }
            Self::ToggleGitOperationsAutogenSetting => {
                "Toggled on/off the git operations autogen setting"
            }
            Self::ToggleVoiceInputSetting => "Toggled on/off the voice input setting",
            Self::UnitTestSuggestionShown { .. } => "Suggested prompt shown",
            Self::UnitTestSuggestionAccepted { .. } => "Suggested prompt accepted",
            Self::UnitTestSuggestionCancelled { .. } => "Suggested prompt cancelled",
            Self::PromptSuggestionShown => "Prompt Suggestions banner shown",
            Self::SuggestedCodeDiffBannerShown => "Suggested Code Diff banner shown",
            Self::SuggestedCodeDiffFailed => "Suggested Code Diff Failed",
            Self::PromptSuggestionAccepted => "Prompt Suggestion accepted",
            Self::StaticPromptSuggestionsBannerShown => "Static Prompt Suggestions banner shown",
            Self::StaticPromptSuggestionAccepted => "Static Prompt Suggestion accepted",
            Self::ZeroStatePromptSuggestionUsed => "Used a zero state prompt suggestion",
            Self::AgentModeCodeSuggestionEditedByUser => {
                "Agent Mode Code suggestion edited by user"
            }
            Self::AgentModeCodeFilesNavigated => "Agent Mode Code files navigated",
            Self::AgentModeCodeDiffHunksNavigated => "Agent Mode Code diff hunks navigated",
            Self::EnvVarCollectionInvoked => "Invoked an environment variables object",
            Self::EnvVarWorkflowParameterization => {
                "Selected from environment variables dropdown to parameterize workflow"
            }
            Self::ObjectLinkCopied => "The web link to an object has been copied.",
            Self::FileTreeToggled => "Opened the file tree/project explorer",
            Self::GlobalSearchOpened => "Opened the global search view",
            Self::GlobalSearchQueryStarted => "Started a global search (warp_ripgrep) search",
            Self::FileTreeItemAttachedAsContext => {
                "Attached a file or directory as context from the file tree"
            }
            Self::CodeSelectionAddedAsContext => {
                "Added selected code as context from the code editor"
            }
            Self::FileTreeItemCreated => "Created a new file from the file tree",
            Self::ConversationListViewOpened => {
                "Opened the conversation list view in the left panel"
            }
            Self::ConversationListItemOpened => "Opened a conversation from the conversation list",
            Self::ConversationListItemDeleted => {
                "Deleted a conversation from the conversation list"
            }
            Self::ConversationListLinkCopied => {
                "Copied a conversation link from the conversation list"
            }
            Self::AgentViewEntered => "User entered the Agent View",
            Self::AgentViewExited => "User exited the Agent View",
            Self::InlineConversationMenuOpened => {
                "User opened the inline conversation menu in Agent View"
            }
            Self::InlineConversationMenuItemSelected => {
                "User selected an item from the inline conversation menu"
            }
            Self::AgentShortcutsViewToggled => "User toggled the shortcuts view in Agent View",
            Self::CreateProjectPromptSubmitted => {
                "User submitted a prompt from the create project view"
            }
            Self::CreateProjectPromptSubmittedContent => {
                "User submitted custom prompt content from the create project view"
            }
            Self::CloneRepoPromptSubmitted => {
                "User submitted a repository URL from the clone repo view"
            }
            Self::GetStartedSkipToTerminal => "User clicked skip to terminal from get started view",
            Self::CompletedSettingsImport => {
                "Imported a terminal's settings via the settings import onboarding block"
            }
            Self::SettingsImportConfigFocused => {
                "Selected a terminal in the settings import onboarding block"
            }
            Self::SettingsImportResetButtonClicked => {
                "Reset the imported settings in the settings import onboarding block"
            }
            Self::SettingsImportConfigParsed => {
                "Parsed a terminal's settings as part of settings import"
            }
            Self::ITermMultipleHotkeys => {
                "Attempted to import an iTerm profile that contained multiple hotkey window bindings"
            }
            Self::ToggleWorkspaceDecorationVisibility => "Toggled when to display the tab bar",
            Self::UpdateAltScreenPaddingMode => {
                "Updated the custom padding setting for the alt-screen"
            }
            Self::AddTabWithShell => "Added a tab with specific shell",
            Self::AgentModeSurfacedCitations => {
                "Agent mode used and cited external sources that were used in its response"
            }
            Self::AgentModeOpenedCitation => "Opened a citation that was surfaced in agent mode",
            Self::OpenedSharingDialog => {
                "Opened the sharing settings dialog for a session or Warp Drive object"
            }
            Self::ToggleGlobalAI => "Toggled global AI enablement.",
            Self::ToggleActiveAI => "Toggled active AI enablement.",
            Self::ToggleLigatureRendering => "Toggled ligature rendering",
            Self::WorkflowAliasAdded => "Added an alias to a Warp Drive workflow",
            Self::WorkflowAliasRemoved => "Removed an alias from a Warp Drive workflow",
            Self::WorkflowAliasArgumentEdited => {
                "Edited an argument in a Warp Drive workflow alias"
            }
            Self::WorkflowAliasEnvVarsAttached => {
                "Added or removed environment variables for a Warp Drive workflow alias"
            }
            Self::ToggledAgentModeAutoexecuteReadonlyCommandsSetting => {
                "Toggled setting to autoexecute readonly Agent Mode requested commands"
            }
            Self::ChangedAgentModeCodingPermissions => {
                "Changed Agent Mode permissions for coding tasks"
            }
            Self::ChangedAgentModeAskUserQuestionPermission => {
                "Changed Agent Mode permission for asking user questions"
            }
            Self::AutoexecutedAgentModeRequestedCommand => {
                "Autoexecuted an Agent Mode requested command"
            }
            Self::AgenticOnboardingBlockSelected => {
                "Selected an agentic onboarding block to execute"
            }
            Self::AttachedImagesToAgentModeQuery => "Attached images to an Agent Mode query",
            #[cfg(windows)]
            Self::WSLRegistryError => {
                "Encountered an error while fetching WSL distributions from the registry"
            }
            #[cfg(windows)]
            Self::AutoupdateUnableToCloseApplications => {
                "The Windows auto-update installer was unable to automatically close all applications before installing the update"
            }
            #[cfg(windows)]
            Self::AutoupdateFileInUse => {
                "The Windows auto-update installer encountered a file-in-use error during installation"
            }
            #[cfg(windows)]
            Self::AutoupdateMutexTimeout => {
                "The Windows auto-update installer timed out waiting for Warp to release its mutex; a force-kill was attempted"
            }
            #[cfg(windows)]
            Self::AutoupdateForcekillFailed { .. } => {
                "The Windows auto-update installer failed to force-kill Warp after the mutex timeout"
            }
            #[cfg(windows)]
            Self::AutoupdateMinidumpCleanupFailed { .. } => {
                "The Windows auto-update installer failed to clean up the orphaned minidump server process"
            }
            Self::ToggleCodebaseContext => {
                "Toggled on/off the enablement of codebase context usage for Agent Mode."
            }
            Self::ToggleAutoIndexing => {
                "Toggled on/off the enablement of autoindexing for codebase context."
            }
            Self::ActiveIndexedReposChanged => {
                "Active indexed repositories changed, affecting codebase context."
            }
            Self::ExecutedWarpDrivePrompt => "Executed a saved prompt.",
            Self::ImageReceived => "Received an image through an image protocol over the pty",
            Self::FileExceededContextLimit => "File from AI exceeded context limit",
            Self::AgentModeError => "Received an error when getting Agent Mode response",
            Self::AgentModeRequestRetrySucceeded => {
                "Agent Mode request succeeded after retrying following an initial error"
            }
            Self::GrepToolSucceeded => "The grep tool completed successfully",
            Self::GrepToolFailed => "The grep tool failed to complete",
            Self::FileGlobToolSucceeded => "The file glob tool completed successfully",
            Self::FileGlobToolFailed { .. } => "The file glob tool failed to complete",
            Self::ShellTerminatedPrematurely { .. } => "The shell process terminated prematurely",
            Self::FullEmbedCodebaseContextSearchSuccess => {
                "Successfully searched full embed codebase context"
            }
            Self::FullEmbedCodebaseContextSearchFailed => {
                "Failed to search full embed codebase context"
            }
            Self::ShowedSuggestedAgentModeWorkflowChip => {
                "Showed the Suggested Agent Mode workflow chip to the user"
            }
            Self::SearchCodebaseRequested { .. } => "Ran the Search Codebase tool",
            Self::SearchCodebaseRepoUnavailable { .. } => {
                "Tried to use the Search Codebase tool on a repo that is unavailable"
            }
            Self::InputUXModeChanged { .. } => "Changed the input UX mode",
            Self::ContextChipInteracted { .. } => "Interacted with a context chip",
            Self::VoiceInputUsed { .. } => "Used voice input",
            Self::AtMenuInteracted { .. } => "Interacted with the @ menu",
            Self::UserMenuUpgradeClicked => "Clicked the 'Upgrade' menu item in the user menu",
            Self::TabCloseButtonPositionUpdated { .. } => "Updated the tab close button position",
            Self::ExpandedCodeSuggestions { .. } => "Expanded the passive code diff suggestion",
            Self::AIExecutionProfileCreated => "A new AI execution profile was created",
            Self::AIExecutionProfileDeleted => "An AI execution profile was deleted",
            Self::AIExecutionProfileSettingUpdated { .. } => {
                "An AI execution profile setting was updated"
            }
            Self::AIExecutionProfileAddedToAllowlist { .. } => {
                "An item was added to an AI execution profile allowlist"
            }
            Self::AIExecutionProfileAddedToDenylist { .. } => {
                "An item was added to an AI execution profile denylist"
            }
            Self::AIExecutionProfileRemovedFromAllowlist { .. } => {
                "An item was removed from an AI execution profile allowlist"
            }
            Self::AIExecutionProfileRemovedFromDenylist { .. } => {
                "An item was removed from an AI execution profile denylist"
            }
            Self::AIExecutionProfileModelSelected { .. } => {
                "An AI model was selected for an AI execution profile"
            }
            Self::AIInputNotSent { .. } => "The AI input was not sent",
            Self::OpenSlashMenu { .. } => "Opened the slash commands menu",
            Self::SlashCommandAccepted { .. } => "User accepted a slash command",
            Self::AgentModeSetupBannerAccepted { .. } => "Agent Mode setup banner accepted",
            Self::AgentModeSetupBannerDismissed => "Agent Mode setup banner dismissed",
            Self::AgentModeSetupProjectScopedRulesAction { .. } => {
                "User clicked a button in the Agent Mode setup project scoped rules step"
            }
            Self::AgentModeSetupCodebaseContextAction { .. } => {
                "User clicked a button in the Agent Mode setup codebase context step"
            }
            Self::AgentModeSetupCreateEnvironmentAction { .. } => {
                "User clicked a button in the Agent Mode setup create environment step"
            }
            Self::InputBufferSubmitted => "Input buffer submitted",
            Self::RecentMenuItemSelected { .. } => {
                "User selected an item from the recents list on the new tab zero state"
            }
            Self::OpenRepoFolderSubmitted { .. } => {
                "User selected a folder to open as a repo from the \"Open repository\" button"
            }
            Self::OutOfCreditsBannerClosed => {
                "User closed the 'Out of credits' banner (dismissed or purchased credits)"
            }
            Self::AutoReloadModalClosed => {
                "User closed the auto-reload modal (either dismissed or enabled auto-reload)"
            }
            Self::AutoReloadToggledFromBillingSettings => {
                "User toggled auto-reload in Billing & Usage settings"
            }
            Self::CLISubagentControlStateChanged { .. } => {
                "Control state changed in CLI subagent (agent in control, agent blocked, user in control, or agent tagged in)"
            }
            Self::CLISubagentResponsesToggled { .. } => {
                "User toggled the visibility of agent responses in CLI subagent"
            }
            Self::CLISubagentInputDismissed { .. } => {
                "User dismissed the input in the CLI subagent"
            }
            Self::CLISubagentActionExecuted { .. } => {
                "User approved a blocked action from the CLI subagent"
            }
            Self::CLISubagentActionRejected { .. } => {
                "User rejected a blocked action from the CLI subagent"
            }
            Self::AgentManagementViewToggled { .. } => {
                "User toggled the Agent Management View open or closed"
            }
            Self::AgentManagementViewOpenedSession => {
                "User opened a session from the Agent Management View"
            }
            Self::AgentManagementViewCopiedSessionLink => {
                "User copied a session link from the Agent Management View"
            }
            Self::DetectedIsolationPlatform { .. } => {
                "Detected that Warp is running in an isolated sandbox"
            }
            Self::AgentTipShown => "Selected an Agent Tip to show in the Agent Mode status bar",
            Self::AgentTipClicked => "User clicked a link or action in an Agent Tip",
            Self::AgentExitedShellProcess => {
                "An agent-requested command caused the shell process to exit"
            }
            Self::CLIAgentToolbarVoiceInputUsed { .. } => {
                "User used voice input from the CLI agent footer"
            }
            Self::CLIAgentToolbarImageAttached { .. } => {
                "User attached an image from the CLI agent footer"
            }
            Self::CLIAgentToolbarShown { .. } => "CLI agent footer was shown to the user",
            Self::CLIAgentPluginChipClicked { .. } => {
                "User clicked the plugin install or update chip"
            }
            Self::CLIAgentPluginChipDismissed { .. } => {
                "User dismissed the plugin install or update chip"
            }
            Self::CLIAgentPluginOperationSucceeded { .. } => {
                "Auto plugin install or update completed successfully"
            }
            Self::CLIAgentPluginOperationFailed { .. } => {
                "Auto plugin install or update failed"
            }
            Self::CLIAgentPluginDetected { .. } => {
                "A CLI agent plugin was detected via a SessionStart event"
            }
            Self::AgentNotificationShown { .. } => {
                "An agent notification was shown to the user (toast or mailbox)"
            }
            Self::CLIAgentRichInputOpened { .. } => "User opened CLI agent Rich Input",
            Self::CLIAgentRichInputClosed { .. } => "CLI agent Rich Input was closed",
            Self::CLIAgentRichInputSubmitted { .. } => {
                "User submitted a prompt via CLI agent Rich Input"
            }
            Self::ToggleCLIAgentToolbarSetting { .. } => {
                "User toggled the CLI agent footer setting"
            }
            Self::ToggleUseAgentToolbarSetting { .. } => {
                "User toggled the Use Agent footer setting"
            }
            Self::CodexModalOpened => "User opened the Codex modal",
            Self::CodexModalUseCodexClicked => "User clicked 'Use Codex' in the Codex modal",
            Self::LinearIssueLinkOpened => {
                "User opened a warp://linear deeplink to work on an issue"
            }
            Self::CloudAgentCapacityModalOpened => "User opened the cloud agent capacity modal",
            Self::CloudAgentCapacityModalDismissed => {
                "User dismissed the cloud agent capacity modal"
            }
            Self::CloudAgentCapacityModalUpgradeClicked => {
                "User clicked the upgrade button in the cloud agent capacity modal"
            }
            Self::ComputerUseApproved => {
                "A RequestComputerUse action was approved (manually or auto-executed)"
            }
            Self::ComputerUseCancelled => "A RequestComputerUse action was cancelled/rejected",
            Self::FreeTierLimitHitInterstitialDisplayed { .. } => {
                "The free tier limit hit interstitial was displayed"
            }
            Self::FreeTierLimitHitInterstitialUpgradeButtonClicked { .. } => {
                "User clicked the 'Upgrade' button in the free tier limit hit interstitial"
            }
            Self::FreeTierLimitHitInterstitialClosed { .. } => {
                "User closed the free tier limit hit interstitial"
            }
            Self::RemoteServerBinaryCheck => {
                "Remote server binary check completed (found, not found, or error)"
            }
            Self::RemoteServerInstallation => {
                "Remote server binary installation completed (success or failure)"
            }
            Self::RemoteServerInitialization => {
                "Remote server connection and initialization completed (success or failure)"
            }
            Self::RemoteServerDisconnection => {
                "An established remote server connection was dropped"
            }
            Self::RemoteServerClientRequestError => {
                "A client request to the remote server failed"
            }
            Self::RemoteServerMessageDecodingError => {
                "A server message could not be decoded (no parseable request_id)"
            }
            Self::RemoteServerSetupDuration => {
                "End-to-end duration of the remote server setup flow"
            }
            Self::RemoteServerHostUnsupported => {
                "Preinstall check classified the remote host as unsupported, \
                 falling back to the legacy SSH flow"
            }
            Self::RemoteServerReconnection => {
                "A reconnection attempt succeeded after a spontaneous disconnect"
            }
            Self::RemoteServerReconnectExhausted => {
                "All reconnection attempts were exhausted after a spontaneous disconnect"
            }
        }
    }
}

warp_core::register_telemetry_event!(TelemetryEvent);

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
