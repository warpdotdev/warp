use std::fmt;

use std::ops::Range;
use std::path::PathBuf;

use ai::skills::SkillReference;
use command_corrections::Correction;
use pathfinder_geometry::vector::Vector2F;
use session_sharing_protocol::common::Role;
use session_sharing_protocol::sharer::RoleUpdateReason;
use warp_util::user_input::UserInput;
use warpui::elements::HyperlinkUrl;
use warpui::event::ModifiersState;
use warpui::units::Lines;
use warpui::EntityId;

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::blocklist::codebase_index_speedbump_banner::CodebaseIndexSpeedbumpBannerAction;
use crate::code_review::telemetry_event::CodeReviewPaneEntrypoint;
use crate::server::telemetry::{AgentModeRewindEntrypoint, PaletteSource, ToggleBlockFilterSource};
use crate::terminal::available_shells::AvailableShell;
use crate::terminal::model::completions::ShellCompletion;
use crate::terminal::shared_session::SharedSessionActionSource;
use crate::terminal::ssh::error::SshErrorBlockAction;
use crate::terminal::view::inline_banner::AgentModeSetupSpeedbumpBannerAction;
use crate::terminal::view::passive_suggestions::PromptSuggestionResolution;
use crate::terminal::view::RichContentSecretTooltipInfo;
use crate::workflows::workflow::Workflow;
use crate::{
    server::ids::SyncId,
    terminal::{
        block_list_element::{
            BlockHoverAction, BlockListMenuSource, BlockSelectAction, BlockTextSelectAction,
        },
        block_list_viewport::OverhangingBlock,
        model::{
            index::Point,
            mouse::MouseState,
            selection::{SelectAction, SelectionDirection},
            terminal_model::{BlockIndex, WithinModel},
            SecretHandle,
        },
    },
};

use super::inline_banner::{
    AnonymousUserLoginBannerAction, AwsBedrockLoginBannerAction, AwsCliNotInstalledBannerAction,
    OpenInWarpBannerAction, VimModeBannerAction,
};
use super::{
    AliasExpansionBannerAction, ContextMenuAction, GridHighlightedLink, InputContextMenuAction,
    NotificationsDiscoveryBannerAction, NotificationsErrorBannerAction, RichContentLink,
    SSHBannerAction, TerminalEditor,
};

pub use onboarding::OnboardingIntention;

/// Version of the agent onboarding flow (non-legacy).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentOnboardingVersion {
    UniversalInput {
        has_project: bool,
    },
    AgentModality {
        has_project: bool,
        intention: OnboardingIntention,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnboardingVersion {
    Legacy,
    Agent(AgentOnboardingVersion),
}

/// This represents whether entering a subshell for a particular command should become automatic in
/// the future, or to ask again.
#[derive(Clone, Debug)]
pub enum RememberForWarpification {
    /// If yes, need to transmit the command itself so it can be persisted to user-defaults
    RememberSubshellCommand(String),
    RememberSSHHost(String),
    DoNotRememberSubshellCommand,
    DoNotRememberSSHHost,
}

impl RememberForWarpification {
    pub fn as_bool(&self) -> bool {
        match self {
            RememberForWarpification::RememberSubshellCommand(_) => true,
            RememberForWarpification::RememberSSHHost(_) => true,
            RememberForWarpification::DoNotRememberSubshellCommand => false,
            RememberForWarpification::DoNotRememberSSHHost => false,
        }
    }

    pub fn is_ssh(&self) -> bool {
        match self {
            RememberForWarpification::RememberSSHHost(_) => true,
            RememberForWarpification::DoNotRememberSSHHost => true,
            RememberForWarpification::RememberSubshellCommand(_) => false,
            RememberForWarpification::DoNotRememberSubshellCommand => false,
        }
    }
}

#[derive(Clone)]
pub enum TerminalAction {
    Scroll {
        delta: Lines,
    },
    AltScroll {
        delta: i32,
    },
    SharedSessionViewerAltScroll {
        new_scroll_top: Lines,
    },
    ScrollToTopOfBlock {
        topmost_block: BlockIndex,
    },
    BlockTextSelect(BlockTextSelectAction),
    BlockSelect {
        action: BlockSelectAction,
        should_redetermine_focus: bool,
    },
    BlockHover(BlockHoverAction),
    BlockSnackbarHover {
        is_hovered: bool,
    },
    BlockNearSnackbarHover {
        is_hovered: bool,
    },

    // TODO: we should eventually use a Modifiers struct here instead of using
    // an aggregated is_selecting_blocks when we need better granularity.
    // This refactor will need to start from the Events themselves.
    ClickOnGrid {
        position: WithinModel<Point>,
        modifiers: ModifiersState,
    },
    MiddleClickOnGrid {
        /// `None` here means that the click was on the Block List but not on a particular blockgrid.
        position: Option<WithinModel<Point>>,
    },
    MiddleClickOnInput,
    MaybeLinkHover {
        position: Option<WithinModel<Point>>,
        from_editor: TerminalEditor,
    },
    MaybeHoverSecret {
        secret_handle: Option<SecretHandle>,
    },
    MaybeDismissToolTip {
        from_keybinding: bool,
    },
    AltScreenContextMenu {
        position: Vector2F,
    },
    AltSelect(SelectAction<Point>),
    MaybeClearAltSelect,
    AltMouseAction(MouseState),
    InsertCommandCorrection {
        correction: Correction,
    },
    BlockListContextMenu(BlockListMenuSource),
    CloseContextMenu,
    Paste,
    Copy,
    CopyOutputs,
    CopyCommands,
    CopyGitBranch,
    OpenShareModal,
    ReinputCommands,
    ReinputCommandsWithSudo,
    ClearBuffer,
    Focus,
    FocusInputAndClearSelection,
    ShowFindBar,
    SelectPriorBlock,
    SelectBookmarkDown,
    SelectBookmarkUp,
    BookmarkSelectedBlock,
    ScrollToBottomOfSelectedBlocks,
    ScrollToTopOfSelectedBlocks,
    ScrollToBottomOfOverhangingBlock(OverhangingBlock),
    SelectNextBlock,
    Up,
    OpenBlockListContextMenu,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    KeyboardSelectText(SelectionDirection),
    UserInputSequence(Vec<u8>),
    ControlSequence(Vec<u8>),
    RunNativeShellCompletions {
        buffer_text: String,
        results_tx: async_channel::Sender<Vec<ShellCompletion>>,
    },
    KeyDown(String),
    TypedCharacters(String),
    ContextMenu(ContextMenuAction),
    // IMPORTANT: Do not add a binding for ctrl_d, as we don't want this behavior to leak out to
    // parts of the terminal unrelated to the block list
    CtrlD,
    CtrlC,
    ClearSelectionsWhenShellMode,
    Close,
    ToggleMaximizePane,
    SplitRight(Option<AvailableShell>),
    SplitLeft(Option<AvailableShell>),
    SplitDown(Option<AvailableShell>),
    SplitUp(Option<AvailableShell>),
    /// The context menu that's used for the prompt directly above input editor
    PromptContextMenu {
        position_offset_from_prompt: Vector2F,
    },
    OpenInputContextMenu {
        position: Vector2F,
    },
    InputContextMenuItem(InputContextMenuAction),
    /// Open the menu on the specified [`crate::ai::blocklist::AIBlock`] that lists the blocks that
    /// were attached to the query in the specified [`crate::ai::blocklist::AIAgentExchange`] which
    /// is part of the specified [`crate::ai::blocklist::AIConversation`].
    OpenAIBlockAttachedBlocksMenu {
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
    },
    /// Open the overflow context menu for an AI block with copy options
    OpenAIBlockOverflowMenu {
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
        is_restored: bool,
    },
    /// Show the confirmation dialog before rewinding an AI conversation
    RewindAIConversation {
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
        /// The entrypoint from which this action was triggered (for telemetry).
        entrypoint: AgentModeRewindEntrypoint,
    },
    /// Actually execute the rewind (called after user confirms in the dialog)
    ExecuteRewindAIConversation {
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
    },
    /// Execute rewind from the inline menu (looks up ai_block_view_id from exchange_id)
    ExecuteRewindFromInlineMenu {
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
    },
    SelectAllBlocks,
    ExpandBlockSelectionAbove,
    ExpandBlockSelectionBelow,
    NotificationsDiscoveryBanner(NotificationsDiscoveryBannerAction),
    BookmarkBlock(BlockIndex),
    NotificationsErrorBanner(NotificationsErrorBannerAction),
    LegacySSHBanner(SSHBannerAction),
    JumpToBookmark(BlockIndex),
    OpenGridLink(GridHighlightedLink),
    OpenRichContentLink(RichContentLink),
    ToggleGridSecret {
        handle: WithinModel<SecretHandle>,
        show_secret: bool,
    },
    CopyGridSecret(WithinModel<SecretHandle>),
    ToggleRichContentSecret {
        rich_content_tooltip_info: RichContentSecretTooltipInfo,
        show_secret: bool,
    },
    CopyRichContentSecret(RichContentSecretTooltipInfo),
    ShowInFileExplorer(PathBuf),
    OpenFileInWarp(PathBuf),
    #[cfg(feature = "local_fs")]
    OpenCodeInWarp {
        path: PathBuf,
        layout: crate::util::file::external_editor::settings::EditorLayout,
        line_col: Option<warp_util::path::LineAndColumnArg>,
    },
    OpenWorkflowModal,
    OpenWorkflowModalForAIWorkflow(Workflow),
    OpenWorkflowModalForBlock(BlockIndex),
    OpenWorkflowModalWithCloudWorkflow(SyncId),
    AskAIAssistant {
        block_index: BlockIndex,
    },
    /// Starts a subshell in the active session.
    TriggerSubshellBootstrap,
    /// If the user says "no" to Warpification, possibly requesting not to be asked again
    DismissWarpifyBanner(RememberForWarpification),
    /// Triggers the banner asking to turn the running block into a subshell. The String is the
    /// command that the user entered.
    ShowSubshellBanner(String),
    /// Triggers the banner asking to Warpify the active ssh session. The String is the
    /// command that the user entered.
    ShowWarpifySshBanner(String, Option<String>),
    InsertMostRecentCommandCorrection,
    AliasExpansionBanner(AliasExpansionBannerAction),
    OpenInWarpBanner(OpenInWarpBannerAction),
    OpenBlockFilterEditor(BlockIndex),
    OnboardingFlow(OnboardingVersion),
    ImportSettings,
    StopSharingCurrentSession {
        source: SharedSessionActionSource,
    },
    OpenSharedSessionOnDesktop {
        source: SharedSessionActionSource,
    },
    ToggleBlockFilterOnSelectedOrLastBlock(ToggleBlockFilterSource),
    OpenShareSessionModal {
        source: SharedSessionActionSource,
    },
    CopySharedSessionLink {
        source: SharedSessionActionSource,
    },
    VimModeBanner(VimModeBannerAction),
    ToggleSnackbarInActivePane,
    MakeAllParticipantsReaders {
        reason: RoleUpdateReason,
    },
    OpenSharedSessionViewerRoleMenu,
    RequestSharedSessionRole(Role),
    /// User selected a block inside an AI block's attached block menu so we jump to it and select
    /// it if possible.
    SelectAIAttachedBlock(BlockIndex),
    DragAndDropFiles(Vec<String>),
    /// Triggers an ssh session to warpify, even if there is no Warpify Block.
    WarpifySSHSession,
    NotifySshErrorBlock(SshErrorBlockAction),
    /// Sets the input mode to Agent Mode
    SetInputModeAgent,
    /// Sets the input mode to Terminal Mode
    SetInputModeTerminal,
    /// Toggle voice input for CLI agent footer (dispatched from alt screen/blocklist when footer is visible)
    #[cfg(feature = "voice_input")]
    ToggleCLIAgentVoiceInput(voice_input::VoiceInputToggledFrom),

    HyperlinkClick(HyperlinkUrl),
    AttemptLoginGatedFeature,
    StartFileDropTarget,
    StopFileDropTarget,
    OpenTeamSettingsPage,
    SetMarkedText {
        marked_text: UserInput<String>,
        selected_range: Range<usize>,
    },
    ClearMarkedText,
    SelectAgenticSuggestion(i32),
    HideTelemetryBannerPermanently,
    ShowInitializationBlock,
    GenerateCodebaseIndex,
    /// This is for debugging, dev only for now
    LoadAgentModeConversation,
    ShowWarpifySettings,
    /// Removes a pending attachment (image or file) by index in the unified list.
    DeleteAttachment {
        index: usize,
    },
    WriteCodebaseIndex,
    ToggleAutoexecuteMode,
    ToggleQueueNextPrompt,
    CodebaseIndexSpeedbumpBanner(CodebaseIndexSpeedbumpBannerAction),
    AgentModeSetupSpeedbumpBanner(AgentModeSetupSpeedbumpBannerAction),
    AnonymousUserAISignUpBanner(AnonymousUserLoginBannerAction),
    ResumeConversation,
    ForkConversationFromLastKnownGoodState,
    ToggleAIDocumentPane,
    ToggleTodoPopup,
    CloseTodoPopup,
    ToggleCodeReviewPane {
        entrypoint: CodeReviewPaneEntrypoint,
    },
    InitProject,
    SummarizeConversation,
    IndexProjectSpeedbump,
    AddProjectAtCurrentDirectory,
    OpenProjectRulesPane,
    OpenViewMCPPane,
    OpenAddMCPPane,
    OpenAddRulePane,
    OpenRulesPane,
    OpenEditSkillPane {
        skill_reference: SkillReference,
    },
    OpenAddPromptPane,
    OpenBillingAndUsagePane,
    OpenConversationsPalette,
    PickRepoToOpen,
    OpenFilesPalette {
        source: PaletteSource,
    },
    DismissCodeToolbeltTooltip,
    /// Start a Language Server for the current working directory (if supported)
    StartLspServer,
    /// Start the guided Warp Environment setup flow (inserts the inline setup block).
    SetupCloudEnvironment(Vec<String>),
    /// Start the guided Warp Environment setup flow immediately (no inline setup block).
    SetupCloudEnvironmentAndStart(Vec<String>),
    /// Show the environment setup mode selector to choose between remote GitHub or local agent flow.
    TriggerEnvironmentSetupSelection(Vec<String>),
    /// Open the Environment Management pane.
    OpenEnvironmentManagementPane,
    ToggleLongRunningCommandControl,
    ToggleHideCliResponses,
    ExitAgentView,
    EnterCloudAgentView,
    StartNewAgentConversation,
    /// Toggle the cloud mode conversation details panel
    ToggleConversationDetailsPanel,
    /// Cancel the ambient agent task while it's loading
    CancelAmbientAgentTask,
    OpenInlineHistoryMenu,
    OpenModelSelector,
    ResolvePromptSuggestion(PromptSuggestionResolution),
    AwsBedrockLoginBanner(AwsBedrockLoginBannerAction),
    AwsCliNotInstalledBanner(AwsCliNotInstalledBannerAction),
    /// Toggle the usage footer on the last AI block in the active conversation.
    ToggleUsageFooter,
    /// Reveal a hidden child agent pane from the orchestrator status card.
    RevealChildAgent {
        conversation_id: AIConversationId,
    },
    /// Switch the active terminal view's agent view to display the given
    /// conversation in place, without spawning or revealing a separate pane.
    /// Used by the orchestration pill bar to navigate the current pane to a
    /// sibling/parent conversation.
    SwitchAgentViewToConversation {
        conversation_id: AIConversationId,
    },
    /// Open a child agent conversation in a separate pane (split off from
    /// the orchestrator). Dispatched from the orchestration pill bar's
    /// 3-dot overflow menu ("Open in new pane"). For child agents that have
    /// a hidden pane in `child_agent_panes` this reveals the existing pane;
    /// for already-visible panes it focuses the existing pane.
    OpenChildAgentInNewPane {
        conversation_id: AIConversationId,
    },
    /// Open a child agent conversation in a separate tab. V2-of-V2 stub:
    /// dispatched from the orchestration pill bar's 3-dot overflow menu
    /// ("Open in new tab"). For now this falls back to the same path as
    /// `OpenChildAgentInNewPane` until tab-level routing is wired through.
    OpenChildAgentInNewTab {
        conversation_id: AIConversationId,
    },
    /// Stop a child agent conversation: cancel the in-flight ambient task
    /// (if any) and the local conversation's controller. The conversation
    /// itself stays alive so the user can still navigate to it. Dispatched
    /// from the orchestration pill bar's 3-dot overflow menu ("Stop agent").
    StopAgentConversation {
        conversation_id: AIConversationId,
    },
    /// Kill a child agent conversation: stop it if running, best-effort cancel
    /// any backing cloud task, then remove the conversation from local history.
    /// Dispatched from the orchestration pill bar's 3-dot overflow menu
    /// ("Kill agent").
    KillAgentConversation {
        conversation_id: AIConversationId,
    },
    /// Toggle PTY recording for this session.
    ToggleSessionRecording,
    /// Toggle the rich input editor for composing a prompt to send to a CLI agent.
    /// Triggered by Ctrl-G when a CLI agent is detected, or from the footer button.
    ToggleCLIAgentRichInput,
}

// Manually implementing Debug to avoid leaking sensitive information in logs
impl fmt::Debug for TerminalAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use TerminalAction::*;

        match self {
            Scroll { delta } => write!(f, "Scroll {{ delta: {delta} }}"),
            AltScroll { delta } => write!(f, "AltScroll {{ delta: {delta} }}"),
            SharedSessionViewerAltScroll { new_scroll_top } => write!(
                f,
                "SharedSessionViewerAltScroll {{ new_scroll_top: {new_scroll_top} }}"
            ),
            ScrollToTopOfBlock { topmost_block } => write!(
                f,
                "JumpToPreviousCommand {{ topmost_block: {topmost_block} }}"
            ),
            ScrollToTopOfSelectedBlocks => f.write_str("ScrollToTopOfSelectedBlocks"),
            ScrollToBottomOfSelectedBlocks => f.write_str("ScrollToBottomOfSelectedBlocks"),
            ScrollToBottomOfOverhangingBlock(overhanging_block) => {
                write!(f, "ScrollToBottomOfOverhangingBlock {overhanging_block:?}")
            }
            BlockTextSelect(action) => write!(f, "BlockTextSelect({action:?})"),
            BlockSelect { action, .. } => write!(f, "BlockSelect({action:?})"),
            BlockHover(action) => write!(f, "BlockHover({action:?})"),
            BlockSnackbarHover { is_hovered } => {
                write!(f, "BlockSnackbarHover{{ is_hovered {is_hovered} }}")
            }
            BlockNearSnackbarHover { is_hovered } => {
                write!(f, "BlockNearSnackbarHover{{ is_hovered {is_hovered} }}")
            }
            ClickOnGrid {
                position,
                modifiers,
            } => write!(
                f,
                "ClickOnGrid {{ position: {position:?}, modifiers: {modifiers:?} }}"
            ),
            MaybeLinkHover {
                position,
                from_editor,
            } => write!(
                f,
                "MaybeLinkHover {{ position: {position:?}, from_editor: {from_editor:?} }}"
            ),
            MaybeHoverSecret { secret_handle } => {
                write!(f, "MaybeHoverSecret {{ secret_handle: {secret_handle:?} }}")
            }
            MaybeDismissToolTip { from_keybinding } => write!(
                f,
                "MaybeDismissToolTip {{ from_keybinding: {from_keybinding:?}}}"
            ),
            AltSelect(action) => write!(f, "AltSelect({action:?})"),
            MaybeClearAltSelect => f.write_str("MaybeClearAltSelect"),
            AltMouseAction(action) => write!(f, "AltMouseAction({action:?})"),
            AltScreenContextMenu { position } => {
                write!(f, "AltScreenContextMenu {{ position: {position:?} }}")
            }
            BlockListContextMenu(menu) => write!(f, "BlockListContextMenu({menu:?})"),
            CloseContextMenu => f.write_str("CloseContextMenu"),
            Paste => f.write_str("Paste"),
            Copy => f.write_str("Copy"),
            CopyOutputs => f.write_str("CopyOutputs"),
            CopyCommands => f.write_str("CopyCommands"),
            CopyGitBranch => f.write_str("CopyGitBranch"),
            OpenShareModal => f.write_str("OpenShareModal"),
            ReinputCommands => f.write_str("ReinputCommands"),
            ReinputCommandsWithSudo => f.write_str("ReinputCommandsWithSudo"),
            ClearBuffer => f.write_str("ClearBuffer"),
            SelectBookmarkUp => f.write_str("SelectBookmarkUp"),
            SelectBookmarkDown => f.write_str("SelectBookmarkDown"),
            Focus => f.write_str("Focus"),
            FocusInputAndClearSelection => f.write_str("FocusInputAndClearSelection"),
            ShowFindBar => f.write_str("ShowFindBar"),
            SelectPriorBlock => f.write_str("SelectPriorBlock"),
            SelectNextBlock => f.write_str("SelectNextBlock"),
            BookmarkSelectedBlock => f.write_str("BookmarkSelectedBlock"),
            Up => f.write_str("Up"),
            Down => f.write_str("Down"),
            PageUp => f.write_str("PageUp"),
            PageDown => f.write_str("PageDown"),
            Home => f.write_str("Home"),
            End => f.write_str("End"),
            KeyboardSelectText(direction) => write!(f, "KeyboardSelectText({direction:?})"),
            ContextMenu(action) => write!(f, "ContextMenu({action:?})"),
            CtrlD => f.write_str("CtrlD"),
            CtrlC => f.write_str("CtrlC"),
            ClearSelectionsWhenShellMode => {
                f.write_str("ClearSelectionsWhenShellMode(TerminalAction)")
            }
            Close => f.write_str("Close"),
            SplitRight(_) => f.write_str("SplitRight"),
            SplitLeft(_) => f.write_str("SplitLeft"),
            SplitDown(_) => f.write_str("SplitDown"),
            SplitUp(_) => f.write_str("SplitUp"),
            ToggleMaximizePane => f.write_str("ToggleMaximizeActivePane"),
            PromptContextMenu {
                position_offset_from_prompt,
            } => write!(
                f,
                "PromptContextMenu {{ position_offset_from_prompt: {position_offset_from_prompt:?} }}"
            ),
            OpenInputContextMenu { position } => {
                write!(f, "OpenInputContextMenu {{ position: {position:?} }}")
            }
            InputContextMenuItem(action) => write!(f, "InputContextMenuItem({action:?})"),
            SelectAllBlocks => f.write_str("SelectAllBlocks"),
            ExpandBlockSelectionAbove => f.write_str("ExpandBlockSelectionAbove"),
            ExpandBlockSelectionBelow => f.write_str("ExpandBlockSelectionBelow"),
            UserInputSequence(_) => f.write_str("UserInputSequence"),
            ControlSequence(_) => f.write_str("ControlSequence"),
            KeyDown(_) => f.write_str("KeyDown"),
            TypedCharacters(_) => f.write_str("TypedCharacters"),
            NotificationsDiscoveryBanner(action) => {
                write!(f, "NotificationsDiscoveryBanner({action:?})")
            }
            BookmarkBlock(index) => {
                write!(f, "BookmarkBlock({index:?})")
            }
            NotificationsErrorBanner(action) => write!(f, "NotificationsErrorBanner({action:?})"),
            LegacySSHBanner(action) => write!(f, "SSHBanner({action:?})"),
            JumpToBookmark(index) => write!(f, "JumpToBookmark({index:?})"),
            InsertCommandCorrection { .. } => {
                write!(f, "InsertCommandCorrection",)
            }
            OpenGridLink(_) => f.write_str("OpenGridLink"),
            OpenRichContentLink(_) => f.write_str("OpenRichContentLink"),
            ToggleGridSecret { show_secret, .. } => write!(f, "ToggleGridSecret {show_secret:?}"),
            ToggleRichContentSecret { show_secret, .. } => {
                write!(f, "ToggleRichContentSecret {show_secret:?}")
            }
            CopyGridSecret(_) => f.write_str("CopyGridSecret"),
            CopyRichContentSecret(_) => f.write_str("CopyRichContentSecret"),
            ShowInFileExplorer(_) => f.write_str("ShowInFileExplorer"),
            OpenFileInWarp(_) => f.write_str("OpenFileInWarp"),
            #[cfg(feature = "local_fs")]
            OpenCodeInWarp { .. } => f.write_str("OpenCodeInWarp"),
            OpenWorkflowModal => f.write_str("OpenWorkflowModal"),
            OpenWorkflowModalForAIWorkflow(_) => f.write_str("OpenWorkflowModalForAIWorkflow"),
            OpenWorkflowModalForBlock(block_index) => {
                write!(f, "OpenWorkflowModalForBlock({block_index:?})")
            }
            OpenWorkflowModalWithCloudWorkflow(_) => {
                f.write_str("OpenWorkflowModalWithCloudWorkflow")
            }
            OpenBlockListContextMenu => f.write_str("OpenBlockListContextMenu"),
            AskAIAssistant { block_index } => write!(f, "AskAIAssistant({block_index:?})"),
            TriggerSubshellBootstrap => f.write_str("TriggerSubshellBootstrap"),
            DismissWarpifyBanner(remember) => write!(f, "DismissWarpifyBanner({remember:?})"),
            ShowSubshellBanner(_) => f.write_str("ShowSubshellBanner"),
            ShowWarpifySshBanner(_, _) => f.write_str("ShowWarpifySshBanner"),
            InsertMostRecentCommandCorrection => f.write_str("InsertMostRecentCommandCorrection"),
            AliasExpansionBanner(action) => write!(f, "AliasExpansionBanner({action:?}"),
            OpenInWarpBanner(action) => write!(f, "OpenInWarpBanner({action:?})"),
            OpenBlockFilterEditor(block_index) => {
                write!(f, "OpenBlockFilterEditor({block_index:?})")
            }
            OnboardingFlow(version) => write!(f, "OnboardingFlow({version:?})"),
            ImportSettings => write!(f, "ImportSettings"),
            StopSharingCurrentSession { source } => {
                write!(f, "StopSharingCurrentSession({source:?})")
            }
            OpenSharedSessionOnDesktop { source } => {
                write!(f, "OpenSharedSessionOnDesktop({source:?})")
            }
            ToggleBlockFilterOnSelectedOrLastBlock(_) => {
                f.write_str("ToggleBlockFilterOnSelectedOrLastBlock")
            }
            OpenShareSessionModal { source } => write!(f, "OpenShareSessionModal({source:?})"),
            CopySharedSessionLink { .. } => f.write_str("CopySharedSessionLink"),
            VimModeBanner(action) => write!(f, "VimModeBanner({action:?})"),
            ToggleSnackbarInActivePane => write!(f, "ToggleSnackbarInActivePane"),
            MakeAllParticipantsReaders { reason } => {
                write!(f, "MakeAllParticipantsReaders {{ reason: {reason:?} }}")
            }
            OpenSharedSessionViewerRoleMenu => write!(f, "OpenSharedSessionViewerRoleMenu"),
            RequestSharedSessionRole(role) => write!(f, "RequestSharedSessionRole({role:?})"),
            MiddleClickOnGrid { position } => {
                write!(f, "MiddleClickonGrid {{ position: {position:?} }}")
            }
            MiddleClickOnInput => write!(f, "MiddleClickOnInput"),
            OpenAIBlockAttachedBlocksMenu { .. } => write!(f, "OpenAIBlockAttachedBlocksMenu"),
            OpenAIBlockOverflowMenu { .. } => write!(f, "OpenAIBlockOverflowMenu"),
            RewindAIConversation { .. } => write!(f, "RewindAIConversation"),
            ExecuteRewindAIConversation { .. } => write!(f, "ExecuteRewindAIConversation"),
            ExecuteRewindFromInlineMenu { .. } => write!(f, "ExecuteRewindFromInlineMenu"),
            SelectAIAttachedBlock(_) => write!(f, "SelectAIAttachedBlock"),
            DragAndDropFiles(_) => write!(f, "DragAndDropFiles"),
            WarpifySSHSession => write!(f, "WarpifySSHSession"),
            NotifySshErrorBlock(action) => write!(f, "NotifySshErrorBlock({action:?})"),
            SetInputModeAgent => write!(f, "SetInputModeAgent"),
            SetInputModeTerminal => write!(f, "SetInputModeTerminal"),
            #[cfg(feature = "voice_input")]
            ToggleCLIAgentVoiceInput(source) => write!(f, "ToggleCLIAgentVoiceInput({source:?})"),
            HyperlinkClick(hyperlink_url) => write!(f, "HyperlinkClick({hyperlink_url:?})"),
            AttemptLoginGatedFeature => write!(f, "AttemptLoginGatedFeature"),
            StartFileDropTarget => write!(f, "StartFileDropTarget"),
            StopFileDropTarget => write!(f, "StopFileDropTarget"),
            RunNativeShellCompletions { buffer_text, .. } => {
                write!(f, "RunNativeShellCompletions({buffer_text:?})")
            }
            OpenTeamSettingsPage => write!(f, "OpenTeamSettingsPage"),
            SetMarkedText {
                marked_text,
                selected_range,
            } => write!(f, "SetMarkedText {{{marked_text:?}, {selected_range:?}}}"),
            ClearMarkedText => write!(f, "ClearMarkedText"),
            SelectAgenticSuggestion(index) => write!(f, "SelectAgenticSuggestion({index:?})"),
            HideTelemetryBannerPermanently => write!(f, "HideTelemetryBannerPermanently"),
            ShowInitializationBlock => write!(f, "ShowInitializationBlock"),
            GenerateCodebaseIndex => write!(f, "GenerateIndexForRepo"),
            LoadAgentModeConversation => write!(f, "LoadAgentModeConversation"),
            ShowWarpifySettings => write!(f, "ShowWarpifySettings"),
            DeleteAttachment { index } => write!(f, "DeleteAttachment({index:?})"),
            WriteCodebaseIndex => write!(f, "PersistCodebaseIndex"),
            ToggleAutoexecuteMode => write!(f, "ToggleAutoexecuteMode"),
            ToggleQueueNextPrompt => write!(f, "ToggleQueueNextPrompt"),
            CodebaseIndexSpeedbumpBanner(action) => {
                write!(f, "CodebaseIndexSpeedbumpBanner({action:?})")
            }
            AgentModeSetupSpeedbumpBanner(action) => {
                write!(f, "AgentModeSetupSpeedbumpBanner({action:?})")
            }
            AnonymousUserAISignUpBanner(action) => {
                write!(f, "AnonymousUserLoginBanner({action:?})")
            }
            ResumeConversation => write!(f, "ResumeConversation"),
            ForkConversationFromLastKnownGoodState => {
                write!(f, "ForkConversationFromLastKnownGoodState")
            }
            ToggleAIDocumentPane => write!(f, "ToggleAIDocumentPane"),
            ToggleTodoPopup => write!(f, "ToggleTodoPopup"),
            CloseTodoPopup => write!(f, "CloseTodoPopup"),
            ToggleCodeReviewPane { .. } => write!(f, "ToggleCodeReviewPane"),
            InitProject => write!(f, "InitProject"),
            IndexProjectSpeedbump => write!(f, "IndexProject"),
            AddProjectAtCurrentDirectory => write!(f, "AddProjectAtCurrentDirectory"),
            OpenProjectRulesPane => write!(f, "OpenProjectRulesPane"),
            OpenViewMCPPane => write!(f, "OpenViewMCPPane"),
            OpenAddMCPPane => write!(f, "OpenAddMCPPane"),
            OpenAddRulePane => write!(f, "OpenAddRulePane"),
            OpenRulesPane => write!(f, "OpenRulesPane"),
            OpenEditSkillPane { .. } => write!(f, "OpenEditSkillPane"),
            OpenAddPromptPane => write!(f, "OpenAddPromptPane"),
            OpenBillingAndUsagePane => write!(f, "OpenBillingAndUsagePane"),
            OpenConversationsPalette => write!(f, "OpenConversationsPalette"),
            PickRepoToOpen => write!(f, "PickRepoToOpen"),
            OpenFilesPalette { .. } => write!(f, "OpenFilesPalette"),
            DismissCodeToolbeltTooltip => write!(f, "DismissCodeToolbeltTooltip"),
            StartLspServer => write!(f, "StartLspServer"),
            SetupCloudEnvironment(_) => write!(f, "SetupCloudEnvironment"),
            SetupCloudEnvironmentAndStart(_) => write!(f, "SetupCloudEnvironmentAndStart"),
            TriggerEnvironmentSetupSelection(_) => write!(f, "TriggerEnvironmentSetupSelection"),
            OpenEnvironmentManagementPane => write!(f, "OpenEnvironmentManagementPane"),
            SummarizeConversation => write!(f, "SummarizeConversation"),
            ToggleLongRunningCommandControl => {
                write!(f, "TakeOverLongRunningCommandControlForUser")
            }
            ToggleHideCliResponses => write!(f, "ToggleHideCliResponses"),
            ExitAgentView => write!(f, "ExitAgentView"),
            EnterCloudAgentView => write!(f, "EnterCloudAgentView"),
            StartNewAgentConversation => write!(f, "StartNewAgentConversation"),
            ToggleConversationDetailsPanel => write!(f, "ToggleConversationDetailsPanel"),
            CancelAmbientAgentTask => write!(f, "CancelAmbientAgentTask"),
            OpenInlineHistoryMenu => write!(f, "OpenInlineHistoryMenu"),
            OpenModelSelector => write!(f, "OpenModelSelector"),
            ResolvePromptSuggestion(..) => write!(f, "ResolvePromptSuggestion"),
            AwsBedrockLoginBanner(action) => write!(f, "AwsBedrockLoginBanner({action:?})"),
            AwsCliNotInstalledBanner(action) => write!(f, "AwsCliNotInstalledBanner({action:?})"),
            ToggleUsageFooter => write!(f, "ToggleUsageFooter"),
            RevealChildAgent { .. } => write!(f, "RevealChildAgent"),
            SwitchAgentViewToConversation { .. } => write!(f, "SwitchAgentViewToConversation"),
            OpenChildAgentInNewPane { .. } => write!(f, "OpenChildAgentInNewPane"),
            OpenChildAgentInNewTab { .. } => write!(f, "OpenChildAgentInNewTab"),
            StopAgentConversation { .. } => write!(f, "StopAgentConversation"),
            KillAgentConversation { .. } => write!(f, "KillAgentConversation"),
            ToggleSessionRecording => write!(f, "ToggleSessionRecording"),
            ToggleCLIAgentRichInput => write!(f, "ToggleCLIAgentRichInput"),
        }
    }
}
