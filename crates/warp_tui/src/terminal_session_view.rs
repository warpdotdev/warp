//! Authenticated terminal-session TUI surface.
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use ai::project_context::model::{ProjectContextModel, ProjectContextModelEvent};
use async_channel::Sender;
use instant::Instant;
use parking_lot::FairMutex;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::settings::{AISettings, AISettingsChangedEvent};
use warp::tui_export::{
    AIAgentActionId, AIAgentActionResultType, AIAgentContext, AIAgentPtyWriteMode, AIConversation,
    AIConversationId, AcceptSlashCommandOrSavedPrompt, ActiveSession, ActiveSessionEvent,
    AgentConversationEntryId, AgentConversationListEntryState, AgentConversationsModel,
    AgentInteractionMetadata, AgentViewEntryOrigin, BlockId, BlocklistAIActionEvent,
    BlocklistAIActionModel, BlocklistAIContextModel, BlocklistAIController,
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, BlocklistAIInputModel, CLISubagentController,
    CLISubagentEvent, CLISubagentTarget, COMMAND_REGISTRY, CancellationReason, ChangelogModel,
    ChangelogModelEvent, ChangelogRequestType, CloudConversationData, CommandExecutionSource,
    ConversationFileExport, ConversationSelection, ConversationSelectionHandle,
    ConversationUsageTotals, ExecuteCommandEvent, GetRelevantFilesController, GitRepoModels,
    GitRepoStatusModel, GitStatusMetadata, LLMId, LLMPreferences, LLMPreferencesEvent,
    LOCAL_SKILLS_REMOTE_EXECUTION_ERROR_MESSAGE, ModelEvent, ParsedSlashCommandInput, PtyIntent,
    PtyIntentEvent, RepoDetectionSessionType, RepoDetectionSource, ServerConversationToken,
    ShellCommandExecutorEvent, SizeInfo, SizeUpdate, SkillReference, SlashCommandDataSource as _,
    SlashCommandSelectionBehavior, StartAgentExecutorEvent, StartAgentRequest, StaticCommand,
    TerminalModel, TerminalSurface, TerminalSurfaceInit, TranscriptScope, TuiMcpAction,
    TuiMcpManager, TuiSlashCommand, TuiSlashCommandDataSource, TuiSlashCommandDataSourceArgs,
    TuiZeroStateDataSource, UserTakeOverReason, WAKEUP_THROTTLE_PERIOD,
    block_context_from_terminal_model, build_slash_command_mixer, detect_possible_git_repo,
    export_conversation_markdown, prepare_conversation_block_restoration,
    record_saved_prompt_accepted, record_static_slash_command_accepted, saved_prompt_text_for_id,
    slash_command_selection_behavior, throttle,
};
use warp_core::features::FeatureFlag;
use warp_core::settings::Setting;
use warp_editor::model::CoreEditorModel;
use warp_errors::report_error;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::SingletonEntity;
use warpui_core::r#async::{SpawnedFutureHandle, Timer};
use warpui_core::elements::tui::{
    TuiChildView, TuiConstrainedBox, TuiContainer, TuiElement, TuiFlex, TuiSize, TuiStyle, TuiText,
};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, EditableBinding, FixedBinding};
use warpui_core::platform::TerminationMode;
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::alt_screen_view::AltScreenElement;
use crate::autoupdate::{TuiAutoupdater, TuiAutoupdaterEvent};
use crate::clipboard::copy_to_clipboard;
use crate::conversation_menu::{TuiConversationMenuEvent, TuiConversationMenuModel};
use crate::conversation_selection::TuiConversationSelection;
use crate::editor_interaction::TuiEditorCommand;
use crate::exit_confirmation::{CTRL_C_EXIT_WINDOW, ExitConfirmation};
use crate::inline_menu::{MAX_INLINE_MENU_ROWS, TuiInlineMenu, active_inline_menu};
use crate::input::view::TuiInputAction;
use crate::input::{TuiInputView, TuiInputViewEvent};
use crate::input_mode_policy::{self, TuiInputModePolicy};
use crate::input_suggestions_mode::TuiInputSuggestionsModeModel;
use crate::keybindings::{
    CONTEXTUAL_PLAN_TOGGLE_BINDING_NAME, KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG,
    PLAN_TOGGLE_AVAILABLE_FLAG, PLAN_TOGGLE_BINDING_NAME, TUI_BINDING_GROUP,
};
use crate::mcp_menu::{TuiMcpMenuEvent, TuiMcpMenuModel};
use crate::model_menu::{TuiModelMenuEvent, TuiModelMenuModel};
use crate::orchestrated_agent_identity_styling::assign_agent_identity_indices;
use crate::orchestration_block::TuiOrchestrationBlock;
use crate::orchestration_model::{TuiOrchestrationModel, TuiOrchestrationSnapshot};
use crate::platform::reveal_path_in_file_manager;
use crate::resume::TuiExitSummaryHandle;
use crate::session_registry::TuiSessions;
use crate::skills_menu::{TuiSkillMenuEvent, TuiSkillMenuModel};
use crate::slash_commands::TuiSlashCommandModel;
use crate::tab_bar::{
    TuiTab, TuiTabBarConfig, TuiTabBarEvent, TuiTabBarNavigationDirection, TuiTabBarSecondaryEdge,
    TuiTabBarView,
};
use crate::terminal_content_element::TuiTerminalContentElement;
use crate::terminal_use::{
    TerminalUseInterruptAction, TuiInputTarget, hide_agent_requested_command_from_top_level,
    terminal_use_conversation_to_resume, terminal_use_interrupt_action, tui_input_target,
};
use crate::transcript_view::{TuiTranscriptView, TuiTranscriptViewEvent};
use crate::transient_hint::{TransientHint, TransientHintTone};
use crate::tui_builder::TuiUiBuilder;
use crate::tui_cli_subagent_view::{
    HAND_BACK_KEY_BINDING, TAKE_CONTROL_KEY_BINDING, TuiCLISubagentView,
};
use crate::ui::{compact_footer_path, conversation_restore_failed, conversation_restoring};
use crate::usage::UsageToggle;
use crate::warping_indicator::{render_response_summary, render_warping_indicator};
use crate::zero_state::render_zero_state;
mod input_detection;

use self::input_detection::InputDetectionState;

/// Width used before the first layout pass pushes the real terminal width into the editor.
const INITIAL_INPUT_WIDTH: u16 = 80;
const MAX_INPUT_TEXT_ROWS: u16 = 6;
const ORCHESTRATION_TAB_BAR_FOCUSED_FLAG: &str = "TuiOrchestrationTabBarFocused";
const ORCHESTRATION_TAB_LABEL_MAX_COLUMNS: u16 = 20;

/// The footer hint shown while the ctrl-c exit confirmation is armed.
const CTRL_C_EXIT_HINT: &str = "ctrl-c again to exit";
const SESSION_CAN_CANCEL_RESTORE_FLAG: &str = "TuiSessionCanCancelRestore";
const SESSION_CAN_HAND_BACK_CONTROL_FLAG: &str = "TuiSessionCanHandBackControl";

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

/// Footer hint shown while the input is in `!` shell mode.
const SHELL_MODE_HINT: &str = "shell mode · esc to exit";
const COPY_SELECTION_HINT: &str = "copied to clipboard";
const COPY_FAILED_HINT: &str = "failed to copy to clipboard";
const LOG_BUNDLE_FAILED_HINT: &str = "Failed to create log bundle (check logs)";

fn log_bundle_success_message(path: &Path) -> String {
    format!("Log bundle saved to {}", path.display())
}

fn raw_prompt_if_not_blank(input: &str) -> Option<&str> {
    (!input.trim().is_empty()).then_some(input)
}

fn render_left_footer_hint(
    hint: Option<(&str, TuiStyle)>,
    show_conversations_hint: bool,
    builder: &TuiUiBuilder,
) -> Option<Box<dyn TuiElement>> {
    match hint {
        Some((text, style)) => Some(TuiText::new(text).with_style(style).truncate().finish()),
        None if show_conversations_hint => Some(
            TuiText::from_spans([
                ("←".to_owned(), builder.accent_text_style()),
                (" for conversations".to_owned(), builder.muted_text_style()),
            ])
            .truncate()
            .finish(),
        ),
        None => None,
    }
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
    /// Raw user bytes to forward to the foreground PTY process.
    ForwardUserPtyBytes(Vec<u8>),
    /// Ctrl-d while the prompt is focused: exit the TUI immediately when the
    /// prompt is empty, else delete the next character.
    Eof,
    /// Toggle the latest exposed inline plan.
    TogglePlan,
    /// Return keyboard focus from tabs to the session's default interaction target.
    FocusDefaultInteractionTarget,
    /// Select the previous tab using the tab view's semantic order.
    SelectPreviousOrchestrationTab,
    /// Select the next tab using the tab view's semantic order.
    SelectNextOrchestrationTab,
    /// Select the first child tab, excluding the orchestrator.
    SelectFirstOrchestrationChild,
    /// Select the last child tab, excluding the orchestrator.
    SelectLastOrchestrationChild,
}

/// The authenticated terminal/session surface rendered inside [`RootTuiView`].
pub(crate) struct TuiTerminalSessionView {
    transcript: ViewHandle<TuiTranscriptView>,
    input_view: ViewHandle<TuiInputView>,
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
    conversation_restore_state: ConversationRestoreState,
    next_restore_request_id: u64,
    exit_summary: TuiExitSummaryHandle,
    /// The view id of the blocker currently holding focus, tracked only to
    /// detect blocker transitions in [`Self::sync_blocker_focus`]. Input
    /// visibility itself is derived at render time, never stored.
    active_blocker_view_id: Option<EntityId>,
    orchestration_tab_bar: ViewHandle<TuiTabBarView>,
    orchestration_tabs_focused: bool,
}

/// Registers the session surface's keybindings. Called once at TUI startup
/// from `keybindings::init`. Ctrl-c is a fixed (non-remappable) binding,
/// mirroring peer agent CLIs that treat it as reserved.
pub(crate) fn init(app: &mut AppContext) {
    app.register_fixed_bindings([
        FixedBinding::new(
            TAKE_CONTROL_KEY_BINDING,
            TuiTerminalSessionAction::Interrupt,
            id!(TuiTerminalSessionView::ui_name()),
        )
        .with_group(TUI_BINDING_GROUP),
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
            PLAN_TOGGLE_BINDING_NAME,
            "Toggle the latest plan",
            TuiTerminalSessionAction::TogglePlan,
        )
        .with_context_predicate(id!(TuiTerminalSessionView::ui_name()))
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
    ]);

    // Tab navigation is user-remappable, unlike the reserved session-control
    // bindings above, so the two groups use different registration APIs.
    let tab_context =
        id!(TuiTerminalSessionView::ui_name()) & id!(ORCHESTRATION_TAB_BAR_FOCUSED_FLAG);
    app.register_editable_bindings([
        EditableBinding::new(
            "tui:orchestration_tabs:previous",
            "Select the previous orchestration tab",
            TuiTerminalSessionAction::SelectPreviousOrchestrationTab,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("left"),
        EditableBinding::new(
            "tui:orchestration_tabs:previous",
            "Select the previous orchestration tab",
            TuiTerminalSessionAction::SelectPreviousOrchestrationTab,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-tab"),
        EditableBinding::new(
            "tui:orchestration_tabs:next",
            "Select the next orchestration tab",
            TuiTerminalSessionAction::SelectNextOrchestrationTab,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("right"),
        EditableBinding::new(
            "tui:orchestration_tabs:next",
            "Select the next orchestration tab",
            TuiTerminalSessionAction::SelectNextOrchestrationTab,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("tab"),
        EditableBinding::new(
            "tui:orchestration_tabs:first_child",
            "Select the first child agent",
            TuiTerminalSessionAction::SelectFirstOrchestrationChild,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-left"),
        EditableBinding::new(
            "tui:orchestration_tabs:last_child",
            "Select the last child agent",
            TuiTerminalSessionAction::SelectLastOrchestrationChild,
        )
        .with_context_predicate(tab_context.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-right"),
        EditableBinding::new(
            "tui:orchestration_tabs:focus_input",
            "Return focus to the session input",
            TuiTerminalSessionAction::FocusDefaultInteractionTarget,
        )
        .with_context_predicate(tab_context)
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-down"),
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

    fn focus_current_owner(&mut self, ctx: &mut ViewContext<Self>) {
        match self.input_target() {
            TuiInputTarget::Disabled | TuiInputTarget::Pty => {
                self.orchestration_tabs_focused = false;
                ctx.focus_self();
            }
            TuiInputTarget::AgentEditor => {
                if let Some(blocker) = self.active_blocking_child(ctx) {
                    self.orchestration_tabs_focused = false;
                    ctx.focus(&blocker);
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
        ];
        let inline_menus_for_input = inline_menus.clone();
        let suggestions_mode_for_input = suggestions_mode.clone();
        let transcript_for_input = transcript.clone();
        let orchestration_tab_bar = ctx.add_typed_action_tui_view(|_| TuiTabBarView::empty());
        let orchestration_tab_bar_for_input = orchestration_tab_bar.clone();
        let input_view = ctx.add_typed_action_tui_view(move |ctx| {
            TuiInputView::new(
                input_editor_model,
                input_mode_for_input_view,
                suggestions_mode_for_input,
                inline_menus_for_input,
                transcript_for_input,
                move |ctx| orchestration_tab_bar_for_input.as_ref(ctx).has_tabs(),
                ctx,
            )
            .with_keyboard_enhancement_supported(keyboard_enhancement_supported)
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
        });

        ctx.subscribe_to_view(&input_view, |view, _, event, ctx| match event {
            TuiInputViewEvent::Submitted(text) => view.handle_submitted(text.clone(), ctx),
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

        // The zero state's "What's new" section: fetch the changelog once at
        // startup and re-render when it arrives. The model no-ops when a
        // changelog is already cached; the other changelog events (request
        // failed, image fetched) don't change what the zero state renders.
        ChangelogModel::handle(ctx).update(ctx, |changelog, ctx| {
            changelog.check_for_changelog(ChangelogRequestType::WindowLaunch, ctx);
        });
        ctx.subscribe_to_model(&ChangelogModel::handle(ctx), |_, _, event, ctx| {
            if let ChangelogModelEvent::ChangelogRequestComplete { .. } = event {
                ctx.notify();
            }
        });
        // The zero state's version line shows the background auto-update
        // status: re-render as the updater progresses.
        ctx.subscribe_to_model(&TuiAutoupdater::handle(ctx), |_, _, event, ctx| {
            let TuiAutoupdaterEvent::StatusChanged = event;
            ctx.notify();
        });
        // The zero state's project section: rules/skills discovery is
        // asynchronous, so re-render as indexed results land. `PathIndexed`
        // accompanies every project-rules mutation (`KnownRulesChanged` is a
        // persistence-oriented duplicate), and `GlobalRulesChanged` covers
        // global rules, which the zero state doesn't show.
        ctx.subscribe_to_model(&ProjectContextModel::handle(ctx), |_, _, event, ctx| {
            if let ProjectContextModelEvent::PathIndexed = event {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&TuiMcpManager::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
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
            ModelEvent::BootstrapPrecmdDone => {
                view.update_process_input_focus(ctx);
                ctx.notify();
            }
            ModelEvent::BlockMetadataReceived(_)
            | ModelEvent::BlockWorkingDirectoryUpdated(_)
            | ModelEvent::BackgroundBlockStarted
            | ModelEvent::TerminalClear
            | ModelEvent::PromptUpdated
            | ModelEvent::Typeahead
            | ModelEvent::Handler(_)
            | ModelEvent::FinishUpdate(_) => ctx.notify(),
            _ => {}
        });
        // The footer shows the active model, working directory, and usage
        // entry: re-render when the TUI model or usage-display-mode settings
        // change (click or settings-file hot reload), when model display
        // names arrive from the server post-login, or when the session's
        // working directory changes.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |view, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::TuiAgentModel { .. }
                    | AISettingsChangedEvent::TuiUsageDisplayMode { .. }
            ) {
                ctx.notify();
            }
            if matches!(event, AISettingsChangedEvent::AIAutoDetectionEnabled { .. }) {
                view.schedule_input_detection(ctx);
            }
        });
        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |_, _, event, ctx| {
            if let LLMPreferencesEvent::UpdatedAvailableLLMs = event {
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
        Self {
            transcript,
            input_view,
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
            keyboard_enhancement_supported,
            ai_context_model: context_model,
            ai_input_model,
            input_detection: InputDetectionState::default(),
            terminal_model: model,
            size_info,
            terminal_resize_tx,
            transient_hint: TransientHint::default(),
            conversation_restore_state: ConversationRestoreState::Idle,
            next_restore_request_id: 0,
            exit_summary,
            active_blocker_view_id: None,
            orchestration_tab_bar,
            orchestration_tabs_focused: false,
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
    fn set_orchestration_tab_focus(&mut self, focused: bool, ctx: &mut ViewContext<Self>) {
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
        let target_view = TuiSessions::as_ref(ctx)
            .session(session_id)
            .map(|session| session.view().clone());
        let Some(target_view) = target_view else {
            return;
        };
        target_view.update(ctx, |target, target_ctx| {
            target.set_orchestration_tab_focus(keep_tab_focus, target_ctx);
        });
    }

    /// Builds the tab child-view configuration for an orchestration snapshot.
    fn orchestration_tab_bar_config(
        &self,
        snapshot: &TuiOrchestrationSnapshot,
        builder: &TuiUiBuilder,
    ) -> TuiTabBarConfig {
        let palette = builder.agent_identity_palette();
        let mut children_in_spawn_order = snapshot.children.iter().collect::<Vec<_>>();
        children_in_spawn_order.sort_by_key(|child| child.spawn_index);
        let identity_indices = assign_agent_identity_indices(
            children_in_spawn_order
                .iter()
                .map(|child| child.label.as_str()),
            palette.len(),
        );
        let identity_by_conversation = children_in_spawn_order
            .into_iter()
            .map(|child| child.conversation_id)
            .zip(identity_indices)
            .collect::<HashMap<_, _>>();
        let tabs = snapshot
            .children
            .iter()
            .map(|child| {
                let identity = palette
                    .get(
                        identity_by_conversation
                            .get(&child.conversation_id)
                            .copied()
                            .unwrap_or_default(),
                    )
                    .or_else(|| palette.first())
                    .cloned()
                    .unwrap_or_default();
                TuiTab::new(child.conversation_id.to_string(), child.label.clone())
                    .with_leading_text(identity.glyph, identity.style)
            })
            .collect();
        let mut config = TuiTabBarConfig::new(tabs);
        config.leading = Some("   Agents:   ".to_owned());
        config.main_tab = Some(TuiTab::new(
            snapshot.root_conversation_id.to_string(),
            "orchestrator",
        ));
        config.selected_key = Some(snapshot.selected_conversation_id.to_string());
        config.focused = self.orchestration_tabs_focused;
        config.page_anchor = snapshot.page_anchor.map(|id| id.to_string());
        config.reveal_selected = snapshot.reveal_selected;
        config.maximum_label_columns = Some(ORCHESTRATION_TAB_LABEL_MAX_COLUMNS);
        config.secondary_gap_columns = 3;
        config.styles = builder.orchestration_tab_bar_styles();
        config
    }

    /// Synchronizes the retained tab child view from current orchestration state.
    fn sync_orchestration_tab_bar(
        &self,
        snapshot: &TuiOrchestrationSnapshot,
        builder: &TuiUiBuilder,
        ctx: &mut ViewContext<Self>,
    ) {
        let config = self.orchestration_tab_bar_config(snapshot, builder);
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
        let primary = builder.primary_text_style();
        let muted = builder.muted_text_style();
        TuiText::from_spans([
            ("Tab or ← →".to_string(), primary),
            (" to navigate  ".to_string(), muted),
            ("Shift + ← →".to_string(), primary),
            (" to go to start/end  ".to_string(), muted),
            ("Shift + ↓".to_string(), primary),
            (" to send a message".to_string(), muted),
        ])
        .truncate()
        .finish()
    }

    /// The active front-of-queue blocking interaction, if any.
    fn active_blocking_child(&self, ctx: &AppContext) -> Option<ViewHandle<TuiOrchestrationBlock>> {
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
        let blocker_view_id = blocker.as_ref().map(ViewHandle::id);
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
                    "The Warp TUI only supports Oz/Warp conversations.".to_owned(),
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
        if matches!(
            event,
            BlocklistAIHistoryEvent::AppendedExchange { .. }
                | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
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

    /// Builds the status footer under the input box. The left slot shows one
    /// hint at a time — the ctrl-c exit confirmation while armed, else a
    /// transient notice, else the shell-mode callout, else the conversations
    /// callout while the input is empty and no inline menu is visible, else the
    /// orchestration-tab callout; the active model and working directory are
    /// pushed to the right edge behind a flex spacer. Every child truncates to
    /// a single row, so the row lays out one row tall.
    fn render_footer(&self, orchestration_tabs_available: bool, ctx: &AppContext) -> TuiFlex {
        let builder = TuiUiBuilder::from_app(ctx);
        let muted = builder.muted_text_style();
        let mut left = TuiFlex::row();
        // Left slot, highest priority first: while armed, the ctrl-c hint
        // replaces the other hints in place.
        let hint = if self.exit_confirmation.is_armed() {
            Some((CTRL_C_EXIT_HINT, muted))
        } else if matches!(
            &self.conversation_restore_state,
            ConversationRestoreState::Loading {
                origin: TuiConversationRestoreOrigin::ConversationList,
                ..
            }
        ) {
            Some((LOADING_CONVERSATION_HINT, muted))
        } else if let Some((transient, tone)) = self.transient_hint.current() {
            let style = match tone {
                TransientHintTone::Muted => muted,
                TransientHintTone::Success => builder.success_glyph_style(),
            };
            Some((transient, style))
        } else if self.is_shell_mode(ctx) {
            Some((SHELL_MODE_HINT, builder.shell_mode_accent_style()))
        } else if orchestration_tabs_available {
            Some(("Shift + ↑ sub-agents", muted))
        } else {
            None
        };

        let show_conversations_hint = self.input_view.as_ref(ctx).is_empty(ctx)
            && !self.suggestions_mode.as_ref(ctx).mode().is_visible();
        if let Some(hint) = render_left_footer_hint(hint, show_conversations_hint, &builder) {
            left = left.child(hint);
        }
        let mut footer = TuiFlex::row().flex_child(left.finish());
        let model_name = LLMPreferences::as_ref(ctx)
            .get_active_base_model(ctx, Some(self.terminal_surface_id))
            .display_name
            .clone();
        footer = footer.child(TuiText::new(" ").truncate().finish()).child(
            TuiText::new(model_name)
                .with_style(builder.primary_text_style())
                .truncate()
                .finish(),
        );
        if let Some(cwd) = self.current_working_directory(ctx) {
            footer = footer.child(
                TuiText::new(format!(" {}", compact_footer_path(&cwd)))
                    .with_style(muted)
                    .truncate()
                    .finish(),
            );
        }
        let git_stats = if let Some(metadata) = self.git_status_metadata(ctx) {
            footer = footer.child(
                TuiText::new(format!(" ↬ {}", metadata.current_branch_name))
                    .with_style(muted)
                    .truncate()
                    .finish(),
            );
            Some(metadata.stats_against_head)
        } else {
            None
        };
        // Usage entry: the selected conversation's credits/cost totals,
        // hidden until any usage has been reported. The displayed unit is the
        // persisted `agents.usage_display_mode` setting; a click dispatches
        // the toggle action (the element pass cannot write settings
        // directly).
        if let Some(totals) = self.selected_conversation_usage_totals(ctx) {
            let mode = AISettings::as_ref(ctx).usage_display_mode;
            footer = footer
                .child(TuiText::new(" • ").with_style(muted).truncate().finish())
                .child(
                    self.usage_toggle
                        .render_entry(mode, totals, ctx, |event_ctx, _| {
                            event_ctx.dispatch_typed_action(
                                TuiTerminalSessionAction::ToggleUsageDisplay,
                            );
                        }),
                );
        }
        if let Some(stats) = git_stats
            && (stats.total_additions > 0 || stats.total_deletions > 0)
        {
            footer = footer.child(TuiText::new(" • ").with_style(muted).truncate().finish());
            if stats.total_additions > 0 {
                footer = footer.child(
                    TuiText::new(format!("+{}", stats.total_additions))
                        .with_style(builder.diff_added_style())
                        .truncate()
                        .finish(),
                );
            }
            if stats.total_deletions > 0 {
                if stats.total_additions > 0 {
                    footer = footer.child(TuiText::new(" ").truncate().finish());
                }
                footer = footer.child(
                    TuiText::new(format!("-{}", stats.total_deletions))
                        .with_style(builder.diff_removed_style())
                        .truncate()
                        .finish(),
                );
            }
        }
        footer
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
        let result = AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.agent_model.set_value(id.as_str().to_owned(), ctx)
        });
        if let Err(error) = result {
            report_error!(error.context("Failed to persist the TUI agent model"));
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
        let Some(tui_command) = TuiSlashCommand::from_static_command(command) else {
            log::debug!(
                "TUI slash command selection is not supported yet: {}",
                command.name
            );
            return;
        };

        match tui_command {
            TuiSlashCommand::Agent | TuiSlashCommand::New => {
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
            TuiSlashCommand::Conversations => {
                self.conversation_menu
                    .update(ctx, |menu, ctx| menu.open(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            TuiSlashCommand::Model => {
                self.model_menu.update(ctx, |menu, ctx| menu.open(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            TuiSlashCommand::Skills => {
                if !FeatureFlag::ListSkills.is_enabled() {
                    return;
                }
                self.skills_menu.update(ctx, |menu, ctx| menu.open(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            TuiSlashCommand::Mcp => {
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
                self.mcp_menu.update(ctx, |menu, ctx| menu.open(ctx));
                record_static_slash_command_accepted(command.name, true, ctx);
            }
            TuiSlashCommand::Exit => {
                record_static_slash_command_accepted(command.name, true, ctx);
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
            TuiSlashCommand::ViewLogs => {
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
            TuiSlashCommand::CreateNewProject => {
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
            TuiSlashCommand::ExportToClipboard => {
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
            TuiSlashCommand::ExportToFile => {
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
            TuiSlashCommand::Compact | TuiSlashCommand::Plan => {
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
        }
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
        let (alt_screen_active, input_target) = {
            let terminal_model = self.terminal_model.lock();
            (
                terminal_model.is_alt_screen_active(),
                tui_input_target(&terminal_model),
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
            content = content.flex_child(render_zero_state(
                self.current_working_directory(ctx).as_deref(),
                ctx,
            ));
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
                        TuiContainer::new(render_warping_indicator(label, elapsed, ctx))
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
                if let Some(duration) = wall_to_wall {
                    content = content.child(
                        TuiContainer::new(render_response_summary(
                            duration,
                            conversation.credits_spent_for_last_block(),
                            ctx,
                        ))
                        .with_padding_top(1)
                        .finish(),
                    );
                }
            }
        }
        if !blocker_active && input_target.agent_editor_owns_input() {
            if let Some(menu) = inline_menu {
                content = content.child(
                    TuiConstrainedBox::new(menu)
                        .with_max_rows(MAX_INLINE_MENU_ROWS)
                        .finish(),
                );
            }
            let border_style = if self.is_shell_mode(ctx) {
                builder.shell_mode_accent_style()
            } else {
                builder.accent_border_style()
            };
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
            let footer = if self.orchestration_tabs_focused {
                self.render_orchestration_tab_footer(&builder)
            } else {
                self.render_footer(orchestration_tabs_available, ctx)
                    .finish()
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
            TuiTerminalSessionAction::FocusDefaultInteractionTarget => {
                self.set_orchestration_tab_focus(false, ctx)
            }
            TuiTerminalSessionAction::SelectPreviousOrchestrationTab => {
                let key = self
                    .orchestration_tab_bar
                    .as_ref(ctx)
                    .navigation_target(TuiTabBarNavigationDirection::Previous);
                self.switch_to_orchestration_tab(key, true, ctx);
            }
            TuiTerminalSessionAction::SelectNextOrchestrationTab => {
                let key = self
                    .orchestration_tab_bar
                    .as_ref(ctx)
                    .navigation_target(TuiTabBarNavigationDirection::Next);
                self.switch_to_orchestration_tab(key, true, ctx);
            }
            TuiTerminalSessionAction::SelectFirstOrchestrationChild => {
                let key = self
                    .orchestration_tab_bar
                    .as_ref(ctx)
                    .secondary_edge_target(TuiTabBarSecondaryEdge::First);
                self.switch_to_orchestration_tab(key, true, ctx);
            }
            TuiTerminalSessionAction::SelectLastOrchestrationChild => {
                let key = self
                    .orchestration_tab_bar
                    .as_ref(ctx)
                    .secondary_edge_target(TuiTabBarSecondaryEdge::Last);
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
