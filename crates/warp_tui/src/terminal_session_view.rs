//! Authenticated terminal-session TUI surface.
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use instant::Instant;
use parking_lot::FairMutex;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::settings::{AISettings, AISettingsChangedEvent};
use warp::tui_export::{
    AIAgentActionId, AIAgentActionResultType, AIAgentContext, AIAgentExchangeId,
    AIAgentPtyWriteMode, AIConversation, AIConversationId, AcceptSlashCommandOrSavedPrompt,
    ActiveSession, ActiveSessionEvent, AgentConversationEntryId, AgentConversationListEntryState,
    AgentConversationsModel, AgentInteractionMetadata, AgentViewEntryOrigin, BlockId,
    BlocklistAIActionEvent, BlocklistAIActionModel, BlocklistAIContextModel, BlocklistAIController,
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, BlocklistAIInputModel, CLISubagentController,
    CLISubagentEvent, CLISubagentTarget, COMMAND_REGISTRY, CancellationReason, ChangelogModel,
    ChangelogRequestType, CloudConversationData, CommandExecutionSource, ConversationFileExport,
    ConversationSelection, ConversationSelectionHandle, ConversationUsageTotals,
    ExecuteCommandEvent, GetRelevantFilesController, GitRepoModels, GitRepoStatusModel,
    GitStatusMetadata, LLMId, LLMPreferences, LLMPreferencesEvent,
    LOCAL_SKILLS_REMOTE_EXECUTION_ERROR_MESSAGE, ModelEvent, ParsedSlashCommandInput,
    PersistenceWriter, PtyIntent, PtyIntentEvent, RepoDetectionSessionType, RepoDetectionSource,
    ServerConversationToken, ShellCommandExecutorEvent, SizeInfo, SizeUpdate, SkillReference,
    SlashCommandDataSource as _, SlashCommandKind, SlashCommandSelectionBehavior,
    StartAgentExecutorEvent, StartAgentRequest, StaticCommand, TerminalModel, TerminalSurface,
    TerminalSurfaceInit, TranscriptScope, TuiMcpAction, TuiMcpManager, TuiSlashCommandDataSource,
    TuiSlashCommandDataSourceArgs, TuiZeroStateDataSource, UserTakeOverReason,
    WAKEUP_THROTTLE_PERIOD, block_context_from_terminal_model, build_slash_command_mixer,
    detect_possible_git_repo, export_conversation_markdown, log_out_tui,
    maybe_build_ai_query_upsert_event, prepare_conversation_block_restoration,
    record_autodetection_toggle_from_slash_command, record_saved_prompt_accepted,
    record_static_slash_command_accepted, saved_prompt_text_for_id,
    slash_command_selection_behavior, throttle,
};
use warp_core::features::FeatureFlag;
use warp_core::settings::Setting;
use warp_editor::model::CoreEditorModel;
use warp_errors::report_error;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::SingletonEntity;
use warpui_core::r#async::{SpawnedFutureHandle, Timer};
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{
    TuiChildView, TuiConstrainedBox, TuiContainer, TuiElement, TuiFlex, TuiHoverable, TuiSize,
    TuiText,
};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, EditableBinding, FixedBinding};
use warpui_core::platform::TerminationMode;
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::agent_block::TuiBlockingChild;
use crate::alt_screen_view::AltScreenElement;
use crate::attachment_bar::{
    FOCUS_ATTACHMENTS_BINDING_NAME, TuiAttachmentBar, TuiAttachmentBarEvent, TuiAttachmentModel,
    TuiAttachmentPasteDisposition,
};
use crate::clipboard::copy_to_clipboard;
use crate::conversation_menu::{TuiConversationMenuEvent, TuiConversationMenuModel};
use crate::conversation_selection::TuiConversationSelection;
use crate::editor_interaction::TuiEditorCommand;
use crate::exit_confirmation::{CTRL_C_EXIT_WINDOW, ExitConfirmation};
use crate::inline_menu::{MAX_INLINE_MENU_ROWS, TuiInlineMenu, active_inline_menu};
use crate::input::view::TuiInputAction;
use crate::input::{TuiInputView, TuiInputViewEvent};
use crate::input_hints;
use crate::input_mode_policy::{self, TuiInputModePolicy};
use crate::input_suggestions_mode::TuiInputSuggestionsModeModel;
use crate::keybindings::{
    ATTACHMENTS_AVAILABLE_FLAG, CONTEXTUAL_PLAN_TOGGLE_BINDING_NAME,
    KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG, PLAN_TOGGLE_AVAILABLE_FLAG, PLAN_TOGGLE_BINDING_NAME,
    TUI_BINDING_GROUP,
};
use crate::mcp_menu::{TuiMcpMenuEvent, TuiMcpMenuModel};
use crate::model_menu::{TuiModelMenuEvent, TuiModelMenuModel};
use crate::orchestration_model::{TuiOrchestrationModel, TuiOrchestrationSnapshot};
use crate::orchestration_tab_bar::{
    ORCHESTRATION_TAB_BAR_FOCUSED_FLAG, TuiOrchestrationTabNavigationAction,
    orchestration_tab_bar_config, register_orchestration_surface_bindings,
    render_orchestration_tab_footer,
};
use crate::platform::reveal_path_in_file_manager;
use crate::prompt_history_menu::{TuiPromptHistoryMenuEvent, TuiPromptHistoryMenuModel};
use crate::resume::TuiExitSummaryHandle;
use crate::session_registry::TuiSessions;
use crate::skills_menu::{TuiSkillMenuEvent, TuiSkillMenuModel};
use crate::slash_commands::TuiSlashCommandModel;
use crate::tab_bar::{TuiTabBarConfig, TuiTabBarEvent, TuiTabBarView};
use crate::terminal_content_element::TuiTerminalContentElement;
use crate::terminal_use::{
    TerminalUseInterruptAction, TuiInputTarget, hide_agent_requested_command_from_top_level,
    inline_process_owns_input, terminal_use_conversation_to_resume, terminal_use_interrupt_action,
    tui_input_target,
};
use crate::transcript_view::{TuiTranscriptView, TuiTranscriptViewEvent};
use crate::transient_hint::{TransientHint, TransientHintTone};
use crate::tui_builder::TuiUiBuilder;
use crate::tui_cli_subagent_view::{HAND_BACK_KEY_BINDING, TuiCLISubagentView};
use crate::ui::{compact_footer_path, conversation_restore_failed, conversation_restoring};
use crate::usage::UsageToggle;
use crate::warping_indicator::{render_response_summary, render_warping_indicator_row};
use crate::zero_state::TuiZeroStateView;
mod input_detection;

use self::input_detection::InputDetectionState;

/// Width used before the first layout pass pushes the real terminal width into the editor.
const INITIAL_INPUT_WIDTH: u16 = 80;
const INLINE_MENU_TOP_PADDING_ROWS: u16 = 1;
const MAX_INPUT_TEXT_ROWS: u16 = 6;
const AUTO_APPROVE_FEEDBACK_DURATION: Duration = Duration::from_secs(3);

/// The footer hint shown while the ctrl-c exit confirmation is armed.
const CTRL_C_EXIT_HINT: &str = "ctrl-c again to exit";
const STARTING_SHELL_HINT: &str = "Starting shell...";
const SESSION_CAN_CANCEL_RESTORE_FLAG: &str = "TuiSessionCanCancelRestore";
const SESSION_CAN_HAND_BACK_CONTROL_FLAG: &str = "TuiSessionCanHandBackControl";
pub(crate) const SESSION_COMPOSER_OWNS_INPUT_FLAG: &str = "TuiSessionComposerOwnsInput";
pub(crate) const PASTE_IMAGE_BINDING_NAME: &str = "tui:session:paste_image";
pub(crate) const AUTO_APPROVE_TOGGLE_BINDING_NAME: &str = "tui:session:toggle_auto_approve";

/// Events emitted by the TUI terminal session surface.
pub(crate) enum TuiTerminalSessionEvent {
    ExecuteCommand(Box<ExecuteCommandEvent>),
    InterruptPty,
    WriteAgentInput {
        bytes: Cow<'static, [u8]>,
        mode: AIAgentPtyWriteMode,
    },
    WriteUserInput(Cow<'static, [u8]>),
    Resize(SizeUpdate),
    StartAgentConversation {
        request: Box<StartAgentRequest>,
        working_directory: Option<PathBuf>,
    },
    CleanupFailedChildLaunch {
        conversation_id: AIConversationId,
    },
}

impl PtyIntentEvent for TuiTerminalSessionEvent {
    fn pty_intent(&self) -> Option<PtyIntent> {
        match self {
            Self::ExecuteCommand(event) => Some(PtyIntent::ExecuteCommand((**event).clone())),
            Self::InterruptPty => Some(PtyIntent::Interrupt),
            Self::WriteAgentInput { bytes, mode } => Some(PtyIntent::WriteAgentInput {
                bytes: bytes.clone(),
                mode: *mode,
            }),
            Self::WriteUserInput(bytes) => Some(PtyIntent::WriteBytes(bytes.clone())),
            Self::Resize(size_update) => Some(PtyIntent::Resize(*size_update)),
            Self::StartAgentConversation { .. } | Self::CleanupFailedChildLaunch { .. } => None,
        }
    }
}

/// Transient hint shown when a shell command is rejected because the PTY is
/// already running a command.
const COMMAND_ALREADY_RUNNING_HINT: &str = "cannot run — command already running";
const NEW_CONVERSATION_COMMAND_RUNNING_HINT: &str =
    "cannot start new conversation while terminal command is running";
const SWITCH_COMMAND_RUNNING_HINT: &str =
    "Cannot switch conversations while a command is in progress.";
const SWITCH_CONVERSATION_RUNNING_HINT: &str =
    "Cannot switch conversations while the current conversation is in progress.";
const SWITCH_LOADING_HINT: &str = "Another conversation is already loading.";
const SWITCH_UNAVAILABLE_HINT: &str = "That conversation is no longer available.";
const LOADING_CONVERSATION_HINT: &str = "Loading conversation…";
const MODEL_PERSISTENCE_FAILED_HINT: &str = "Could not save the selected model.";

/// Footer label shown while the input is in `!` shell mode. The how-to-exit
/// guidance lives in the input's placeholder ghost text, so the footer only
/// names the mode.
const SHELL_MODE_HINT: &str = "Shell mode";
const COPY_SELECTION_HINT: &str = "copied to clipboard";
const COPY_FAILED_HINT: &str = "failed to copy to clipboard";
const LOG_BUNDLE_FAILED_HINT: &str = "Failed to create log bundle (check logs)";
const NLD_ENABLED_HINT: &str = "Natural language detection enabled.";
const NLD_DISABLED_HINT: &str = "Natural language detection disabled.";
const NLD_PERSISTENCE_FAILED_HINT: &str = "Could not save the natural language detection setting.";
const COST_NO_ACTIVE_CONVERSATION_HINT: &str =
    "Cannot show conversation cost: no active conversation";
const COST_EMPTY_CONVERSATION_HINT: &str = "Cannot show conversation cost: conversation is empty";
const COST_CONVERSATION_IN_PROGRESS_HINT: &str =
    "Cannot show conversation cost: conversation is in progress";

fn log_bundle_success_message(path: &Path) -> String {
    format!("Log bundle saved to {}", path.display())
}

fn raw_prompt_if_not_blank(input: &str) -> Option<&str> {
    (!input.trim().is_empty()).then_some(input)
}
fn cost_command_unavailable_hint(
    selected_conversation: Option<(bool, bool)>,
) -> Option<&'static str> {
    match selected_conversation {
        None => Some(COST_NO_ACTIVE_CONVERSATION_HINT),
        Some((true, _)) => Some(COST_EMPTY_CONVERSATION_HINT),
        Some((false, false)) => Some(COST_CONVERSATION_IN_PROGRESS_HINT),
        Some((false, true)) => None,
    }
}

/// Resolved segments for the footer's left-aligned sectioned status row.
/// [`TuiTerminalSessionView::render_footer`] builds this from view state and
/// delegates to [`render_status_footer_row`]; keeping the row layout separate
/// makes the left alignment, section order, and shell-mode omissions directly
/// render-to-lines testable without view-state plumbing.
struct FooterSegments {
    /// Whether the input is in `!` shell mode: the shell-mode indicator leads
    /// the row and the model/usage segments are hidden.
    shell_mode: bool,
    /// The clickable active-model label. Hidden in shell mode.
    model_label: Option<Box<dyn TuiElement>>,
    /// The session's compacted working directory. Part of the combined
    /// cwd/branch section.
    cwd: Option<String>,
    /// The current branch name, appended to the cwd segment as ` ↬ branch`.
    branch: Option<String>,
    /// The clickable usage entry. Hidden in shell mode.
    usage: Option<Box<dyn TuiElement>>,
    /// Uncommitted diff additions; rendered as `+N` when > 0.
    diff_additions: usize,
    /// Uncommitted diff deletions; rendered as `-M` when > 0.
    diff_deletions: usize,
}

/// Builds the left-aligned sectioned status row from resolved segments.
///
/// Agent mode orders the sections `[model] [cwd ↬ branch] • [usage] •
/// [+N -M]`; shell mode leads with the shell-mode indicator and hides the
/// model and usage segments, yielding `[Shell mode] [cwd ↬ branch] •
/// [+N -M]`. A plain space separates the model from cwd/branch; a ` • `
/// separator precedes usage and diff. Absent metadata never leaves a stray
/// separator. Every child truncates to a single row, so the row lays out one
/// row tall.
fn render_status_footer_row(segments: FooterSegments, builder: &TuiUiBuilder) -> TuiFlex {
    let muted = builder.muted_text_style();
    let mut row = TuiFlex::row();
    let mut has_segment = false;

    // First segment: the shell-mode indicator (Shell mode) or the clickable
    // model label (agent mode). Shell mode hides the model segment so the
    // indicator leads.
    if segments.shell_mode {
        row = row.child(
            TuiText::new(SHELL_MODE_HINT)
                .with_style(builder.shell_command_accent_style())
                .truncate()
                .finish(),
        );
        has_segment = true;
    } else if let Some(model_label) = segments.model_label {
        row = row.child(model_label);
        has_segment = true;
    }

    // Combined cwd/branch section: the branch stays contextual to its path.
    // A plain space separates this from the leading model/shell-mode segment,
    // matching master's layout where model and cwd run together without a dot.
    if segments.cwd.is_some() || segments.branch.is_some() {
        if has_segment {
            row = row.child(TuiText::new(" ").truncate().finish());
        }
        if let Some(cwd) = segments.cwd {
            row = row.child(TuiText::new(cwd).with_style(muted).truncate().finish());
        }
        if let Some(branch) = segments.branch {
            row = row.child(
                TuiText::new(format!(" ↬ {branch}"))
                    .with_style(muted)
                    .truncate()
                    .finish(),
            );
        }
        has_segment = true;
    }

    // Usage entry (agent mode only): the clickable credits/cost toggle.
    if !segments.shell_mode
        && let Some(usage) = segments.usage
    {
        if has_segment {
            row = row.child(TuiText::new(" • ").with_style(muted).truncate().finish());
        }
        row = row.child(usage);
        has_segment = true;
    }

    // Diff counts retain their existing added/removed styles.
    if segments.diff_additions > 0 || segments.diff_deletions > 0 {
        if has_segment {
            row = row.child(TuiText::new(" • ").with_style(muted).truncate().finish());
        }
        if segments.diff_additions > 0 {
            row = row.child(
                TuiText::new(format!("+{}", segments.diff_additions))
                    .with_style(builder.diff_added_style())
                    .truncate()
                    .finish(),
            );
        }
        if segments.diff_deletions > 0 {
            if segments.diff_additions > 0 {
                row = row.child(TuiText::new(" ").truncate().finish());
            }
            row = row.child(
                TuiText::new(format!("-{}", segments.diff_deletions))
                    .with_style(builder.diff_removed_style())
                    .truncate()
                    .finish(),
            );
        }
    }

    row
}
/// Entry point that requested conversation restoration.
#[derive(Clone, Copy, Debug)]
pub(crate) enum TuiConversationRestoreOrigin {
    Startup,
    ConversationList,
}

impl TuiConversationRestoreOrigin {
    fn agent_view_origin(self) -> AgentViewEntryOrigin {
        match self {
            Self::Startup | Self::ConversationList => {
                AgentViewEntryOrigin::RestoreExistingConversation
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum TuiConversationRestoreTarget {
    Local(AIConversationId),
    Server(ServerConversationToken),
}

#[derive(Default)]
enum ConversationRestoreState {
    #[default]
    Idle,
    Loading {
        origin: TuiConversationRestoreOrigin,
        request_id: u64,
        future: Option<SpawnedFutureHandle>,
    },
    Failed(String),
}
fn export_file_success_message(export: &ConversationFileExport) -> String {
    let path = export.path().display();
    if export.overwrote_existing() {
        format!("Conversation exported to {path} (overwrote existing file)")
    } else {
        format!("Conversation exported to {path}")
    }
}

/// Typed actions handled by [`TuiTerminalSessionView`].
#[derive(Debug, Clone)]
pub(crate) enum TuiTerminalSessionAction {
    /// Ctrl-c anywhere in the session surface: cancel the running
    /// conversation, else clear the input; a second press within
    /// [`CTRL_C_EXIT_WINDOW`] exits the TUI.
    Interrupt,
    /// Cancel an in-flight conversation restore.
    CancelRestore,
    /// Return a user-controlled terminal-use command to the agent.
    HandBackTerminalUseControl,
    /// Click on the footer's usage entry: flips the persisted credits⇄cost
    /// display-mode setting.
    ToggleUsageDisplay,
    /// Toggle the completed-response summary for the selected conversation.
    ToggleResponseSummaryVisibility,
    /// Click on the footer's active-model label: toggles the inline model
    /// picker (the same menu `/model` surfaces).
    ToggleModelMenu,
    /// Toggle per-conversation auto approve.
    ToggleAutoApprove { show_feedback: bool },
    /// Raw user bytes to forward to the foreground PTY process.
    ForwardUserPtyBytes(Vec<u8>),
    /// Ctrl-d while the prompt is focused: exit the TUI immediately when the
    /// prompt is empty, else delete the next character.
    Eof,
    /// Toggle the latest exposed inline plan.
    TogglePlan,
    /// Return keyboard focus from tabs to the session's default interaction target.
    FocusDefaultInteractionTarget,
    /// Return to the main/root orchestration agent and focus its input.
    ///
    /// When a child tab is selected, switches the focused session to the
    /// root/main agent; when the root is already selected, only clears tab
    /// focus and restores input focus.
    FocusMainOrchestrationTab,
    /// Navigate the orchestration tabs using their semantic order.
    NavigateOrchestrationTabs(TuiOrchestrationTabNavigationAction),
    /// Move focus from the prompt input into the attachment bar.
    FocusAttachments,
    /// Paste host clipboard text or attach image data and image paths.
    PasteFromClipboard,
}

/// The authenticated terminal/session surface rendered inside [`RootTuiView`].
pub(crate) struct TuiTerminalSessionView {
    transcript: ViewHandle<TuiTranscriptView>,
    input_view: ViewHandle<TuiInputView>,
    attachment_bar: ViewHandle<TuiAttachmentBar>,
    inline_menus: Vec<TuiInlineMenu>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    conversation_menu: ModelHandle<TuiConversationMenuModel>,
    model_menu: ModelHandle<TuiModelMenuModel>,
    skills_menu: ModelHandle<TuiSkillMenuModel>,
    mcp_menu: ModelHandle<TuiMcpMenuModel>,
    slash_commands_source: ModelHandle<TuiSlashCommandDataSource>,
    conversation_selection: ConversationSelectionHandle,
    ai_action_model: ModelHandle<BlocklistAIActionModel>,
    ai_controller: ModelHandle<BlocklistAIController>,
    cli_subagent_controller: ModelHandle<CLISubagentController>,
    cli_subagent_views: HashMap<BlockId, ViewHandle<TuiCLISubagentView>>,
    /// Read by the footer for the active session's working directory.
    active_session: ModelHandle<ActiveSession>,
    /// Repository currently containing the active session's working directory.
    current_repo_path: Option<LocalOrRemotePath>,
    /// Watcher-backed branch and uncommitted diff metadata for the footer.
    git_repo_status: Option<ModelHandle<GitRepoStatusModel>>,
    /// This view's surface id, used to resolve the active model for the footer
    /// the same way the request path does.
    terminal_surface_id: EntityId,
    /// Armed by a ctrl-c press; a second press while armed exits the TUI.
    /// The footer shows [`CTRL_C_EXIT_HINT`] while armed.
    exit_confirmation: ExitConfirmation,
    /// Credits⇄cost display state for the footer's clickable usage entry.
    usage_toggle: UsageToggle,
    /// Last-response exchanges whose completed summary has been hidden with
    /// `/cost`. A later response has a new exchange ID and starts visible,
    /// matching the GUI's per-last-block state.
    hidden_response_summary_exchange_ids: HashSet<AIAgentExchangeId>,
    /// Hover state for the footer's clickable active-model label, owned here
    /// (not created inline during render) so it survives element-tree rebuilds
    /// — the same `MouseStateHandle` pattern as [`UsageToggle`].
    model_label_hover: MouseStateHandle,
    keyboard_enhancement_supported: bool,
    ai_context_model: ModelHandle<BlocklistAIContextModel>,
    ai_input_model: ModelHandle<BlocklistAIInputModel>,
    input_detection: InputDetectionState,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    /// Last dimensions applied to the terminal model and PTY.
    size_info: SizeInfo,
    /// Reports the area allocated to whichever element displays PTY content
    /// (the block-list content column or the full-screen alt-screen grid).
    /// This layout→channel→view pathway is the GUI's terminal-resize prior
    /// art (`TerminalSizeElement::after_layout` → `resize_tx` →
    /// `after_terminal_view_layout`): layout lacks a `ViewContext`, so the
    /// settled size is handed off to a view-side handler to apply.
    terminal_resize_tx: Sender<TuiSize>,
    /// Transient notice shown in the footer's hint slot (e.g. a rejected
    /// shell submission).
    transient_hint: TransientHint,
    auto_approve_feedback_conversation_id: Option<AIConversationId>,
    auto_approve_feedback_timer: Option<SpawnedFutureHandle>,
    auto_approve_mouse: MouseStateHandle,
    conversation_restore_state: ConversationRestoreState,
    next_restore_request_id: u64,
    exit_summary: TuiExitSummaryHandle,
    /// The view id of the blocker currently holding focus, tracked only to
    /// detect blocker transitions in [`Self::sync_blocker_focus`]. Input
    /// visibility itself is derived at render time, never stored.
    active_blocker_view_id: Option<EntityId>,
    orchestration_tab_bar: ViewHandle<TuiTabBarView>,
    orchestration_tabs_focused: bool,
    zero_state_view: ViewHandle<TuiZeroStateView>,
}

/// Registers the session surface's keybindings. Called once at TUI startup
/// from `keybindings::init`. Ctrl-c is a fixed (non-remappable) binding,
/// mirroring peer agent CLIs that treat it as reserved.
pub(crate) fn init(app: &mut AppContext) {
    let view_context = id!(TuiTerminalSessionView::ui_name());
    register_orchestration_surface_bindings(
        app,
        view_context.clone(),
        TuiTerminalSessionAction::Interrupt,
        TuiTerminalSessionAction::NavigateOrchestrationTabs,
    );
    app.register_fixed_bindings([
        FixedBinding::new(
            "ctrl-d",
            TuiTerminalSessionAction::Eof,
            id!(TuiInputView::ui_name()),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "escape",
            TuiTerminalSessionAction::CancelRestore,
            id!(SESSION_CAN_CANCEL_RESTORE_FLAG),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            HAND_BACK_KEY_BINDING,
            TuiTerminalSessionAction::HandBackTerminalUseControl,
            id!(SESSION_CAN_HAND_BACK_CONTROL_FLAG),
        )
        .with_group(TUI_BINDING_GROUP),
    ]);
    app.register_editable_bindings([
        EditableBinding::new(
            AUTO_APPROVE_TOGGLE_BINDING_NAME,
            "Toggle auto approve",
            TuiTerminalSessionAction::ToggleAutoApprove {
                show_feedback: true,
            },
        )
        .with_context_predicate(view_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-shift-I"),
        EditableBinding::new(
            PLAN_TOGGLE_BINDING_NAME,
            "Toggle the latest plan",
            TuiTerminalSessionAction::TogglePlan,
        )
        .with_context_predicate(view_context)
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-shift-P"),
        EditableBinding::new(
            CONTEXTUAL_PLAN_TOGGLE_BINDING_NAME,
            "Toggle the latest visible plan",
            TuiTerminalSessionAction::TogglePlan,
        )
        .with_context_predicate(
            (id!(TuiInputView::ui_name()) | id!(TuiTerminalSessionView::ui_name()))
                & id!(PLAN_TOGGLE_AVAILABLE_FLAG)
                & !id!(KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG),
        )
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-p"),
        EditableBinding::new(
            FOCUS_ATTACHMENTS_BINDING_NAME,
            "Focus image attachments",
            TuiTerminalSessionAction::FocusAttachments,
        )
        .with_context_predicate(
            (id!(TuiInputView::ui_name()) | id!(TuiTerminalSessionView::ui_name()))
                & id!(ATTACHMENTS_AVAILABLE_FLAG),
        )
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("tab"),
        EditableBinding::new(
            PASTE_IMAGE_BINDING_NAME,
            "Paste from the clipboard",
            TuiTerminalSessionAction::PasteFromClipboard,
        )
        .with_context_predicate(
            (id!(TuiInputView::ui_name()) | id!(TuiTerminalSessionView::ui_name()))
                & id!(SESSION_COMPOSER_OWNS_INPUT_FLAG),
        )
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-v"),
        EditableBinding::new(
            PASTE_IMAGE_BINDING_NAME,
            "Paste from the clipboard",
            TuiTerminalSessionAction::PasteFromClipboard,
        )
        .with_context_predicate(
            (id!(TuiInputView::ui_name()) | id!(TuiTerminalSessionView::ui_name()))
                & id!(SESSION_COMPOSER_OWNS_INPUT_FLAG),
        )
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-shift-V"),
        #[cfg(windows)]
        EditableBinding::new(
            PASTE_IMAGE_BINDING_NAME,
            "Paste from the clipboard",
            TuiTerminalSessionAction::PasteFromClipboard,
        )
        .with_context_predicate(
            (id!(TuiInputView::ui_name()) | id!(TuiTerminalSessionView::ui_name()))
                & id!(SESSION_COMPOSER_OWNS_INPUT_FLAG),
        )
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("alt-v"),
    ]);

    let tab_context =
        id!(TuiTerminalSessionView::ui_name()) & id!(ORCHESTRATION_TAB_BAR_FOCUSED_FLAG);
    app.register_editable_bindings([
        EditableBinding::new(
            "tui:orchestration_tabs:focus_input",
            "Return focus to the session input",
            TuiTerminalSessionAction::FocusDefaultInteractionTarget,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("down"),
        EditableBinding::new(
            "tui:orchestration_tabs:focus_input",
            "Return focus to the session input",
            TuiTerminalSessionAction::FocusDefaultInteractionTarget,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-down"),
        EditableBinding::new(
            "tui:orchestration_tabs:focus_main",
            "Return to the main agent and focus its input",
            TuiTerminalSessionAction::FocusMainOrchestrationTab,
        )
        .with_context_predicate(tab_context)
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("escape"),
    ]);
}

impl TuiTerminalSessionView {
    /// Selects the sole input destination for the current terminal lifecycle
    /// state. The result drives focus, rendering, and event routing together.
    fn input_target(&self) -> TuiInputTarget {
        let terminal_model = self.terminal_model.lock();
        tui_input_target(&terminal_model)
    }

    fn update_process_input_focus(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus_current_owner_if_active(ctx);
    }

    fn focus_blocking_child(blocker: TuiBlockingChild, ctx: &mut ViewContext<Self>) {
        match blocker {
            TuiBlockingChild::AskQuestion(view) => {
                view.update(ctx, |view, ctx| view.focus(ctx));
            }
            TuiBlockingChild::Permission(view) => {
                view.update(ctx, |view, ctx| view.focus(ctx));
            }
            TuiBlockingChild::Orchestration(view) => ctx.focus(&view),
        }
    }

    fn focus_current_owner(&mut self, ctx: &mut ViewContext<Self>) {
        match self.input_target() {
            TuiInputTarget::Disabled => {
                if let Some(blocker) = self.active_blocking_child(ctx) {
                    self.orchestration_tabs_focused = false;
                    Self::focus_blocking_child(blocker, ctx);
                } else if self.orchestration_tabs_focused {
                    ctx.focus_self();
                } else {
                    ctx.focus(&self.input_view);
                }
            }
            TuiInputTarget::Pty => {
                self.orchestration_tabs_focused = false;
                ctx.focus_self();
            }
            TuiInputTarget::AgentEditor => {
                if let Some(blocker) = self.active_blocking_child(ctx) {
                    self.orchestration_tabs_focused = false;
                    Self::focus_blocking_child(blocker, ctx);
                } else if self.orchestration_tabs_focused {
                    ctx.focus_self();
                } else {
                    ctx.focus(&self.input_view);
                }
            }
        }
    }

    fn focus_current_owner_if_active(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_focused_session(ctx) {
            let tabs_were_focused = self.orchestration_tabs_focused;
            self.focus_current_owner(ctx);
            if tabs_were_focused && !self.orchestration_tabs_focused {
                self.refresh_orchestration_tab_bar(ctx);
                ctx.notify();
            }
        }
    }

    fn focus_input_if_active(&self, ctx: &mut ViewContext<Self>) {
        if self.is_focused_session(ctx) {
            ctx.focus(&self.input_view);
        }
    }

    fn resume_after_user_controlled_command(
        &mut self,
        block_id: &BlockId,
        ctx: &mut ViewContext<Self>,
    ) {
        let conversation_id = {
            let terminal_model = self.terminal_model.lock();
            terminal_use_conversation_to_resume(&terminal_model, block_id)
        };
        let Some(conversation_id) = conversation_id else {
            return;
        };
        let resume_context = {
            let terminal_model = self.terminal_model.lock();
            block_context_from_terminal_model(&terminal_model, block_id, false)
                .map(Box::new)
                .map(AIAgentContext::Block)
                .into_iter()
                .collect()
        };
        self.ai_controller.update(ctx, |controller, ctx| {
            controller.resume_conversation(
                conversation_id,
                /*can_attempt_resume_on_error*/ true,
                /*is_auto_resume_after_error*/ false,
                resume_context,
                ctx,
            );
        });
    }

    fn detach_cli_subagent_view(
        &mut self,
        block_id: &BlockId,
        initial_requested_command_action_id: Option<&AIAgentActionId>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(view) = self.cli_subagent_views.remove(block_id) {
            self.transcript.update(ctx, |transcript, ctx| {
                transcript.detach_cli_subagent(initial_requested_command_action_id, view.id(), ctx);
            });
        }
        self.focus_input_if_active(ctx);
    }
    fn handle_cli_subagent_event(&mut self, event: &CLISubagentEvent, ctx: &mut ViewContext<Self>) {
        match event {
            CLISubagentEvent::SpawnedSubagent {
                block_id,
                initial_requested_command_action_id,
                ..
            } => {
                hide_agent_requested_command_from_top_level(
                    &self.terminal_model,
                    initial_requested_command_action_id.as_ref(),
                );
                self.input_view
                    .update(ctx, |input, ctx| input.exit_shell_mode(ctx));
                if let Some(target) = self
                    .cli_subagent_controller
                    .as_ref(ctx)
                    .target_for_block(block_id)
                {
                    let controller = self.cli_subagent_controller.clone();
                    let action_model = self.ai_action_model.clone();
                    let terminal_model = self.terminal_model.clone();
                    let view = ctx.add_typed_action_tui_view(|ctx| {
                        TuiCLISubagentView::new(
                            target,
                            controller,
                            action_model,
                            terminal_model,
                            ctx,
                        )
                    });
                    self.transcript.update(ctx, |transcript, ctx| {
                        transcript.attach_cli_subagent(
                            initial_requested_command_action_id.as_ref(),
                            view.clone(),
                            ctx,
                        );
                    });
                    self.cli_subagent_views.insert(block_id.clone(), view);
                }
            }
            CLISubagentEvent::FinishedSubagent {
                block_id,
                initial_requested_command_action_id,
                ..
            } => {
                self.detach_cli_subagent_view(
                    block_id,
                    initial_requested_command_action_id.as_ref(),
                    ctx,
                );
            }
            CLISubagentEvent::UpdatedControl { .. }
            | CLISubagentEvent::UpdatedInstruction { .. }
            | CLISubagentEvent::UpdatedLastSnapshot
            | CLISubagentEvent::ToggledHideResponses => {}
            CLISubagentEvent::ControlHandedBackAfterTransfer => {
                let executor = self.ai_action_model.as_ref(ctx).shell_command_executor(ctx);
                executor.update(ctx, |executor, _| {
                    executor.notify_control_handed_back();
                });
            }
        }
        self.update_process_input_focus(ctx);
        ctx.notify();
    }

    fn handle_terminal_use_interrupt(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let control_state = self
            .cli_subagent_controller
            .as_ref(ctx)
            .active_target()
            .map(|target| target.control_state);
        let Some(action) = terminal_use_interrupt_action(
            control_state.as_ref(),
            self.input_target().pty_owns_input(),
        ) else {
            return false;
        };
        match action {
            TerminalUseInterruptAction::TakeControl => {
                self.cli_subagent_controller.update(ctx, |controller, ctx| {
                    controller.switch_control_to_user(
                        UserTakeOverReason::Stop {
                            should_auto_resume: true,
                        },
                        ctx,
                    );
                });
                self.update_process_input_focus(ctx);
                true
            }
            TerminalUseInterruptAction::InterruptCommand => {
                ctx.emit(TuiTerminalSessionEvent::InterruptPty);
                true
            }
        }
    }

    fn hand_back_terminal_use_control(&mut self, ctx: &mut ViewContext<Self>) {
        if self.active_user_controlled_target(ctx).is_none() {
            return;
        }
        self.cli_subagent_controller.update(ctx, |controller, ctx| {
            controller.handoff_active_command_control_to_agent(ctx);
        });
        self.update_process_input_focus(ctx);
    }

    fn active_agent_controlled_target(&self, ctx: &AppContext) -> Option<CLISubagentTarget> {
        self.cli_subagent_controller
            .as_ref(ctx)
            .active_target()
            .filter(|target| target.control_state.is_agent_in_control())
    }

    fn active_user_controlled_target(&self, ctx: &AppContext) -> Option<CLISubagentTarget> {
        self.cli_subagent_controller
            .as_ref(ctx)
            .active_target()
            .filter(|target| target.control_state.is_user_in_control())
    }

    fn send_terminal_use_prompt(&mut self, input: &str, ctx: &mut ViewContext<Self>) -> bool {
        let Some(prompt) = raw_prompt_if_not_blank(input) else {
            return false;
        };
        let Some(target) = self.active_agent_controlled_target(ctx) else {
            return false;
        };
        let prompt = prompt.to_owned();
        let block_id = target.block_id;
        let conversation_id = target.conversation_id;
        let previous_instruction = self.cli_subagent_controller.update(ctx, |controller, ctx| {
            controller.set_latest_instruction(block_id.clone(), prompt.clone(), ctx)
        });
        self.input_view.update(ctx, |input, ctx| input.clear(ctx));
        ctx.notify();

        let dispatched = self.ai_controller.update(ctx, |controller, ctx| {
            controller.send_user_query_in_conversation(prompt.clone(), conversation_id, None, ctx)
        });
        if !dispatched {
            self.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.restore_latest_instruction(block_id, previous_instruction, ctx);
            });
            if self.input_view.as_ref(ctx).is_empty(ctx) {
                self.input_view.update(ctx, |input, ctx| {
                    input.set_text(&prompt, ctx);
                });
            }
        }
        true
    }

    /// Builds the transcript-capable terminal surface for a manager-backed session.
    pub(crate) fn new(
        surface_init: TerminalSurfaceInit,
        exit_summary: TuiExitSummaryHandle,
        keyboard_enhancement_supported: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let TerminalSurfaceInit {
            model,
            sessions,
            model_events,
            wakeups_rx,
            size_info,
            ..
        } = surface_init;
        let (terminal_resize_tx, terminal_resize_rx) = async_channel::unbounded();
        model
            .lock()
            .block_list_mut()
            .set_transcript_scope(TranscriptScope::Unfiltered);

        let terminal_surface_id: EntityId = ctx.view_id();
        let active_session =
            ctx.add_model(|ctx| ActiveSession::new(sessions.clone(), model_events.clone(), ctx));
        let model_for_conversation_selection = model.clone();
        let conversation_selection = ctx.add_model(|ctx| {
            Box::new(TuiConversationSelection::new(
                terminal_surface_id,
                model_for_conversation_selection,
                ctx,
            )) as Box<dyn ConversationSelection>
        });
        let context_model = ctx.add_model(|ctx| {
            BlocklistAIContextModel::new(
                sessions,
                &model_events,
                model.clone(),
                terminal_surface_id,
                conversation_selection.clone(),
                ctx,
            )
        });
        let ai_input_model = ctx.add_model(|ctx| {
            BlocklistAIInputModel::new(
                model.clone(),
                conversation_selection.clone(),
                context_model.clone(),
                Rc::new(TuiInputModePolicy),
                terminal_surface_id,
                ctx,
            )
        });
        let get_relevant_files_controller = ctx.add_model(GetRelevantFilesController::new);
        let action_model = ctx.add_model(|ctx| {
            BlocklistAIActionModel::new(
                model.clone(),
                active_session.clone(),
                &model_events,
                get_relevant_files_controller,
                terminal_surface_id,
                ctx,
            )
        });
        let start_agent_executor = action_model.as_ref(ctx).start_agent_executor(ctx);
        ctx.subscribe_to_model(&start_agent_executor, |view, _, event, ctx| match event {
            StartAgentExecutorEvent::CreateAgent(request) => {
                ctx.emit(TuiTerminalSessionEvent::StartAgentConversation {
                    request: request.clone(),
                    working_directory: view.current_working_directory(ctx).map(PathBuf::from),
                });
            }
            StartAgentExecutorEvent::CleanupFailedChildLaunch { conversation_id } => {
                ctx.emit(TuiTerminalSessionEvent::CleanupFailedChildLaunch {
                    conversation_id: *conversation_id,
                });
            }
        });
        let ai_controller = ctx.add_model(|ctx| {
            BlocklistAIController::new(
                ai_input_model.clone(),
                context_model.clone(),
                conversation_selection.clone(),
                action_model.clone(),
                active_session.clone(),
                model.clone(),
                terminal_surface_id,
                ctx,
            )
        });
        let cli_subagent_controller = ctx.add_model(|ctx| {
            CLISubagentController::new(
                &ai_controller,
                &action_model,
                None,
                model.clone(),
                &model_events,
                terminal_surface_id,
                ctx,
            )
        });
        ctx.subscribe_to_model(&cli_subagent_controller, |view, _, event, ctx| {
            view.handle_cli_subagent_event(event, ctx);
        });
        let transcript = ctx.add_typed_action_tui_view(|ctx| {
            TuiTranscriptView::new(
                terminal_surface_id,
                model.clone(),
                action_model.clone(),
                &model_events,
                ctx,
            )
        });
        // Input visibility and focus derive from the front-of-queue blocker;
        // re-derive on every action-queue transition (queued, blocked,
        // finished). No suppression flag is stored.
        ctx.subscribe_to_model(&action_model, |view, _, _, ctx| {
            view.sync_blocker_focus(ctx);
        });
        let input_editor_model =
            ctx.add_model(|ctx| CodeEditorModel::new_tui(INITIAL_INPUT_WIDTH, ctx));
        let suggestions_mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
        let slash_commands_source = ctx.add_model(|ctx| {
            TuiSlashCommandDataSource::new(
                TuiSlashCommandDataSourceArgs {
                    active_session: active_session.clone(),
                    cli_subagent_controller: cli_subagent_controller.clone(),
                    terminal_view_id: terminal_surface_id,
                    terminal_model: model.clone(),
                },
                ctx,
            )
        });
        let zero_state_source = TuiZeroStateDataSource::new(&slash_commands_source);
        let slash_commands_mixer = ctx.add_model(|ctx| {
            build_slash_command_mixer(slash_commands_source.clone(), zero_state_source, ctx)
        });
        let slash_commands = ctx.add_model(|ctx| {
            TuiSlashCommandModel::new(
                input_editor_model.clone(),
                suggestions_mode.clone(),
                slash_commands_source.clone(),
                slash_commands_mixer,
                conversation_selection.clone(),
                ctx,
            )
        });
        ctx.subscribe_to_model(&slash_commands, |_, _, _, ctx| ctx.notify());
        let window_id = ctx.window_id();
        let conversation_menu = ctx.add_model(|ctx| {
            TuiConversationMenuModel::new(
                input_editor_model.clone(),
                suggestions_mode.clone(),
                conversation_selection.clone(),
                window_id,
                ctx,
            )
        });
        ctx.subscribe_to_model(&conversation_menu, |view, _, event, ctx| match event {
            TuiConversationMenuEvent::Updated => ctx.notify(),
            TuiConversationMenuEvent::CloudMetadataUnavailable => {
                view.show_transient_hint(
                    "Could not load cloud conversations. Showing local conversations only."
                        .to_owned(),
                    ctx,
                );
            }
        });
        let model_menu = ctx.add_model(|ctx| {
            TuiModelMenuModel::new(input_editor_model.clone(), suggestions_mode.clone(), ctx)
        });
        ctx.subscribe_to_model(&model_menu, |_, _, _: &TuiModelMenuEvent, ctx| {
            ctx.notify();
        });
        let skills_menu = ctx.add_model(|ctx| {
            TuiSkillMenuModel::new(
                input_editor_model.clone(),
                suggestions_mode.clone(),
                active_session.clone(),
                slash_commands_source.clone(),
                terminal_surface_id,
                ctx,
            )
        });
        ctx.subscribe_to_model(&skills_menu, |_, _, _: &TuiSkillMenuEvent, ctx| {
            ctx.notify();
        });
        let mcp_menu = ctx.add_model(|ctx| TuiMcpMenuModel::new(suggestions_mode.clone(), ctx));
        ctx.subscribe_to_model(&mcp_menu, |_, _, event, ctx| {
            let TuiMcpMenuEvent::Updated = event;
            ctx.notify();
        });
        let prompt_history_menu = ctx.add_model(|ctx| {
            TuiPromptHistoryMenuModel::new(
                input_editor_model.clone(),
                suggestions_mode.clone(),
                terminal_surface_id,
                ctx,
            )
        });
        ctx.subscribe_to_model(&prompt_history_menu, |_, _, event, ctx| {
            let TuiPromptHistoryMenuEvent::Updated = event;
            ctx.notify();
        });
        // The footer's conversations callout depends on whether the input is
        // empty, so content changes must invalidate this parent view as well as
        // the input child. Typing after ctrl-c also disarms the pending exit
        // confirmation; the ctrl-c buffer clear leaves the buffer empty, so the
        // window it arms survives its own clear.
        let editor_for_footer = input_editor_model.clone();
        ctx.subscribe_to_model(&input_editor_model, move |view, _, event, ctx| {
            let CodeEditorModelEvent::ContentChanged { origin } = event else {
                return;
            };
            let is_empty = editor_for_footer
                .as_ref(ctx)
                .content()
                .as_ref(ctx)
                .is_empty();
            if !is_empty {
                view.exit_confirmation.disarm();
            }
            view.handle_input_content_changed(origin.from_user(), ctx);
            ctx.notify();
        });

        let editor_for_selection = input_editor_model.clone();
        let transcript_for_selection = transcript.clone();
        ctx.subscribe_to_model(&input_editor_model, move |_, _, event, ctx| {
            if !matches!(event, CodeEditorModelEvent::SelectionChanged) {
                return;
            }

            let has_selection = !editor_for_selection
                .as_ref(ctx)
                .buffer_selection_model()
                .as_ref(ctx)
                .first_selection_is_single_cursor();
            if has_selection {
                transcript_for_selection.update(ctx, |transcript, ctx| {
                    transcript.clear_selection(ctx);
                });
            }
        });

        let input_mode_for_input_view = ai_input_model.clone();
        let inline_menus = vec![
            TuiInlineMenu::new(slash_commands.clone()),
            TuiInlineMenu::new(conversation_menu.clone()),
            TuiInlineMenu::new(model_menu.clone()),
            TuiInlineMenu::new(skills_menu.clone()),
            TuiInlineMenu::new(mcp_menu.clone()),
            TuiInlineMenu::new(prompt_history_menu.clone()),
        ];
        let inline_menus_for_input = inline_menus.clone();
        let suggestions_mode_for_input = suggestions_mode.clone();
        let transcript_for_input = transcript.clone();
        let terminal_model_for_input = model.clone();
        let orchestration_tab_bar = ctx.add_typed_action_tui_view(|_| TuiTabBarView::empty());
        let orchestration_tab_bar_for_input = orchestration_tab_bar.clone();
        let input_editor_for_input = input_editor_model.clone();
        let input_view = ctx.add_typed_action_tui_view(move |ctx| {
            TuiInputView::new(
                input_editor_for_input,
                input_mode_for_input_view,
                suggestions_mode_for_input,
                inline_menus_for_input,
                transcript_for_input,
                move |ctx| orchestration_tab_bar_for_input.as_ref(ctx).has_tabs(),
                ctx,
            )
            .with_inline_menu_actions_allowed(move |_| {
                let terminal_model = terminal_model_for_input.lock();
                tui_input_target(&terminal_model).agent_editor_owns_input()
            })
            .with_keyboard_enhancement_supported(keyboard_enhancement_supported)
        });
        let attachment_model = ctx.add_model(|ctx| {
            TuiAttachmentModel::new(
                context_model.clone(),
                ai_input_model.clone(),
                input_editor_model,
                active_session.clone(),
                terminal_surface_id,
                ctx,
            )
        });
        let attachment_bar =
            ctx.add_typed_action_tui_view(|ctx| TuiAttachmentBar::new(attachment_model, ctx));
        ctx.subscribe_to_view(&attachment_bar, |view, _, event, ctx| {
            view.handle_attachment_bar_event(event, ctx);
        });

        ctx.subscribe_to_view(&transcript, |view, _, event, ctx| match event {
            TuiTranscriptViewEvent::SelectionStarted => {
                view.input_view
                    .update(ctx, |input, ctx| input.clear_selection(ctx));
            }
            TuiTranscriptViewEvent::SelectionEnded(text) => match copy_to_clipboard(text) {
                Ok(()) => view.show_copy_hint(ctx),
                Err(error) => {
                    log::warn!("Failed to copy TUI selection: {error}");
                    view.show_transient_hint(COPY_FAILED_HINT.to_owned(), ctx);
                }
            },
            TuiTranscriptViewEvent::BlockingStateChanged => {
                view.sync_blocker_focus(ctx);
            }
            TuiTranscriptViewEvent::PermissionReplacementGuidanceSubmitted {
                conversation_id,
                text,
            } => {
                view.ai_controller.update(ctx, |controller, ctx| {
                    controller.send_user_query_in_conversation(
                        text.clone(),
                        *conversation_id,
                        None,
                        ctx,
                    );
                });
            }
        });

        ctx.subscribe_to_view(&input_view, |view, _, event, ctx| match event {
            TuiInputViewEvent::Submitted(text) => view.handle_submitted(text.clone(), ctx),
            TuiInputViewEvent::Pasted(text) => view.handle_pasted(text.clone(), ctx),
            TuiInputViewEvent::BackspaceAtEmptyInput => {
                view.attachment_bar
                    .update(ctx, |bar, ctx| bar.remove_selected(ctx));
            }
            TuiInputViewEvent::AcceptedSlashCommand(action) => {
                view.handle_accepted_slash_command(action, ctx);
            }
            TuiInputViewEvent::AcceptedConversation(entry_id) => {
                view.handle_accepted_conversation(*entry_id, ctx);
            }
            TuiInputViewEvent::AcceptedModel(id) => {
                view.handle_accepted_model(id, ctx);
            }
            TuiInputViewEvent::AcceptedMcp(action) => {
                view.handle_accepted_mcp_action(*action, ctx);
            }
            TuiInputViewEvent::AcceptedPromptHistory(text) => {
                view.handle_accepted_prompt_history(text.clone(), ctx);
            }
            TuiInputViewEvent::ClipboardCopySucceeded => view.show_copy_hint(ctx),
            TuiInputViewEvent::ClipboardCopyFailed => {
                view.show_transient_hint(COPY_FAILED_HINT.to_owned(), ctx);
            }
            TuiInputViewEvent::MoveFocusUp => {
                view.focus_orchestration_tabs(ctx);
            }
        });
        ctx.subscribe_to_model(&action_model, |view, action_model, event, ctx| {
            let BlocklistAIActionEvent::FinishedAction { action_id, .. } = event else {
                return;
            };
            let finished_asking_question = action_model
                .as_ref(ctx)
                .get_action_result(action_id)
                .is_some_and(|result| {
                    matches!(&result.result, AIAgentActionResultType::AskUserQuestion(_))
                });
            if finished_asking_question {
                ctx.focus(&view.input_view);
            }
        });
        ctx.subscribe_to_view(&orchestration_tab_bar, |view, _, event, ctx| match event {
            TuiTabBarEvent::SelectTab(conversation_id) => {
                view.switch_to_orchestration_tab(
                    Some(conversation_id.clone()),
                    view.orchestration_tabs_focused,
                    ctx,
                );
            }
            TuiTabBarEvent::PageChanged(page_anchor) => {
                let Ok(page_anchor) = AIConversationId::try_from(page_anchor.clone()) else {
                    return;
                };
                TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                    model.set_explicit_page(page_anchor, ctx);
                });
            }
        });
        // The input box border color and the footer's shell-mode hint depend
        // on the input mode.
        ctx.subscribe_to_model(&ai_input_model, |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&suggestions_mode, |_, _, _, ctx| ctx.notify());
        // The warping indicator between the transcript and the input box
        // tracks the selected conversation: re-render when its status changes
        // or an exchange starts (the elapsed counter's anchor) on this
        // surface, and when the selected conversation changes.
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |view, _, event, ctx| view.handle_history_event(event, ctx),
        );
        ctx.subscribe_to_model(&conversation_selection, |view, _, _, ctx| {
            view.refresh_exit_summary(ctx);
            if view.is_focused_session(ctx) {
                view.refresh_orchestration_tab_state(ctx);
            }
        });

        // Trigger the changelog fetch once at startup so `TuiZeroStateView`
        // has data to display.  The re-render subscription lives in the view.
        ChangelogModel::handle(ctx).update(ctx, |changelog, ctx| {
            changelog.check_for_changelog(ChangelogRequestType::WindowLaunch, ctx);
        });

        // Bridge shared shell-tool executor events into terminal-manager PTY intents.
        let shell_command_executor = action_model.as_ref(ctx).shell_command_executor(ctx);
        let model_for_shell_events = model.clone();
        ctx.subscribe_to_model(&shell_command_executor, move |view, _, event, ctx| {
            view.handle_shell_command_executor_event(event, &model_for_shell_events, ctx);
        });

        // These events update block metadata or grids the transcript reads.
        // PTY output redraws are driven by `wakeups_rx` below.
        ctx.subscribe_to_model(&model_events, |view, _, event, ctx| match event {
            ModelEvent::BlockCompleted(completed) => {
                view.resume_after_user_controlled_command(&completed.block_id, ctx);
                view.update_process_input_focus(ctx);
                ctx.notify();
            }
            ModelEvent::AfterBlockStarted { .. } => {
                view.update_process_input_focus(ctx);
                ctx.notify();
            }
            ModelEvent::VisibleBootstrapBlock | ModelEvent::BootstrapPrecmdDone => {
                view.update_process_input_focus(ctx);
                ctx.notify();
            }
            ModelEvent::Typeahead => view.handle_typeahead_event(ctx),
            ModelEvent::BlockMetadataReceived(_)
            | ModelEvent::BlockWorkingDirectoryUpdated(_)
            | ModelEvent::BackgroundBlockStarted
            | ModelEvent::TerminalClear
            | ModelEvent::PromptUpdated
            | ModelEvent::Handler(_)
            | ModelEvent::FinishUpdate(_) => ctx.notify(),
            _ => {}
        });
        // The footer shows the active model, working directory, and usage
        // entry: re-render when the usage-display-mode setting changes (click
        // or settings-file hot reload), when the active model or its display
        // name changes, or when the session's working directory changes.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |view, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::TuiUsageDisplayMode { .. }) {
                ctx.notify();
            }
            if matches!(event, AISettingsChangedEvent::AIAutoDetectionEnabled { .. }) {
                view.schedule_input_detection(ctx);
            }
        });
        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |_, _, event, ctx| {
            if matches!(
                event,
                LLMPreferencesEvent::UpdatedAvailableLLMs
                    | LLMPreferencesEvent::UpdatedActiveAgentModeLLM
            ) {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&active_session, |view, _, event, ctx| match event {
            ActiveSessionEvent::UpdatedPwd => {
                // Run repo detection so project rules and skills follow the
                // session's working directory (the GUI's equivalent lives in
                // `TerminalView::apply_block_metadata_update`). The first
                // post-bootstrap precmd metadata transitions the cwd from
                // `None` to `Some`, so this also covers the launch directory.
                let Some(cwd) = view
                    .active_session
                    .as_ref(ctx)
                    .current_working_directory()
                    .cloned()
                else {
                    view.slash_commands_source.update(ctx, |source, ctx| {
                        source.set_active_repo_root(None, ctx);
                    });
                    view.update_git_status_subscription(None, ctx);
                    ctx.notify();
                    return;
                };
                let detection = detect_possible_git_repo(
                    RepoDetectionSessionType::Local,
                    &cwd,
                    RepoDetectionSource::TerminalNavigation,
                    ctx,
                );
                ctx.spawn(detection, move |view, repo_path, ctx| {
                    if view.active_session.as_ref(ctx).current_working_directory() != Some(&cwd) {
                        return;
                    }
                    view.update_git_status_subscription(repo_path.clone(), ctx);
                    let repo_root = repo_path
                        .as_ref()
                        .and_then(|path| path.to_local_path())
                        .map(ToOwned::to_owned);
                    view.slash_commands_source.update(ctx, |source, ctx| {
                        source.set_active_repo_root(repo_root, ctx);
                    });
                });
                ctx.notify();
            }
            ActiveSessionEvent::Bootstrapped => {}
        });
        // The footer's usage entry shows the selected conversation's token/cost
        // totals: re-render when that conversation's usage metadata updates.
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |view, _, event, ctx| {
                if let BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated {
                    conversation_id,
                } = event
                {
                    let selected = view
                        .conversation_selection
                        .as_ref(ctx)
                        .selected_conversation_id(ctx);
                    if selected == Some(*conversation_id) {
                        ctx.notify();
                    }
                }
            },
        );

        // A wakeup is also how a running block becomes visible: its height is 0
        // until the long-running render-delay timer fires and sends a wakeup
        // (see `Block::wakeup_after_delay`). Heights are otherwise only
        // recomputed when PTY bytes arrive, so a silent command (e.g. `sleep`)
        // would stay invisible until it finishes. Mirror the GUI's
        // `handle_terminal_wakeup` by throttling the stream and refreshing
        // live block heights here.
        ctx.spawn_stream_local(
            throttle(WAKEUP_THROTTLE_PERIOD, wakeups_rx),
            |view, _, ctx| {
                view.handle_terminal_wakeup(ctx);
            },
            |_, _| {},
        );
        ctx.spawn_stream_local(terminal_resize_rx, Self::handle_terminal_resize, |_, _| {});
        let zero_state_view =
            ctx.add_tui_view(|ctx| TuiZeroStateView::new(active_session.clone(), ctx));
        Self {
            transcript,
            input_view,
            attachment_bar,
            inline_menus,
            suggestions_mode,
            conversation_menu,
            model_menu,
            skills_menu,
            mcp_menu,
            slash_commands_source,
            conversation_selection,
            ai_action_model: action_model,
            ai_controller,
            cli_subagent_controller,
            cli_subagent_views: HashMap::new(),
            active_session,
            current_repo_path: None,
            git_repo_status: None,
            terminal_surface_id,
            exit_confirmation: ExitConfirmation::default(),
            usage_toggle: UsageToggle::default(),
            hidden_response_summary_exchange_ids: HashSet::new(),
            model_label_hover: MouseStateHandle::default(),
            keyboard_enhancement_supported,
            ai_context_model: context_model,
            ai_input_model,
            input_detection: InputDetectionState::default(),
            terminal_model: model,
            size_info,
            terminal_resize_tx,
            transient_hint: TransientHint::default(),
            auto_approve_feedback_conversation_id: None,
            auto_approve_feedback_timer: None,
            auto_approve_mouse: MouseStateHandle::default(),
            conversation_restore_state: ConversationRestoreState::Idle,
            next_restore_request_id: 0,
            exit_summary,
            active_blocker_view_id: None,
            orchestration_tab_bar,
            orchestration_tabs_focused: false,
            zero_state_view,
        }
    }

    /// Starts the first request for a child conversation hosted by this
    /// background session.
    pub(crate) fn start_orchestrated_child(
        &mut self,
        task_id: warp::tui_export::AmbientAgentTaskId,
        prompt: String,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.ai_controller.update(ctx, |controller, ctx| {
            controller.set_ambient_agent_task_id(Some(task_id), ctx);
            controller.send_agent_query_in_conversation(prompt, conversation_id, ctx);
        });
    }

    /// Initializes a background child session with the conversation it owns.
    pub(crate) fn initialize_orchestrated_child_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.conversation_selection.update(ctx, |selection, ctx| {
            selection.select_existing_conversation(conversation_id, AgentViewEntryOrigin::Tui, ctx);
        });
    }

    /// Resolves live semantic orchestration state for this session.
    fn compute_orchestration_tab_snapshot(
        &self,
        ctx: &AppContext,
    ) -> Option<TuiOrchestrationSnapshot> {
        if !ctx.has_singleton_model::<TuiOrchestrationModel>()
            || !ctx.has_singleton_model::<TuiSessions>()
        {
            return None;
        }
        let selected_conversation_id = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx)?;
        TuiOrchestrationModel::as_ref(ctx).snapshot(selected_conversation_id, ctx)
    }
    /// Refreshes this session's retained bar from live semantic state.
    pub(crate) fn refresh_orchestration_tab_state(&mut self, ctx: &mut ViewContext<Self>) {
        let snapshot = self.compute_orchestration_tab_snapshot(ctx);
        let tabs_were_available = self.orchestration_tab_bar.as_ref(ctx).has_tabs();
        if let Some(snapshot) = snapshot.as_ref() {
            let builder = TuiUiBuilder::from_app(ctx);
            self.sync_orchestration_tab_bar(snapshot, &builder, ctx);
        } else {
            self.clear_orchestration_tab_bar(ctx);
        }
        let tabs_are_available = self.orchestration_tab_bar.as_ref(ctx).has_tabs();
        let availability_changed = tabs_were_available != tabs_are_available;
        let mut focus_changed = false;
        if !tabs_are_available && self.orchestration_tabs_focused {
            self.orchestration_tabs_focused = false;
            focus_changed = true;
            self.focus_current_owner(ctx);
        }
        if availability_changed || focus_changed {
            ctx.notify();
        }
    }

    /// Gives keyboard focus to the orchestration tab bar when it is available.
    fn focus_orchestration_tabs(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.orchestration_tab_bar.as_ref(ctx).has_tabs() {
            return;
        }
        self.set_orchestration_tab_focus(true, ctx);
    }

    /// Applies tab-focus mode, synchronizes presentation, and resolves the focus owner.
    pub(crate) fn set_orchestration_tab_focus(
        &mut self,
        focused: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.orchestration_tabs_focused = focused;
        self.focus_current_owner(ctx);
        self.refresh_orchestration_tab_bar(ctx);
        ctx.notify();
    }

    fn refresh_orchestration_tab_bar(&self, ctx: &mut ViewContext<Self>) {
        if let Some(snapshot) = self.compute_orchestration_tab_snapshot(ctx) {
            let builder = TuiUiBuilder::from_app(ctx);
            self.sync_orchestration_tab_bar(&snapshot, &builder, ctx);
        }
    }

    fn switch_to_orchestration_tab(
        &mut self,
        key: Option<String>,
        keep_tab_focus: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(conversation_id) = key.and_then(|key| AIConversationId::try_from(key).ok()) else {
            return;
        };
        self.switch_to_orchestration_conversation(conversation_id, keep_tab_focus, ctx);
    }

    /// Switches to the retained session that owns an orchestration conversation.
    fn switch_to_orchestration_conversation(
        &mut self,
        conversation_id: AIConversationId,
        keep_tab_focus: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let session_id = TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
            model.focus_conversation_session(conversation_id, ctx)
        });
        let Some(session_id) = session_id else {
            return;
        };
        if session_id.surface_id() == self.terminal_surface_id {
            self.refresh_orchestration_tab_state(ctx);
            self.set_orchestration_tab_focus(keep_tab_focus, ctx);
            return;
        }
        self.orchestration_tabs_focused = false;
        ctx.notify();
        TuiSessions::set_orchestration_tab_focus(session_id, keep_tab_focus, ctx);
    }

    /// Synchronizes the retained tab child view from current orchestration state.
    fn sync_orchestration_tab_bar(
        &self,
        snapshot: &TuiOrchestrationSnapshot,
        builder: &TuiUiBuilder,
        ctx: &mut ViewContext<Self>,
    ) {
        let config =
            orchestration_tab_bar_config(snapshot, self.orchestration_tabs_focused, builder);
        self.set_orchestration_tab_bar_config(config, ctx);
    }

    fn clear_orchestration_tab_bar(&self, ctx: &mut ViewContext<Self>) {
        self.set_orchestration_tab_bar_config(TuiTabBarConfig::new(Vec::new()), ctx);
    }

    fn set_orchestration_tab_bar_config(
        &self,
        config: TuiTabBarConfig,
        ctx: &mut ViewContext<Self>,
    ) {
        let result = self
            .orchestration_tab_bar
            .update(ctx, |tab_bar, ctx| tab_bar.set_config(config, ctx));
        if let Err(error) = result {
            report_error!(
                anyhow::Error::new(error)
                    .context("Failed to update orchestration tab bar configuration"),
                warp_errors::ReportErrorLogMode::OncePerRun
            );
        }
    }

    /// Footer shown while orchestration tabs own keyboard focus.
    fn render_orchestration_tab_footer(&self, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        render_orchestration_tab_footer(builder)
    }
    /// The active front-of-queue blocking interaction, if any.
    fn active_blocking_child(&self, ctx: &AppContext) -> Option<TuiBlockingChild> {
        self.transcript.as_ref(ctx).active_blocking_child(ctx)
    }

    /// Activates this session after the registry has made it authoritative.
    pub(crate) fn activate(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus_current_owner(ctx);
        self.write_exit_summary(ctx);
        ctx.notify();
    }

    /// Whether this view projects the focused session.
    fn is_focused_session(&self, ctx: &AppContext) -> bool {
        TuiSessions::as_ref(ctx)
            .focused_session_id()
            .is_some_and(|id| id.surface_id() == self.terminal_surface_id)
    }

    /// Reconciles focus with the derived blocker: a newly active blocker is
    /// focused (handing off directly between consecutive blockers with no
    /// intermediate editable input), and focus returns to the input when the
    /// last blocker resolves. Nothing here writes to the input model, so its
    /// draft/cursor/selection are untouched.
    fn sync_blocker_focus(&mut self, ctx: &mut ViewContext<Self>) {
        let blocker = self.active_blocking_child(ctx);
        let blocker_view_id = blocker.as_ref().map(TuiBlockingChild::id);
        if blocker_view_id != self.active_blocker_view_id {
            self.active_blocker_view_id = blocker_view_id;
            self.focus_current_owner_if_active(ctx);
        }
        ctx.notify();
    }

    /// Restores an Oz conversation into the TUI's sole conversation surface.
    pub(crate) fn restore_conversation(
        &mut self,
        target: TuiConversationRestoreTarget,
        origin: TuiConversationRestoreOrigin,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.is_conversation_restore_loading() {
            return;
        }
        self.next_restore_request_id = self.next_restore_request_id.wrapping_add(1);
        let request_id = self.next_restore_request_id;
        self.conversation_restore_state = ConversationRestoreState::Loading {
            origin,
            request_id,
            future: None,
        };

        ctx.notify();
        let future =
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| match &target {
                TuiConversationRestoreTarget::Local(conversation_id) => {
                    history.load_conversation_data(*conversation_id, ctx)
                }
                TuiConversationRestoreTarget::Server(server_token) => {
                    history.load_conversation_by_server_token(server_token, ctx)
                }
            });

        let future_handle = ctx.spawn(future, move |view, result, ctx| {
            view.handle_conversation_restore_result(target, origin, request_id, result, ctx);
        });
        match &mut self.conversation_restore_state {
            ConversationRestoreState::Loading {
                request_id: active_request_id,
                future,
                ..
            } if *active_request_id == request_id => {
                *future = Some(future_handle);
            }
            ConversationRestoreState::Idle
            | ConversationRestoreState::Failed(_)
            | ConversationRestoreState::Loading { .. } => future_handle.abort(),
        }
    }

    /// Validates a completed load before starting synchronous surface replacement.
    fn handle_conversation_restore_result(
        &mut self,
        target: TuiConversationRestoreTarget,
        origin: TuiConversationRestoreOrigin,
        request_id: u64,
        result: Option<CloudConversationData>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.is_current_restore_request(request_id) {
            return;
        }

        let conversation = match result {
            Some(CloudConversationData::Oz(conversation)) => conversation,
            Some(CloudConversationData::CLIAgent(_)) => {
                self.fail_conversation_restore(
                    request_id,
                    "Warp Agent CLI only supports Oz/Warp conversations.".to_owned(),
                    ctx,
                );
                return;
            }
            None => {
                self.fail_conversation_restore(
                    request_id,
                    "The conversation could not be loaded.".to_owned(),
                    ctx,
                );
                return;
            }
        };

        let matches_target = match &target {
            TuiConversationRestoreTarget::Local(conversation_id) => {
                conversation.id() == *conversation_id
            }
            TuiConversationRestoreTarget::Server(server_token) => {
                conversation.server_conversation_token() == Some(server_token)
            }
        };
        if !matches_target {
            self.fail_conversation_restore(
                request_id,
                "The restored conversation did not match the requested conversation.".to_owned(),
                ctx,
            );
            return;
        }

        self.replace_conversation_surface(*conversation, origin, ctx);
    }

    /// Replaces the visible conversation and completes the restore state transition.
    fn replace_conversation_surface(
        &mut self,
        conversation: AIConversation,
        origin: TuiConversationRestoreOrigin,
        ctx: &mut ViewContext<Self>,
    ) {
        let previous_conversation_id = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx);
        if let Some(previous_conversation_id) = previous_conversation_id {
            self.transcript.update(ctx, |transcript, ctx| {
                transcript.clear_for_replacement(ctx);
            });

            self.terminal_model
                .lock()
                .block_list_mut()
                .remove_command_blocks_for_conversation(previous_conversation_id);

            self.ai_action_model.update(ctx, |actions, _| {
                actions.clear_restored_action_results();
            });

            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.clear_conversations_for_terminal_surface(self.terminal_surface_id, ctx);
            });
        }

        let conversation_id = conversation.id();
        let restoration_plan = {
            let mut terminal_model = self.terminal_model.lock();
            prepare_conversation_block_restoration(&conversation, &mut terminal_model)
        };

        self.ai_action_model.update(ctx, |actions, _| {
            actions.restore_action_results_from_exchanges(restoration_plan.exchanges().collect());
        });

        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.restore_conversations(self.terminal_surface_id, vec![conversation], ctx);
        });

        self.transcript.update(ctx, |transcript, ctx| {
            transcript.restore_conversation(conversation_id, restoration_plan, ctx);
        });

        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.set_active_conversation_id(conversation_id, self.terminal_surface_id, ctx);
        });

        self.conversation_selection.update(ctx, |selection, ctx| {
            selection.select_existing_conversation(
                conversation_id,
                origin.agent_view_origin(),
                ctx,
            );
        });

        self.conversation_restore_state = ConversationRestoreState::Idle;
        self.refresh_exit_summary(ctx);
        self.focus_input_if_active(ctx);
        ctx.notify();
    }

    fn is_current_restore_request(&self, request_id: u64) -> bool {
        matches!(
            &self.conversation_restore_state,
            ConversationRestoreState::Loading {
                request_id: active_request_id,
                ..
            } if *active_request_id == request_id
        )
    }

    fn is_conversation_restore_loading(&self) -> bool {
        matches!(
            &self.conversation_restore_state,
            ConversationRestoreState::Loading { .. }
        )
    }

    fn cancel_conversation_restore(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let state = std::mem::take(&mut self.conversation_restore_state);
        let ConversationRestoreState::Loading { future, .. } = state else {
            self.conversation_restore_state = state;
            return false;
        };
        if let Some(future) = future {
            future.abort();
        }
        self.next_restore_request_id = self.next_restore_request_id.wrapping_add(1);
        self.focus_input_if_active(ctx);
        ctx.notify();
        true
    }

    fn fail_conversation_restore(
        &mut self,
        request_id: u64,
        message: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let origin = match &self.conversation_restore_state {
            ConversationRestoreState::Loading {
                origin,
                request_id: active_request_id,
                ..
            } if *active_request_id == request_id => *origin,
            ConversationRestoreState::Idle
            | ConversationRestoreState::Failed(_)
            | ConversationRestoreState::Loading { .. } => return,
        };
        match origin {
            TuiConversationRestoreOrigin::Startup => {
                self.conversation_restore_state = ConversationRestoreState::Failed(message);
            }
            TuiConversationRestoreOrigin::ConversationList => {
                self.conversation_restore_state = ConversationRestoreState::Idle;
                self.show_transient_hint(message, ctx);
                self.focus_input_if_active(ctx);
            }
        }
        ctx.notify();
    }

    fn refresh_exit_summary(&self, ctx: &AppContext) {
        if !self.is_focused_session(ctx) {
            return;
        }
        self.write_exit_summary(ctx);
    }

    fn write_exit_summary(&self, ctx: &AppContext) {
        let token = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation(ctx)
            .filter(|conversation| !conversation.is_empty())
            .and_then(|conversation| conversation.server_conversation_token())
            .cloned();
        self.exit_summary.set_token(token);
    }

    /// Applies a laid-out terminal content size to the terminal model and PTY.
    /// TUI counterpart of the GUI's `after_terminal_view_layout`
    /// (`app/src/terminal/view.rs`): consumes the after-layout resize channel
    /// and commits the resize with a `ViewContext`. Fed by the
    /// [`TuiTerminalContentElement`] wrapping the block-list content column or the
    /// alt-screen grid, so the PTY tracks whichever region PTY content
    /// currently occupies.
    fn handle_terminal_resize(&mut self, size: TuiSize, ctx: &mut ViewContext<Self>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        let size_update = SizeUpdate::from_cell_dimensions(
            self.size_info,
            usize::from(size.height),
            usize::from(size.width),
        );
        if !size_update.rows_or_columns_changed() {
            return;
        }

        self.terminal_model.lock().resize(size_update);
        self.size_info = size_update.new_size();
        ctx.emit(TuiTerminalSessionEvent::Resize(size_update));
        ctx.notify();
    }
    /// Refreshes terminal model geometry and redraws only when this session is visible.
    fn handle_terminal_wakeup(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        {
            let mut model = self.terminal_model.lock();
            if !model.is_alt_screen_active() {
                model.block_list_mut().update_background_block_height();
                model.block_list_mut().update_active_block_height();
            }
        }
        let is_focused = self.is_focused_session(ctx);
        if is_focused {
            self.update_process_input_focus(ctx);
            ctx.notify();
        }
        is_focused
    }

    /// Re-renders on history events that can change the warping indicator:
    /// the selected conversation's status changing, or an exchange starting
    /// (which re-anchors the elapsed counter) on this surface.
    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if event
            .terminal_surface_id()
            .is_some_and(|id| id != self.terminal_surface_id)
        {
            return;
        }
        if let Some(persistence_event) =
            maybe_build_ai_query_upsert_event(event, self.terminal_surface_id, false, ctx)
            && let Some(model_event_sender) = PersistenceWriter::handle(ctx).as_ref(ctx).sender()
        {
            let _ = ctx.spawn(
                async move { model_event_sender.send(persistence_event) },
                |_, result, _| {
                    if let Err(error) = result {
                        report_error!(
                            anyhow::Error::new(error)
                                .context("Error sending TUI upsert AI query event")
                        );
                    }
                },
            );
        }
        if matches!(
            event,
            BlocklistAIHistoryEvent::AppendedExchange { .. }
                | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
                | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
        ) {
            ctx.notify();
        }

        if matches!(
            event,
            BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
                | BlocklistAIHistoryEvent::RestoredConversations { .. }
        ) {
            self.refresh_exit_summary(ctx);
        }
        match event {
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces {
                conversation_id,
                ..
            } => {
                self.cli_subagent_views
                    .retain(|_, view| view.as_ref(ctx).conversation_id() != *conversation_id);
            }
            BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface { .. } => {
                self.cli_subagent_views.clear();
            }
            _ => {}
        }
    }

    fn show_auto_approve_feedback(&mut self, ctx: &mut ViewContext<Self>) {
        self.auto_approve_feedback_conversation_id = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx);
        let timer = ctx.spawn(
            Timer::after(AUTO_APPROVE_FEEDBACK_DURATION),
            |view, _, ctx| {
                view.auto_approve_feedback_conversation_id = None;
                view.auto_approve_feedback_timer = None;
                ctx.notify();
            },
        );
        if let Some(previous_timer) = self.auto_approve_feedback_timer.replace(timer) {
            previous_timer.abort();
        }
        ctx.notify();
    }

    fn clear_auto_approve_feedback(&mut self, ctx: &mut ViewContext<Self>) {
        self.auto_approve_feedback_conversation_id = None;
        if let Some(timer) = self.auto_approve_feedback_timer.take() {
            timer.abort();
        }
        ctx.notify();
    }

    fn toggle_auto_approve(&mut self, show_feedback: bool, ctx: &mut ViewContext<Self>) {
        self.conversation_selection.update(ctx, |selection, ctx| {
            selection.toggle_pending_query_autoexecute(ctx);
        });
        if show_feedback {
            self.show_auto_approve_feedback(ctx);
        } else {
            self.clear_auto_approve_feedback(ctx);
        }
    }

    fn handle_pasted(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        let disposition = self
            .attachment_bar
            .update(ctx, |bar, ctx| bar.try_attach_paste(text.clone(), ctx));
        if disposition == TuiAttachmentPasteDisposition::NotHandled {
            self.input_view
                .update(ctx, |input, ctx| input.insert_pasted_text(&text, ctx));
        }
    }

    fn handle_attachment_bar_event(
        &mut self,
        event: &TuiAttachmentBarEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiAttachmentBarEvent::AbortInputDetection => self.abort_input_detection(ctx),
            TuiAttachmentBarEvent::RequestInputDetection => self.schedule_input_detection(ctx),
            TuiAttachmentBarEvent::RestorePastedText(text) => {
                self.input_view
                    .update(ctx, |input, ctx| input.insert_pasted_text(text, ctx));
            }
            TuiAttachmentBarEvent::ShowHint(text) => {
                self.show_transient_hint(text.clone(), ctx);
            }
            TuiAttachmentBarEvent::ReturnFocus => ctx.focus(&self.input_view),
        }
        ctx.notify();
    }

    /// Displays `text` in the footer's hint slot for the transient-hint
    /// duration, then reverts to the persistent content.
    fn show_transient_hint(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        self.transient_hint
            .show(text, ctx, |view| &mut view.transient_hint);
    }

    /// Displays success-colored feedback in the transient footer slot.
    fn show_success_hint(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        self.transient_hint
            .show_success(text, ctx, |view| &mut view.transient_hint);
    }

    /// Displays success-colored feedback in the transient footer slot.
    fn show_copy_hint(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_success_hint(COPY_SELECTION_HINT.to_owned(), ctx);
    }

    /// Handles a ctrl-c press: a second press within [`CTRL_C_EXIT_WINDOW`]
    /// exits the TUI; otherwise one contextual action runs — cancel the running
    /// conversation if there is one, else clear the input — and the exit
    /// confirmation is (re-)armed, surfacing [`CTRL_C_EXIT_HINT`] in the footer.
    fn handle_interrupt(&mut self, ctx: &mut ViewContext<Self>) {
        if self.cancel_conversation_restore(ctx) {
            return;
        }
        if matches!(
            &self.conversation_restore_state,
            ConversationRestoreState::Failed(_)
        ) {
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
            return;
        }
        if self.handle_terminal_use_interrupt(ctx) {
            self.exit_confirmation.disarm();
            ctx.notify();
            return;
        }
        let now = Instant::now();
        if self.exit_confirmation.should_exit(now) {
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
            return;
        }

        if !self.cancel_active_conversation(ctx) {
            self.input_view.update(ctx, |input, ctx| input.clear(ctx));
        }

        // Arm (or re-arm) the confirmation, and disarm + repaint when the
        // window lapses. A re-arm supersedes this (now stale) timer, making
        // its `disarm_expired` a no-op rather than clearing the newer window.
        let window_expires_at = self.exit_confirmation.arm(now);
        ctx.spawn(Timer::after(CTRL_C_EXIT_WINDOW), move |view, _, ctx| {
            if view.exit_confirmation.disarm_expired(window_expires_at) {
                ctx.notify();
            }
        });
        ctx.notify();
    }

    /// Handles ctrl-d while the prompt is focused. Unlike ctrl-c, ctrl-d exits
    /// immediately when the prompt is empty; otherwise it keeps its editing
    /// role of deleting the next character.
    fn handle_eof(&mut self, ctx: &mut ViewContext<Self>) {
        if self.input_view.as_ref(ctx).is_empty(ctx) {
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
        } else {
            self.input_view.update(ctx, |input, ctx| {
                input.handle_action(
                    &TuiInputAction::EditorCommand(TuiEditorCommand::DeleteForward),
                    ctx,
                );
            });
        }
    }

    /// Cancels the surface's running conversation (in-flight stream or pending
    /// tool actions), returning whether there was one to cancel.
    fn cancel_active_conversation(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let terminal_surface_id = ctx.view_id();
        self.ai_controller.update(ctx, |controller, ctx| {
            let conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(terminal_surface_id)
                // A brand-new conversation reports `InProgress` before any
                // exchange exists; there is nothing to cancel yet.
                .filter(|conversation| !conversation.is_empty())
                .filter(|conversation| {
                    let status = conversation.status();
                    status.is_in_progress() || status.is_blocked()
                })
                .map(|conversation| conversation.id());
            let Some(conversation_id) = conversation_id else {
                return false;
            };
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
            true
        })
    }

    fn render_warping_indicator(
        &self,
        label: &'static str,
        elapsed: Duration,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(ctx);
        let is_hovered = self
            .auto_approve_mouse
            .lock()
            .is_ok_and(|state| state.is_hovered());
        let style = if is_hovered {
            builder.primary_text_style()
        } else if self.auto_approve_feedback_conversation_id == Some(conversation_id) {
            builder.success_glyph_style()
        } else {
            builder.muted_text_style()
        };
        let enabled = self
            .conversation_selection
            .as_ref(ctx)
            .pending_query_autoexecute_override(ctx)
            .is_autoexecute_any_action();
        let auto_approve = TuiHoverable::new(
            self.auto_approve_mouse.clone(),
            TuiText::new(format!(
                "▶▶ Auto approve {}",
                if enabled { "on" } else { "off" }
            ))
            .with_style(style)
            .truncate()
            .finish(),
        )
        .on_click(|event_ctx, _| {
            event_ctx.dispatch_typed_action(TuiTerminalSessionAction::ToggleAutoApprove {
                show_feedback: false,
            });
        })
        .finish();
        render_warping_indicator_row(label, elapsed, auto_approve, ctx)
    }

    /// Builds the status footer under the input box. The row is left-aligned:
    /// in agent mode `[model] [cwd ↬ branch] • [usage] • [+N -M]`, and in shell
    /// mode `[shell mode] [cwd ↬ branch] • [+N -M]` (model and usage hidden).
    /// A replacing hint — the ctrl-c exit confirmation while armed, the
    /// conversation-list loading hint, or an active transient notice — occupies
    /// the whole row instead. Every child truncates to a single row, so the row
    /// lays out one row tall.
    fn render_footer(&self, ctx: &AppContext) -> TuiFlex {
        let builder = TuiUiBuilder::from_app(ctx);
        let muted = builder.muted_text_style();

        // Replacing hints occupy the entire status row, in the existing
        // priority order: ctrl-c → loading → transient.
        if self.exit_confirmation.is_armed() {
            return TuiFlex::row().child(
                TuiText::new(CTRL_C_EXIT_HINT)
                    .with_style(muted)
                    .truncate()
                    .finish(),
            );
        }
        if matches!(
            &self.conversation_restore_state,
            ConversationRestoreState::Loading {
                origin: TuiConversationRestoreOrigin::ConversationList,
                ..
            }
        ) {
            return TuiFlex::row().child(
                TuiText::new(LOADING_CONVERSATION_HINT)
                    .with_style(muted)
                    .truncate()
                    .finish(),
            );
        }
        if let Some((transient, tone)) = self.transient_hint.current() {
            let style = match tone {
                TransientHintTone::Muted => muted,
                TransientHintTone::Success => builder.success_glyph_style(),
            };
            return TuiFlex::row().child(
                TuiText::new(transient)
                    .with_style(style)
                    .truncate()
                    .finish(),
            );
        }
        let shell_mode = self.is_shell_mode(ctx);

        // Normal left-aligned sectioned status row.
        let git_metadata = self.git_status_metadata(ctx);
        let model_label = if shell_mode {
            None
        } else {
            let model_name = LLMPreferences::as_ref(ctx)
                .get_active_base_model(ctx, Some(self.terminal_surface_id))
                .display_name
                .clone();
            // The active-model label is clickable: a left click toggles the
            // inline model picker (the same menu `/model` surfaces). The hover
            // state lives on a retained [`MouseStateHandle`] so it survives
            // element-tree rebuilds, and the click dispatches a typed action
            // since the element pass only has an immutable [`AppContext`] —
            // mirroring the usage entry.
            let model_label_hovered = self
                .model_label_hover
                .lock()
                .is_ok_and(|state| state.is_hovered());
            let model_label_style = if model_label_hovered {
                builder.primary_text_style()
            } else {
                builder.muted_text_style()
            };
            Some(
                TuiHoverable::new(
                    self.model_label_hover.clone(),
                    TuiText::new(model_name)
                        .with_style(model_label_style)
                        .truncate()
                        .finish(),
                )
                .on_click(|event_ctx, _| {
                    event_ctx.dispatch_typed_action(TuiTerminalSessionAction::ToggleModelMenu);
                })
                .finish(),
            )
        };
        let cwd = self
            .current_working_directory(ctx)
            .map(|cwd| compact_footer_path(&cwd));
        let branch = git_metadata.map(|metadata| metadata.current_branch_name.clone());
        // Usage entry: the selected conversation's credits/cost totals, hidden
        // until any usage has been reported (and hidden in shell mode, where it
        // is stale AI-conversation metadata). The displayed unit is the
        // persisted `agents.usage_display_mode` setting; a click dispatches the
        // toggle action (the element pass cannot write settings directly).
        let usage = if shell_mode {
            None
        } else {
            self.selected_conversation_usage_totals(ctx).map(|totals| {
                let mode = AISettings::as_ref(ctx).usage_display_mode;
                self.usage_toggle
                    .render_entry(mode, totals, ctx, |event_ctx, _| {
                        event_ctx
                            .dispatch_typed_action(TuiTerminalSessionAction::ToggleUsageDisplay);
                    })
            })
        };
        let (diff_additions, diff_deletions) = git_metadata
            .filter(|metadata| {
                let stats = metadata.stats_against_head;
                stats.total_additions > 0 || stats.total_deletions > 0
            })
            .map(|metadata| {
                let stats = metadata.stats_against_head;
                (stats.total_additions, stats.total_deletions)
            })
            .unwrap_or_default();

        render_status_footer_row(
            FooterSegments {
                shell_mode,
                model_label,
                cwd,
                branch,
                usage,
                diff_additions,
                diff_deletions,
            },
            &builder,
        )
    }

    /// Updates the watcher-backed git-status subscription after repository
    /// detection completes for the active working directory.
    fn update_git_status_subscription(
        &mut self,
        repo_path: Option<LocalOrRemotePath>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.current_repo_path == repo_path && self.git_repo_status.is_some() {
            return;
        }
        self.current_repo_path = repo_path.clone();
        self.git_repo_status = None;

        let Some(repo_path) = repo_path else {
            ctx.notify();
            return;
        };
        match GitRepoModels::handle(ctx)
            .update(ctx, |models, ctx| models.subscribe(&repo_path, ctx))
        {
            Ok(handle) => {
                ctx.subscribe_to_model(&handle, |_, _, _, ctx| ctx.notify());
                self.git_repo_status = Some(handle);
            }
            Err(error) => {
                log::warn!("Unable to subscribe TUI footer to git status: {error}");
            }
        }
        ctx.notify();
    }

    fn git_status_metadata<'a>(&self, ctx: &'a AppContext) -> Option<&'a GitStatusMetadata> {
        self.git_repo_status.as_ref()?.as_ref(ctx).metadata(ctx)
    }

    /// Flips the footer usage entry's persisted credits⇄cost display mode.
    /// The settings-changed event re-renders every subscribed surface.
    fn toggle_usage_display(&mut self, ctx: &mut ViewContext<Self>) {
        let next = AISettings::as_ref(ctx).usage_display_mode.toggled();
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(error) = settings.usage_display_mode.set_value(next, ctx) {
                report_error!("failed to persist the TUI usage display mode: {error:#}");
            }
        });
    }
    /// Mirrors the GUI `/cost` eligibility checks, then toggles the selected
    /// conversation's completed-response summary without changing the
    /// persistent footer's independent credits⇄cost setting.
    fn toggle_response_summary_visibility(&mut self, ctx: &mut ViewContext<Self>) {
        let selected_conversation = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation(ctx)
            .map(|conversation| {
                (
                    conversation.latest_exchange().map(|exchange| exchange.id),
                    conversation.is_empty(),
                    conversation.status().is_done(),
                )
            });
        if let Some(hint) = cost_command_unavailable_hint(
            selected_conversation.map(|(_, is_empty, is_done)| (is_empty, is_done)),
        ) {
            self.show_transient_hint(hint.to_owned(), ctx);
            return;
        }
        let Some((Some(exchange_id), _, _)) = selected_conversation else {
            self.show_transient_hint(COST_NO_ACTIVE_CONVERSATION_HINT.to_owned(), ctx);
            return;
        };
        self.toggle_response_summary_visibility_for_exchange(exchange_id);
        ctx.notify();
    }
    fn toggle_response_summary_visibility_for_exchange(&mut self, exchange_id: AIAgentExchangeId) {
        if !self
            .hidden_response_summary_exchange_ids
            .remove(&exchange_id)
        {
            self.hidden_response_summary_exchange_ids
                .insert(exchange_id);
        }
    }

    fn render_response_summary_for_exchange(
        &self,
        exchange_id: AIAgentExchangeId,
        duration: Duration,
        block_credits: Option<f32>,
        ctx: &AppContext,
    ) -> Option<Box<dyn TuiElement>> {
        (!self
            .hidden_response_summary_exchange_ids
            .contains(&exchange_id))
        .then(|| render_response_summary(duration, block_credits, ctx))
    }

    /// Toggles the inline model picker from the footer's active-model label —
    /// the same menu `/model` surfaces. The model's existing open/dismiss paths
    /// preserve active-menu arbitration, input cleanup, and selection handling.
    fn toggle_model_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.model_menu.update(ctx, |menu, ctx| {
            if menu.is_open(ctx) {
                menu.dismiss(ctx);
            } else {
                menu.open(ctx);
            }
        });
    }

    /// The selected conversation's accumulated usage totals, or `None` (entry
    /// hidden) until any usage has been reported.
    fn selected_conversation_usage_totals(
        &self,
        ctx: &AppContext,
    ) -> Option<ConversationUsageTotals> {
        let totals = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation(ctx)?
            .usage_totals();
        (totals != ConversationUsageTotals::default()).then_some(totals)
    }

    /// The session's working directory. The cwd only arrives once shell
    /// metadata flows (warpified sessions); until then fall back to the
    /// process cwd the TUI's shell was spawned with.
    fn current_working_directory(&self, ctx: &AppContext) -> Option<String> {
        self.active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned()
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|cwd| cwd.to_string_lossy().into_owned())
            })
    }

    /// Whether the input is in detected or explicitly locked shell mode.
    fn is_shell_mode(&self, ctx: &AppContext) -> bool {
        input_mode_policy::is_shell_mode(self.ai_input_model.as_ref(ctx))
    }

    /// Routes a submission to shell execution or the agent conversation based
    /// on the input mode.
    fn handle_submitted(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        // A stale editor frame must not submit into a shell that is still
        // bootstrapping or has handed input to a foreground process.
        if !self.input_target().agent_editor_owns_input() {
            return;
        }
        if !matches!(
            self.conversation_restore_state,
            ConversationRestoreState::Idle
        ) {
            return;
        }
        if self.send_terminal_use_prompt(&text, ctx) {
            self.input_view
                .update(ctx, |input, ctx| input.exit_shell_mode(ctx));
        } else if self.is_shell_mode(ctx) {
            self.execute_user_command(&text, ctx);
        } else {
            self.handle_submitted_input(&text, ctx);
        }
        ctx.notify();
    }

    /// Executes `command` in the session's PTY as a plain user command.
    ///
    /// Mirrors the GUI's shell-mode submission: rejected while the agent holds
    /// the PTY with an active long-running command (the input keeps its text
    /// and a transient hint is shown), and an in-progress conversation is
    /// cancelled when the command runs. On success the input clears and exits
    /// shell mode back to agent input.
    fn execute_user_command(&mut self, command: &str, ctx: &mut ViewContext<Self>) {
        // A whitespace-only command is a no-op; stay in shell mode. The command
        // itself is sent to the PTY untrimmed, exactly as typed.
        if command.trim().is_empty() {
            return;
        }

        // Keep the lock scope to these reads only (see the terminal-model
        // locking guidance).
        let (is_pty_busy, session_id) = {
            let terminal_model = self.terminal_model.lock();
            let block_list = terminal_model.block_list();
            let active_block = block_list.active_block();
            let is_pty_busy = !block_list.is_bootstrapped()
                || (active_block.is_active_and_long_running()
                    && !active_block.is_in_band_command_block());
            (is_pty_busy, active_block.session_id())
        };
        let Some(session_id) = session_id else {
            log::warn!("Unable to execute TUI user command: no active session");
            return;
        };
        if is_pty_busy {
            self.show_transient_hint(COMMAND_ALREADY_RUNNING_HINT.to_owned(), ctx);
            return;
        }

        // Executing a shell command cancels an in-progress conversation
        // (mirrors the GUI; the running command above is left untouched).
        if let Some(conversation_id) = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        {
            let is_in_progress = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .is_some_and(|conversation| conversation.status().is_in_progress());
            if is_in_progress {
                self.ai_controller.update(ctx, |controller, ctx| {
                    controller.cancel_conversation_progress(
                        conversation_id,
                        CancellationReason::UserCommandExecuted,
                        ctx,
                    );
                });
            }
        }

        ctx.emit(TuiTerminalSessionEvent::ExecuteCommand(Box::new(
            ExecuteCommandEvent {
                command: command.to_owned(),
                session_id,
                workflow_id: None,
                workflow_command: None,
                should_add_command_to_history: true,
                source: CommandExecutionSource::User,
            },
        )));

        // The submission was accepted: clear the input and return to the
        // setting-derived agent default.
        self.input_view
            .update(ctx, |input_view, ctx| input_view.clear(ctx));
    }

    /// Sends a prompt to the TUI session's eagerly selected conversation.
    fn send_prompt(&mut self, prompt: String, ctx: &mut ViewContext<Self>) {
        let active_long_running_block_id = {
            let terminal_model = self.terminal_model.lock();
            let active_block = terminal_model.block_list().active_block();
            active_block
                .is_active_and_long_running()
                .then(|| active_block.id().clone())
        };
        let Some(conversation_id) = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            report_error!("TUI prompt submitted without an eagerly selected conversation");
            return;
        };
        let dispatched = self.ai_controller.update(ctx, |controller, ctx| {
            controller.send_user_query_in_conversation(prompt.clone(), conversation_id, None, ctx)
        });
        if dispatched && let Some(block_id) = active_long_running_block_id {
            self.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.set_latest_instruction(block_id, prompt, ctx);
            });
        }
    }

    fn handle_submitted_input(&mut self, input: &str, ctx: &mut ViewContext<Self>) {
        if self.is_conversation_restore_loading() {
            return;
        }
        match self
            .slash_commands_source
            .as_ref(ctx)
            .parse_input(input, ctx)
        {
            ParsedSlashCommandInput::SlashCommand(detected_command) => {
                self.execute_tui_slash_command(
                    &detected_command.command,
                    detected_command.argument.as_ref(),
                    ctx,
                );
            }
            ParsedSlashCommandInput::SkillCommand(detected_skill) => {
                self.execute_skill_command(detected_skill.reference, detected_skill.argument, ctx);
            }
            ParsedSlashCommandInput::None | ParsedSlashCommandInput::Composing { .. } => {
                let prompt = raw_prompt_if_not_blank(input);
                self.input_view.update(ctx, |input_view, ctx| {
                    input_view.clear(ctx);
                });
                if let Some(prompt) = prompt {
                    self.send_prompt(prompt.to_owned(), ctx);
                }
            }
        }
    }

    fn execute_skill_command(
        &mut self,
        reference: SkillReference,
        user_query: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self
            .slash_commands_source
            .as_ref(ctx)
            .local_skills_available(ctx)
        {
            self.show_transient_hint(LOCAL_SKILLS_REMOTE_EXECUTION_ERROR_MESSAGE.to_owned(), ctx);
            return;
        }
        let result = self.ai_controller.update(ctx, |controller, ctx| {
            controller.send_invoke_skill_request(reference, user_query, ctx)
        });
        match result {
            Ok(()) => {
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
            }
            Err(error) => {
                self.show_transient_hint(error.to_string(), ctx);
            }
        }
    }

    fn handle_accepted_slash_command(
        &mut self,
        action: &AcceptSlashCommandOrSavedPrompt,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            AcceptSlashCommandOrSavedPrompt::SlashCommand { id } => {
                let Some(command) = COMMAND_REGISTRY.get_command(id) else {
                    log::debug!("TUI slash command selection is not supported yet: {id:?}");
                    ctx.notify();
                    return;
                };
                self.select_tui_slash_command(command, ctx);
            }
            AcceptSlashCommandOrSavedPrompt::SavedPrompt { id } => {
                let Some(prompt) = saved_prompt_text_for_id(id, ctx) else {
                    log::warn!("Tried to insert saved prompt for id {id:?} but it does not exist");
                    return;
                };
                self.input_view.update(ctx, |input, ctx| {
                    input.set_text(&prompt, ctx);
                });
                record_saved_prompt_accepted(true, ctx);
            }
            AcceptSlashCommandOrSavedPrompt::Skill { name, .. } => {
                self.input_view.update(ctx, |input, ctx| {
                    input.set_text(&format!("/{name} "), ctx);
                });
            }
        }
        ctx.notify();
    }

    fn handle_accepted_conversation(
        &mut self,
        entry_id: AgentConversationEntryId,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.is_conversation_restore_loading() {
            self.show_transient_hint(SWITCH_LOADING_HINT.to_owned(), ctx);
            return;
        }
        if !self
            .ai_context_model
            .as_ref(ctx)
            .can_start_new_conversation()
        {
            self.show_transient_hint(SWITCH_COMMAND_RUNNING_HINT.to_owned(), ctx);
            return;
        }
        let current_conversation_is_busy = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation(ctx)
            .is_some_and(|conversation| {
                !conversation.is_empty() && !conversation.status().is_done()
            });
        if current_conversation_is_busy {
            self.show_transient_hint(SWITCH_CONVERSATION_RUNNING_HINT.to_owned(), ctx);
            return;
        }

        let Some(entry) = AgentConversationsModel::as_ref(ctx).get_entry_by_id(&entry_id, ctx)
        else {
            self.show_transient_hint(SWITCH_UNAVAILABLE_HINT.to_owned(), ctx);
            return;
        };
        if self
            .conversation_selection
            .as_ref(ctx)
            .classify_entry(&entry, ctx)
            != AgentConversationListEntryState::Available
        {
            self.show_transient_hint(SWITCH_UNAVAILABLE_HINT.to_owned(), ctx);
            return;
        }
        let target = match (
            entry.identity.local_conversation_id,
            entry.identity.server_conversation_token,
        ) {
            (Some(conversation_id), _) => TuiConversationRestoreTarget::Local(conversation_id),
            (None, Some(server_token)) => TuiConversationRestoreTarget::Server(server_token),
            (None, None) => {
                self.show_transient_hint(SWITCH_UNAVAILABLE_HINT.to_owned(), ctx);
                return;
            }
        };

        self.conversation_menu
            .update(ctx, |menu, ctx| menu.dismiss(ctx));
        self.restore_conversation(target, TuiConversationRestoreOrigin::ConversationList, ctx);
    }

    fn handle_accepted_model(&mut self, id: &LLMId, ctx: &mut ViewContext<Self>) {
        let terminal_view_id = ctx.view_id();
        let persisted = LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
            preferences.update_active_profile_base_model(id, Some(terminal_view_id), ctx)
        });
        if !persisted {
            self.show_transient_hint(MODEL_PERSISTENCE_FAILED_HINT.to_owned(), ctx);
            return;
        }
        self.model_menu.update(ctx, |menu, ctx| menu.dismiss(ctx));
    }
    fn handle_accepted_mcp_action(&mut self, action: TuiMcpAction, ctx: &mut ViewContext<Self>) {
        TuiMcpManager::handle(ctx).update(ctx, |model, ctx| {
            model.apply_action(action, ctx);
        });
        ctx.notify();
    }

    /// Fills the accepted prompt-history prompt into the input and submits it
    /// immediately, matching the GUI's accept-a-prompt-from-history behavior.
    /// The menu has already closed itself.
    fn handle_accepted_prompt_history(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        self.input_view.update(ctx, |input, ctx| {
            input.set_text(&text, ctx);
        });
        self.handle_submitted(text, ctx);
    }

    fn select_tui_slash_command(&mut self, command: &StaticCommand, ctx: &mut ViewContext<Self>) {
        match slash_command_selection_behavior(command) {
            SlashCommandSelectionBehavior::InsertCommandText(text) => {
                self.input_view.update(ctx, |input, ctx| {
                    input.set_text(&text, ctx);
                });
            }
            SlashCommandSelectionBehavior::Execute => {
                self.execute_tui_slash_command(command, None, ctx);
            }
        }
    }

    fn execute_tui_slash_command(
        &mut self,
        command: &StaticCommand,
        argument: Option<&String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !command.supports_tui() {
            log::debug!(
                "TUI slash command selection is not supported yet: {}",
                command.name
            );
            return;
        }

        match command.kind {
            SlashCommandKind::Agent | SlashCommandKind::New => {
                if !self
                    .ai_context_model
                    .as_ref(ctx)
                    .can_start_new_conversation()
                {
                    self.show_transient_hint(NEW_CONVERSATION_COMMAND_RUNNING_HINT.to_owned(), ctx);
                    return;
                }
                self.cancel_active_conversation(ctx);
                let terminal_surface_id = ctx.view_id();
                self.transcript.update(ctx, |transcript, ctx| {
                    transcript.clear_for_new_conversation(ctx);
                });
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.clear_conversations_for_terminal_surface(terminal_surface_id, ctx);
                });
                self.conversation_selection.update(ctx, |selection, ctx| {
                    selection.select_new_conversation(AgentViewEntryOrigin::Tui, ctx);
                });
                if let Some(prompt) = argument
                    .map(|argument| argument.trim())
                    .filter(|argument| !argument.is_empty())
                {
                    self.send_prompt(prompt.to_owned(), ctx);
                }
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::Conversations => {
                self.conversation_menu
                    .update(ctx, |menu, ctx| menu.open(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::AutoApprove => {
                self.toggle_auto_approve(true, ctx);
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::Cost => {
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                ctx.dispatch_typed_action_deferred(
                    TuiTerminalSessionAction::ToggleResponseSummaryVisibility,
                );
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::Model => {
                self.model_menu.update(ctx, |menu, ctx| menu.open(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::InvokeSkill => {
                if !FeatureFlag::ListSkills.is_enabled() {
                    return;
                }
                self.skills_menu.update(ctx, |menu, ctx| menu.open(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::Mcp => {
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                self.mcp_menu.update(ctx, |menu, ctx| menu.open(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::Exit => {
                record_static_slash_command_accepted(command.name, true, ctx);
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
            SlashCommandKind::Logout => {
                record_static_slash_command_accepted(command.name, true, ctx);
                log_out_tui(ctx);
            }
            SlashCommandKind::ViewLogs => {
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                ctx.spawn(
                    async move {
                        tokio::task::spawn_blocking(|| {
                            let path = warp_logging::create_log_bundle_zip()?;
                            reveal_path_in_file_manager(&path);
                            Ok::<_, anyhow::Error>(path)
                        })
                        .await
                    },
                    |me, result, ctx| match result {
                        Ok(Ok(path)) => {
                            me.show_success_hint(log_bundle_success_message(&path), ctx);
                        }
                        Ok(Err(error)) => {
                            report_error!(error.context("Failed to create TUI log bundle"));
                            me.show_transient_hint(LOG_BUNDLE_FAILED_HINT.to_owned(), ctx);
                        }
                        Err(error) => {
                            report_error!(
                                anyhow::Error::new(error).context("TUI log bundle task failed")
                            );
                            me.show_transient_hint(LOG_BUNDLE_FAILED_HINT.to_owned(), ctx);
                        }
                    },
                );
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::CreateNewProject => {
                let Some(query) = argument
                    .map(|argument| argument.trim())
                    .filter(|argument| !argument.is_empty())
                else {
                    self.show_transient_hint(
                        "Please describe the project you want to create after /create-new-project"
                            .to_owned(),
                        ctx,
                    );
                    return;
                };
                self.ai_controller.update(ctx, |controller, ctx| {
                    controller.send_create_new_project_request(query.to_owned(), ctx);
                });
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::ExportToClipboard => {
                if let Some(conversation) = self
                    .conversation_selection
                    .as_ref(ctx)
                    .selected_conversation(ctx)
                {
                    let markdown =
                        conversation.export_to_markdown(Some(self.ai_action_model.as_ref(ctx)));
                    match copy_to_clipboard(&markdown) {
                        Ok(()) => {
                            self.show_success_hint(
                                "Conversation copied to clipboard".to_owned(),
                                ctx,
                            );
                        }
                        Err(error) => {
                            log::warn!("Failed to export TUI conversation: {error}");
                            self.show_transient_hint(COPY_FAILED_HINT.to_owned(), ctx);
                        }
                    }
                } else {
                    self.show_transient_hint("No active conversation to export".to_owned(), ctx);
                }
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::ExportToFile => {
                let Some(conversation) = self
                    .conversation_selection
                    .as_ref(ctx)
                    .selected_conversation(ctx)
                else {
                    self.show_transient_hint("No active conversation to export".to_owned(), ctx);
                    return;
                };
                let title = conversation.title();
                let markdown =
                    conversation.export_to_markdown(Some(self.ai_action_model.as_ref(ctx)));
                let current_directory = self
                    .active_session
                    .as_ref(ctx)
                    .current_working_directory()
                    .cloned();
                match export_conversation_markdown(
                    current_directory.as_deref(),
                    argument.map(String::as_str),
                    title.as_deref(),
                    &markdown,
                ) {
                    Ok(export) => {
                        self.show_success_hint(export_file_success_message(&export), ctx);
                    }
                    Err(error) => {
                        let message = error.user_message();
                        let path = error.path().to_path_buf();
                        report_error!(
                            anyhow::Error::new(error)
                                .context("Failed to write TUI conversation to file"),
                            extra: { "path" => %path.display() }
                        );
                        self.show_transient_hint(message, ctx);
                    }
                }
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::Compact | SlashCommandKind::Plan => {
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                let command_name = command.name;
                let prompt = argument
                    .map(|argument| {
                        if argument.is_empty() {
                            command_name.to_owned()
                        } else {
                            format!("{command_name} {argument}")
                        }
                    })
                    .unwrap_or_else(|| command_name.to_owned());
                self.send_prompt(prompt, ctx);
                record_static_slash_command_accepted(command_name, true, ctx);
            }
            SlashCommandKind::NaturalLanguageDetection => {
                self.toggle_nld(command.name, ctx);
            }
            SlashCommandKind::CloudAgent
            | SlashCommandKind::AddMcp
            | SlashCommandKind::CreateEnvironment
            | SlashCommandKind::CreateDockerSandbox
            | SlashCommandKind::EditSkill
            | SlashCommandKind::AddPrompt
            | SlashCommandKind::AddRule
            | SlashCommandKind::Edit
            | SlashCommandKind::RenameTab
            | SlashCommandKind::RenameConversation
            | SlashCommandKind::SetTabColor
            | SlashCommandKind::Fork
            | SlashCommandKind::MoveToCloud
            | SlashCommandKind::OpenCodeReview
            | SlashCommandKind::Index
            | SlashCommandKind::Init
            | SlashCommandKind::OpenProjectRules
            | SlashCommandKind::OpenMcpServers
            | SlashCommandKind::OpenSettingsFile
            | SlashCommandKind::Changelog
            | SlashCommandKind::Feedback
            | SlashCommandKind::OpenRepo
            | SlashCommandKind::OpenRules
            | SlashCommandKind::Host
            | SlashCommandKind::Harness
            | SlashCommandKind::Environment
            | SlashCommandKind::Profile
            | SlashCommandKind::Orchestrate
            | SlashCommandKind::CompactAnd
            | SlashCommandKind::Queue
            | SlashCommandKind::ForkAndCompact
            | SlashCommandKind::ForkFrom
            | SlashCommandKind::ContinueLocally
            | SlashCommandKind::Usage
            | SlashCommandKind::RemoteControl
            | SlashCommandKind::Prompts
            | SlashCommandKind::Rewind => {
                debug_assert!(
                    false,
                    "Attempted to execute GUI-only slash command in the TUI: {}",
                    command.name
                );
            }
        }
    }

    /// Toggles and persists natural-language detection (NLD), reports the change
    /// via telemetry, and surfaces a confirmation hint.
    fn toggle_nld(&mut self, command_name: &'static str, ctx: &mut ViewContext<Self>) {
        self.input_view.update(ctx, |input, ctx| input.clear(ctx));
        let enabled = !AISettings::as_ref(ctx).is_ai_autodetection_enabled(ctx);
        let result = AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .ai_autodetection_enabled_internal
                .set_value(enabled, ctx)
        });
        match result {
            Ok(()) => {
                record_autodetection_toggle_from_slash_command(enabled, ctx);
                let hint = if enabled {
                    NLD_ENABLED_HINT
                } else {
                    NLD_DISABLED_HINT
                };
                self.show_success_hint(hint.to_owned(), ctx);
            }
            Err(error) => {
                if enabled {
                    log::warn!("Failed to enable TUI natural language detection: {error}");
                } else {
                    log::warn!("Failed to disable TUI natural language detection: {error}");
                }
                self.show_transient_hint(NLD_PERSISTENCE_FAILED_HINT.to_owned(), ctx);
            }
        }
        record_static_slash_command_accepted(command_name, true, ctx);
    }

    /// Bridges shared shell-tool executor events into terminal-manager PTY intents.
    fn handle_shell_command_executor_event(
        &mut self,
        event: &ShellCommandExecutorEvent,
        model: &Arc<FairMutex<TerminalModel>>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ShellCommandExecutorEvent::ExecuteCommand { action_id, command } => {
                let Some((session_id, conversation_id)) = (|| {
                    let model = model.lock();
                    let session_id = model.block_list().active_block().session_id()?;
                    let conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation_id_for_action(action_id, ctx.view_id())?;
                    Some((session_id, conversation_id))
                })() else {
                    log::warn!(
                        "Unable to execute TUI agent-requested command for action {action_id:?}"
                    );
                    return;
                };

                ctx.emit(TuiTerminalSessionEvent::ExecuteCommand(Box::new(
                    ExecuteCommandEvent {
                        command: command.clone(),
                        session_id,
                        workflow_id: None,
                        workflow_command: None,
                        should_add_command_to_history: true,
                        source: CommandExecutionSource::AI {
                            metadata: AgentInteractionMetadata::new_hidden(
                                action_id.clone(),
                                conversation_id,
                            ),
                        },
                    },
                )));
            }
            ShellCommandExecutorEvent::WriteToPty { input, mode } => {
                ctx.emit(TuiTerminalSessionEvent::WriteAgentInput {
                    bytes: Cow::Owned(input.to_vec()),
                    mode: *mode,
                });
            }
            ShellCommandExecutorEvent::CancelExecution => {
                ctx.emit(TuiTerminalSessionEvent::InterruptPty);
            }
            ShellCommandExecutorEvent::TransferControlToUser {
                action_id: _,
                reason,
            } => {
                let reason = reason.clone();
                self.cli_subagent_controller.update(ctx, |controller, ctx| {
                    controller.switch_control_to_user(
                        UserTakeOverReason::TransferFromAgent { reason },
                        ctx,
                    );
                });
            }
        }
    }
}

impl Entity for TuiTerminalSessionView {
    type Event = TuiTerminalSessionEvent;
}

impl TuiView for TuiTerminalSessionView {
    fn ui_name() -> &'static str {
        "TuiTerminalSessionView"
    }

    fn child_view_ids(&self, _ctx: &AppContext) -> Vec<EntityId> {
        vec![
            self.transcript.id(),
            self.input_view.id(),
            self.orchestration_tab_bar.id(),
            self.attachment_bar.id(),
            self.zero_state_view.id(),
        ]
    }

    fn keymap_context(&self, ctx: &AppContext) -> keymap::Context {
        let mut context = Self::default_keymap_context();
        if self.orchestration_tabs_focused && self.input_target().agent_editor_owns_input() {
            context.set.insert(ORCHESTRATION_TAB_BAR_FOCUSED_FLAG);
        }
        if self.is_conversation_restore_loading() {
            context.set.insert(SESSION_CAN_CANCEL_RESTORE_FLAG);
        }
        if self.active_user_controlled_target(ctx).is_some() {
            context.set.insert(SESSION_CAN_HAND_BACK_CONTROL_FLAG);
        }
        if self.transcript.as_ref(ctx).has_toggleable_plan(ctx) {
            context.set.insert(PLAN_TOGGLE_AVAILABLE_FLAG);
        }
        if self.keyboard_enhancement_supported {
            context.set.insert(KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG);
        }
        if self.input_target().agent_editor_owns_input()
            && !self.suggestions_mode.as_ref(ctx).mode().is_visible()
        {
            context.set.insert(SESSION_COMPOSER_OWNS_INPUT_FLAG);
            if self.attachment_bar.as_ref(ctx).should_render(ctx) {
                context.set.insert(ATTACHMENTS_AVAILABLE_FLAG);
            }
        }
        context
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        match &self.conversation_restore_state {
            ConversationRestoreState::Loading {
                origin: TuiConversationRestoreOrigin::Startup,
                ..
            } => return conversation_restoring(ctx),
            ConversationRestoreState::Loading {
                origin: TuiConversationRestoreOrigin::ConversationList,
                ..
            } => {}
            ConversationRestoreState::Failed(message) => {
                return conversation_restore_failed(message);
            }
            ConversationRestoreState::Idle => {}
        }
        // While a full-screen (alt-screen) app is active, hand the whole pane to
        // it: render its grid and forward input, instead of the block UI.
        let (alt_screen_active, input_target, user_owns_running_command) = {
            let terminal_model = self.terminal_model.lock();
            (
                terminal_model.is_alt_screen_active(),
                tui_input_target(&terminal_model),
                inline_process_owns_input(&terminal_model),
            )
        };
        if alt_screen_active {
            return TuiTerminalContentElement::new(
                self.terminal_resize_tx.clone(),
                AltScreenElement::new(self.terminal_model.clone()).finish(),
            )
            .with_pty_input(self.terminal_model.clone())
            .finish();
        }

        let inline_menu = input_target
            .agent_editor_owns_input()
            .then(|| {
                active_inline_menu(
                    &self.inline_menus,
                    self.suggestions_mode.as_ref(ctx).mode(),
                    ctx,
                )
                .and_then(|menu| menu.render(ctx))
            })
            .flatten();
        let builder = TuiUiBuilder::from_app(ctx);
        let orchestration_tabs_available = self.orchestration_tab_bar.as_ref(ctx).has_tabs();

        // Ctrl-c (cancel/clear/exit) is handled by the keymap pass via the
        // fixed binding registered in [`Self::init`], so no element-level key
        // handling is needed here.
        //
        // While the transcript has nothing to show, the zero state fills its
        // slot; the first accepted submission produces a visible block, which
        // swaps the transcript back in.
        let mut content = TuiFlex::column();
        if self.transcript.as_ref(ctx).is_empty() {
            content = content.flex_child(TuiChildView::new(&self.zero_state_view).finish());
        } else {
            content = content.flex_child(TuiChildView::new(&self.transcript).finish());
        }

        // While a `RunAgents` card (or another blocking interaction) is the
        // active front-of-queue blocker, the input box, inline menus, normal
        // footer, and the warping/summary row are omitted; the blocker
        // renders its own action hints in their place. Visibility is derived
        // fresh each pass — no stored suppression flag — and the hidden
        // input model is never written to, so its draft/cursor/selection/
        // scroll survive untouched.
        let blocker_active = self.active_blocking_child(ctx).is_some();
        if !blocker_active && matches!(input_target, TuiInputTarget::Disabled) {
            content = content.child(
                TuiContainer::new(
                    TuiText::new(STARTING_SHELL_HINT)
                        .with_style(builder.muted_text_style())
                        .truncate()
                        .finish(),
                )
                .with_padding_top(1)
                .finish(),
            );
        }

        // While the selected conversation is in progress (the GUI warping
        // indicator's core condition), the animated warping indicator sits
        // between the transcript and the input box. Hide it while a process
        // owns input or a blocker is active: user takeover intentionally leaves
        // the conversation in progress, and blockers render their own status
        // and actions. Its elapsed counter is anchored to the latest exchange's
        // start so animation survives element-tree rebuilds; the conversation's
        // final status update re-renders the view without it.
        let selected_conversation = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx)
            .and_then(|conversation_id| {
                BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
            })
            .filter(|_| !blocker_active && input_target.agent_editor_owns_input());
        if let Some(conversation) = selected_conversation {
            if conversation.status().is_in_progress() {
                let warping_elapsed = conversation
                    .latest_exchange()
                    .and_then(|exchange| exchange.time_since_start());
                if let Some(elapsed) = warping_elapsed {
                    let label = if conversation.is_summarizing() {
                        "Summarizing conversation..."
                    } else {
                        "Warping..."
                    };
                    content = content.child(
                        TuiContainer::new(self.render_warping_indicator(
                            label,
                            elapsed,
                            conversation.id(),
                            ctx,
                        ))
                        .with_padding_top(1)
                        .finish(),
                    );
                }
            } else {
                // Once the response completes, the indicator's slot rests on
                // the last response's summary: `∷ {duration} • {credits}`.
                // Wall-to-wall duration is only available once the block's
                // final exchange finished, which also keeps the row hidden
                // for brand-new conversations.
                let wall_to_wall = conversation
                    .wall_to_wall_response_time_since_last_query()
                    .and_then(|ms| u64::try_from(ms).ok())
                    .map(Duration::from_millis);
                if let (Some(duration), Some(exchange_id)) = (
                    wall_to_wall,
                    conversation.latest_exchange().map(|exchange| exchange.id),
                ) && let Some(summary) = self.render_response_summary_for_exchange(
                    exchange_id,
                    duration,
                    conversation.credits_spent_for_last_block(),
                    ctx,
                ) {
                    content =
                        content.child(TuiContainer::new(summary).with_padding_top(1).finish());
                }
            }
        }
        // While a user-controlled long-running command owns input, the input
        // box and footer stay hidden; a one-line ghosted hint row takes the
        // input's slot so the interrupt affordance stays discoverable. Gated
        // on the user-controlled-command predicate, not the broader PTY input
        // target: visible startup-script execution also routes input to the
        // PTY but is not a command the user should be told to interrupt.
        // (Agent-driven terminal use keeps the composer, and its control
        // hints come from the CLI-subagent status line.)
        if !blocker_active && user_owns_running_command {
            content = content.child(
                TuiContainer::new(
                    TuiText::new(input_hints::LONG_RUNNING_COMMAND_HINT)
                        .with_style(builder.muted_text_style())
                        .truncate()
                        .finish(),
                )
                .with_padding_top(1)
                .finish(),
            );
        }
        if !blocker_active
            && (input_target.agent_editor_owns_input()
                || matches!(input_target, TuiInputTarget::Disabled))
        {
            if let (true, Some(menu)) = (input_target.agent_editor_owns_input(), inline_menu) {
                content = content.child(
                    TuiConstrainedBox::new(
                        TuiContainer::new(menu)
                            .with_padding_top(INLINE_MENU_TOP_PADDING_ROWS)
                            .finish(),
                    )
                    .with_max_rows(MAX_INLINE_MENU_ROWS + INLINE_MENU_TOP_PADDING_ROWS)
                    .finish(),
                );
            }
            let border_style = if self.is_shell_mode(ctx) {
                builder.shell_mode_accent_style()
            } else {
                builder.accent_border_style()
            };
            if self.attachment_bar.as_ref(ctx).should_render(ctx) {
                content = content.child(
                    TuiConstrainedBox::new(
                        TuiContainer::new(TuiChildView::new(&self.attachment_bar).finish())
                            .with_padding_x(1)
                            .finish(),
                    )
                    .with_max_rows(1)
                    .finish(),
                );
            }
            content = content.child(
                TuiConstrainedBox::new(
                    TuiContainer::new(TuiChildView::new(&self.input_view).finish())
                        .with_padding_x(1)
                        .with_border_style(border_style)
                        .finish(),
                )
                .with_max_rows(MAX_INPUT_TEXT_ROWS + 2)
                .finish(),
            );
            let footer = if matches!(input_target, TuiInputTarget::Disabled) {
                self.render_footer(ctx).finish()
            } else if self.orchestration_tabs_focused {
                self.render_orchestration_tab_footer(&builder)
            } else {
                self.render_footer(ctx).finish()
            };
            content = content.child(TuiConstrainedBox::new(footer).with_max_rows(1).finish());
        }
        let content = content.finish();
        let terminal_content =
            TuiTerminalContentElement::new(self.terminal_resize_tx.clone(), content);
        let terminal_content = if input_target.pty_owns_input() {
            terminal_content.with_pty_input(self.terminal_model.clone())
        } else {
            terminal_content
        };

        // The terminal-content wrapper sits inside the horizontal padding so
        // the PTY's columns match the width block content actually renders at
        // (the GUI wraps its view root, but its padding is sub-cell; here it is
        // 4 whole columns).
        let session = TuiContainer::new(terminal_content.finish())
            .with_padding_x(2)
            .with_padding_top(2)
            .with_padding_bottom(1)
            .finish();
        if orchestration_tabs_available {
            TuiFlex::column()
                .child(TuiChildView::new(&self.orchestration_tab_bar).finish())
                .flex_child(session)
                .finish()
        } else {
            session
        }
    }
}

impl TuiTerminalSessionView {
    fn handle_typeahead_event(&mut self, ctx: &mut ViewContext<Self>) {
        let typeahead = self.terminal_model.lock().take_typeahead_for_input();
        if let Some((text, previously_inserted)) = typeahead {
            self.input_view.update(ctx, |input, ctx| {
                input.insert_typeahead_text(previously_inserted, &text, ctx);
            });
        }
        ctx.notify();
    }
}

impl TypedActionView for TuiTerminalSessionView {
    type Action = TuiTerminalSessionAction;

    fn handle_action(&mut self, action: &TuiTerminalSessionAction, ctx: &mut ViewContext<Self>) {
        match action {
            TuiTerminalSessionAction::Interrupt => self.handle_interrupt(ctx),
            TuiTerminalSessionAction::Eof => self.handle_eof(ctx),
            TuiTerminalSessionAction::CancelRestore => {
                self.cancel_conversation_restore(ctx);
            }
            TuiTerminalSessionAction::HandBackTerminalUseControl => {
                self.hand_back_terminal_use_control(ctx)
            }
            TuiTerminalSessionAction::ToggleUsageDisplay => self.toggle_usage_display(ctx),
            TuiTerminalSessionAction::ToggleResponseSummaryVisibility => {
                self.toggle_response_summary_visibility(ctx)
            }
            TuiTerminalSessionAction::ToggleModelMenu => self.toggle_model_menu(ctx),
            TuiTerminalSessionAction::ToggleAutoApprove { show_feedback } => {
                self.toggle_auto_approve(*show_feedback, ctx)
            }
            TuiTerminalSessionAction::FocusDefaultInteractionTarget => {
                self.set_orchestration_tab_focus(false, ctx)
            }
            TuiTerminalSessionAction::FocusMainOrchestrationTab => {
                let main_tab_key = self.orchestration_tab_bar.as_ref(ctx).main_tab_key();
                if let Some(key) = main_tab_key {
                    self.switch_to_orchestration_tab(Some(key), false, ctx);
                } else {
                    self.set_orchestration_tab_focus(false, ctx);
                }
            }
            TuiTerminalSessionAction::NavigateOrchestrationTabs(action) => {
                let key = action.target(self.orchestration_tab_bar.as_ref(ctx));
                self.switch_to_orchestration_tab(key, true, ctx);
            }
            TuiTerminalSessionAction::ForwardUserPtyBytes(bytes) => {
                // Raw passthrough: the bytes are already the app's escape
                // sequence, so write them to the PTY unmodified.
                ctx.emit(TuiTerminalSessionEvent::WriteUserInput(Cow::Owned(
                    bytes.clone(),
                )));
            }
            TuiTerminalSessionAction::TogglePlan => {
                self.transcript
                    .update(ctx, |transcript, ctx| transcript.toggle_latest_plan(ctx));
            }
            TuiTerminalSessionAction::FocusAttachments => {
                if self.attachment_bar.as_ref(ctx).should_render(ctx) {
                    ctx.focus(&self.attachment_bar);
                }
            }
            TuiTerminalSessionAction::PasteFromClipboard => {
                self.attachment_bar
                    .update(ctx, |bar, ctx| bar.paste_from_clipboard(ctx));
            }
        }
    }
}

impl TerminalSurface for TuiTerminalSessionView {
    fn on_shell_determined(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn on_pty_spawn_failed(&mut self, error: anyhow::Error, ctx: &mut ViewContext<Self>) {
        report_error!(error.context("TUI PTY spawn failed"));
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "terminal_session_view_tests.rs"]
mod tests;
