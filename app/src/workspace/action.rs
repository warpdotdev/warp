use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use session_sharing_protocol::common::SessionId;
use ui_components::lightbox;
use warp_util::path::LineAndColumnArg;
use warpui::accessibility::AccessibilityVerbosity;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::Vector2F;
use warpui::platform::Cursor;
use warpui::{EntityId, WeakViewHandle, WindowId};

use super::global_actions::{ForkFromExchange, ForkedConversationDestination};
use super::tab_settings::{
    VerticalTabsCompactSubtitle, VerticalTabsDisplayGranularity, VerticalTabsPrimaryInfo,
    VerticalTabsTabItemMode, VerticalTabsViewMode,
};
use super::view::{OnboardingTutorial, WorkspaceBanner};
use crate::ai::agent::api::ServerConversationToken;
#[cfg(not(target_family = "wasm"))]
use crate::ai::agent::conversation::AIAgentHarness;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::PendingAttachment;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentVersion};
use crate::auth::auth_manager::LoginGatedFeature;
use crate::drive::items::WarpDriveItemId;
use crate::drive::CloudObjectTypeAndId;
use crate::palette::PaletteMode;
use crate::pane_group::PaneGroup;
use crate::prompt::editor_modal::OpenSource as PromptEditorOpenSource;
use crate::search;
use crate::server::ids::SyncId;
use crate::server::telemetry::{
    AddTabWithShellSource, AgentModeEntrypoint, PaletteSource, SharingDialogSource,
};
use crate::settings_view::{SettingsAction as SettingsTabAction, SettingsSection};
use crate::tab::{NewSessionMenuItem, SelectedTabColor};
use crate::tab_configs::TabConfig;
use crate::terminal::available_shells::AvailableShell;
use crate::terminal::view::inline_banner::ZeroStatePromptSuggestionType;
use crate::themes::theme::AnsiColorIdentifier;
use crate::themes::theme_chooser::ThemeChooserMode;
use crate::workflows::{WorkflowSelectionSource, WorkflowSource, WorkflowType};
use crate::workspace::tab_group::TabGroupId;
use crate::workspace::PaneViewLocator;

/// This enum determines how the search query is initialized when opening command search.
#[derive(Clone, Default, Debug)]
pub enum InitContent {
    /// Read the content of the active terminal input, and make that the initial search query.
    #[default]
    FromInputBuffer,
    /// Specify an exact string to initialize the query to.
    Custom(String),
}

/// To initialize command search, we may want to specify a search filter, or the content of the
/// query itself.
#[derive(Clone, Default, Debug)]
pub struct CommandSearchOptions {
    pub filter: Option<search::QueryFilter>,
    pub init_content: InitContent,
}

/// Specifies how to restore a conversation when it's not already open in a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum RestoreConversationLayout {
    /// Restore the conversation into the currently active pane.
    ActivePane,
    /// Restore the conversation in a new split pane.
    SplitPane,
    /// Restore the conversation in a new tab.
    #[default]
    NewTab,
}

#[derive(Debug, Clone, Copy)]
pub enum TabContextMenuAnchor {
    Pointer(Vector2F),
    VerticalTabsKebab,
}

/// Describes how the new-session dropdown menu was opened so the renderer
/// can pick the right anchor strategy.
#[derive(Debug, Clone, Copy)]
pub enum NewSessionMenuAnchor {
    /// Menu was opened from the `+` add-tab button. When vertical tabs are
    /// active, the renderer anchors below the button's save position;
    /// otherwise the contained position is used directly.
    AddTabButton(Vector2F),
    /// Menu was opened by right-clicking the vertical tabs panel.
    /// Always anchored at the contained pointer position.
    Pointer(Vector2F),
}

impl NewSessionMenuAnchor {
    pub fn position(&self) -> Vector2F {
        match self {
            Self::AddTabButton(position) | Self::Pointer(position) => *position,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum VerticalTabsPaneContextMenuTarget {
    ClickedPane(PaneViewLocator),
    ActivePane(PaneViewLocator),
}

impl VerticalTabsPaneContextMenuTarget {
    pub fn locator(self) -> PaneViewLocator {
        match self {
            Self::ClickedPane(locator) | Self::ActivePane(locator) => locator,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoCloudHandoffTrigger {
    MacOsSleep,
    Uri,
}

#[derive(Debug, Clone)]
pub enum WorkspaceAction {
    ActivateTab(usize),
    ActivatePrevTab,
    ActivateNextTab,
    ActivateLastTab,
    CyclePrevSession,
    CycleNextSession,
    MoveActiveTabLeft,
    MoveActiveTabRight,
    MoveTabLeft(usize),
    MoveTabRight(usize),
    RenameTab(usize),
    ResetTabName(usize),
    RenamePane(PaneViewLocator),
    ResetPaneName(PaneViewLocator),
    RenameActiveTab,
    /// Renames the focused pane in the active tab. Mirrors `RenameActiveTab`
    /// so the action is reachable from the binding registry / Command Palette
    /// (see #9351). The context-menu path keeps using `RenamePane(locator)`.
    RenameActivePane,
    SetActiveTabName(String),
    /// Sets the manual color override for the active tab.
    ///
    /// - `Color(_)` — apply that color.
    /// - `Cleared` — explicitly clear (suppresses any directory default).
    /// - `Unset` — remove the manual override (lets the directory default apply, if any).
    SetActiveTabColor(SelectedTabColor),
    ToggleTabRightClickMenu {
        tab_index: usize,
        anchor: TabContextMenuAnchor,
    },
    /// Toggles the multi-tab selection right-click menu.
    /// Dispatched by the UI when the right-clicked tab is part of a multi-tab
    /// selection (cmd-click or shift-click).
    ToggleTabSelectionRightClickMenu {
        tab_index: usize,
        anchor: TabContextMenuAnchor,
    },
    ToggleVerticalTabsPaneContextMenu {
        tab_index: usize,
        target: VerticalTabsPaneContextMenuTarget,
        position: Vector2F,
    },
    TabHoverWidthStart {
        width: f32,
    },
    TabHoverWidthEnd,
    ToggleTabBarOverflowMenu,
    ToggleWelcomeTips,
    CloseTab(usize),
    CloseActiveTab,
    CloseOtherTabs(usize),
    CloseNonActiveTabs,
    CloseTabsRight(usize),
    CloseTabsRightActiveTab,
    /// Close every tab that belongs to the given tab group.
    CloseTabGroup(TabGroupId),
    /// Toggle collapsed state for the given tab group.
    ToggleTabGroupCollapsed(TabGroupId),
    /// Opens an inline editor over the given group's header for renaming.
    RenameTabGroup(TabGroupId),
    /// Cancels any active rename (tab, pane, or group) without committing the
    /// new name. Dispatched when clicking on the vtab panel background while a
    /// rename editor is open.
    CancelActiveRename,
    /// Creates a new tab group containing the tab at the given index.
    NewTabGroupFromTab(usize),
    /// Moves the tab at `tab_index` into `group_id`, appending it to the
    /// end of the group's contiguous run.
    MoveTabToGroup {
        tab_index: usize,
        group_id: TabGroupId,
    },
    /// Removes the tab at the given index from its current group.
    RemoveTabFromGroup(usize),
    /// Selects every tab between the active tab and the shift-clicked row (inclusive).
    ShiftSelectTabRange {
        locator: PaneViewLocator,
    },
    /// Toggles whether the tab at `locator` is part of the active multi-selection.
    /// Dispatched on cmd-click of a vertical tab row.
    ToggleTabMultiSelection {
        locator: PaneViewLocator,
    },
    /// Clears the tab multi-selection. Dispatched from the UI when the user takes
    /// an action that should cancel any active selections.
    ClearTabMultiSelection,
    /// Creates a new tab group from the current tab multi-selection.
    NewTabGroupFromSelectedTabs,
    /// Context-aware "create group" entry point for the keybinding: groups
    /// the multi-selection when 2+ tabs are selected, otherwise groups the
    /// active tab.
    NewTabGroupFromActiveOrSelectedTabs,
    /// Moves every selected tab into `group_id`.
    MoveSelectedTabsToGroup {
        group_id: TabGroupId,
    },
    /// Removes every selected tab from its group (requires a single shared group).
    RemoveSelectedTabsFromGroup,
    /// Context-aware "remove from group" entry point for the keybinding:
    /// removes the multi-selection from its shared group when 2+ tabs are
    /// selected, otherwise removes the active tab.
    RemoveActiveOrSelectedTabsFromGroup,
    ToggleTabGroupRightClickMenu {
        group_id: TabGroupId,
        anchor: TabContextMenuAnchor,
    },
    UngroupTabs(TabGroupId),
    NewTabInGroup(TabGroupId),
    MoveTabGroupUp(TabGroupId),
    MoveTabGroupDown(TabGroupId),
    CloseTabsOutsideGroup(TabGroupId),
    CloseTabsAboveGroup(TabGroupId),
    CloseTabsBelowGroup(TabGroupId),
    /// Pins the tab at the given index. If the tab is part of a group, it
    /// is first extracted from the group and then pinned as ungrouped.
    PinTab(usize),
    /// Unpins the tab at the given index.
    UnpinTab(usize),
    /// Pins the active tab.
    PinActiveTab,
    /// Unpins the active tab.
    UnpinActiveTab,
    /// Pins the entire tab group: sets the group as pinned
    /// and moves the group block to the end of the pinned region.
    PinTabGroup(TabGroupId),
    /// Unpins the entire tab group: clears the pinned flag on the group
    /// and moves the group block to the start of the unpinned region.
    UnpinTabGroup(TabGroupId),
    /// Pins the active tab's group.
    PinActiveTabGroup,
    /// Unpins the active tab's group.
    UnpinActiveTabGroup,
    AddDefaultTab,
    AddTerminalTab {
        hide_homepage: bool,
    },
    AddTabWithShell {
        shell: AvailableShell,
        source: AddTabWithShellSource,
    },
    AddGetStartedTab,
    AddAmbientAgentTab,
    /// Add a new tab that immediately enters agent view with a new conversation.
    AddAgentTab,
    /// Add a new tab running a local Docker sandbox via `sbx`.
    AddDockerSandboxTab,
    OpenNewSessionMenu {
        anchor: NewSessionMenuAnchor,
    },
    ToggleTabConfigsMenu,
    ToggleNewSessionMenu {
        anchor: NewSessionMenuAnchor,
    },
    SelectNewSessionMenuItem(NewSessionMenuItem),
    AutoupdateFailureLink,
    ApplyUpdate,
    LogOut,
    CopyVersion(&'static str),
    DownloadNewVersion,
    ConfigureKeybindingSettings {
        keybinding_name: Option<String>,
    },
    ShowSettings,
    ShowSettingsPage(SettingsSection),
    ShowSettingsPageWithSearch {
        search_query: String,
        section: Option<SettingsSection>,
    },
    ShowThemeChooser(ThemeChooserMode),
    ShowThemeChooserForActiveTheme,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    IncreaseZoom,
    DecreaseZoom,
    ResetZoom,
    ActivateTabByNumber(usize),
    OpenPalette {
        mode: PaletteMode,
        source: PaletteSource,
        query: Option<String>,
    },
    TogglePalette {
        mode: PaletteMode,
        source: PaletteSource,
    },
    ShowUpgrade,
    ShowReferralSettingsPage,
    JoinSlack,
    ViewUserDocs,
    ViewLatestChangelog,
    ViewPrivacyPolicy,
    SendFeedback,
    /// Open the log directory in the system file explorer with the current log file selected.
    #[cfg(not(target_family = "wasm"))]
    ViewLogs,
    ChangeCursor(Cursor),
    ToggleBlockSnackbar,
    ToggleErrorUnderlining,
    ToggleSyntaxHighlighting,
    CheckForUpdate,
    ExportAllWarpDriveObjects,
    SetA11yVerbosityLevel(AccessibilityVerbosity),
    ToggleNotifications,
    ToggleTabColor {
        color: AnsiColorIdentifier,
        tab_index: usize,
    },
    /// Toggles the color for a tab group. Clears the color if it was already
    /// set to `color`; otherwise applies `color` as the uniform group color.
    ToggleTabGroupColor {
        color: AnsiColorIdentifier,
        group_id: TabGroupId,
    },
    OpenLaunchConfigSaveModal,
    SelectTabConfig(TabConfig),
    DispatchToSettingsTab(SettingsTabAction),
    ToggleResourceCenter,
    ToggleUserMenu,
    ToggleAIAssistant,
    ClickedAIAssistantIcon,
    ToggleKeybindingsPage,
    ShowCommandSearch(CommandSearchOptions),
    CreatePersonalNotebook,
    ImportToPersonalDrive,
    ImportToTeamDrive,
    CreateTeamNotebook,
    CreatePersonalWorkflow,
    CreateTeamWorkflow,
    CreatePersonalFolder,
    CreateTeamFolder,
    CreateTeamEnvVarCollection,
    CreatePersonalEnvVarCollection,
    CreatePersonalAIPrompt,
    CreateTeamAIPrompt,
    ToggleMouseReporting,
    ToggleScrollReporting,
    ToggleFocusReporting,
    StartTabDrag,
    DragTab {
        tab_index: usize,
        tab_position: RectF,
    },
    DropTab,
    StartGroupDrag(TabGroupId),
    DragGroup {
        group_id: TabGroupId,
        /// The dragged group's painted rect.
        position: RectF,
        /// The position of the cursor while dragging a group.
        cursor_position: Vector2F,
    },
    DropGroup,
    /// Toggles the left panel. In Code Mode V1 this toggles Warp Drive.
    /// In Code Mode V2 this toggles the left panel which contains both the project explorer and
    /// Warp Drive. This happens as explicit action from the user.
    ToggleLeftPanel,
    /// Toggles directly to the Warp Drive tab of the left panel in Code Mode V2
    ToggleWarpDrive,
    /// Unconditionally opens Warp Drive. This is used in the case of user lifecycle
    /// events like new user onboarding or when the user joins a team.
    OpenWarpDrive,
    /// Toggles the right panel. This happens as an explicit action from the user.
    ToggleRightPanel,
    /// Opens the code review panel (right panel) without toggling. If already open,
    /// switches to the target pane's repo. Used by vertical tabs diff stats chip.
    OpenCodeReviewPanel(PaneViewLocator),
    /// Toggles the vertical tabs panel. This happens as an explicit action from the user.
    ToggleVerticalTabsPanel,
    OpenVerticalTabsPanel,
    ToggleVerticalTabsSettingsPopup,
    SetVerticalTabsDisplayGranularity(VerticalTabsDisplayGranularity),
    SetVerticalTabsTabItemMode(VerticalTabsTabItemMode),
    SetVerticalTabsViewMode(VerticalTabsViewMode),
    SetVerticalTabsPrimaryInfo(VerticalTabsPrimaryInfo),
    SetVerticalTabsCompactSubtitle(VerticalTabsCompactSubtitle),
    ToggleVerticalTabsShowPrLink,
    ToggleVerticalTabsShowDiffStats,
    ToggleVerticalTabsShowDetailsOnHover,
    /// Closes the focused panel. This happens as an explicit action from the user.
    ClosePanel,
    CopyTextToClipboard(String),
    /// An action only registered in dev and local builds, which writes the user's current access
    /// token to the system clipboard to aid debugging and development.
    CopyAccessTokenToClipboard,
    DismissWorkspaceBanner(WorkspaceBanner),
    /// An action only registered in dev and local builds, which crashes the
    /// app (via a Sentry helper method) immediately when called.
    Crash,
    /// An action only registered in dev and local builds, which triggers a
    /// panic immediately when called.
    Panic,
    /// Writes a heap profile to disk.
    DumpHeapProfile,
    ShowAIAssistantWarmWelcome,
    ClickedAIAssistantWarmWelcome,
    /// An action to open a new window with a view hierarchy debugger.
    OpenViewTreeDebugWindow,
    DismissAIAssistantWarmWelcome,
    /// An action to either upgrade syncing status from none or just in one tab
    /// to syncing all tabs, or downgrade from syncing all tabs to no syncing
    ToggleSyncAllTerminalInputsInAllTabs,
    /// An action to either cancel syncing
    /// or switch from no syncing/syncing all tabs to syncing within one tab
    ToggleSyncTerminalInputsInTab,
    /// An action to force terminal input syncing off
    DisableTerminalInputSync,
    HandleConflictingWorkflow(SyncId),
    HandleConflictingEnvVarCollection(SyncId),
    OpenPromptEditor {
        open_source: PromptEditorOpenSource,
    },
    OpenAgentToolbarEditor,
    OpenCLIAgentToolbarEditor,
    OpenHeaderToolbarEditor,
    ShowHeaderToolbarContextMenu {
        position: Vector2F,
    },
    Reauth,
    SignupAnonymousUser,
    SignInAnonymousWebUser,
    OpenLink(String),
    /// On WASM, opens a given URL in the desktop Warp app (if installed) or redirects to download page.
    #[cfg(target_family = "wasm")]
    OpenLinkOnDesktop(url::Url),
    ReopenClosedSession,
    OpenShareSessionModal(usize),
    StopSharingSessionFromTabMenu {
        terminal_view_id: EntityId,
    },
    StopSharingAllSessionsInTab {
        pane_group: WeakViewHandle<PaneGroup>,
    },
    CopySharedSessionLinkFromTab {
        tab_index: usize,
    },
    OpenSharedSessionQrCode {
        session_id: SessionId,
    },
    AddWindow,
    AddWindowWithShell {
        shell: AvailableShell,
    },
    /// Moves focus to the panel on the left
    FocusLeftPanel,
    /// Moves focus to the panel on the right
    FocusRightPanel,
    /// An action to view a newly created/edited workflow in WD from the toast
    ViewObjectInWarpDrive(WarpDriveItemId),
    /// Open the object's sharing settings in WD.
    OpenObjectSharingSettings {
        object_id: CloudObjectTypeAndId,
        source: SharingDialogSource,
    },
    UndoTrash(CloudObjectTypeAndId),
    /// Open a local path in the file explorer.
    OpenInExplorer {
        path: PathBuf,
    },
    /// Open a local file with the system's default application.
    OpenFilePath {
        path: PathBuf,
    },
    TerminateApp,
    CloseWindow,
    /// Help the user call the Warp executable with the [`crate::args::DEBUG_DUMP_FLAG`].
    DumpDebugInfo,
    /// Log review comment send eligibility for panes in the active tab.
    LogReviewCommentSendStatusForActiveTab,
    ToggleRecordingMode,
    ToggleInBandGenerators,
    ToggleDebugNetworkStatus,
    ToggleShowMemoryStats,
    RunAISuggestedCommand(String),
    RunCommand(String),
    InsertInInput {
        content: String,
        replace_buffer: bool,
        /// Whether to ensure agent mode is enabled when inserting content
        ensure_agent_mode: bool,
    },
    /// Open a new tab with its input in AI mode.
    NewTabInAgentMode {
        /// The entrypoint that triggered this action.
        entrypoint: AgentModeEntrypoint,
        /// The type of zero state prompt suggestion to start with (optional).
        zero_state_prompt_suggestion_type: Option<ZeroStatePromptSuggestionType>,
    },
    /// Open a new pane with its input in AI mode.
    NewPaneInAgentMode {
        /// The entrypoint that triggered this action.
        entrypoint: AgentModeEntrypoint,
        /// The type of zero state prompt suggestion to start with (optional).
        zero_state_prompt_suggestion_type: Option<ZeroStatePromptSuggestionType>,
    },
    OpenCloudAgentSetupGuide,
    AttemptLoginGatedAIUpgrade,
    /// Open the modal explaining Prompt Suggestions aren't available on the Free plan.
    OpenPromptSuggestionsUnavailableModal,
    /// Dismisses the Wayland crash recovery banner and opens a link to our docs page with more
    /// information.
    #[cfg(target_os = "linux")]
    DismissWaylandCrashRecoveryBannerAndOpenLink,
    /// Open a new pane with its input in AI mode
    /// with query "Fix this" with error name and details from AI summary.
    FixInAgentMode {
        query: String,
    },
    OpenAIFactCollection,
    OpenMCPServerCollection,
    /// Open the Environment Management pane in Create mode.
    OpenEnvironmentManagementPane,
    ToggleAIDocumentPane {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
    },
    /// Closes all visible AI document panes in the active pane group.
    HideAIDocumentPanes,
    /// Closes any other ai document panes in the active pane group, and opens the specified document_id.
    OpenAIDocumentPane {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
    },
    FocusTerminalViewInWorkspace {
        terminal_view_id: EntityId,
    },
    /// Focus a specific pane by its locator (pane_group_id and pane_id).
    FocusPane(PaneViewLocator),
    /// Start a new AI conversation in a terminal view. This sets the pending query state
    /// to default and focuses the terminal view.
    StartNewConversation {
        terminal_view_id: EntityId,
    },
    /// Jump to the terminal pane of the most recent agent toast
    JumpToLatestToast,
    /// Open a file in a new tab with a code pane
    OpenFileInNewTab {
        full_path: PathBuf,
        line_and_column: Option<LineAndColumnArg>,
    },
    OpenNotebook {
        id: SyncId,
    },
    RunWorkflow {
        workflow: Arc<WorkflowType>,
        workflow_source: WorkflowSource,
        workflow_selection_source: WorkflowSelectionSource,
        argument_override: Option<HashMap<String, String>>,
    },
    ScrollToSettingsWidget {
        page: SettingsSection,
        widget_id: &'static str,
    },
    /// Navigate to an existing AI conversation, focusing on its terminal view.
    ///
    /// If the conversation is not in an open pane, restore it based on the layout setting or override.
    RestoreOrNavigateToConversation {
        pane_view_locator: Option<PaneViewLocator>,
        window_id: Option<WindowId>,
        conversation_id: AIConversationId,
        terminal_view_id: Option<EntityId>,
        /// If provided, use this layout to restore the conversation.
        /// Otherwise, fall back to the user's setting.
        restore_layout: Option<RestoreConversationLayout>,
    },
    /// Fork an existing AI conversation.
    /// Optionally summarizes the conversation after forking and/or sends an initial prompt.
    ForkAIConversation {
        conversation_id: AIConversationId,
        /// When Some, fork from the given response (or exchange if `fork_from_exact_exchange`
        /// is true). When None, fork from the last exchange.
        fork_from_exchange: Option<ForkFromExchange>,
        /// Whether to summarize the conversation after forking.
        summarize_after_fork: bool,
        /// Prompt to use for summarization when `summarize_after_fork` is true.
        summarization_prompt: Option<String>,
        /// Initial prompt to send in the forked conversation (sent after summarization if enabled).
        initial_prompt: Option<String>,
        /// Attachments (images/files) to send along with the initial prompt in the forked pane.
        initial_attachments: Vec<PendingAttachment>,
        /// Where to open the forked conversation.
        destination: ForkedConversationDestination,
    },
    /// Fork an existing AI conversation into a new pane and prefill the input with a local
    /// continuation command (selecting all text).
    #[cfg(not(target_family = "wasm"))]
    ContinueConversationLocally {
        conversation_id: AIConversationId,
    },
    /// Continue a completed third-party cloud harness run in a local split pane.
    #[cfg(not(target_family = "wasm"))]
    ContinueThirdPartyConversationLocally {
        task_id: AmbientAgentTaskId,
        harness: AIAgentHarness,
    },
    /// Insert the /fork slash command into the active terminal's input.
    InsertForkSlashCommand,
    /// Open a local-to-cloud handoff pane next to the active conversation
    /// (REMOTE-1486). Triggered by the `/move-to-cloud` slash command
    /// and the footer chip of the same name. The dispatch site reads the
    /// active conversation's `server_conversation_token` and gates on
    /// `FeatureFlag::OzHandoff && FeatureFlag::HandoffLocalCloud`.
    /// Falls through to splitting a fresh cloud-mode pane when the active
    /// conversation isn't handoff-able (no synced server token, empty, or no
    /// active conversation at all).
    OpenLocalToCloudHandoffPane {
        #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
        launch: Option<crate::ai::blocklist::handoff::PendingCloudLaunch>,
        #[cfg(not(all(feature = "local_fs", not(target_family = "wasm"))))]
        launch: Option<()>,
        environment_id: Option<crate::server::ids::SyncId>,
        entry_point: crate::ai::ambient_agents::telemetry::HandoffEntryPoint,
    },
    /// Automatically hand off the active running local agent conversation in the
    /// given terminal view to Cloud Mode.
    AutoHandoffActiveAgentToCloud {
        terminal_view_id: EntityId,
        conversation_id: AIConversationId,
        trigger: AutoCloudHandoffTrigger,
    },
    /// Show the environment creation modal during `&` handoff compose when no
    /// environments exist.
    ShowHandoffEnvironmentCreationModal,
    ShowCloudModeV2EnvironmentCreationModal,
    /// Open the workspace modal for creating a new managed auth secret.
    /// Dispatched by orchestration card pickers' "New API key…" item.
    OpenCreateAuthSecretModal {
        harness: warp_cli::agent::Harness,
    },
    /// Summarize the active AI conversation in the focused pane.
    SummarizeAIConversation {
        prompt: Option<String>,
        /// Optional prompt to send after summarization completes successfully.
        initial_prompt: Option<String>,
    },
    /// Install the Oz CLI command to /usr/local/bin
    #[cfg(target_os = "macos")]
    InstallOz,
    /// Uninstall the Oz CLI command from /usr/local/bin
    #[cfg(target_os = "macos")]
    UninstallOz,
    /// Install the Warp Control CLI command to /usr/local/bin
    #[cfg(target_os = "macos")]
    InstallWarpctrl,
    /// Uninstall the Warp Control CLI command from /usr/local/bin
    #[cfg(target_os = "macos")]
    UninstallWarpctrl,
    UndoRevertInCodeReviewPane {
        window_id: WindowId,
        view_id: EntityId,
    },
    /// Handle a file being renamed in the file tree
    #[cfg(feature = "local_fs")]
    FileRenamed {
        old_path: PathBuf,
        new_path: PathBuf,
    },
    /// Handle a file being deleted in the file tree
    #[cfg(feature = "local_fs")]
    FileDeleted {
        path: PathBuf,
    },
    /// Open a repository directory via file picker. The `path` is an `Option` because some
    /// dispatchers don't know the path to open yet (so the Workspace must open the file picker)
    /// and some do, e.g. the GetStartedView. The GetStartedView needs to handle the file picker
    /// because it needs to determine whether or not to close itself based on whether the user
    /// actually selects a file in the file picker or cancels it.
    OpenRepository {
        path: Option<String>,
    },
    /// Open the native folder picker for a repo param in the tab-config modal after the
    /// current interaction cycle finishes.
    OpenTabConfigRepoPicker {
        param_index: usize,
    },
    /// Open a new blank code file in the current tab
    NewCodeFile,
    NavigatePrevPaneOrPanel,
    NavigateNextPaneOrPanel,
    ToggleProjectExplorer,
    OpenProjectExplorer,
    ToggleGlobalSearch,
    ToggleHiddenFiles,
    OpenGlobalSearch,
    ToggleConversationListView,
    OpenConversationListView,
    OpenAgentManagementView,
    /// Open the Build Plan Migration Modal (for debugging)
    #[cfg(debug_assertions)]
    OpenBuildPlanMigrationModal,
    /// Reset the build plan migration modal dismissed state (for debugging)
    #[cfg(debug_assertions)]
    ResetBuildPlanMigrationModalState,
    /// Reset the AWS Bedrock login banner dismissed state (for debugging).
    #[cfg(debug_assertions)]
    DebugResetAwsBedrockLoginBannerDismissed,
    /// Open the Oz Launch Modal (for debugging)
    #[cfg(debug_assertions)]
    OpenOzLaunchModal,
    /// Reset the Oz launch modal dismissed state (for debugging)
    #[cfg(debug_assertions)]
    ResetOzLaunchModalState,
    /// Open the OpenWarp Launch Modal (for debugging)
    #[cfg(debug_assertions)]
    OpenOpenWarpLaunchModal,
    /// Reset the OpenWarp launch modal dismissed state (for debugging)
    #[cfg(debug_assertions)]
    ResetOpenWarpLaunchModalState,
    /// Open the Orchestration Launch Modal (for debugging)
    #[cfg(debug_assertions)]
    OpenOrchestrationLaunchModal,
    /// Reset the orchestration launch modal dismissed state (for debugging)
    #[cfg(debug_assertions)]
    ResetOrchestrationLaunchModalState,
    /// Open the auto-handoff sleep modal (for debugging)
    #[cfg(debug_assertions)]
    OpenAutoHandoffSleepModal,
    /// Reset the auto-handoff sleep modal shown state (for debugging)
    #[cfg(debug_assertions)]
    ResetAutoHandoffSleepModalState,
    /// Trigger the auto-handoff-to-cloud flow in-process, as if the machine
    /// were about to sleep (for debugging)
    #[cfg(debug_assertions)]
    TriggerAutoHandoffToCloud,
    /// Open the Free AI Removal Modal (for debugging)
    #[cfg(debug_assertions)]
    OpenFreeAiRemovalModal,
    /// Reset the free AI removal modal seen state (for debugging)
    #[cfg(debug_assertions)]
    ResetFreeAiRemovalModalState,
    /// Install the opencode-warp plugin from GitHub into the global opencode config.
    #[cfg(debug_assertions)]
    InstallOpenCodeWarpPlugin,
    /// Use a local checkout of the opencode-warp plugin (for testing/development).
    #[cfg(debug_assertions)]
    UseLocalOpenCodeWarpPlugin,
    /// Take a process sample of the app (equivalent to Activity Monitor > Sample Process).
    #[cfg(target_os = "macos")]
    SampleProcess,
    ToggleNotificationMailbox {
        select_first: bool,
    },
    ToggleAgentManagementView,
    ViewAgentRunsForEnvironment {
        environment_id: String,
    },
    /// Show the rewind confirmation dialog before rewinding an AI conversation
    ShowRewindConfirmationDialog {
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
    },
    /// Execute the actual rewind after confirmation
    ExecuteRewindAIConversation {
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
    },
    /// Execute the actual deletion of a conversation after confirmation
    ExecuteDeleteConversation {
        conversation_id: AIConversationId,
        terminal_view_id: Option<EntityId>,
    },
    /// Open the canonical ambient agent conversation pane and attach it to a live session.
    OpenOrAttachAmbientAgentConversation {
        session_id: SessionId,
        task_id: AmbientAgentTaskId,
    },
    /// Load cloud conversation data into a transcript viewer.
    /// Used when CloudConversations is enabled and the sandbox is not running.
    OpenConversationTranscriptViewer {
        conversation_id: ServerConversationToken,
        ambient_agent_task_id: Option<AmbientAgentTaskId>,
    },
    /// Toggle the conversation transcript details panel (WASM-only).
    #[cfg(target_family = "wasm")]
    ToggleConversationTranscriptDetailsPanel,
    /// Open a full-window lightbox displaying the given images.
    OpenLightbox {
        images: Vec<lightbox::LightboxImage>,
        /// The index of the image to display initially.
        initial_index: usize,
    },
    /// Update a single image in the currently open lightbox.
    UpdateLightboxImage {
        index: usize,
        image: lightbox::LightboxImage,
    },
    StartAgentOnboardingTutorial(OnboardingTutorial),
    ShowSessionConfigModal,
    DismissSessionConfigTabConfigChip,
    /// Start the HOA onboarding flow (for debugging)
    #[cfg(debug_assertions)]
    ShowHoaOnboardingFlow,
    /// Open the "New worktree" modal for creating a reusable worktree tab config.
    OpenNewWorktreeModal,
    /// Open the native folder picker for the repo field in the new-worktree modal.
    OpenNewWorktreeRepoPicker,
    /// Create a new worktree in the given repo using the default worktree tab config.
    /// The branch name is auto-generated.
    OpenWorktreeInRepo {
        repo_path: String,
    },
    /// Open a folder picker to add a new repo to PersistedWorkspace (from the
    /// "New worktree config" submenu's "+ Add new repo..." item).
    OpenWorktreeAddRepoPicker,
    SaveCurrentTabAsNewConfig(usize),
    SyncTrafficLights,
    /// Opens a tab config file in the editor and dismisses the associated error toast.
    OpenTabConfigErrorFile {
        path: PathBuf,
        toast_object_id: String,
    },
    /// Sidecar action: set the hovered item as the Cmd+T default.
    TabConfigSidecarMakeDefault {
        mode: crate::settings::ai::DefaultSessionMode,
        tab_config_path: Option<PathBuf>,
        shell: Option<AvailableShell>,
    },
    /// Sidecar action: open the tab config TOML in the user's editor.
    TabConfigSidecarEditConfig {
        path: PathBuf,
    },
    /// Sidecar action: show the remove confirmation dialog for a tab config.
    TabConfigSidecarRemoveConfig {
        name: String,
        path: PathBuf,
    },
    /// Opens the settings.toml file in a code editor pane.
    OpenSettingsFile,
    /// Opens a new agent session to fix settings.toml errors using the modify-settings skill.
    FixSettingsWithOz {
        error_description: String,
    },
    /// Opens (or focuses) the in-app network log pane as a right-split of the
    /// active pane group. Gated on `ContextFlag::NetworkLogConsole`.
    OpenNetworkLogPane,
}

impl From<&WorkspaceAction> for LoginGatedFeature {
    fn from(val: &WorkspaceAction) -> LoginGatedFeature {
        use WorkspaceAction::*;
        match val {
            ImportToTeamDrive => "Importing to a team drive",
            CreateTeamNotebook => "Creating a team notebook",
            CreateTeamWorkflow => "Creating a team workflow",
            CreateTeamFolder => "Creating a team folder",
            CreateTeamEnvVarCollection => "Creating a team environment variable collection",
            CreateTeamAIPrompt => "Creating a team prompt",
            OpenShareSessionModal(_) => "Sharing a session",
            _ => "Unknown reason",
        }
    }
}

impl WorkspaceAction {
    pub fn blocked_for_anonymous_user(&self) -> bool {
        use WorkspaceAction::*;
        matches!(
            self,
            ImportToTeamDrive
                | CreateTeamNotebook
                | CreateTeamWorkflow
                | CreateTeamFolder
                | CreateTeamEnvVarCollection
                | CreateTeamAIPrompt
                | OpenShareSessionModal(_)
        )
    }

    /// Matches what actions require the app state to be saved, and which don't. We match all
    /// actions directly, rather than using _, so we're forced to make a conscious decision for each
    /// of them, rather than following some default.
    pub fn should_save_app_state_on_action(&self) -> bool {
        use WorkspaceAction::*;
        match self {
            #[cfg(not(target_family = "wasm"))]
            ContinueConversationLocally { .. } => true,
            #[cfg(not(target_family = "wasm"))]
            ContinueThirdPartyConversationLocally { .. } => true,
            ActivateTab(_)
            | ActivateTabByNumber(_)
            | ActivatePrevTab
            | ActivateNextTab
            | ActivateLastTab
            | CyclePrevSession
            | CycleNextSession
            | MoveActiveTabLeft
            | MoveActiveTabRight
            | MoveTabLeft(_)
            | MoveTabRight(_)
            | DropTab
            | DropGroup
            | RenameTab(_)
            | ResetTabName(_)
            | RenamePane(_)
            | ResetPaneName(_)
            | RenameActiveTab
            | RenameActivePane
            | SetActiveTabName(_)
            | SetActiveTabColor(_)
            | CloseTab(_)
            | CloseActiveTab
            | CloseOtherTabs(_)
            | CloseNonActiveTabs
            | CloseTabsRight(_)
            | CloseTabsRightActiveTab
            | CloseTabGroup(_)
            | ToggleTabGroupCollapsed(_)
            | RenameTabGroup(_)
            | NewTabGroupFromTab(_)
            | MoveTabToGroup { .. }
            | RemoveTabFromGroup(_)
            | NewTabGroupFromSelectedTabs
            | NewTabGroupFromActiveOrSelectedTabs
            | MoveSelectedTabsToGroup { .. }
            | RemoveSelectedTabsFromGroup
            | RemoveActiveOrSelectedTabsFromGroup
            | UngroupTabs(_)
            | NewTabInGroup(_)
            | MoveTabGroupUp(_)
            | MoveTabGroupDown(_)
            | CloseTabsOutsideGroup(_)
            | CloseTabsAboveGroup(_)
            | CloseTabsBelowGroup(_)
            | PinTab(_)
            | UnpinTab(_)
            | PinActiveTab
            | UnpinActiveTab
            | PinTabGroup(_)
            | UnpinTabGroup(_)
            | PinActiveTabGroup
            | UnpinActiveTabGroup
            | ToggleTabColor { .. }
            | ToggleTabGroupColor { .. }
            | AddDefaultTab
            | AddTerminalTab { .. }
            | AddTabWithShell { .. }
            | AddGetStartedTab
            | AddAgentTab
            | AddAmbientAgentTab
            | AddDockerSandboxTab
            | AddWindow
            | AddWindowWithShell { .. }
            | CloseWindow
            | ScrollToSettingsWidget { .. }
            | NewTabInAgentMode { .. }
            | NewPaneInAgentMode { .. }
            | FixInAgentMode { .. }
            | OpenNotebook { .. }
            | RunWorkflow { .. }
            | OpenFileInNewTab { .. }
            | RestoreOrNavigateToConversation { .. }
            | NewCodeFile
            | ForkAIConversation { .. }
            | SummarizeAIConversation { .. }
            | OpenRepository { .. }
            | SelectTabConfig(_)
            | ToggleVerticalTabsPanel
            | OpenVerticalTabsPanel => true, // actions that actually change a state of the state of user's
            // workspace would most likely require a save, so that if the app gets
            // restarted, the user can continue working
            AutoupdateFailureLink
            | ApplyUpdate
            | CopyVersion(_)
            | DownloadNewVersion
            | ConfigureKeybindingSettings { .. }
            | ExportAllWarpDriveObjects
            | ShowSettings
            | ShowSettingsPage(_)
            | ShowSettingsPageWithSearch { .. }
            | ShowThemeChooser(_)
            | ShowThemeChooserForActiveTheme
            | IncreaseFontSize
            | DecreaseFontSize
            | ResetFontSize
            | IncreaseZoom
            | DecreaseZoom
            | ResetZoom
            | OpenPalette { .. }
            | TogglePalette { mode: _, source: _ }
            | ShowUpgrade
            | ShowReferralSettingsPage
            | JoinSlack
            | ViewUserDocs
            | ViewLatestChangelog
            | ViewPrivacyPolicy
            | SendFeedback
            | ChangeCursor(_)
            | ToggleBlockSnackbar
            | ToggleErrorUnderlining
            | ToggleSyntaxHighlighting
            | OpenLaunchConfigSaveModal
            | ToggleTabRightClickMenu { .. }
            | ToggleTabSelectionRightClickMenu { .. }
            | ToggleTabGroupRightClickMenu { .. }
            | ToggleVerticalTabsPaneContextMenu { .. }
            | OpenNewSessionMenu { .. }
            | ToggleTabConfigsMenu
            | ToggleNewSessionMenu { .. }
            | SelectNewSessionMenuItem(_)
            | ToggleTabBarOverflowMenu
            | CheckForUpdate
            | SetA11yVerbosityLevel(_)
            | ToggleNotifications
            | DispatchToSettingsTab { .. }
            | ToggleResourceCenter
            | ToggleUserMenu
            | ClickedAIAssistantIcon
            | ToggleAIAssistant
            | OpenCloudAgentSetupGuide
            | OpenPromptSuggestionsUnavailableModal
            | ToggleKeybindingsPage
            | ShowCommandSearch(_)
            | ToggleMouseReporting
            | ToggleScrollReporting
            | ToggleFocusReporting
            | ImportToPersonalDrive
            | ImportToTeamDrive
            | CreatePersonalNotebook
            | CreateTeamNotebook
            | CreatePersonalWorkflow
            | CreateTeamWorkflow
            | CreatePersonalFolder
            | CreateTeamFolder
            | CreateTeamEnvVarCollection
            | CreatePersonalEnvVarCollection
            | CreatePersonalAIPrompt
            | CreateTeamAIPrompt
            | OpenInExplorer { .. }
            | DragTab { .. }
            | StartTabDrag
            | DragGroup { .. }
            | StartGroupDrag(_)
            | ToggleLeftPanel
            | ToggleWarpDrive
            | OpenWarpDrive
            | ClosePanel
            | ToggleRightPanel
            | OpenCodeReviewPanel(..)
            | ToggleVerticalTabsSettingsPopup
            | SetVerticalTabsDisplayGranularity(_)
            | SetVerticalTabsTabItemMode(_)
            | SetVerticalTabsViewMode(_)
            | SetVerticalTabsPrimaryInfo(_)
            | SetVerticalTabsCompactSubtitle(_)
            | ToggleVerticalTabsShowPrLink
            | ToggleVerticalTabsShowDiffStats
            | ToggleVerticalTabsShowDetailsOnHover
            | ToggleWelcomeTips
            | CopyTextToClipboard(_)
            | CopyAccessTokenToClipboard
            | OpenTabConfigRepoPicker { .. }
            | OpenNewWorktreeModal
            | OpenNewWorktreeRepoPicker
            | OpenWorktreeInRepo { .. }
            | OpenWorktreeAddRepoPicker
            | Crash
            | Panic
            | DumpHeapProfile
            | OpenViewTreeDebugWindow
            | ShowAIAssistantWarmWelcome
            | ClickedAIAssistantWarmWelcome
            | DismissAIAssistantWarmWelcome
            | DismissWorkspaceBanner(..)
            | ToggleSyncAllTerminalInputsInAllTabs
            | ToggleSyncTerminalInputsInTab
            | DisableTerminalInputSync
            | HandleConflictingWorkflow(_)
            | HandleConflictingEnvVarCollection(_)
            | OpenPromptEditor { .. }
            | OpenAgentToolbarEditor
            | OpenCLIAgentToolbarEditor
            | OpenHeaderToolbarEditor
            | ShowHeaderToolbarContextMenu { .. }
            | Reauth
            | SignupAnonymousUser
            | LogOut
            | OpenLink(_)
            | OpenShareSessionModal(_)
            | StopSharingSessionFromTabMenu { .. }
            | StopSharingAllSessionsInTab { .. }
            | CopySharedSessionLinkFromTab { .. }
            | OpenSharedSessionQrCode { .. }
            | ReopenClosedSession
            | FocusLeftPanel
            | FocusRightPanel
            | DumpDebugInfo
            | LogReviewCommentSendStatusForActiveTab
            | ToggleRecordingMode
            | ToggleInBandGenerators
            | ToggleDebugNetworkStatus
            | ToggleShowMemoryStats
            | RunAISuggestedCommand { .. }
            | RunCommand { .. }
            | InsertInInput { .. }
            | InsertForkSlashCommand
            | AttemptLoginGatedAIUpgrade
            | UndoTrash(_)
            | OpenFilePath { .. }
            | ViewObjectInWarpDrive(_)
            | OpenObjectSharingSettings { .. }
            | TerminateApp
            | SignInAnonymousWebUser
            | TabHoverWidthStart { .. }
            | TabHoverWidthEnd
            | OpenAIFactCollection
            | OpenMCPServerCollection
            | FocusTerminalViewInWorkspace { .. }
            | FocusPane(..)
            | ShiftSelectTabRange { .. }
            | ToggleTabMultiSelection { .. }
            | ClearTabMultiSelection
            | CancelActiveRename
            | StartNewConversation { .. }
            | UndoRevertInCodeReviewPane { .. }
            | JumpToLatestToast
            | NavigatePrevPaneOrPanel
            | NavigateNextPaneOrPanel
            | ToggleProjectExplorer
            | OpenProjectExplorer
            | ToggleGlobalSearch
            | ToggleHiddenFiles
            | OpenGlobalSearch
            | ToggleConversationListView
            | OpenConversationListView
            | ToggleNotificationMailbox { .. }
            | ToggleAgentManagementView
            | OpenAgentManagementView
            | ViewAgentRunsForEnvironment { .. }
            | ToggleAIDocumentPane { .. }
            | HideAIDocumentPanes
            | OpenAIDocumentPane { .. }
            | ShowRewindConfirmationDialog { .. }
            | ExecuteRewindAIConversation { .. }
            | ExecuteDeleteConversation { .. }
            | OpenOrAttachAmbientAgentConversation { .. }
            | OpenConversationTranscriptViewer { .. }
            | OpenLightbox { .. }
            | UpdateLightboxImage { .. }
            | StartAgentOnboardingTutorial(_)
            | ShowSessionConfigModal
            | DismissSessionConfigTabConfigChip
            | SaveCurrentTabAsNewConfig(_)
            | SyncTrafficLights
            | OpenTabConfigErrorFile { .. }
            | TabConfigSidecarMakeDefault { .. }
            | TabConfigSidecarEditConfig { .. }
            | TabConfigSidecarRemoveConfig { .. }
            | OpenSettingsFile
            | FixSettingsWithOz { .. }
            | OpenLocalToCloudHandoffPane { .. }
            | AutoHandoffActiveAgentToCloud { .. }
            | ShowHandoffEnvironmentCreationModal
            | ShowCloudModeV2EnvironmentCreationModal
            | OpenCreateAuthSecretModal { .. }
            | OpenNetworkLogPane => false,
            #[cfg(debug_assertions)]
            ShowHoaOnboardingFlow => false,
            #[cfg(target_family = "wasm")]
            ToggleConversationTranscriptDetailsPanel => false,
            #[cfg(debug_assertions)]
            OpenBuildPlanMigrationModal
            | ResetBuildPlanMigrationModalState
            | DebugResetAwsBedrockLoginBannerDismissed
            | OpenOzLaunchModal
            | ResetOzLaunchModalState
            | OpenOpenWarpLaunchModal
            | ResetOpenWarpLaunchModalState
            | OpenOrchestrationLaunchModal
            | ResetOrchestrationLaunchModalState
            | OpenAutoHandoffSleepModal
            | ResetAutoHandoffSleepModalState
            | TriggerAutoHandoffToCloud
            | OpenFreeAiRemovalModal
            | ResetFreeAiRemovalModalState
            | InstallOpenCodeWarpPlugin
            | UseLocalOpenCodeWarpPlugin => false,
            #[cfg(not(target_family = "wasm"))]
            ViewLogs => false,
            #[cfg(target_os = "macos")]
            SampleProcess => false,
            #[cfg(target_os = "macos")]
            InstallOz | UninstallOz => false,
            #[cfg(target_os = "macos")]
            InstallWarpctrl | UninstallWarpctrl => false,
            #[cfg(feature = "local_fs")]
            FileRenamed { .. } => false, // File rename doesn't change workspace state
            #[cfg(feature = "local_fs")]
            FileDeleted { .. } => false, // File deletion doesn't change workspace state
            OpenEnvironmentManagementPane => false,
            #[cfg(target_os = "linux")]
            DismissWaylandCrashRecoveryBannerAndOpenLink => false,
            #[cfg(target_family = "wasm")]
            OpenLinkOnDesktop(_) => false,
            // actions that are related to updating user settings or
            // managing some ui elements (like closing/opening modals)
            // that don't reflect on actual workspace and don't need to
            // be preserved between restarts.
        }
    }
}

#[cfg(test)]
#[path = "action_tests.rs"]
mod tests;
