//! Authenticated terminal-session TUI surface.
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use ai::LLMProvider;
use async_channel::Sender;
use chrono::{Local, NaiveDateTime};
use instant::Instant;
use parking_lot::FairMutex;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::settings::{
    AISettings, AISettingsChangedEvent, TuiStatuslineConfig, TuiStatuslineItem, TuiTheme,
    TuiThemeSettings,
};
use warp::tui_export::{
    AIAgentActionId, AIAgentActionResultType, AIAgentContext, AIAgentExchangeId,
    AIAgentPtyWriteMode, AIConversation, AIConversationId, AcceptSlashCommandOrSavedPrompt,
    ActiveSession, ActiveSessionEvent, AgentConversationEntryId, AgentConversationListEntryState,
    AgentConversationsModel, AgentInteractionMetadata, AgentViewEntryOrigin, Appearance, BlockId,
    BlocklistAIActionEvent, BlocklistAIActionModel, BlocklistAIContextModel, BlocklistAIController,
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, BlocklistAIInputModel, CLISubagentController,
    CLISubagentEvent, CLISubagentTarget, COMMAND_REGISTRY, CancellationReason, ChangelogModel,
    ChangelogRequestType, CloudConversationData, CommandExecutionSource, ConversationFileExport,
    ConversationSelection, ConversationSelectionHandle, ConversationUsageTotals,
    ExecuteCommandEvent, GetRelevantFilesController, GitRepoModels, GitRepoStatusModel,
    GitStatusMetadata, LLMId, LLMPreferences, LLMPreferencesEvent,
    LOCAL_SKILLS_REMOTE_EXECUTION_ERROR_MESSAGE, LinkedWorkflowData, ModelEvent,
    ParsedSlashCommandInput, PersistenceWriter, PtyIntent, PtyIntentEvent, QueuedQueryEvent,
    QueuedQueryModel, RepoDetectionSessionType, RepoDetectionSource, ServerConversationToken,
    Sessions, ShellCommandExecutorEvent, SizeInfo, SizeUpdate, SkillReference,
    SlashCommandDataSource as _, SlashCommandKind, SlashCommandSelectionBehavior,
    StartAgentExecutorEvent, StartAgentRequest, StaticCommand, TerminalModel, TerminalSurface,
    TerminalSurfaceInit, TranscriptScope, TuiMcpAction, TuiMcpManager, TuiSlashCommandDataSource,
    TuiSlashCommandDataSourceArgs, TuiUpArrowHistoryItemKind, TuiUserInfoManager,
    TuiUserInfoManagerEvent, TuiZeroStateDataSource, UserTakeOverReason, WAKEUP_THROTTLE_PERIOD,
    block_context_from_terminal_model, build_slash_command_mixer, detect_possible_git_repo,
    export_conversation_markdown, log_out_tui, maybe_build_ai_query_upsert_event,
    prepare_conversation_block_restoration, record_autodetection_toggle_from_slash_command,
    record_saved_prompt_accepted, record_static_slash_command_accepted, saved_prompt_text_for_id,
    slash_command_selection_behavior, slash_commands, throttle,
};
use warp_core::channel::{Channel, ChannelState};
use warp_core::features::FeatureFlag;
use warp_core::settings::Setting;
use warp_editor::model::CoreEditorModel;
use warp_errors::report_error;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::SingletonEntity;
use warpui_core::r#async::{SpawnedFutureHandle, Timer};
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{
    TuiAnimated, TuiChildView, TuiConstrainedBox, TuiContainer, TuiElement, TuiFlex, TuiHoverable,
    TuiSelectionHandle, TuiSize, TuiStyle, TuiText, text_width,
};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, EditableBinding, FixedBinding};
use warpui_core::platform::TerminationMode;
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::alt_screen_view::AltScreenElement;
use crate::attachment_bar::{
    FOCUS_ATTACHMENTS_BINDING_NAME, TuiAttachmentBar, TuiAttachmentBarEvent, TuiAttachmentModel,
    TuiAttachmentPasteDisposition,
};
use crate::cli_agent_osc_event_publisher::{
    CliAgentOscEventPublisher, host_supports_cli_agent_notifications,
};
use crate::clipboard::copy_to_clipboard;
use crate::completion_menu::TuiCompletionMenuModel;
use crate::conversation_menu::{TuiConversationMenuEvent, TuiConversationMenuModel};
use crate::conversation_selection::TuiConversationSelection;
use crate::editor_interaction::TuiEditorCommand;
use crate::exit_confirmation::{CTRL_C_EXIT_WINDOW, ExitConfirmation};
use crate::grok_oauth::TuiGrokOAuthBlock;
use crate::handoff::TuiHandoffBlock;
use crate::inline_menu::{MAX_INLINE_MENU_ROWS, TuiInlineMenu, active_inline_menu};
use crate::input::view::TuiInputAction;
use crate::input::{TuiInputView, TuiInputViewEvent};
use crate::input_hints;
use crate::input_mode_policy::{self, TuiInputModePolicy};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::keybindings::{
    ATTACHMENTS_AVAILABLE_FLAG, CONTEXTUAL_PLAN_TOGGLE_BINDING_NAME,
    KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG, PLAN_TOGGLE_AVAILABLE_FLAG, PLAN_TOGGLE_BINDING_NAME,
    TUI_BINDING_GROUP, binding_hint,
};
use crate::mcp_menu::{TuiMcpMenuEvent, TuiMcpMenuModel};
use crate::model_menu::{TuiModelMenuEvent, TuiModelMenuModel};
use crate::orchestration_block::TuiOrchestrationBlock;
use crate::orchestration_model::{TuiOrchestrationModel, TuiOrchestrationSnapshot};
use crate::orchestration_tab_bar::{
    ORCHESTRATION_TAB_BAR_FOCUSED_FLAG, TuiOrchestrationTabNavigationAction,
    orchestration_tab_bar_config, register_orchestration_surface_bindings,
    render_orchestration_child_selected_tab_footer, render_orchestration_tab_footer,
};
use crate::platform::reveal_path_in_file_manager;
use crate::prompt_and_command_history_menu::{
    TuiPromptAndCommandHistoryMenuEvent, TuiPromptAndCommandHistoryMenuModel,
};
use crate::read_only_menu::TuiReadOnlyMenuKind;
use crate::resume::TuiExitSummaryHandle;
use crate::session_registry::TuiSessions;
use crate::skills_menu::{TuiSkillMenuEvent, TuiSkillMenuModel};
use crate::slash_commands::TuiSlashCommandModel;
use crate::statusline_config_view::{TuiStatuslineConfigEvent, TuiStatuslineConfigView};
use crate::tab_bar::{TuiTabBarConfig, TuiTabBarEvent, TuiTabBarView};
use crate::terminal_background::probed_colors;
use crate::terminal_content_element::TuiTerminalContentElement;
use crate::terminal_use::{
    TerminalUseInterruptAction, TuiInputTarget, hide_agent_requested_command_from_top_level,
    terminal_use_conversation_to_resume, terminal_use_interrupt_action, tui_input_target,
};
use crate::transcript_view::{TuiTranscriptView, TuiTranscriptViewEvent};
use crate::transient_hint::{TransientHint, TransientHintTone};
use crate::tui_ask_question_view::TuiAskQuestionView;
use crate::tui_builder::TuiUiBuilder;
use crate::tui_cli_subagent_view::{HAND_BACK_KEY_BINDING, TuiCLISubagentView};
use crate::tui_permission_prompt::TuiPermissionPrompt;
use crate::ui::{
    WorkingDirectoryLabel, abbreviate_home_prefix, conversation_restore_failed,
    conversation_restoring,
};
use crate::usage::UsageToggle;
use crate::voice_input::{TuiVoiceInputEvent, TuiVoiceInputState, VoiceInputStartSource};
use crate::warping_indicator::{render_response_summary, render_warping_indicator_row};
use crate::zero_state::TuiZeroStateView;
use crate::zero_state_animation::{
    ZeroStateAnimationConfig, ZeroStateAnimationConfigEvent, ZeroStateAnimationLoadFailure,
};
mod completions;

#[path = "grok_oauth/session.rs"]
mod grok_oauth_session;
#[path = "handoff/session.rs"]
mod handoff_session;
mod input_detection;
mod shortcuts;
pub(crate) mod state;
mod status_menu;
use self::completions::CompletionRequestState;
use self::input_detection::InputDetectionState;
use self::state::{
    TuiTerminalSessionState, TuiTerminalSessionStateModel, TuiTerminalSessionStateResolveError,
};

/// Width used before the first layout pass pushes the real terminal width into the editor.
const INITIAL_INPUT_WIDTH: u16 = 80;
const INLINE_MENU_TOP_PADDING_ROWS: u16 = 1;
const MAX_INPUT_TEXT_ROWS: u16 = 6;
const AUTO_APPROVE_FEEDBACK_DURATION: Duration = Duration::from_secs(3);
const STATUSLINE_DATETIME_REPAINT_INTERVAL: Duration = Duration::from_secs(60);
const VOICE_INPUT_BORDER_REPAINT_INTERVAL: Duration = Duration::from_millis(33);

/// The footer hint shown while the ctrl-c exit confirmation is armed.
const CTRL_C_EXIT_HINT: &str = "ctrl-c again to exit";
const RUNNING_COMMAND_DETACH_HINT: &str = "ctrl-c to return to command";
/// The footer hint shown when the ctrl-c kill-child window is armed.
/// Replaces the exit hint when viewing a child agent conversation.
pub(crate) const CTRL_C_KILL_CHILD_HINT: &str = "ctrl-c again to kill child agent";
const STARTING_SHELL_HINT: &str = "Starting shell...";

/// Fallback strings for the /status status menu.
const STATUS_UNAVAILABLE: &str = "\u{2014}"; // em dash
const STATUS_UNTITLED_SESSION: &str = "Untitled";
const STATUS_DEV_BUILD: &str = "dev build";
const STATUS_NOT_SIGNED_IN: &str = "Not signed in";
const STATUS_SIGNED_IN: &str = "Signed in";

fn status_menu_is_open(mode: TuiInputSuggestionsMode) -> bool {
    matches!(
        mode,
        TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Status)
    )
}
const SESSION_CAN_CANCEL_RESTORE_FLAG: &str = "TuiSessionCanCancelRestore";
const SESSION_CAN_HAND_BACK_CONTROL_FLAG: &str = "TuiSessionCanHandBackControl";
const SESSION_CAN_ATTACH_AGENT_TO_RUNNING_COMMAND_FLAG: &str =
    "TuiSessionCanAttachAgentToRunningCommand";
const SESSION_CAN_DETACH_AGENT_FROM_RUNNING_COMMAND_FLAG: &str =
    "TuiSessionCanDetachAgentFromRunningCommand";
const SESSION_CAN_ACCEPT_BLOCKED_TERMINAL_USE_ACTION_FLAG: &str =
    "TuiSessionCanAcceptBlockedTerminalUseAction";
pub(crate) const SESSION_COMPOSER_OWNS_INPUT_FLAG: &str = "TuiSessionComposerOwnsInput";
pub(crate) const PASTE_IMAGE_BINDING_NAME: &str = "tui:session:paste_image";
pub(crate) const AUTO_APPROVE_TOGGLE_BINDING_NAME: &str = "tui:session:toggle_auto_approve";
pub(crate) const ACCEPT_BLOCKED_TERMINAL_USE_ACTION_BINDING_NAME: &str =
    "tui:session:accept_blocked_terminal_use_action";
pub(crate) const ATTACH_AGENT_TO_RUNNING_COMMAND_BINDING_NAME: &str =
    "tui:session:attach_agent_to_running_command";
pub(crate) const DETACH_AGENT_FROM_RUNNING_COMMAND_BINDING_NAME: &str =
    "tui:session:detach_agent_from_running_command";
pub(crate) const VOICE_INPUT_BINDING_NAME: &str = "tui:session:start_voice_input";

/// The current source preventing the normal session composer from owning input.
#[derive(Clone)]
pub(super) enum BlockingInputSource {
    /// A user-controlled long-running terminal command.
    LongRunningCommand,
    /// An ask-user-question questionnaire.
    AskQuestion(ViewHandle<TuiAskQuestionView>),
    /// A standard Yes/No/Other permission request.
    Permission(ViewHandle<TuiPermissionPrompt>),
    /// The specialized orchestration configuration request.
    Orchestration(ViewHandle<TuiOrchestrationBlock>),
    /// A local-to-cloud handoff configuration or result card.
    Handoff(ViewHandle<TuiHandoffBlock>),
    /// An in-process Grok OAuth connection flow.
    GrokOAuth(ViewHandle<TuiGrokOAuthBlock>),
}

impl BlockingInputSource {
    fn is_interactive(&self) -> bool {
        !matches!(self, Self::LongRunningCommand)
    }

    fn view_element(self) -> Option<Box<dyn TuiElement>> {
        match self {
            Self::LongRunningCommand => None,
            Self::AskQuestion(view) => Some(TuiChildView::new(&view).finish()),
            Self::Permission(view) => Some(TuiChildView::new(&view).finish()),
            Self::Orchestration(view) => Some(TuiChildView::new(&view).finish()),
            Self::Handoff(view) => Some(TuiChildView::new(&view).finish()),
            Self::GrokOAuth(view) => Some(TuiChildView::new(&view).finish()),
        }
    }
}
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

fn zero_state_ascii_load_failure_hint(failure: ZeroStateAnimationLoadFailure) -> &'static str {
    match failure {
        ZeroStateAnimationLoadFailure::InitialLoad => ZERO_STATE_ASCII_INITIAL_LOAD_FAILED_HINT,
        ZeroStateAnimationLoadFailure::Reload => ZERO_STATE_ASCII_RELOAD_FAILED_HINT,
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
const THEME_INVALID_ARGUMENT_HINT: &str = "Theme must be auto, light, or dark.";

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
const THEME_PERSISTENCE_FAILED_HINT: &str = "Could not save the theme setting.";
const ZERO_STATE_ASCII_INITIAL_LOAD_FAILED_HINT: &str =
    "Could not load custom ASCII art. Using the built-in Warp logo.";
const ZERO_STATE_ASCII_RELOAD_FAILED_HINT: &str =
    "Could not reload custom ASCII art. Keeping the current object.";
const VOICE_USAGE_HINT: &str = "Usage: /voice (no arguments)";
const AUTO_APPROVE_ENABLED_HINT: &str = "Auto approve on";
const AUTO_APPROVE_DISABLED_HINT: &str = "Auto approve off";
const STATUSLINE_SAVED_HINT: &str = "Statusline configuration saved.";
const STATUSLINE_PERSISTENCE_FAILED_HINT: &str = "Could not save the statusline configuration.";
const COST_NO_ACTIVE_CONVERSATION_HINT: &str =
    "Cannot show conversation cost: no active conversation";
const COST_EMPTY_CONVERSATION_HINT: &str = "Cannot show conversation cost: conversation is empty";
const COST_CONVERSATION_IN_PROGRESS_HINT: &str =
    "Cannot show conversation cost: conversation is in progress";

struct FooterHint<'a> {
    text: &'a str,
    style: FooterHintStyle,
}

enum FooterHintStyle {
    Muted,
    Success,
    Error,
    VoiceInput,
}

impl<'a> FooterHint<'a> {
    fn muted(text: &'a str) -> Self {
        Self {
            text,
            style: FooterHintStyle::Muted,
        }
    }

    fn voice_input(text: &'a str) -> Self {
        Self {
            text,
            style: FooterHintStyle::VoiceInput,
        }
    }

    fn render(self, builder: &TuiUiBuilder) -> TuiFlex {
        let style = match self.style {
            FooterHintStyle::Muted => builder.muted_text_style(),
            FooterHintStyle::Success => builder.success_glyph_style(),
            FooterHintStyle::Error => builder.error_text_style(),
            FooterHintStyle::VoiceInput => builder.voice_input_status_style(),
        };
        TuiFlex::row().child(
            TuiText::new(self.text)
                .with_style(style)
                .truncate()
                .finish(),
        )
    }
}

fn log_bundle_success_message(path: &Path) -> String {
    format!("Log bundle saved to {}", path.display())
}

/// Shell command that invokes the TUI for the current build channel.
///
/// Local builds run through the repository script so they select and build the
/// local TUI binary; installed builds invoke their channel-specific executable.
fn tui_cli_shell_command(channel: Channel, arguments: &str) -> String {
    let launcher = match channel {
        Channel::Local => "./script/run-tui --",
        Channel::Stable => "warp",
        Channel::Dev => "warp-dev",
        Channel::Preview => "warp-preview",
        Channel::Oss => "warp-oss",
        Channel::Integration => "warp-integration",
    };
    format!("{launcher} {arguments}")
}

/// Shell command used by the exit hint to resume a server conversation.
pub(crate) fn tui_resume_shell_command(channel: Channel, token: &str) -> String {
    tui_cli_shell_command(channel, &format!("--resume {token}"))
}

#[derive(Clone, Copy)]
enum ProviderApiKeyOperation {
    Set,
    Clear,
}

fn provider_api_key_shell_command(
    channel: Channel,
    provider: LLMProvider,
    operation: ProviderApiKeyOperation,
) -> Option<String> {
    if !provider.supports_pasted_api_key() {
        return None;
    }
    let provider = provider.api_key_slug()?;
    let flag = match operation {
        ProviderApiKeyOperation::Set => "--set-provider-api-key",
        ProviderApiKeyOperation::Clear => "--clear-provider-api-key",
    };
    Some(tui_cli_shell_command(
        channel,
        &format!("{flag} {provider}"),
    ))
}

fn raw_prompt_if_not_blank(input: &str) -> Option<&str> {
    (!input.trim().is_empty()).then_some(input)
}
fn format_context_window_usage(usage: f32) -> String {
    format!("{:.0}% context used", usage * 100.0)
}
fn format_statusline_date(now: NaiveDateTime) -> String {
    now.format("%B %-d, %Y").to_string()
}
fn format_statusline_time_12_hour(now: NaiveDateTime) -> String {
    now.format("%-I:%M%P").to_string()
}
fn format_statusline_time_24_hour(now: NaiveDateTime) -> String {
    now.format("%H:%M").to_string()
}
fn render_statusline_datetime(
    formatter: fn(NaiveDateTime) -> String,
    style: TuiStyle,
) -> Box<dyn TuiElement> {
    TuiAnimated::new(STATUSLINE_DATETIME_REPAINT_INTERVAL, move || {
        TuiText::new(formatter(Local::now().naive_local()))
            .with_style(style)
            .truncate()
            .finish()
    })
    .finish()
}
fn format_todo_progress(completed: usize, total: usize, finished: bool) -> String {
    let marker = if finished { "✓" } else { "❒" };
    format!("{marker} {completed}/{total}")
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

fn attachment_focus_available(is_shell_mode: bool, attachments_should_render: bool) -> bool {
    !is_shell_mode && attachments_should_render
}

fn voice_command_argument(input: &str) -> Option<&str> {
    let argument = input.strip_prefix(slash_commands::VOICE.name)?;
    argument
        .chars()
        .next()
        .is_none_or(char::is_whitespace)
        .then_some(argument)
}

fn voice_argument_is_empty(argument: Option<&String>) -> bool {
    argument.is_none_or(|argument| argument.trim().is_empty())
}

fn bordered_input(
    input_view: &ViewHandle<TuiInputView>,
    border_style: TuiStyle,
) -> Box<dyn TuiElement> {
    TuiContainer::new(TuiChildView::new(input_view).finish())
        .with_padding_x(1)
        .with_border_style(border_style)
        .finish()
}

/// One resolved item in the footer's configured presentation order. Segments
/// built from an already-rendered element carry the element's display width so
/// the width-aware working-directory segment can reserve room for the segments
/// that follow it (see [`FooterSegment::content_width`]).
enum FooterSegment {
    ShellMode,
    ActiveIndicator(&'static str),
    Model(Box<dyn TuiElement>, u16),
    WorkingDirectory { path: String, reserved: u16 },
    GitBranch(String),
    CreditUsage(Box<dyn TuiElement>, u16),
    ContextWindowUsage(String),
    GitDiff { additions: usize, deletions: usize },
    GitBranchStatus(String),
    DateTime(Box<dyn TuiElement>, u16),
    AgentTodoList(String),
    VoiceInput(Box<dyn TuiElement>, u16),
}

impl FooterSegment {
    fn separator_to(&self, next: &Self) -> &'static str {
        match (self, next) {
            (Self::ShellMode | Self::Model(_, _), Self::WorkingDirectory { .. }) => " ",
            (Self::WorkingDirectory { .. }, Self::GitBranch(_)) => " ⊢ ",
            (Self::ActiveIndicator(_), Self::ActiveIndicator(_)) => " • ",
            (
                Self::WorkingDirectory { .. } | Self::GitBranch(_),
                Self::WorkingDirectory { .. } | Self::GitBranch(_),
            )
            | (Self::DateTime(_, _), Self::DateTime(_, _))
            | (Self::ShellMode, _)
            | (_, Self::ShellMode) => " • ",
            (
                Self::ActiveIndicator(_)
                | Self::Model(_, _)
                | Self::WorkingDirectory { .. }
                | Self::GitBranch(_)
                | Self::CreditUsage(_, _)
                | Self::ContextWindowUsage(_)
                | Self::GitDiff { .. }
                | Self::GitBranchStatus(_)
                | Self::DateTime(_, _)
                | Self::AgentTodoList(_)
                | Self::VoiceInput(_, _),
                Self::ActiveIndicator(_)
                | Self::Model(_, _)
                | Self::WorkingDirectory { .. }
                | Self::GitBranch(_)
                | Self::CreditUsage(_, _)
                | Self::ContextWindowUsage(_)
                | Self::GitDiff { .. }
                | Self::GitBranchStatus(_)
                | Self::DateTime(_, _)
                | Self::AgentTodoList(_)
                | Self::VoiceInput(_, _),
            ) => " | ",
        }
    }

    /// The display-cell width this segment occupies when rendered. Used to
    /// reserve room for the segments after the working directory so eliding
    /// the cwd never crowds them out. The working directory itself is
    /// width-aware and reports its natural (unelided) width here, which the
    /// reservation logic ignores.
    fn content_width(&self) -> u16 {
        match self {
            Self::ShellMode => text_width(SHELL_MODE_HINT),
            Self::ActiveIndicator(label) => text_width(label),
            Self::Model(_, width)
            | Self::CreditUsage(_, width)
            | Self::DateTime(_, width)
            | Self::VoiceInput(_, width) => *width,
            Self::WorkingDirectory { path, .. } => text_width(path),
            Self::GitBranch(value)
            | Self::ContextWindowUsage(value)
            | Self::GitBranchStatus(value)
            | Self::AgentTodoList(value) => text_width(value),
            Self::GitDiff {
                additions,
                deletions,
            } => {
                let mut width: u16 = 0;
                if *additions > 0 {
                    width = width.saturating_add(text_width(&format!("+{additions}")));
                }
                if *deletions > 0 {
                    if *additions > 0 {
                        // The space child rendered between `+n` and `-n`.
                        width = width.saturating_add(1);
                    }
                    width = width.saturating_add(text_width(&format!("-{deletions}")));
                }
                width
            }
        }
    }
}

/// Records how many columns the working-directory segment must leave for the
/// segments rendered after it, so it only elides the cwd when the row runs out
/// of room. The reservation is the total width of every trailing segment plus
/// the separators from the cwd onward.
fn set_working_directory_reserved_width(segments: &mut [FooterSegment]) {
    let Some(cwd_index) = segments
        .iter()
        .position(|segment| matches!(segment, FooterSegment::WorkingDirectory { .. }))
    else {
        return;
    };
    let mut reserved: u16 = 0;
    for index in cwd_index..segments.len() {
        if let Some(next) = segments.get(index + 1) {
            reserved = reserved.saturating_add(text_width(segments[index].separator_to(next)));
        }
        if index > cwd_index {
            reserved = reserved.saturating_add(segments[index].content_width());
        }
    }
    if let Some(FooterSegment::WorkingDirectory { reserved: slot, .. }) =
        segments.get_mut(cwd_index)
    {
        *slot = reserved;
    }
}

/// Resolved segments for the footer's left-aligned status row.
struct FooterSegments {
    ordered: Vec<FooterSegment>,
}
/// Builds the status row from resolved segments. Working directory follows a
/// leading shell-mode or model label with a plain space; an immediately
/// following branch uses ` ⊢ ` as the relationship marker. Items in
/// different Figma groups use ` | `; other adjacent pairs use ` • `. The first
/// item never receives a separator.
fn render_status_footer_row(segments: FooterSegments, builder: &TuiUiBuilder) -> TuiFlex {
    let muted = builder.muted_text_style();
    let mut row = TuiFlex::row();
    let mut segments = segments.ordered.into_iter().peekable();
    while let Some(segment) = segments.next() {
        let separator = segments.peek().map(|next| segment.separator_to(next));
        match segment {
            FooterSegment::ShellMode => {
                row = row.child(
                    TuiText::new(SHELL_MODE_HINT)
                        .with_style(builder.shell_command_accent_style())
                        .truncate()
                        .finish(),
                );
            }
            FooterSegment::ActiveIndicator(label) => {
                row = row.child(
                    TuiText::new(label)
                        .with_style(builder.success_glyph_style())
                        .truncate()
                        .finish(),
                );
            }
            FooterSegment::Model(element, _)
            | FooterSegment::CreditUsage(element, _)
            | FooterSegment::DateTime(element, _)
            | FooterSegment::VoiceInput(element, _) => {
                row = row.child(element);
            }
            FooterSegment::WorkingDirectory { path, reserved } => {
                row = row.child(WorkingDirectoryLabel::new(path, muted, reserved).finish());
            }
            FooterSegment::GitBranch(cwd) => {
                row = row.child(TuiText::new(cwd).with_style(muted).truncate().finish());
            }
            FooterSegment::ContextWindowUsage(usage) => {
                row = row.child(TuiText::new(usage).with_style(muted).truncate().finish());
            }
            FooterSegment::GitBranchStatus(value) | FooterSegment::AgentTodoList(value) => {
                row = row.child(TuiText::new(value).with_style(muted).truncate().finish());
            }
            FooterSegment::GitDiff {
                additions,
                deletions,
            } => {
                if additions > 0 {
                    row = row.child(
                        TuiText::new(format!("+{additions}"))
                            .with_style(builder.diff_added_style())
                            .truncate()
                            .finish(),
                    );
                }
                if deletions > 0 {
                    if additions > 0 {
                        row = row.child(TuiText::new(" ").truncate().finish());
                    }
                    row = row.child(
                        TuiText::new(format!("-{deletions}"))
                            .with_style(builder.diff_removed_style())
                            .truncate()
                            .finish(),
                    );
                }
            }
        }
        if let Some(separator) = separator {
            row = row.child(
                TuiText::new(separator)
                    .with_style(muted)
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

/// Resolves the Email field for the `/status` menu using the same fallback
/// chain as `render_login_line`:
/// 1. `email` when non-empty
/// 2. `username` when non-empty (display name or email fallback from auth)
/// 3. `STATUS_SIGNED_IN` when `is_logged_in` is true but no identifier
/// 4. `STATUS_NOT_SIGNED_IN` when fully logged out
fn resolve_status_email(
    email: Option<String>,
    username: Option<String>,
    is_logged_in: bool,
) -> String {
    email
        .filter(|e| !e.is_empty())
        .or_else(|| username.filter(|u| !u.is_empty()))
        .unwrap_or_else(|| {
            if is_logged_in {
                STATUS_SIGNED_IN.to_owned()
            } else {
                STATUS_NOT_SIGNED_IN.to_owned()
            }
        })
}

fn format_status_conversation_id(conversation_id: Option<AIConversationId>) -> String {
    conversation_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "None".to_owned())
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
    /// Show the agent composer for an eligible user-started running command.
    AttachAgentToRunningCommand,
    /// Hide the composer before submission and return input to the running command.
    DetachAgentFromRunningCommand,
    /// Accept the active terminal-use agent's blocked action.
    AcceptBlockedTerminalUseAction,
    /// Reject the active terminal-use agent's blocked action.
    RejectBlockedTerminalUseAction,
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
    /// Start recording voice input from the session composer.
    StartVoiceInput,
    /// Left-click on the inline menu at absolute snapshot index `index`:
    /// selects and accepts that row.
    InlineMenuMouseAcceptRow(usize),
    /// Scroll-wheel over the inline menu: scrolls the viewport by `delta` rows
    /// without changing the selection.
    InlineMenuMouseScrollBy(isize),
    /// Start or stop voice input from the configured statusline control.
    ToggleVoiceInput,
    /// A drag selection started inside the shared read-only menu.
    ReadOnlyMenuSelectionStarted,
    /// A non-empty read-only menu selection completed.
    ReadOnlyMenuSelectionEnded(String),
}

/// The authenticated terminal/session surface rendered inside [`RootTuiView`].
pub(crate) struct TuiTerminalSessionView {
    transcript: ViewHandle<TuiTranscriptView>,
    input_view: ViewHandle<TuiInputView>,
    attachment_bar: ViewHandle<TuiAttachmentBar>,
    inline_menus: Vec<TuiInlineMenu>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    read_only_menu_selection: TuiSelectionHandle,
    /// Session-owned live state model shared by this surface and its input view.
    session_state: ModelHandle<TuiTerminalSessionStateModel>,
    conversation_menu: ModelHandle<TuiConversationMenuModel>,
    model_menu: ModelHandle<TuiModelMenuModel>,
    skills_menu: ModelHandle<TuiSkillMenuModel>,
    mcp_menu: ModelHandle<TuiMcpMenuModel>,
    completion_menu: ModelHandle<TuiCompletionMenuModel>,
    slash_commands_source: ModelHandle<TuiSlashCommandDataSource>,
    conversation_selection: ConversationSelectionHandle,
    ai_action_model: ModelHandle<BlocklistAIActionModel>,
    cli_agent_osc_event_publisher: Option<ModelHandle<CliAgentOscEventPublisher>>,
    ai_controller: ModelHandle<BlocklistAIController>,
    cli_subagent_controller: ModelHandle<CLISubagentController>,
    cli_subagent_views: HashMap<BlockId, ViewHandle<TuiCLISubagentView>>,
    /// Read by the footer for the active session's working directory.
    active_session: ModelHandle<ActiveSession>,
    sessions: ModelHandle<Sessions>,
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
    /// Hover and click state for the configured Voice statusline control.
    voice_input_mouse: MouseStateHandle,
    keyboard_enhancement_supported: bool,
    ai_context_model: ModelHandle<BlocklistAIContextModel>,
    ai_input_model: ModelHandle<BlocklistAIInputModel>,
    input_detection: InputDetectionState,
    completion_request: CompletionRequestState,
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
    handoff: Option<ViewHandle<TuiHandoffBlock>>,
    grok_oauth: Option<ViewHandle<TuiGrokOAuthBlock>>,
    statusline_config_view: Option<ViewHandle<TuiStatuslineConfigView>>,
    orchestration_tab_bar: ViewHandle<TuiTabBarView>,
    orchestration_tabs_focused: bool,
    /// When set, the `exit_confirmation` window was armed to kill this child
    /// rather than exit the TUI. The footer shows [`CTRL_C_KILL_CHILD_HINT`]
    /// while armed, and a second ctrl-c within the window kills the child.
    child_kill_armed_conversation: Option<AIConversationId>,
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
            ATTACH_AGENT_TO_RUNNING_COMMAND_BINDING_NAME,
            "Use the agent with the running command",
            TuiTerminalSessionAction::AttachAgentToRunningCommand,
        )
        .with_context_predicate(
            (id!(TuiInputView::ui_name()) | view_context.clone())
                & id!(SESSION_CAN_ATTACH_AGENT_TO_RUNNING_COMMAND_FLAG),
        )
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-shift-enter"),
        EditableBinding::new(
            DETACH_AGENT_FROM_RUNNING_COMMAND_BINDING_NAME,
            "Return control to the running command",
            TuiTerminalSessionAction::DetachAgentFromRunningCommand,
        )
        .with_context_predicate(
            (id!(TuiInputView::ui_name()) | view_context.clone())
                & id!(SESSION_CAN_DETACH_AGENT_FROM_RUNNING_COMMAND_FLAG),
        )
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("escape"),
        EditableBinding::new(
            ACCEPT_BLOCKED_TERMINAL_USE_ACTION_BINDING_NAME,
            "Accept the blocked terminal-use action",
            TuiTerminalSessionAction::AcceptBlockedTerminalUseAction,
        )
        .with_context_predicate(
            (id!(TuiInputView::ui_name()) | view_context.clone())
                & id!(SESSION_CAN_ACCEPT_BLOCKED_TERMINAL_USE_ACTION_FLAG),
        )
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-enter"),
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
        EditableBinding::new(
            VOICE_INPUT_BINDING_NAME,
            "Start voice input",
            TuiTerminalSessionAction::StartVoiceInput,
        )
        .with_context_predicate(
            (id!(TuiInputView::ui_name()) | id!(TuiTerminalSessionView::ui_name()))
                & id!(SESSION_COMPOSER_OWNS_INPUT_FLAG),
        )
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-s"),
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

    fn session_state(
        &self,
        ctx: &AppContext,
    ) -> Result<TuiTerminalSessionState, TuiTerminalSessionStateResolveError> {
        let state = self.session_state.as_ref(ctx).resolve(ctx)?;
        Ok(if let Some(grok_oauth) = self.active_grok_oauth(ctx) {
            state.with_blocking_input_source(BlockingInputSource::GrokOAuth(grok_oauth))
        } else if let Some(handoff) = self.active_handoff(ctx) {
            state.with_blocking_input_source(BlockingInputSource::Handoff(handoff))
        } else {
            state
        })
    }

    fn update_process_input_focus(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus_current_owner_if_active(ctx);
    }

    fn refresh_input_focus(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus_current_owner_if_active(ctx);
        ctx.notify();
    }

    fn focus_blocking_input_source(source: BlockingInputSource, ctx: &mut ViewContext<Self>) {
        match source {
            BlockingInputSource::LongRunningCommand => ctx.focus_self(),
            BlockingInputSource::AskQuestion(view) => ctx.focus(&view),
            BlockingInputSource::Permission(view) => ctx.focus(&view),
            BlockingInputSource::Orchestration(view) => ctx.focus(&view),
            BlockingInputSource::Handoff(view) => ctx.focus(&view),
            BlockingInputSource::GrokOAuth(view) => ctx.focus(&view),
        }
    }

    fn focus_current_owner(&mut self, ctx: &mut ViewContext<Self>) {
        let Ok(state) = self.session_state(ctx) else {
            return;
        };
        let blocking_input_source = state.blocking_input_source().cloned();
        match state.input_target() {
            TuiInputTarget::Disabled => {
                if let Some(source) = blocking_input_source {
                    self.orchestration_tabs_focused = false;
                    Self::focus_blocking_input_source(source, ctx);
                } else if let Some(statusline_config_view) = self.statusline_config_view.as_ref() {
                    self.orchestration_tabs_focused = false;
                    statusline_config_view.update(ctx, |view, ctx| view.focus(ctx));
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
                if let Some(source) = blocking_input_source {
                    self.orchestration_tabs_focused = false;
                    Self::focus_blocking_input_source(source, ctx);
                } else if let Some(statusline_config_view) = self.statusline_config_view.as_ref() {
                    self.orchestration_tabs_focused = false;
                    statusline_config_view.update(ctx, |view, ctx| view.focus(ctx));
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
    fn handle_block_completed(&mut self, block_id: &BlockId, ctx: &mut ViewContext<Self>) {
        self.input_view.update(ctx, |input, ctx| {
            input.reset_after_agent_control(ctx);
        });
        self.resume_after_user_controlled_command(block_id, ctx);
        self.refresh_input_focus(ctx);
        ctx.notify();
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
                    .update(ctx, |input, ctx| input.lock_for_agent_control(ctx));
                if let Some(target) = self
                    .cli_subagent_controller
                    .as_ref(ctx)
                    .target_for_block(block_id)
                {
                    let controller = self.cli_subagent_controller.clone();
                    let action_model = self.ai_action_model.clone();
                    let terminal_model = self.terminal_model.clone();
                    let view = ctx.add_tui_view(|ctx| {
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
                // `SpawnedSubagent` locks the input to AI while the agent owns
                // the terminal-use block. Once that block completes, restore
                // the setting-derived unlocked state so the next prompt can
                // resume natural-language detection.
                self.input_view.update(ctx, |input, ctx| {
                    input.reset_after_agent_control(ctx);
                });
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
        self.refresh_input_focus(ctx);
        ctx.notify();
    }

    fn handle_terminal_use_interrupt(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if self.try_detach_agent_from_running_command(ctx) {
            return true;
        }
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
                self.refresh_input_focus(ctx);
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
        self.refresh_input_focus(ctx);
    }
    /// Attempts to expose the agent composer for the active user-controlled LRC.
    ///
    /// Returns false when a stale action no longer targets an eligible block.
    fn try_attach_agent_to_running_command(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let did_attach = {
            let mut terminal_model = self.terminal_model.lock();
            let active_block = terminal_model.block_list_mut().active_block_mut();
            if !active_block.is_eligible_to_tag_in_agent() {
                false
            } else {
                active_block.set_is_agent_tagged_in(true);
                true
            }
        };
        if !did_attach {
            return false;
        }
        self.input_view.update(ctx, |input, ctx| {
            input.clear(ctx);
            input.lock_for_agent_control(ctx);
        });
        self.refresh_input_focus(ctx);
        ctx.notify();
        true
    }

    /// Attempts to return input to the active manually tagged LRC.
    ///
    /// Discards any unsent agent prompt. Returns false when no active block has
    /// a manually attached agent.
    fn try_detach_agent_from_running_command(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let did_detach = {
            let mut terminal_model = self.terminal_model.lock();
            let active_block = terminal_model.block_list_mut().active_block_mut();
            if !active_block.is_agent_tagged_in() {
                false
            } else {
                active_block.set_is_agent_tagged_in(false);
                true
            }
        };
        if !did_detach {
            return false;
        }
        self.input_view.update(ctx, |input, ctx| {
            input.clear(ctx);
        });
        self.refresh_input_focus(ctx);
        ctx.notify();
        true
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
    fn active_cli_subagent_view(&self, ctx: &AppContext) -> Option<ViewHandle<TuiCLISubagentView>> {
        let target = self.cli_subagent_controller.as_ref(ctx).active_target()?;
        self.cli_subagent_views.get(&target.block_id).cloned()
    }

    fn accept_active_cli_subagent_action(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(view) = self.active_cli_subagent_view(ctx) else {
            return false;
        };
        if !view.as_ref(ctx).has_blocked_action(ctx) {
            return false;
        }
        view.update(ctx, |view, ctx| {
            view.accept_blocked_terminal_use_action(ctx)
        });
        true
    }

    fn reject_active_cli_subagent_action(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(view) = self.active_cli_subagent_view(ctx) else {
            return false;
        };
        if !view.as_ref(ctx).has_blocked_action(ctx) {
            return false;
        }
        view.update(ctx, |view, ctx| {
            view.reject_blocked_terminal_use_action(ctx)
        });
        true
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
        let zero_state_animation_config = ZeroStateAnimationConfig::handle(ctx);
        let initial_zero_state_load_failure =
            zero_state_animation_config.as_ref(ctx).load_failure();
        ctx.subscribe_to_model(
            &zero_state_animation_config,
            |view, _, event, ctx| match event {
                ZeroStateAnimationConfigEvent::Updated => {}
                ZeroStateAnimationConfigEvent::LoadFailed(failure) => {
                    view.show_zero_state_ascii_load_failure(*failure, ctx);
                }
            },
        );
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
                sessions.clone(),
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
        // Only action lifecycle transitions can change the blocking input
        // owner. Presentation updates stay within the focused blocker.
        ctx.subscribe_to_model(
            &action_model,
            |view, action_model, event, ctx| match event {
                BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(_)
                | BlocklistAIActionEvent::ExecutingAction(_) => view.refresh_input_focus(ctx),
                BlocklistAIActionEvent::FinishedAction { action_id, .. } => {
                    view.refresh_input_focus(ctx);
                    let finished_asking_question = action_model
                        .as_ref(ctx)
                        .get_action_result(action_id)
                        .is_some_and(|result| {
                            matches!(&result.result, AIAgentActionResultType::AskUserQuestion(_))
                        });
                    if finished_asking_question {
                        ctx.focus(&view.input_view);
                    }
                }
                BlocklistAIActionEvent::QueuedAction(_)
                | BlocklistAIActionEvent::InitProject(_)
                | BlocklistAIActionEvent::ToggleCodeReview(_)
                | BlocklistAIActionEvent::InsertCodeReviewComments { .. } => {}
            },
        );
        let input_editor_model =
            ctx.add_model(|ctx| CodeEditorModel::new_tui(INITIAL_INPUT_WIDTH, ctx));
        let suggestions_mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
        let suggestions_mode_for_user_info = suggestions_mode.clone();
        ctx.subscribe_to_model(&TuiUserInfoManager::handle(ctx), move |_, _, event, ctx| {
            let TuiUserInfoManagerEvent::Updated = event;
            if status_menu_is_open(suggestions_mode_for_user_info.as_ref(ctx).mode()) {
                ctx.notify();
            }
        });
        let read_only_menu_selection = TuiSelectionHandle::default();
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
        let prompt_and_command_history_menu = ctx.add_model(|ctx| {
            TuiPromptAndCommandHistoryMenuModel::new(
                input_editor_model.clone(),
                ai_input_model.clone(),
                suggestions_mode.clone(),
                active_session.clone(),
                terminal_surface_id,
                ctx,
            )
        });
        ctx.subscribe_to_model(&prompt_and_command_history_menu, |_, _, event, ctx| {
            let TuiPromptAndCommandHistoryMenuEvent::Updated = event;
            ctx.notify();
        });
        let completion_menu =
            ctx.add_model(|_| TuiCompletionMenuModel::new(suggestions_mode.clone()));
        ctx.subscribe_to_model(&completion_menu, |_, _, _, ctx| ctx.notify());
        // The footer's conversations callout depends on whether the input is
        // empty, so content changes must invalidate this parent view as well as
        // the input child. Typing after ctrl-c also disarms the pending exit
        // confirmation (and any child-kill window); the ctrl-c buffer clear
        // leaves the buffer empty, so the window it arms survives its own clear.
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
                view.child_kill_armed_conversation = None;
            }
            view.handle_input_content_changed(origin.from_user(), ctx);
            ctx.notify();
        });

        let editor_for_selection = input_editor_model.clone();
        let transcript_for_selection = transcript.clone();
        ctx.subscribe_to_model(&input_editor_model, move |view, _, event, ctx| {
            if !matches!(event, CodeEditorModelEvent::SelectionChanged) {
                return;
            }
            view.handle_completion_editor_changed(ctx);

            let has_selection = !editor_for_selection
                .as_ref(ctx)
                .buffer_selection_model()
                .as_ref(ctx)
                .first_selection_is_single_cursor();
            if has_selection {
                view.read_only_menu_selection.clear();
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
            TuiInlineMenu::new(prompt_and_command_history_menu.clone()),
            TuiInlineMenu::new(completion_menu.clone()),
        ];
        let inline_menus_for_input = inline_menus.clone();
        let suggestions_mode_for_input = suggestions_mode.clone();
        let terminal_model_for_input = model.clone();
        let orchestration_tab_bar = ctx.add_typed_action_tui_view(|_| TuiTabBarView::empty());
        let session_state = ctx.add_model(|_| {
            TuiTerminalSessionStateModel::new(
                &model,
                &cli_subagent_controller,
                &transcript,
                &ai_input_model,
                &suggestions_mode,
                &orchestration_tab_bar,
            )
        });
        let input_editor_for_input = input_editor_model.clone();
        let session_state_for_input = session_state.clone();
        let input_view = ctx.add_typed_action_tui_view(move |ctx| {
            TuiInputView::new(
                input_editor_for_input,
                input_mode_for_input_view,
                suggestions_mode_for_input,
                inline_menus_for_input,
                session_state_for_input,
                ctx,
            )
            .with_inline_menu_actions_allowed(move |_| {
                let terminal_model = terminal_model_for_input.lock();
                tui_input_target(&terminal_model).agent_editor_owns_input()
            })
            .with_keyboard_enhancement_supported(keyboard_enhancement_supported)
        });
        let voice_input_model = input_view.as_ref(ctx).voice_input_model().clone();
        ctx.subscribe_to_model(&voice_input_model, |view, _, event, ctx| {
            view.handle_voice_input_event(event, ctx);
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
                view.read_only_menu_selection.clear();
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
                view.refresh_input_focus(ctx);
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
            TuiInputViewEvent::Submitted(text) => view.handle_submitted(text.clone(), None, ctx),
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
            TuiInputViewEvent::AcceptedPromptAndCommandHistory { text, kind } => {
                view.handle_accepted_prompt_and_command_history(text.clone(), kind.clone(), ctx);
            }
            TuiInputViewEvent::RequestShellCompletion => {
                view.request_shell_completion(ctx);
            }
            TuiInputViewEvent::ClipboardCopySucceeded => view.show_copy_hint(ctx),
            TuiInputViewEvent::ClipboardCopyFailed => {
                view.show_transient_hint(COPY_FAILED_HINT.to_owned(), ctx);
            }
            TuiInputViewEvent::MoveFocusUp => {
                view.focus_orchestration_tabs(ctx);
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
        ctx.subscribe_to_model(&ai_input_model, |view, _, _, ctx| {
            view.handle_completion_editor_changed(ctx);
            ctx.notify();
        });
        ctx.subscribe_to_model(&suggestions_mode, |view, _, _, ctx| {
            view.read_only_menu_selection.clear();
            ctx.notify();
        });
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
        ctx.subscribe_to_model(
            &QueuedQueryModel::handle(ctx),
            |view, _, event, ctx| match event {
                QueuedQueryEvent::QueueNextPromptToggled { conversation_id } => {
                    if view
                        .conversation_selection
                        .as_ref(ctx)
                        .selected_conversation_id(ctx)
                        == Some(*conversation_id)
                    {
                        ctx.notify();
                    }
                }
                QueuedQueryEvent::DefaultModeChanged => ctx.notify(),
                QueuedQueryEvent::Appended { .. }
                | QueuedQueryEvent::RowUnlocked { .. }
                | QueuedQueryEvent::Removed { .. }
                | QueuedQueryEvent::Reordered { .. }
                | QueuedQueryEvent::EditEntered { .. }
                | QueuedQueryEvent::EditCommitted { .. }
                | QueuedQueryEvent::EditCancelled { .. }
                | QueuedQueryEvent::Cleared { .. } => {}
            },
        );

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
                view.handle_block_completed(&completed.block_id, ctx);
            }
            ModelEvent::AfterBlockStarted { .. } => {
                view.refresh_input_focus(ctx);
                ctx.notify();
            }
            ModelEvent::VisibleBootstrapBlock | ModelEvent::BootstrapPrecmdDone => {
                view.refresh_input_focus(ctx);
                ctx.notify();
            }
            ModelEvent::TerminalModeSwapped(_) => {
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
        // Re-render when the configured statusline or usage-display mode
        // changes (click or settings-file hot reload). Model, working
        // directory, git, queue, and conversation subscriptions below cover
        // changes in the configured items' live values.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |view, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::TuiUsageDisplayMode { .. }
                    | AISettingsChangedEvent::TuiStatusline { .. }
                    | AISettingsChangedEvent::VoiceInputEnabled { .. }
            ) {
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
                view.abort_shell_completion(ctx);
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
            ActiveSessionEvent::Bootstrapped => view.abort_shell_completion(ctx),
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
        let mut view = Self {
            transcript,
            input_view,
            attachment_bar,
            inline_menus,
            suggestions_mode,
            read_only_menu_selection,
            session_state,
            conversation_menu,
            model_menu,
            skills_menu,
            mcp_menu,
            completion_menu,
            slash_commands_source,
            conversation_selection,
            ai_action_model: action_model,
            cli_agent_osc_event_publisher: None,
            ai_controller,
            cli_subagent_controller,
            cli_subagent_views: HashMap::new(),
            active_session,
            sessions,
            current_repo_path: None,
            git_repo_status: None,
            terminal_surface_id,
            exit_confirmation: ExitConfirmation::default(),
            usage_toggle: UsageToggle::default(),
            hidden_response_summary_exchange_ids: HashSet::new(),
            model_label_hover: MouseStateHandle::default(),
            voice_input_mouse: MouseStateHandle::default(),
            keyboard_enhancement_supported,
            ai_context_model: context_model,
            ai_input_model,
            input_detection: InputDetectionState::default(),
            completion_request: CompletionRequestState::default(),
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
            handoff: None,
            grok_oauth: None,
            statusline_config_view: None,
            orchestration_tab_bar,
            orchestration_tabs_focused: false,
            child_kill_armed_conversation: None,
            zero_state_view,
        };
        if let Some(failure) = initial_zero_state_load_failure {
            view.show_zero_state_ascii_load_failure(failure, ctx);
        }
        view
    }

    /// Enables CLI-agent lifecycle notifications for the root TUI session.
    pub(crate) fn enable_cli_agent_osc_event_publishing(&mut self, ctx: &mut ViewContext<Self>) {
        if self.cli_agent_osc_event_publisher.is_some() || !host_supports_cli_agent_notifications()
        {
            return;
        }
        let terminal_surface_id = self.terminal_surface_id;
        let active_session = self.active_session.clone();
        let conversation_selection = self.conversation_selection.clone();
        let action_model = self.ai_action_model.clone();
        let publisher = ctx.add_model(|ctx| {
            CliAgentOscEventPublisher::new(
                terminal_surface_id,
                active_session,
                conversation_selection,
                &action_model,
                ctx,
            )
        });
        publisher.as_ref(ctx).publish_session_start(ctx);
        self.cli_agent_osc_event_publisher = Some(publisher);
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
        // Disarm the child-kill window when the child is no longer reachable.
        if !tabs_are_available && self.child_kill_armed_conversation.is_some() {
            self.exit_confirmation.disarm();
            self.child_kill_armed_conversation = None;
            focus_changed = true;
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

    /// If the orchestration snapshot shows a child tab selected (not the root),
    /// returns that child's conversation id. Used to decide between the kill
    /// path and the normal exit path on a ctrl-c press.
    fn is_child_conversation_selected(&self, ctx: &AppContext) -> Option<AIConversationId> {
        let snapshot = self.compute_orchestration_tab_snapshot(ctx)?;
        (snapshot.selected_conversation_id != snapshot.root_conversation_id)
            .then_some(snapshot.selected_conversation_id)
    }

    /// Kills a child agent: tombstones late events, deletes the conversation
    /// from history, removes its retained TUI session, and returns focus to the
    /// root/main orchestration agent. Equivalent to the GUI's Kill agent path.
    fn kill_child_agent(&mut self, conversation_id: AIConversationId, ctx: &mut ViewContext<Self>) {
        // Clear any armed kill or exit window.
        self.exit_confirmation.disarm();
        self.child_kill_armed_conversation = None;
        // Return tab bar to unfocused state before the session is removed so
        // the focus fall-back lands on the right surface.
        self.orchestration_tabs_focused = false;
        // Resolve the root session id BEFORE the kill clears the snapshot.
        // We bypass `focus_conversation_session` because after the child is
        // deleted the parent is no longer an orchestration root (no children),
        // and that helper gates on the root check.
        let root_session_id = self
            .compute_orchestration_tab_snapshot(ctx)
            .and_then(|snap| {
                let history = BlocklistAIHistoryModel::as_ref(ctx);
                TuiSessions::as_ref(ctx)
                    .session_ids_by_conversation(history)
                    .get(&snap.root_conversation_id)
                    .copied()
            });
        // Tombstone + delete + remove session via the orchestration model.
        TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
            model.kill_child_agent(conversation_id, ctx);
        });
        // Focus the root session directly using the pre-kill resolved id.
        if let Some(session_id) = root_session_id {
            TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.focus_session(session_id, ctx);
            });
        } else {
            self.set_orchestration_tab_focus(false, ctx);
        }
    }

    /// Footer shown while orchestration tabs own keyboard focus.
    fn render_orchestration_tab_footer(
        &self,
        builder: &TuiUiBuilder,
        ctx: &AppContext,
    ) -> Box<dyn TuiElement> {
        // Show the kill hint when a child tab is selected so the user knows
        // that a single ctrl-c will terminate that agent.
        if self.is_child_conversation_selected(ctx).is_some() {
            render_orchestration_child_selected_tab_footer(builder)
        } else {
            render_orchestration_tab_footer(builder)
        }
    }
    fn running_command_hint(&self, ctx: &AppContext) -> Option<String> {
        let context = self.keymap_context(ctx);
        let attach_key = binding_hint(ATTACH_AGENT_TO_RUNNING_COMMAND_BINDING_NAME, &context, ctx);
        input_hints::long_running_command_hint(attach_key.as_deref())
    }

    fn render_input_area(
        &self,
        state: &TuiTerminalSessionState,
        input_target: TuiInputTarget,
        inline_menu: Option<Box<dyn TuiElement>>,
        builder: &TuiUiBuilder,
        ctx: &AppContext,
    ) -> Box<dyn TuiElement> {
        let mut content = TuiFlex::column();
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
        if let Some(menu) = state.read_only_menu() {
            let menu = match menu {
                TuiReadOnlyMenuKind::Shortcuts => {
                    let keymap_context = self.keymap_context(ctx);
                    shortcuts::menu(state, &keymap_context, builder, ctx)
                }
                TuiReadOnlyMenuKind::Status => {
                    status_menu::menu(self.compute_status_info(ctx), builder)
                }
            };
            let menu = menu.render(
                self.read_only_menu_selection.clone(),
                builder,
                |event_ctx, _| {
                    event_ctx.dispatch_typed_action(
                        TuiTerminalSessionAction::ReadOnlyMenuSelectionStarted,
                    );
                },
                |text, event_ctx, _| {
                    event_ctx.dispatch_typed_action(
                        TuiTerminalSessionAction::ReadOnlyMenuSelectionEnded(text),
                    );
                },
            );
            content = content.child(
                TuiContainer::new(menu)
                    .with_padding_top(INLINE_MENU_TOP_PADDING_ROWS)
                    .finish(),
            );
        }
        let input = if self.input_view.as_ref(ctx).voice_state(ctx) == TuiVoiceInputState::Listening
        {
            let input_view = self.input_view.clone();
            let builder = builder.clone();
            let clock = self.input_view.as_ref(ctx).voice_animation_clock(ctx);
            TuiAnimated::new(VOICE_INPUT_BORDER_REPAINT_INTERVAL, move || {
                bordered_input(
                    &input_view,
                    builder.voice_input_border_style(clock.elapsed()),
                )
            })
            .finish()
        } else {
            let border_style = if self.is_shell_mode(ctx) {
                builder.shell_mode_accent_style()
            } else {
                builder.accent_border_style()
            };
            bordered_input(&self.input_view, border_style)
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
            TuiConstrainedBox::new(input)
                .with_max_rows(MAX_INPUT_TEXT_ROWS + 2)
                .finish(),
        );
        let footer = if matches!(input_target, TuiInputTarget::Disabled) {
            self.render_footer(ctx).finish()
        } else if self.orchestration_tabs_focused {
            self.render_orchestration_tab_footer(builder, ctx)
        } else {
            self.render_footer(ctx).finish()
        };
        content
            .child(TuiConstrainedBox::new(footer).with_max_rows(1).finish())
            .finish()
    }

    /// Activates this session after the registry has made it authoritative.
    pub(crate) fn activate(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus_current_owner(ctx);
        self.write_exit_summary(ctx);
        ctx.notify();
    }

    /// Computes the current session and account status fields for the dedicated
    /// status menu (opened by the `/status` slash command). Always returns a
    /// complete set of fields; individual fields fall back to their `STATUS_*`
    /// placeholder constants when the underlying data is unavailable.
    fn compute_status_info(&self, ctx: &AppContext) -> status_menu::TuiStatusInfo {
        let user_info = TuiUserInfoManager::as_ref(ctx).snapshot(ctx);
        let cwd = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .map(|cwd| abbreviate_home_prefix(cwd))
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|cwd| abbreviate_home_prefix(&cwd.display().to_string()))
            });
        let session_name = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation(ctx)
            .and_then(|conversation| conversation.title())
            .unwrap_or_else(|| STATUS_UNTITLED_SESSION.to_owned());
        let conversation_id = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx);
        let version = ChannelState::app_version()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| STATUS_DEV_BUILD.to_owned());
        let org = user_info
            .org
            .unwrap_or_else(|| STATUS_UNAVAILABLE.to_owned());
        let email =
            resolve_status_email(user_info.email, user_info.username, user_info.is_logged_in);
        status_menu::TuiStatusInfo {
            version,
            session: session_name,
            conversation_id: format_status_conversation_id(conversation_id),
            working_directory: cwd.unwrap_or_else(|| STATUS_UNAVAILABLE.to_owned()),
            org,
            email,
        }
    }

    /// Whether this view projects the focused session.
    fn is_focused_session(&self, ctx: &AppContext) -> bool {
        TuiSessions::as_ref(ctx)
            .focused_session_id()
            .is_some_and(|id| id.surface_id() == self.terminal_surface_id)
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
            self.refresh_input_focus(ctx);
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
            let enabled = self
                .conversation_selection
                .as_ref(ctx)
                .pending_query_autoexecute_override(ctx)
                .is_autoexecute_any_action();
            self.show_success_hint(
                if enabled {
                    AUTO_APPROVE_ENABLED_HINT
                } else {
                    AUTO_APPROVE_DISABLED_HINT
                }
                .to_owned(),
                ctx,
            );
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

    /// Displays error-colored feedback in the transient footer slot.
    fn show_error_hint(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        self.transient_hint
            .show_error(text, ctx, |view| &mut view.transient_hint);
    }

    fn show_zero_state_ascii_load_failure(
        &mut self,
        failure: ZeroStateAnimationLoadFailure,
        ctx: &mut ViewContext<Self>,
    ) {
        self.show_error_hint(zero_state_ascii_load_failure_hint(failure).to_owned(), ctx);
    }

    /// Displays success-colored feedback in the transient footer slot.
    fn show_copy_hint(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_success_hint(COPY_SELECTION_HINT.to_owned(), ctx);
    }

    /// Handles a ctrl-c press.
    ///
    /// Priority order:
    /// 1. Cancel in-flight conversation restore.
    /// 2. Reject a pending terminal-use action / handle terminal-use takeover.
    /// 3. **Kill-child path (tab-bar focused + child tab selected):** a single
    ///    ctrl-c immediately kills the selected child agent and returns focus to
    ///    the root/main orchestration agent.
    /// 4. **Kill-child path (viewing a child conversation without tab focus):**
    ///    the first ctrl-c arms a 1-second kill window and shows a child-kill
    ///    footer hint; a second ctrl-c within the window kills the child agent.
    /// 5. **Exit path (root/main agent or no orchestration):** the first press
    ///    cancels the running conversation or clears input and arms the exit
    ///    confirmation window; a second press within the window exits the TUI.
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
        if self.reject_active_cli_subagent_action(ctx) {
            self.exit_confirmation.disarm();
            self.child_kill_armed_conversation = None;
            ctx.notify();
            return;
        }
        self.suggestions_mode.update(ctx, |mode, ctx| {
            if let Some(kind) = mode.mode().read_only_menu() {
                mode.close_if_active(TuiInputSuggestionsMode::ReadOnlyMenu(kind), ctx);
            }
        });
        if self.handle_terminal_use_interrupt(ctx) {
            self.exit_confirmation.disarm();
            self.child_kill_armed_conversation = None;
            ctx.notify();
            return;
        }

        // Path 1: tab-bar focused + child tab selected → single ctrl-c kills.
        if self.orchestration_tabs_focused
            && let Some(child_id) = self.is_child_conversation_selected(ctx)
        {
            self.kill_child_agent(child_id, ctx);
            return;
        }

        // Path 2: tab-bar not focused, viewing a child conversation.
        // First ctrl-c arms the kill window; second within ~1s kills the child.
        if !self.orchestration_tabs_focused
            && let Some(child_id) = self.is_child_conversation_selected(ctx)
        {
            let now = Instant::now();
            if self.child_kill_armed_conversation == Some(child_id)
                && self.exit_confirmation.should_exit(now)
            {
                // Second ctrl-c: kill the child and return to main agent.
                self.kill_child_agent(child_id, ctx);
                return;
            }
            // First ctrl-c: arm the kill window with the child-specific hint.
            self.child_kill_armed_conversation = Some(child_id);
            let window_expires_at = self.exit_confirmation.arm(now);
            ctx.spawn(Timer::after(CTRL_C_EXIT_WINDOW), move |view, _, ctx| {
                if view.exit_confirmation.disarm_expired(window_expires_at) {
                    view.child_kill_armed_conversation = None;
                    ctx.notify();
                }
            });
            ctx.notify();
            return;
        }

        // Path 3 (original): root/main agent or no orchestration.
        // Ensure any stale kill window is cleared before the normal exit path.
        self.child_kill_armed_conversation = None;
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
    pub(crate) fn cancel_active_conversation(&mut self, ctx: &mut ViewContext<Self>) -> bool {
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

    /// Selects the single message that replaces the normal footer, preserving
    /// the priority order between competing session states.
    fn footer_hint(
        &self,
        voice_statusline_visible: bool,
        ctx: &AppContext,
    ) -> Option<FooterHint<'_>> {
        if self.exit_confirmation.is_armed() {
            // When the kill-child window is armed, show the child-specific hint
            // so the user knows the next ctrl-c will kill the child agent rather
            // than exiting the whole TUI.
            if self.child_kill_armed_conversation.is_some() {
                return Some(FooterHint::muted(CTRL_C_KILL_CHILD_HINT));
            }
            return Some(FooterHint::muted(CTRL_C_EXIT_HINT));
        }
        if matches!(
            &self.conversation_restore_state,
            ConversationRestoreState::Loading {
                origin: TuiConversationRestoreOrigin::ConversationList,
                ..
            }
        ) {
            return Some(FooterHint::muted(LOADING_CONVERSATION_HINT));
        }
        if let Some((text, tone)) = self.transient_hint.current() {
            let style = match tone {
                TransientHintTone::Muted => FooterHintStyle::Muted,
                TransientHintTone::Success => FooterHintStyle::Success,
                TransientHintTone::Error => FooterHintStyle::Error,
            };
            return Some(FooterHint { text, style });
        }
        if self
            .session_state(ctx)
            .is_ok_and(|state| state.agent_is_tagged_in())
        {
            return Some(FooterHint::muted(RUNNING_COMMAND_DETACH_HINT));
        }
        if voice_statusline_visible {
            return None;
        }
        match self.input_view.as_ref(ctx).voice_state(ctx) {
            TuiVoiceInputState::Listening => {
                return Some(FooterHint::voice_input(
                    "listening to voice input... · esc or enter to stop",
                ));
            }
            TuiVoiceInputState::Transcribing => {
                return Some(FooterHint::voice_input("Transcribing... · esc to cancel"));
            }
            TuiVoiceInputState::Idle => {}
        }
        None
    }

    /// Builds the configured statusline under the input box. Normal mode uses
    /// the persisted item order and visibility; shell mode always leads with
    /// its mode label and resolves configured shell-relevant metadata. A
    /// replacing hint — the ctrl-c exit confirmation while armed, the
    /// conversation-list loading hint, an active transient notice, or the
    /// interrupt hint for a manually attached running command — occupies the
    /// whole row instead. An empty resolved configuration consumes no row.
    fn render_footer(&self, ctx: &AppContext) -> TuiFlex {
        let builder = TuiUiBuilder::from_app(ctx);
        let shell_mode = self.is_shell_mode(ctx);
        let config = AISettings::as_ref(ctx).tui_statusline.normalized();
        let voice_statusline_visible = config.is_enabled(TuiStatuslineItem::VoiceInput)
            && self.voice_statusline_is_available(shell_mode, ctx);
        if let Some(hint) = self.footer_hint(voice_statusline_visible, ctx) {
            return hint.render(&builder);
        }
        let git_metadata = self.git_status_metadata(ctx);
        let mut ordered = Vec::new();
        if shell_mode {
            ordered.push(FooterSegment::ShellMode);
        }
        for item in config.order.iter().copied() {
            if !config.is_enabled(item) {
                continue;
            }
            let segment = match item {
                TuiStatuslineItem::AutoApprove => (!shell_mode
                    && self
                        .conversation_selection
                        .as_ref(ctx)
                        .pending_query_autoexecute_override(ctx)
                        .is_autoexecute_any_action())
                .then_some(FooterSegment::ActiveIndicator("Auto-approve")),
                TuiStatuslineItem::AutoQueue => (!shell_mode && self.is_auto_queue_enabled(ctx))
                    .then_some(FooterSegment::ActiveIndicator("Auto-queue")),
                TuiStatuslineItem::Model => (!shell_mode).then(|| {
                    let model_name = LLMPreferences::as_ref(ctx)
                        .get_active_base_model(ctx, Some(self.terminal_surface_id))
                        .display_name
                        .clone();
                    let model_label_hovered = self
                        .model_label_hover
                        .lock()
                        .is_ok_and(|state| state.is_hovered());
                    let model_label_style = if model_label_hovered {
                        builder.primary_text_style()
                    } else {
                        builder.muted_text_style()
                    };
                    let model_width = text_width(&model_name);
                    FooterSegment::Model(
                        TuiHoverable::new(
                            self.model_label_hover.clone(),
                            TuiText::new(model_name)
                                .with_style(model_label_style)
                                .truncate()
                                .finish(),
                        )
                        .on_click(|event_ctx, _| {
                            event_ctx
                                .dispatch_typed_action(TuiTerminalSessionAction::ToggleModelMenu);
                        })
                        .finish(),
                        model_width,
                    )
                }),
                TuiStatuslineItem::WorkingDirectory => {
                    self.current_working_directory(ctx)
                        .map(|cwd| FooterSegment::WorkingDirectory {
                            path: cwd,
                            reserved: 0,
                        })
                }
                TuiStatuslineItem::GitBranch => git_metadata
                    .map(|metadata| FooterSegment::GitBranch(metadata.current_branch_name.clone())),
                TuiStatuslineItem::GitBranchStatus => git_metadata.and_then(|metadata| {
                    metadata
                        .branch_tracking_status
                        .status_text()
                        .map(FooterSegment::GitBranchStatus)
                }),
                TuiStatuslineItem::GitDiffStatus => git_metadata.and_then(|metadata| {
                    let stats = metadata.stats_against_head;
                    (stats.total_additions > 0 || stats.total_deletions > 0).then_some(
                        FooterSegment::GitDiff {
                            additions: stats.total_additions,
                            deletions: stats.total_deletions,
                        },
                    )
                }),
                TuiStatuslineItem::CreditUsage => (!shell_mode)
                    .then(|| self.selected_conversation_usage_totals(ctx))
                    .flatten()
                    .map(|totals| {
                        let mode = AISettings::as_ref(ctx).usage_display_mode;
                        let usage_width = text_width(&crate::usage::entry_text(mode, totals));
                        FooterSegment::CreditUsage(
                            self.usage_toggle
                                .render_entry(mode, totals, ctx, |event_ctx, _| {
                                    event_ctx.dispatch_typed_action(
                                        TuiTerminalSessionAction::ToggleUsageDisplay,
                                    );
                                }),
                            usage_width,
                        )
                    }),
                TuiStatuslineItem::ContextWindowUsage => (!shell_mode)
                    .then(|| {
                        self.conversation_selection
                            .as_ref(ctx)
                            .selected_conversation(ctx)
                    })
                    .flatten()
                    .map(|conversation| {
                        FooterSegment::ContextWindowUsage(format_context_window_usage(
                            conversation.context_window_usage(),
                        ))
                    }),
                TuiStatuslineItem::Date => Some(FooterSegment::DateTime(
                    render_statusline_datetime(format_statusline_date, builder.muted_text_style()),
                    text_width(&format_statusline_date(Local::now().naive_local())),
                )),
                TuiStatuslineItem::Time12Hour => Some(FooterSegment::DateTime(
                    render_statusline_datetime(
                        format_statusline_time_12_hour,
                        builder.muted_text_style(),
                    ),
                    text_width(&format_statusline_time_12_hour(Local::now().naive_local())),
                )),
                TuiStatuslineItem::Time24Hour => Some(FooterSegment::DateTime(
                    render_statusline_datetime(
                        format_statusline_time_24_hour,
                        builder.muted_text_style(),
                    ),
                    text_width(&format_statusline_time_24_hour(Local::now().naive_local())),
                )),
                TuiStatuslineItem::AgentTodoList => (!shell_mode)
                    .then(|| {
                        self.conversation_selection
                            .as_ref(ctx)
                            .selected_conversation(ctx)
                    })
                    .flatten()
                    .and_then(|conversation| conversation.active_todo_list())
                    .filter(|todo_list| !todo_list.is_empty())
                    .map(|todo_list| {
                        FooterSegment::AgentTodoList(format_todo_progress(
                            todo_list.completed_items().len(),
                            todo_list.len(),
                            todo_list.is_finished(),
                        ))
                    }),
                TuiStatuslineItem::VoiceInput => voice_statusline_visible.then(|| {
                    let (element, width) = self.render_voice_statusline(&builder, ctx);
                    FooterSegment::VoiceInput(element, width)
                }),
            };
            if let Some(segment) = segment {
                ordered.push(segment);
            }
        }
        set_working_directory_reserved_width(&mut ordered);
        render_status_footer_row(FooterSegments { ordered }, &builder)
    }

    fn voice_statusline_is_available(&self, shell_mode: bool, ctx: &AppContext) -> bool {
        !shell_mode && AISettings::as_ref(ctx).is_voice_input_enabled(ctx)
    }

    /// Builds the voice-input footer segment and reports its display width so
    /// the width-aware working-directory segment can reserve room for it.
    fn render_voice_statusline(
        &self,
        builder: &TuiUiBuilder,
        ctx: &AppContext,
    ) -> (Box<dyn TuiElement>, u16) {
        let state = self.input_view.as_ref(ctx).voice_state(ctx);
        let hovered = self
            .voice_input_mouse
            .lock()
            .is_ok_and(|state| state.is_hovered());
        let (label, style) = match state {
            TuiVoiceInputState::Idle => (
                "Voice",
                if hovered {
                    builder.primary_text_style()
                } else {
                    builder.muted_text_style()
                },
            ),
            TuiVoiceInputState::Listening => ("■ Listening", builder.error_text_style()),
            TuiVoiceInputState::Transcribing => {
                let label = "… Transcribing";
                return (
                    TuiText::new(label)
                        .with_style(builder.voice_input_status_style())
                        .truncate()
                        .finish(),
                    text_width(label),
                );
            }
        };
        (
            TuiHoverable::new(
                self.voice_input_mouse.clone(),
                TuiText::new(label).with_style(style).truncate().finish(),
            )
            .on_click(|event_ctx, _| {
                event_ctx.dispatch_typed_action(TuiTerminalSessionAction::ToggleVoiceInput);
            })
            .finish(),
            text_width(label),
        )
    }

    fn is_auto_queue_enabled(&self, ctx: &AppContext) -> bool {
        if !FeatureFlag::QueueSlashCommand.is_enabled() {
            return false;
        }
        let Some(conversation_id) = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            return false;
        };
        let terminal_model = self.terminal_model.lock();
        QueuedQueryModel::as_ref(ctx).is_queue_next_prompt_enabled(
            conversation_id,
            terminal_model.block_list().active_block(),
            ctx,
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

    fn execute_provider_api_key_command(
        &mut self,
        command: &StaticCommand,
        argument: Option<&String>,
        operation: ProviderApiKeyOperation,
        ctx: &mut ViewContext<Self>,
    ) {
        let provider = argument
            .map(String::as_str)
            .ok_or_else(|| "provider is required".to_owned())
            .and_then(LLMProvider::from_api_key_slug);
        let Ok(provider) = provider else {
            self.show_error_hint(
                format!(
                    "Usage: {} <{}>",
                    command.name,
                    LLMProvider::API_KEY_PROVIDER_VALUE_NAME
                ),
                ctx,
            );
            return;
        };
        match (provider, operation) {
            (LLMProvider::Xai, ProviderApiKeyOperation::Set) => {
                self.start_grok_oauth(command.name, ctx);
            }
            (LLMProvider::Xai, ProviderApiKeyOperation::Clear) => {
                self.clear_grok_oauth(command.name, ctx);
            }
            (LLMProvider::OpenAI | LLMProvider::Anthropic | LLMProvider::Google, operation) => {
                let command_text =
                    provider_api_key_shell_command(ChannelState::channel(), provider, operation)
                        .expect("pasted-key providers have canonical API-key slugs");
                self.execute_user_command(&command_text, None, ctx);
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            (LLMProvider::Unknown, _) => {
                self.show_error_hint(
                    format!(
                        "Usage: {} <{}>",
                        command.name,
                        LLMProvider::API_KEY_PROVIDER_VALUE_NAME
                    ),
                    ctx,
                );
            }
        }
    }

    /// Routes a submission to shell execution or the agent conversation based
    /// on the input mode.
    fn handle_submitted(
        &mut self,
        text: String,
        linked_workflow_data: Option<LinkedWorkflowData>,
        ctx: &mut ViewContext<Self>,
    ) {
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
                .update(ctx, |input, ctx| input.lock_for_agent_control(ctx));
        } else if self.is_shell_mode(ctx) {
            self.execute_user_command(&text, linked_workflow_data, ctx);
        } else {
            self.handle_submitted_input(&text, ctx);
        }
        ctx.notify();
    }

    /// Executes `command` in the session's PTY as a user command, preserving
    /// workflow origin metadata when it was recalled from history.
    ///
    /// Mirrors the GUI's shell-mode submission: rejected while the agent holds
    /// the PTY with an active long-running command (the input keeps its text
    /// and a transient hint is shown), and an in-progress conversation is
    /// cancelled when the command runs. On success the input clears and exits
    /// shell mode back to agent input.
    fn execute_user_command(
        &mut self,
        command: &str,
        linked_workflow_data: Option<LinkedWorkflowData>,
        ctx: &mut ViewContext<Self>,
    ) {
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

        let (workflow_id, workflow_command) = match linked_workflow_data {
            Some(LinkedWorkflowData::Id(workflow_id)) => (Some(workflow_id), None),
            Some(LinkedWorkflowData::Command(workflow_command)) => (None, Some(workflow_command)),
            None => (None, None),
        };
        ctx.emit(TuiTerminalSessionEvent::ExecuteCommand(Box::new(
            ExecuteCommandEvent {
                command: command.to_owned(),
                session_id,
                workflow_id,
                workflow_command,
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
        if dispatched && let Some(publisher) = &self.cli_agent_osc_event_publisher {
            publisher
                .as_ref(ctx)
                .publish_prompt_submit(prompt.clone(), ctx);
        }
        if dispatched && let Some(block_id) = active_long_running_block_id {
            self.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.set_latest_instruction(block_id, prompt, ctx);
            });
        }
    }

    /// Asks the input-owned voice model to start recording.
    fn start_voice_input(&mut self, source: VoiceInputStartSource, ctx: &mut ViewContext<Self>) {
        let local_skills_available = self
            .slash_commands_source
            .as_ref(ctx)
            .local_skills_available(ctx);
        let started = self.input_view.update(ctx, |input, ctx| {
            if source.clears_input() {
                input.clear(ctx);
            }
            input.start_voice_input(local_skills_available, source, ctx)
        });
        if started && matches!(source, VoiceInputStartSource::SlashCommand) {
            record_static_slash_command_accepted("/voice", true, ctx);
        }
    }
    fn toggle_voice_input(&mut self, ctx: &mut ViewContext<Self>) {
        match self.input_view.as_ref(ctx).voice_state(ctx) {
            TuiVoiceInputState::Idle => {
                self.start_voice_input(VoiceInputStartSource::Button, ctx);
            }
            TuiVoiceInputState::Listening => {
                self.input_view
                    .update(ctx, |input, ctx| input.stop_voice_input(ctx));
            }
            TuiVoiceInputState::Transcribing => {}
        }
    }

    fn handle_voice_input_event(
        &mut self,
        event: &TuiVoiceInputEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiVoiceInputEvent::Completed(text) => {
                if !text.trim().is_empty() {
                    self.input_view.update(ctx, |input, ctx| {
                        input.insert_text(text, ctx);
                    });
                }
            }
            TuiVoiceInputEvent::Failed(hint) => {
                self.show_transient_hint(hint.clone(), ctx);
            }
            TuiVoiceInputEvent::Cancelled => {
                self.show_transient_hint("Voice input cancelled".to_owned(), ctx);
            }
            TuiVoiceInputEvent::StateChanged(_) => {
                ctx.notify();
            }
        }
    }

    fn handle_submitted_input(&mut self, input: &str, ctx: &mut ViewContext<Self>) {
        if self.is_conversation_restore_loading() {
            return;
        }

        if voice_command_argument(input).is_some_and(|argument| !argument.trim().is_empty()) {
            self.show_transient_hint(VOICE_USAGE_HINT.to_owned(), ctx);
            self.input_view.update(ctx, |input, ctx| input.clear(ctx));
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

    fn handle_accepted_prompt_and_command_history(
        &mut self,
        text: String,
        kind: TuiUpArrowHistoryItemKind,
        ctx: &mut ViewContext<Self>,
    ) {
        let linked_workflow_data = self.input_view.update(ctx, |input, ctx| {
            input.set_text(&text, ctx);
            match kind {
                TuiUpArrowHistoryItemKind::Prompt => {
                    input.exit_shell_mode(ctx);
                    None
                }
                TuiUpArrowHistoryItemKind::Command {
                    linked_workflow_data,
                } => {
                    input.enter_shell_mode(ctx);
                    linked_workflow_data
                }
            }
        });
        self.handle_submitted(text, linked_workflow_data, ctx);
    }

    /// Handles a mouse-click accept on the inline menu: selects the row at
    /// `index` in the active menu and dispatches the result through the same
    /// path as keyboard-based acceptance.
    fn handle_inline_menu_mouse_accept(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let mode = self.suggestions_mode.as_ref(ctx).mode();
        let Some(menu) = active_inline_menu(&self.inline_menus, mode, ctx) else {
            return;
        };
        // Guard: only fire accept when select_by_snapshot_index confirms the
        // selection was made. The default no-op impl returns false, preventing
        // a future menu that omits the override from silently accepting
        // whatever row happened to be keyboard-selected.
        if !menu.select_by_snapshot_index(index, ctx) {
            return;
        }
        let Some(accepted) = menu.accept(ctx) else {
            return;
        };
        self.input_view.update(ctx, |input, ctx| {
            input.route_inline_menu_acceptance(accepted, ctx);
        });
    }

    fn select_tui_slash_command(&mut self, command: &StaticCommand, ctx: &mut ViewContext<Self>) {
        if command.kind == SlashCommandKind::MoveToCloud {
            self.input_view.update(ctx, |input, ctx| {
                input.set_text("/handoff ", ctx);
            });
            ctx.notify();
            return;
        }
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

    fn start_new_conversation(
        &mut self,
        prompt: Option<&String>,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if !self
            .ai_context_model
            .as_ref(ctx)
            .can_start_new_conversation()
        {
            self.show_transient_hint(NEW_CONVERSATION_COMMAND_RUNNING_HINT.to_owned(), ctx);
            return false;
        }
        if let Some(conversation_id) = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.kill_descendant_agents(conversation_id, ctx);
            });
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
        if let Some(prompt) = prompt
            .map(|argument| argument.trim())
            .filter(|argument| !argument.is_empty())
        {
            self.send_prompt(prompt.to_owned(), ctx);
        }
        self.input_view.update(ctx, |input, ctx| input.clear(ctx));
        true
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
            SlashCommandKind::Agent | SlashCommandKind::New | SlashCommandKind::Clear => {
                if self.start_new_conversation(argument, ctx) {
                    record_static_slash_command_accepted(command.name, true, ctx);
                }
            }
            SlashCommandKind::Conversations => {
                self.conversation_menu
                    .update(ctx, |menu, ctx| menu.open(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::MoveToCloud => {
                self.start_handoff(argument, ctx);
            }
            SlashCommandKind::AutoApprove => {
                self.toggle_auto_approve(true, ctx);
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            SlashCommandKind::Statusline => {
                self.open_statusline_config(command.name, ctx);
            }
            SlashCommandKind::AddApiKey => {
                self.execute_provider_api_key_command(
                    command,
                    argument,
                    ProviderApiKeyOperation::Set,
                    ctx,
                );
            }
            SlashCommandKind::ClearApiKey => {
                self.execute_provider_api_key_command(
                    command,
                    argument,
                    ProviderApiKeyOperation::Clear,
                    ctx,
                );
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
            SlashCommandKind::Status => {
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                self.suggestions_mode.update(ctx, |mode, ctx| {
                    mode.set_mode(
                        TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Status),
                        ctx,
                    );
                });
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
            SlashCommandKind::Voice => {
                if !voice_argument_is_empty(argument) {
                    self.show_transient_hint(VOICE_USAGE_HINT.to_owned(), ctx);
                    self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                    return;
                }
                self.start_voice_input(VoiceInputStartSource::SlashCommand, ctx);
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
            SlashCommandKind::Theme => {
                self.toggle_theme(command.name, argument.map(String::as_str), ctx);
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

    fn open_statusline_config(&mut self, command_name: &'static str, ctx: &mut ViewContext<Self>) {
        let Ok(state) = self.session_state(ctx) else {
            return;
        };
        if state.blocking_input_source().is_some() || self.statusline_config_view.is_some() {
            return;
        }
        let config = AISettings::as_ref(ctx).tui_statusline.normalized();
        let statusline_config_view =
            ctx.add_typed_action_tui_view(|ctx| TuiStatuslineConfigView::new(config, ctx));
        ctx.subscribe_to_view(&statusline_config_view, |view, _, event, ctx| {
            view.handle_statusline_config_event(event, ctx);
        });
        self.statusline_config_view = Some(statusline_config_view);
        self.input_view.update(ctx, |input, ctx| input.clear(ctx));
        self.orchestration_tabs_focused = false;
        self.focus_current_owner_if_active(ctx);
        record_static_slash_command_accepted(command_name, true, ctx);
        ctx.notify();
    }

    fn handle_statusline_config_event(
        &mut self,
        event: &TuiStatuslineConfigEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiStatuslineConfigEvent::Saved(config) => {
                self.persist_statusline_config(config.clone(), ctx);
            }
            TuiStatuslineConfigEvent::Cancelled => {
                self.statusline_config_view = None;
                self.focus_current_owner_if_active(ctx);
                ctx.notify();
            }
            TuiStatuslineConfigEvent::LayoutChanged => ctx.notify(),
        }
    }

    fn persist_statusline_config(
        &mut self,
        config: TuiStatuslineConfig,
        ctx: &mut ViewContext<Self>,
    ) {
        let result = AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.tui_statusline.set_value(config.normalized(), ctx)
        });
        self.statusline_config_view = None;
        self.focus_current_owner_if_active(ctx);
        match result {
            Ok(()) => self.show_success_hint(STATUSLINE_SAVED_HINT.to_owned(), ctx),
            Err(error) => {
                report_error!("failed to persist the TUI statusline config: {error:#}");
                self.show_transient_hint(STATUSLINE_PERSISTENCE_FAILED_HINT.to_owned(), ctx);
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

    fn toggle_theme(
        &mut self,
        command_name: &'static str,
        argument: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.input_view.update(ctx, |input, ctx| input.clear(ctx));
        let Some(theme) = argument.and_then(|argument| argument.trim().parse::<TuiTheme>().ok())
        else {
            self.show_transient_hint(THEME_INVALID_ARGUMENT_HINT.to_owned(), ctx);
            record_static_slash_command_accepted(command_name, true, ctx);
            return;
        };
        let result = TuiThemeSettings::handle(ctx)
            .update(ctx, |settings, ctx| settings.theme.set_value(theme, ctx));
        match result {
            Ok(()) => {
                Appearance::handle(ctx).update(ctx, |appearance, ctx| {
                    appearance.set_theme(
                        theme.resolve_for_background(probed_colors().background_luminance()),
                        ctx,
                    );
                });
                let hint = match theme {
                    TuiTheme::Auto => format!(
                        "Theme set to auto mode (currently {}).",
                        TuiTheme::from(Appearance::as_ref(ctx).theme()).display_name()
                    ),
                    TuiTheme::Light | TuiTheme::Dark => {
                        format!("Theme set to {} mode.", theme.display_name())
                    }
                };
                self.show_success_hint(hint, ctx);
            }
            Err(error) => {
                log::warn!("Failed to save TUI theme selection: {error}");
                self.show_transient_hint(THEME_PERSISTENCE_FAILED_HINT.to_owned(), ctx);
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

    fn child_view_ids(&self, ctx: &AppContext) -> Vec<EntityId> {
        let mut view_ids = vec![
            self.transcript.id(),
            self.input_view.id(),
            self.orchestration_tab_bar.id(),
            self.attachment_bar.id(),
            self.zero_state_view.id(),
        ];
        if let Some(handoff) = self.active_handoff(ctx) {
            view_ids.push(handoff.id());
        }
        if let Some(grok_oauth) = self.active_grok_oauth(ctx) {
            view_ids.push(grok_oauth.id());
        }
        if let Some(statusline_config_view) = self.statusline_config_view.as_ref() {
            view_ids.push(statusline_config_view.id());
        }
        view_ids
    }

    fn keymap_context(&self, ctx: &AppContext) -> keymap::Context {
        let state = self.session_state(ctx).ok();
        let mut context = Self::default_keymap_context();
        if self.orchestration_tabs_focused
            && state
                .as_ref()
                .is_some_and(|state| state.input_target().agent_editor_owns_input())
        {
            context.set.insert(ORCHESTRATION_TAB_BAR_FOCUSED_FLAG);
        }
        if self.is_conversation_restore_loading() {
            context.set.insert(SESSION_CAN_CANCEL_RESTORE_FLAG);
        }
        if state
            .as_ref()
            .is_some_and(|state| state.can_hand_back_terminal_use())
        {
            context.set.insert(SESSION_CAN_HAND_BACK_CONTROL_FLAG);
        }
        if state
            .as_ref()
            .is_some_and(|state| state.can_attach_agent_to_running_command())
        {
            context
                .set
                .insert(SESSION_CAN_ATTACH_AGENT_TO_RUNNING_COMMAND_FLAG);
        }
        if state
            .as_ref()
            .is_some_and(|state| state.agent_is_tagged_in() && state.composer_owns_input())
        {
            context
                .set
                .insert(SESSION_CAN_DETACH_AGENT_FROM_RUNNING_COMMAND_FLAG);
        }
        if self
            .active_cli_subagent_view(ctx)
            .is_some_and(|view| view.as_ref(ctx).has_blocked_action(ctx))
        {
            context
                .set
                .insert(SESSION_CAN_ACCEPT_BLOCKED_TERMINAL_USE_ACTION_FLAG);
        }
        if state.as_ref().is_some_and(|state| state.plan_available()) {
            context.set.insert(PLAN_TOGGLE_AVAILABLE_FLAG);
        }
        if self.keyboard_enhancement_supported {
            context.set.insert(KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG);
        }
        if state
            .as_ref()
            .is_some_and(|state| state.composer_owns_input())
        {
            context.set.insert(SESSION_COMPOSER_OWNS_INPUT_FLAG);
            if attachment_focus_available(
                self.is_shell_mode(ctx),
                self.attachment_bar.as_ref(ctx).should_render(ctx),
            ) {
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
        let Ok(state) = self.session_state(ctx) else {
            return TuiText::new("").finish();
        };
        let input_target = state.input_target();
        let cli_subagent_view = {
            let terminal_model = self.terminal_model.lock();
            let active_block = terminal_model.block_list().active_block();
            self.cli_subagent_views.get(active_block.id()).cloned()
        };
        let inline_menu = input_target
            .agent_editor_owns_input()
            .then(|| {
                active_inline_menu(
                    &self.inline_menus,
                    self.suggestions_mode.as_ref(ctx).mode(),
                    ctx,
                )
                .and_then(|menu| {
                    menu.render_with_interaction(
                        ctx,
                        |index, event_ctx, _| {
                            event_ctx.dispatch_typed_action(
                                TuiTerminalSessionAction::InlineMenuMouseAcceptRow(index),
                            );
                        },
                        |delta, event_ctx, _| {
                            event_ctx.dispatch_typed_action(
                                TuiTerminalSessionAction::InlineMenuMouseScrollBy(delta),
                            );
                        },
                    )
                })
            })
            .flatten();
        let builder = TuiUiBuilder::from_app(ctx);
        let orchestration_tabs_available = state.orchestration_available();
        let blocker_active = state.has_blocking_interaction();

        if state.is_alt_screen() {
            let terminal_content = TuiTerminalContentElement::new(
                self.terminal_resize_tx.clone(),
                AltScreenElement::new(self.terminal_model.clone()).finish(),
            );
            let terminal_content = if input_target.pty_owns_input() {
                terminal_content.with_pty_input(self.terminal_model.clone())
            } else {
                terminal_content
            };
            let mut content = TuiFlex::column().flex_child(terminal_content.finish());
            if input_target.pty_owns_input()
                && state.user_owns_running_command()
                && let Some(hint) = self.running_command_hint(ctx)
            {
                content = content.child(
                    TuiContainer::new(
                        TuiText::new(hint)
                            .with_style(builder.muted_text_style())
                            .truncate()
                            .finish(),
                    )
                    .with_padding_x(2)
                    .with_padding_bottom(1)
                    .finish(),
                );
            }
            if input_target.agent_editor_owns_input() {
                let mut agent_area = TuiFlex::column();
                if let Some(cli_subagent_view) = cli_subagent_view {
                    agent_area = agent_area.child(TuiChildView::new(&cli_subagent_view).finish());
                }
                if let Some(blocker) = state
                    .blocking_input_source()
                    .cloned()
                    .and_then(BlockingInputSource::view_element)
                {
                    agent_area = agent_area.child(blocker);
                } else {
                    agent_area = agent_area.child(self.render_input_area(
                        &state,
                        input_target,
                        inline_menu,
                        &builder,
                        ctx,
                    ));
                }
                content = content.child(
                    TuiContainer::new(agent_area.finish())
                        .with_padding_x(2)
                        .with_padding_bottom(1)
                        .finish(),
                );
            }

            let session = content.finish();
            return if orchestration_tabs_available {
                TuiFlex::column()
                    .child(TuiChildView::new(&self.orchestration_tab_bar).finish())
                    .flex_child(session)
                    .finish()
            } else {
                session
            };
        }

        // Ctrl-c (cancel/clear/exit) is handled by the keymap pass via the
        // fixed binding registered in [`Self::init`], so no element-level key
        // handling is needed here.
        //
        // While the transcript has nothing to show, the zero state fills its
        // slot; the first accepted submission produces a visible block, which
        // swaps the transcript back in.
        let mut content = TuiFlex::column();
        let transcript_is_empty = self.transcript.as_ref(ctx).is_empty();
        if transcript_is_empty {
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
            .filter(|_| {
                !blocker_active
                    && self.statusline_config_view.is_none()
                    && input_target.agent_editor_owns_input()
            });
        if let Some(conversation) = selected_conversation {
            if conversation.status().is_in_progress() {
                let warping_elapsed = conversation
                    .latest_exchange()
                    .and_then(|exchange| exchange.time_since_start());
                if let Some(elapsed) = warping_elapsed {
                    let label = if conversation.is_summarizing() {
                        "Summarizing conversation"
                    } else {
                        "Warping"
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
        // input's slot when manual attachment is available. Gated on the
        // user-controlled-command predicate, not the broader PTY input target:
        // visible startup-script execution also routes input to the PTY but
        // does not support agent attachment. (Agent-driven terminal use keeps
        // the composer, and its control hints come from the CLI-subagent status
        // line.)
        if !blocker_active
            && state.user_owns_running_command()
            && let Some(hint) = self.running_command_hint(ctx)
        {
            content = content.child(
                TuiContainer::new(
                    TuiText::new(hint)
                        .with_style(builder.muted_text_style())
                        .truncate()
                        .finish(),
                )
                .with_padding_top(1)
                .finish(),
            );
        }
        if let Some(BlockingInputSource::Handoff(handoff)) = state.blocking_input_source() {
            content = content.child(TuiChildView::new(handoff).finish());
        }
        if let Some(BlockingInputSource::GrokOAuth(grok_oauth)) = state.blocking_input_source() {
            content = content.child(TuiChildView::new(grok_oauth).finish());
        }
        if !blocker_active
            && let Some(statusline_config_view) = self.statusline_config_view.as_ref()
        {
            content = content.child(
                TuiContainer::new(TuiChildView::new(statusline_config_view).finish())
                    .with_padding_top(1)
                    .finish(),
            );
        }
        if !blocker_active
            && self.statusline_config_view.is_none()
            && (input_target.agent_editor_owns_input()
                || matches!(input_target, TuiInputTarget::Disabled))
        {
            content = content.child(self.render_input_area(
                &state,
                input_target,
                inline_menu,
                &builder,
                ctx,
            ));
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
    fn forward_user_pty_bytes(&self, bytes: &[u8], ctx: &mut ViewContext<Self>) {
        let composer_owns_input = self
            .terminal_model
            .lock()
            .block_list()
            .active_block()
            .is_agent_in_control_or_tagged_in();
        if composer_owns_input {
            return;
        }
        ctx.emit(TuiTerminalSessionEvent::WriteUserInput(Cow::Owned(
            bytes.to_vec(),
        )));
    }
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
            TuiTerminalSessionAction::AttachAgentToRunningCommand => {
                let _ = self.try_attach_agent_to_running_command(ctx);
            }
            TuiTerminalSessionAction::DetachAgentFromRunningCommand => {
                let _ = self.try_detach_agent_from_running_command(ctx);
            }
            TuiTerminalSessionAction::AcceptBlockedTerminalUseAction => {
                self.accept_active_cli_subagent_action(ctx);
            }
            TuiTerminalSessionAction::RejectBlockedTerminalUseAction => {
                self.reject_active_cli_subagent_action(ctx);
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
                // sequence. Recheck control at the final write boundary in
                // case the element tree predates an agent takeover.
                self.forward_user_pty_bytes(bytes, ctx);
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
            TuiTerminalSessionAction::StartVoiceInput => {
                self.start_voice_input(VoiceInputStartSource::Keybinding, ctx);
            }
            TuiTerminalSessionAction::InlineMenuMouseAcceptRow(index) => {
                self.handle_inline_menu_mouse_accept(*index, ctx);
            }
            TuiTerminalSessionAction::InlineMenuMouseScrollBy(delta) => {
                let mode = self.suggestions_mode.as_ref(ctx).mode();
                if let Some(menu) = active_inline_menu(&self.inline_menus, mode, ctx) {
                    menu.scroll_by_delta(*delta, ctx);
                    ctx.notify();
                }
            }
            TuiTerminalSessionAction::ToggleVoiceInput => self.toggle_voice_input(ctx),
            TuiTerminalSessionAction::ReadOnlyMenuSelectionStarted => {
                self.transcript
                    .update(ctx, |transcript, ctx| transcript.clear_selection(ctx));
                self.input_view
                    .update(ctx, |input, ctx| input.clear_selection(ctx));
            }
            TuiTerminalSessionAction::ReadOnlyMenuSelectionEnded(text) => {
                match copy_to_clipboard(text) {
                    Ok(()) => self.show_copy_hint(ctx),
                    Err(error) => {
                        log::warn!("Failed to copy TUI read-only menu selection: {error}");
                        self.show_transient_hint(COPY_FAILED_HINT.to_owned(), ctx);
                    }
                }
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
#[path = "handoff/tests.rs"]
mod handoff_tests;
#[cfg(test)]
#[path = "terminal_session_view_tests.rs"]
mod tests;
