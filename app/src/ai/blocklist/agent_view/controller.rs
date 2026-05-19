use std::{sync::Arc, time::Duration};

use instant::Instant;

use parking_lot::FairMutex;
use warp_core::ui::appearance::Appearance;
use warpui::keymap::Keystroke;
use warpui::AppContext;
use warpui::{
    r#async::SpawnedFutureHandle, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity,
};

use crate::terminal::input::message_bar::{Message, MessageItem};
use crate::terminal::input::slash_commands::SlashCommandTrigger;
use crate::util::bindings::keybinding_name_to_keystroke;
use crate::{
    ai::agent::conversation::AIConversationId, terminal::TerminalModel, BlocklistAIHistoryModel,
};

use super::{DismissalStrategy, EphemeralMessage, EphemeralMessageModel};

/// Error returned when entering the agent view fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EnterAgentViewError {
    #[error("Already in agent mode.")]
    AlreadyInAgentView,
    #[error("Cannot enter agent mode while a command is running.")]
    LongRunningCommand,
}

/// Error returned when exiting the agent view fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExitAgentViewError {
    #[error("Cannot exit agent while command is running.")]
    LongRunningCommand,
    #[error("Cannot exit conversation as a viewer.")]
    ConversationViewer,
    #[error("Cannot exit cloud agent.")]
    AmbientAgent,
}

/// The display mode for an active agent view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentViewDisplayMode {
    /// Full-screen agent view (navstack-based).
    FullScreen,
    /// Inline agent view (e.g., for long-running commands).
    Inline,
}

impl AgentViewDisplayMode {
    pub fn is_inline(self) -> bool {
        matches!(self, AgentViewDisplayMode::Inline)
    }

    pub fn is_fullscreen(self) -> bool {
        matches!(self, AgentViewDisplayMode::FullScreen)
    }
}

/// Shared timeout for all "press again to confirm" UX in and around agent view.
///
/// We intentionally keep enter/exit/new-conversation keybinding confirmation windows aligned so
/// users only learn one confirmation cadence.
pub const ENTER_OR_EXIT_CONFIRMATION_WINDOW: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitConfirmationTrigger {
    Escape,
    CtrlC,
}

#[derive(Debug, Clone)]
enum PendingConfirmation {
    Exit {
        conversation_id: AIConversationId,
        expires_at: Instant,
    },
    NewConversationKeybinding {
        conversation_id: AIConversationId,
        normalized_keystroke: String,
        expires_at: Instant,
    },
}

impl PendingConfirmation {
    fn message_id(&self) -> &'static str {
        match self {
            PendingConfirmation::Exit { .. } => EXIT_CONFIRMATION_MESSAGE_ID,
            PendingConfirmation::NewConversationKeybinding { .. } => {
                NEW_CONVERSATION_KEYBINDING_CONFIRMATION_MESSAGE_ID
            }
        }
    }
}

/// The different types of agent view entrypoints.
///
/// Depending on the entrypoint, an `AgentView` block representing the entry may be inserted into
/// the terminal blocklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentViewEntryOrigin {
    /// Entered agent view from user input (e.g. /agent or cmd-enter keypress).
    Input {
        was_prompt_autodetected: bool,
    },
    PromptChip,
    /// Entered agent view by selecting a conversation (e.g. selector).
    ConversationSelector,
    /// Entered agent view clicking a conversation item in the Agent Mode homepage (tab zero state).
    AgentModeHomepage,
    /// Entered agent view by clicking an existing agent view block.
    AgentViewBlock,
    /// Entered agent view from the AI document pane.
    AIDocument,
    /// Entered agent view due to an automatic follow-up (not a direct user selection).
    AutoFollowUp,
    /// Entered agent view due to conversation restoration on startup or forking.
    RestoreExistingConversation,
    /// Entered agent view due to shared-session synchronization.
    SharedSessionSelection,
    /// Entered agent view due to a server-driven conversation split (StartNewConversation client action).
    AgentRequestedNewConversation,
    /// Entered agent view via accepting a prompt suggestion.
    AcceptedPromptSuggestion,
    /// Entered agent view via accepting a unit test suggestion.
    AcceptedUnitTestSuggestion,
    /// Entered agent view via accepting a passive code diff.
    AcceptedPassiveCodeDiff,
    /// Entered agent view by starting conversation with an inline code review submission.
    InlineCodeReview,
    /// Entered agent view through a cloud agent prompt.
    CloudAgent,
    /// Entered agent view by opening an existing non-Oz cloud agent run (live shared-session
    /// viewer or transcript viewer).
    ThirdPartyCloudAgent,
    /// Entered agent view via the CLI (e.g. `warp agent run`).
    Cli,
    /// Entered agent view by adding an image (drag-and-drop or paste).
    ImageAdded,
    /// Entered agent view by executing a slash command that requires agent mode.
    SlashCommand {
        trigger: SlashCommandTrigger,
    },
    SlashInit,
    CreateEnvironment,
    /// Entered agent view by executing a slash command that requires agent mode.
    Keybinding,
    /// Entered agent view by attaching context from the code review panel.
    CodeReviewContext,
    /// Entered agent view from codex integration modal.
    CodexModal,
    /// Entered agent view by selecting a conversation from the inline history menu.
    InlineHistoryMenu,
    InlineConversationMenu,
    OnboardingCallout,
    ConversationListView,
    /// Entered agent view because the default session mode setting is Agent.
    DefaultSessionMode,

    /// Entered agent view by long-running command.
    LongRunningCommand,

    /// Entered agent view from the onboarding flow.
    Onboarding,

    /// Entered agent view because a parent agent started this child agent via StartAgent.
    ChildAgent,

    /// Entered agent view by clicking a pill / breadcrumb in the orchestration
    /// pill bar (or breadcrumb row) to navigate the current pane to a sibling
    /// or parent conversation in the same orchestration tree.
    OrchestrationPillBar,

    /// Entered agent view after opening project from OS directory picker.
    ProjectEntry,

    /// Entered agent view via a Linear "work on issue" deeplink.
    LinearDeepLink,

    /// Entered agent view by clearing the buffer (Cmd+K) while already in agent view.
    ClearBuffer,

    // The variants below actually correspond to callsites where the selected conversation is
    // updated, but don't actually correspond to entering the agent view. They exist so we can
    // continue to call `set_pending_query_state_for_(new|existing)_conversation`, but you'll find
    // that their callsites are actually gated on `AgentView` being disabled. Once `AgentView` is
    // launched, those callsites for updating the selected conversation will be removed along with
    // these variants.
    ContinueConversationButton,
    ViewPassiveCodeDiffDetails,
    ResumeConversationButton,
}

/// Controls when `try_enter_agent_view` is allowed to auto-submit an initial prompt
/// to the LLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoTriggerBehavior {
    /// Always auto-submit the prompt, regardless of prior agent-view state.
    Always,
    /// Auto-submit only when the user was already in agent view before this entry.
    InAgentView,
    /// Never auto-submit. The prompt is placed into the input buffer as a draft.
    Never,
}

impl AgentViewEntryOrigin {
    pub fn is_cloud_agent(&self) -> bool {
        matches!(self, Self::CloudAgent)
    }

    pub fn should_autotrigger_request(&self) -> AutoTriggerBehavior {
        match self {
            AgentViewEntryOrigin::Input {
                was_prompt_autodetected,
            } if *was_prompt_autodetected => AutoTriggerBehavior::Always,
            AgentViewEntryOrigin::SlashCommand { trigger } if !trigger.is_keybinding() => {
                AutoTriggerBehavior::Always
            }
            AgentViewEntryOrigin::Cli => AutoTriggerBehavior::Always,
            AgentViewEntryOrigin::AcceptedPromptSuggestion => AutoTriggerBehavior::Always,
            AgentViewEntryOrigin::LinearDeepLink => AutoTriggerBehavior::Never,
            _ => AutoTriggerBehavior::InAgentView,
        }
    }
}

/// Terminal view-scoped state representing whether or not the user is engaged in an active agent view.
#[derive(Debug, Clone)]
pub enum AgentViewState {
    Active {
        conversation_id: AIConversationId,
        origin: AgentViewEntryOrigin,
        display_mode: AgentViewDisplayMode,

        // The number of exchanges in the conversation when the agent view was entered.
        //
        // This can be used to determine if a conversation has been updated within an agent view
        // 'entry'.
        original_conversation_length: usize,
    },
    Inactive,
}

impl AgentViewState {
    pub fn active_conversation_id(&self) -> Option<AIConversationId> {
        match self {
            AgentViewState::Active {
                conversation_id, ..
            } => Some(*conversation_id),
            AgentViewState::Inactive => None,
        }
    }

    /// Returns the display mode if active, `None` if inactive.
    pub fn display_mode(&self) -> Option<AgentViewDisplayMode> {
        match self {
            AgentViewState::Active { display_mode, .. } => Some(*display_mode),
            AgentViewState::Inactive => None,
        }
    }

    pub fn origin(&self) -> Option<AgentViewEntryOrigin> {
        match self {
            AgentViewState::Active { origin, .. } => Some(*origin),
            AgentViewState::Inactive => None,
        }
    }

    /// Returns `true` if in an active agent view state.
    pub fn is_active(&self) -> bool {
        matches!(self, AgentViewState::Active { .. })
    }

    /// Returns `true` if in inline display mode.
    pub fn is_inline(&self) -> bool {
        self.display_mode().is_some_and(|mode| mode.is_inline())
    }

    /// Returns `true` if in fullscreen display mode.
    pub fn is_fullscreen(&self) -> bool {
        self.display_mode().is_some_and(|mode| mode.is_fullscreen())
    }

    pub fn fullscreen_conversation_id(&self) -> Option<AIConversationId> {
        match self {
            AgentViewState::Active {
                conversation_id,
                display_mode: AgentViewDisplayMode::FullScreen,
                ..
            } => Some(*conversation_id),
            _ => None,
        }
    }

    pub fn is_new(&self) -> bool {
        match self {
            AgentViewState::Active {
                original_conversation_length,
                ..
            } => *original_conversation_length == 0,
            AgentViewState::Inactive => false,
        }
    }

    /// Returns true if the conversation has been modified since the agent view was opened.
    /// New (empty) conversations are always considered modified.
    /// If a conversation is not open in an agent view, this is always false.
    pub fn was_conversation_modified_since_opening(
        &self,
        history_model: &BlocklistAIHistoryModel,
    ) -> bool {
        match self {
            AgentViewState::Active {
                conversation_id,
                original_conversation_length,
                ..
            } => {
                if *original_conversation_length == 0 {
                    return true;
                }

                let current_conversation_length = history_model
                    .conversation(conversation_id)
                    .map(|c| c.exchange_count())
                    .unwrap_or(0);
                current_conversation_length > *original_conversation_length
            }
            AgentViewState::Inactive => false,
        }
    }

    /// Returns the save position ID for the zero state block, if active.
    pub fn zero_state_position_id(&self) -> Option<String> {
        self.active_conversation_id()
            .map(|id| format!("agent_view_zero_state_{}", id))
    }
}

const EXIT_CONFIRMATION_MESSAGE_ID: &str = "exit_confirmation_message";
const NEW_CONVERSATION_KEYBINDING_CONFIRMATION_MESSAGE_ID: &str =
    "new_conversation_keybinding_confirmation_message";

/// Controller responsible for managing and updating agent view state for a given terminal pane.
///
/// `AgentViewState` is stored on the terminal model but should only be updated via the APIs on
/// this controller, which ensures the correct events are emitted and downstream effects take
/// place.
pub struct AgentViewController {
    terminal_model: Arc<FairMutex<TerminalModel>>,
    terminal_view_id: EntityId,
    /// The entity ID of the pane group this terminal view lives in.
    /// Set during terminal pane attach; used for pane-group-scoped visibility checks.
    pane_group_id: Option<EntityId>,
    agent_view_state: AgentViewState,
    ephemeral_message_model: ModelHandle<EphemeralMessageModel>,
    pending_confirmation: Option<PendingConfirmation>,
    pending_confirmation_abort_handle: Option<SpawnedFutureHandle>,
}

#[derive(Debug, Clone, Copy)]
enum ExitConfirmationRequirement {
    /// Unconditionally require confirmation.
    Required,
    /// Require exit confirmation if the conversation is currently in progress (this is the default).
    IfInProgress,
    /// No exit confirmation required.
    None,
}

#[derive(Debug, Clone, Copy)]
struct ExitAgentViewOptions {
    should_confirm: ExitConfirmationRequirement,
}

impl AgentViewController {
    pub fn new(
        terminal_model: Arc<FairMutex<TerminalModel>>,
        terminal_view_id: EntityId,
        ephemeral_message_model: ModelHandle<EphemeralMessageModel>,
    ) -> Self {
        Self {
            terminal_model,
            terminal_view_id,
            pane_group_id: None,
            agent_view_state: AgentViewState::Inactive,
            ephemeral_message_model,
            pending_confirmation: None,
            pending_confirmation_abort_handle: None,
        }
    }

    pub fn pane_group_id(&self) -> Option<EntityId> {
        self.pane_group_id
    }

    /// Returns the [`EntityId`] of the [`TerminalView`] that owns this controller.
    pub fn terminal_view_id(&self) -> EntityId {
        self.terminal_view_id
    }

    pub fn set_pane_group_id(&mut self, pane_group_id: EntityId) {
        self.pane_group_id = Some(pane_group_id);
    }

    pub fn is_active(&self) -> bool {
        self.agent_view_state.is_active()
    }

    pub fn is_inline(&self) -> bool {
        self.agent_view_state.is_inline()
    }

    pub fn is_fullscreen(&self) -> bool {
        self.agent_view_state.is_fullscreen()
    }

    pub fn agent_view_state(&self) -> &AgentViewState {
        &self.agent_view_state
    }

    /// Returns whether the user is allowed to exit agent view.
    /// This is used to determine both whether the escape key should work
    /// and whether the escape keybinding should be displayed.
    pub fn can_exit_agent_view(&self) -> Result<(), ExitAgentViewError> {
        let model = self.terminal_model.lock();

        let is_fullscreen_with_long_running = self.agent_view_state.is_fullscreen()
            && model
                .block_list()
                .active_block()
                .is_active_and_long_running();

        // Cloud agent panes do not have the same underlying terminal ownership
        // constraint (no local shell process), so long-running third party agent
        // commands should not trap the user in agent view.
        if is_fullscreen_with_long_running && !model.is_dummy_cloud_mode_session() {
            return Err(ExitAgentViewError::LongRunningCommand);
        }

        // Conversation viewers have no underlying terminal session to return to,
        // so exiting agent view is not allowed.
        if model.is_conversation_transcript_viewer() {
            return Err(ExitAgentViewError::ConversationViewer);
        }

        Ok(())
    }

    /// If set, indicates the user attempted to exit an in-progress conversation and we are
    /// waiting for a second exit attempt to confirm cancelling/exiting.
    pub fn pending_exit_confirmation_conversation_id(&self) -> Option<AIConversationId> {
        match self.pending_confirmation.as_ref() {
            Some(PendingConfirmation::Exit {
                conversation_id,
                expires_at,
            }) if *expires_at > Instant::now() => Some(*conversation_id),
            _ => None,
        }
    }

    pub fn clear_pending_exit_confirmation(&mut self, ctx: &mut ModelContext<Self>) {
        self.clear_exit_confirmation(ctx);
    }
    fn clear_pending_confirmation(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.pending_confirmation_abort_handle.take() {
            handle.abort();
        }

        let Some(pending_confirmation) = self.pending_confirmation.take() else {
            return;
        };

        if self
            .ephemeral_message_model
            .as_ref(ctx)
            .current_message()
            .is_some_and(|message| message.id() == Some(pending_confirmation.message_id()))
        {
            self.ephemeral_message_model
                .update(ctx, |model, ctx| model.clear_message(ctx));
        }
    }

    fn set_pending_confirmation(
        &mut self,
        pending_confirmation: PendingConfirmation,
        message: Message,
        ctx: &mut ModelContext<Self>,
    ) {
        let message_id = pending_confirmation.message_id();
        self.clear_pending_confirmation(ctx);
        self.pending_confirmation = Some(pending_confirmation);

        let abort_handle = ctx.spawn_abortable(
            async move { warpui::r#async::Timer::after(ENTER_OR_EXIT_CONFIRMATION_WINDOW).await },
            move |me, _, _ctx| {
                me.pending_confirmation = None;
                me.pending_confirmation_abort_handle = None;
            },
            |_, _| (),
        );
        self.pending_confirmation_abort_handle = Some(abort_handle);

        self.ephemeral_message_model.update(ctx, |model, ctx| {
            model.show_ephemeral_message(
                EphemeralMessage::new(
                    message,
                    DismissalStrategy::Timer(ENTER_OR_EXIT_CONFIRMATION_WINDOW),
                )
                .with_id(message_id),
                ctx,
            )
        });
    }

    fn clear_exit_confirmation(&mut self, ctx: &mut ModelContext<Self>) {
        if self
            .pending_confirmation
            .as_ref()
            .is_some_and(|confirmation| matches!(confirmation, PendingConfirmation::Exit { .. }))
        {
            self.clear_pending_confirmation(ctx);
        }
    }

    fn set_exit_confirmation(
        &mut self,
        conversation_id: AIConversationId,
        trigger: ExitConfirmationTrigger,
        ctx: &mut ModelContext<Self>,
    ) {
        self.clear_exit_confirmation(ctx);

        let should_stop_and_exit = BlocklistAIHistoryModel::handle(ctx)
            .as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| {
                conversation.status().is_in_progress() && !conversation.is_empty()
            });
        self.set_pending_confirmation(
            PendingConfirmation::Exit {
                conversation_id,
                expires_at: Instant::now() + ENTER_OR_EXIT_CONFIRMATION_WINDOW,
            },
            exit_confirmation_message(trigger, should_stop_and_exit, ctx),
            ctx,
        );
    }

    fn is_exit_confirmation_active_for(&self, conversation_id: AIConversationId) -> bool {
        matches!(
            self.pending_confirmation.as_ref(),
            Some(PendingConfirmation::Exit {
                conversation_id: pending_conversation_id,
                expires_at,
            }) if *pending_conversation_id == conversation_id && *expires_at > Instant::now()
        )
    }

    /// Decides whether a keybinding-triggered `/agent` or `/new` should proceed immediately.
    ///
    /// We only require a second press when the user is already in an active, non-empty
    /// conversation. This protects against accidental conversation resets from muscle-memory
    /// keypresses, while preserving single-step behavior for explicit typed/slash-menu execution.
    pub fn should_start_new_conversation_for_keybinding(
        &mut self,
        keybinding_name: &str,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        enum Decision {
            StartNewConversation,
            ArmConfirmation {
                conversation_id: AIConversationId,
                keystroke: Keystroke,
                normalized_keystroke: String,
            },
        }

        let decision = 'decision: {
            let Some(conversation_id) = self.agent_view_state.active_conversation_id() else {
                break 'decision Decision::StartNewConversation;
            };

            let Some(conversation) = BlocklistAIHistoryModel::handle(ctx)
                .as_ref(ctx)
                .conversation(&conversation_id)
            else {
                break 'decision Decision::StartNewConversation;
            };

            // Empty conversations have no user/agent history to lose, so no confirmation is needed.
            if conversation.is_empty() {
                break 'decision Decision::StartNewConversation;
            }

            let Some(keystroke) = keybinding_name_to_keystroke(keybinding_name, ctx) else {
                log::warn!(
                    "Expected keybinding for slash command {keybinding_name}, but none was found"
                );
                break 'decision Decision::StartNewConversation;
            };

            let normalized_keystroke = keystroke.normalized();
            if self.is_new_conversation_keybinding_confirmation_active_for(
                conversation_id,
                &normalized_keystroke,
            ) {
                break 'decision Decision::StartNewConversation;
            }

            break 'decision Decision::ArmConfirmation {
                conversation_id,
                keystroke,
                normalized_keystroke,
            };
        };

        match decision {
            Decision::StartNewConversation => {
                self.clear_new_conversation_keybinding_confirmation(ctx);
                true
            }
            Decision::ArmConfirmation {
                conversation_id,
                keystroke,
                normalized_keystroke,
            } => {
                self.set_new_conversation_keybinding_confirmation(
                    conversation_id,
                    keystroke,
                    normalized_keystroke,
                    ctx,
                );
                false
            }
        }
    }

    fn clear_new_conversation_keybinding_confirmation(&mut self, ctx: &mut ModelContext<Self>) {
        if self
            .pending_confirmation
            .as_ref()
            .is_some_and(|confirmation| {
                matches!(
                    confirmation,
                    PendingConfirmation::NewConversationKeybinding { .. }
                )
            })
        {
            self.clear_pending_confirmation(ctx);
        }
    }

    fn set_new_conversation_keybinding_confirmation(
        &mut self,
        conversation_id: AIConversationId,
        keystroke: Keystroke,
        normalized_keystroke: String,
        ctx: &mut ModelContext<Self>,
    ) {
        self.clear_new_conversation_keybinding_confirmation(ctx);

        self.set_pending_confirmation(
            PendingConfirmation::NewConversationKeybinding {
                conversation_id,
                normalized_keystroke,
                expires_at: Instant::now() + ENTER_OR_EXIT_CONFIRMATION_WINDOW,
            },
            new_conversation_keybinding_confirmation_message(keystroke, ctx),
            ctx,
        );
    }

    fn is_new_conversation_keybinding_confirmation_active_for(
        &self,
        conversation_id: AIConversationId,
        normalized_keystroke: &str,
    ) -> bool {
        matches!(
            self.pending_confirmation.as_ref(),
            Some(PendingConfirmation::NewConversationKeybinding {
                conversation_id: pending_conversation_id,
                normalized_keystroke: pending_keystroke,
                expires_at,
            }) if *pending_conversation_id == conversation_id
                && pending_keystroke == normalized_keystroke
                && *expires_at > Instant::now()
        )
    }

    /// Attempts to enter fullscreen agent view for the given conversation ID, creating a new
    /// conversation if none is provided.
    ///
    /// Returns `Ok(conversation_id)` on success, or `Err` if entry is blocked.
    pub fn try_enter_agent_view(
        &mut self,
        conversation_id: Option<AIConversationId>,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Self>,
    ) -> Result<AIConversationId, EnterAgentViewError> {
        // Block entry to fullscreen mode if there's an active long-running command. Transcript
        // viewers and 3p cloud viewers are exempt: in those contexts the long-running block is
        // either a restored snapshot or the harness CLI we want to wrap in agent-view chrome.
        let is_long_running = {
            let terminal_model = self.terminal_model.lock();
            terminal_model
                .block_list()
                .active_block()
                .is_active_and_long_running()
                && !terminal_model.is_conversation_transcript_viewer()
                && !matches!(origin, AgentViewEntryOrigin::ThirdPartyCloudAgent)
        };

        if is_long_running {
            return Err(EnterAgentViewError::LongRunningCommand);
        }

        self.enter_agent_view_internal(
            conversation_id,
            origin,
            AgentViewDisplayMode::FullScreen,
            ctx,
        )
    }

    /// Attempts to enter inline agent view for the given conversation ID, creating a new
    /// conversation if none is provided.
    ///
    /// Returns `Ok(conversation_id)` on success, or `Err` if entry is blocked.
    pub fn try_enter_inline_agent_view(
        &mut self,
        conversation_id: Option<AIConversationId>,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Self>,
    ) -> Result<AIConversationId, EnterAgentViewError> {
        if self.agent_view_state.is_active() {
            return Err(EnterAgentViewError::AlreadyInAgentView);
        }
        self.enter_agent_view_internal(conversation_id, origin, AgentViewDisplayMode::Inline, ctx)
    }

    fn enter_agent_view_internal(
        &mut self,
        conversation_id: Option<AIConversationId>,
        origin: AgentViewEntryOrigin,
        display_mode: AgentViewDisplayMode,
        ctx: &mut ModelContext<Self>,
    ) -> Result<AIConversationId, EnterAgentViewError> {
        self.clear_pending_confirmation(ctx);
        match &self.agent_view_state {
            AgentViewState::Active {
                conversation_id: active_id,
                ..
            } => {
                if conversation_id.is_some_and(|id| id == *active_id) {
                    return Ok(*active_id);
                } else {
                    self.exit_agent_view_internal(
                        ExitAgentViewOptions {
                            should_confirm: ExitConfirmationRequirement::None,
                        },
                        ExitConfirmationTrigger::Escape,
                        true,
                        ctx,
                    );
                }
            }
            AgentViewState::Inactive => {}
        }

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let (conversation_id, exchange_count) = if let Some(conversation) =
            conversation_id.and_then(|id| history_model.as_ref(ctx).conversation(&id))
        {
            (conversation.id(), conversation.exchange_count())
        } else {
            let id = history_model.update(ctx, |history_model, ctx| {
                history_model.start_new_conversation(
                    self.terminal_view_id,
                    false,
                    matches!(origin, AgentViewEntryOrigin::CloudAgent),
                    matches!(origin, AgentViewEntryOrigin::ThirdPartyCloudAgent),
                    ctx,
                )
            });
            (id, 0)
        };
        // Non-transferring: don't rip the conversation out of another terminal
        // view's live list (e.g. a child agent's hidden pane). Explicit
        // cross-view ownership transfer is handled by callers elsewhere.
        history_model.update(ctx, |history_model, ctx| {
            history_model.mark_active_conversation_id(conversation_id, self.terminal_view_id, ctx)
        });

        self.agent_view_state = AgentViewState::Active {
            conversation_id,
            origin,
            display_mode,
            original_conversation_length: exchange_count,
        };
        self.terminal_model
            .lock()
            .block_list_mut()
            .set_agent_view_state(self.agent_view_state.clone());

        ctx.emit(AgentViewControllerEvent::EnteredAgentView {
            conversation_id,
            is_new: exchange_count == 0,
            origin,
            display_mode,
        });

        Ok(conversation_id)
    }

    /// Exits the agent view with required confirmation.
    ///
    /// If there is an active confirmation 'window', exits the view, else starts a confirmation
    /// 'window' for exit to be attempted again, in which case exit will occur.
    pub(crate) fn exit_agent_view_with_required_confirmation(
        &mut self,
        trigger: ExitConfirmationTrigger,
        ctx: &mut ModelContext<Self>,
    ) {
        self.exit_agent_view_internal(
            ExitAgentViewOptions {
                should_confirm: ExitConfirmationRequirement::Required,
            },
            trigger,
            false,
            ctx,
        );
    }

    /// Exits the active agent view without any confirmation.
    pub(crate) fn exit_agent_view_without_confirmation(&mut self, ctx: &mut ModelContext<Self>) {
        self.exit_agent_view_internal(
            ExitAgentViewOptions {
                should_confirm: ExitConfirmationRequirement::None,
            },
            ExitConfirmationTrigger::Escape,
            false,
            ctx,
        );
    }

    /// Exits the active agent view, if there is one.
    pub fn exit_agent_view(&mut self, ctx: &mut ModelContext<Self>) {
        let should_confirm = if self.agent_view_state.is_inline() {
            ExitConfirmationRequirement::None
        } else {
            ExitConfirmationRequirement::IfInProgress
        };
        self.exit_agent_view_internal(
            ExitAgentViewOptions { should_confirm },
            ExitConfirmationTrigger::Escape,
            false,
            ctx,
        );
    }

    fn exit_agent_view_internal(
        &mut self,
        options: ExitAgentViewOptions,
        trigger: ExitConfirmationTrigger,
        is_exit_before_new_entrance: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.clear_new_conversation_keybinding_confirmation(ctx);
        // Check if exiting agent view is allowed.
        if self.can_exit_agent_view().is_err() {
            return;
        }

        let AgentViewState::Active {
            conversation_id,
            original_conversation_length,
            ..
        } = self.agent_view_state
        else {
            return;
        };
        let was_new = original_conversation_length == 0;

        let should_confirm = match options.should_confirm {
            ExitConfirmationRequirement::Required => true,
            ExitConfirmationRequirement::IfInProgress => {
                // If the conversation is still in progress, require a second exit attempt within a short
                // window, unless this is a brand new empty conversation.
                let history_model = BlocklistAIHistoryModel::handle(ctx);
                history_model
                    .as_ref(ctx)
                    .conversation(&conversation_id)
                    .is_some_and(|conversation| {
                        conversation.status().is_in_progress()
                            && !(was_new && conversation.exchange_count() == 0)
                    })
            }
            ExitConfirmationRequirement::None => false,
        };

        if should_confirm {
            if self.is_exit_confirmation_active_for(conversation_id) {
                self.clear_exit_confirmation(ctx);
                ctx.emit(AgentViewControllerEvent::ExitConfirmed { conversation_id });
            } else {
                self.set_exit_confirmation(conversation_id, trigger, ctx);
                return;
            }
        } else {
            self.clear_exit_confirmation(ctx);
        }

        let mut old_state = AgentViewState::Inactive;
        std::mem::swap(&mut self.agent_view_state, &mut old_state);
        let AgentViewState::Active {
            conversation_id,
            origin,
            display_mode,
            original_conversation_length,
        } = old_state
        else {
            return;
        };

        self.terminal_model
            .lock()
            .block_list_mut()
            .set_agent_view_state(self.agent_view_state.clone());

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let final_exchange_count = history_model
            .as_ref(ctx)
            .conversation(&conversation_id)
            .map(|conversation| conversation.exchange_count())
            .unwrap_or(0);

        ctx.emit(AgentViewControllerEvent::ExitedAgentView {
            conversation_id,
            origin,
            display_mode,
            original_exchange_count: original_conversation_length,
            final_exchange_count,
            was_ambient_agent: origin == AgentViewEntryOrigin::CloudAgent,
            is_exit_before_new_entrance,
        });
    }
}

#[derive(Debug, Clone)]
pub enum AgentViewControllerEvent {
    EnteredAgentView {
        conversation_id: AIConversationId,
        origin: AgentViewEntryOrigin,
        display_mode: AgentViewDisplayMode,
        is_new: bool,
    },
    ExitedAgentView {
        /// The conversation ID that was active in the agent view.
        conversation_id: AIConversationId,
        /// The origin of the agent view entry that is now being exited.
        origin: AgentViewEntryOrigin,
        /// The display mode of the agent view that is being exited.
        display_mode: AgentViewDisplayMode,
        /// The number of exchanges in the conversation when agent view was entered.
        original_exchange_count: usize,
        /// The number of exchanges in the conversation when agent view is being exited.
        final_exchange_count: usize,
        /// Whether this was an ambient (cloud) agent session.
        was_ambient_agent: bool,
        /// Whether this exit is immediately followed by entering a new agent view.
        /// (e.g. Cmd+K while already in agent view to start a new conversation).
        is_exit_before_new_entrance: bool,
    },
    ExitConfirmed {
        conversation_id: AIConversationId,
    },
}

impl Entity for AgentViewController {
    type Event = AgentViewControllerEvent;
}

fn exit_confirmation_message(
    trigger: ExitConfirmationTrigger,
    should_stop_and_exit: bool,
    app: &AppContext,
) -> Message {
    use warpui::SingletonEntity;

    use crate::terminal::input::message_bar::{Message, MessageItem};

    let appearance = Appearance::handle(app).as_ref(app);

    let (keystroke, text) = match trigger {
        ExitConfirmationTrigger::Escape => (
            Keystroke {
                key: "escape".to_owned(),
                ..Default::default()
            },
            if should_stop_and_exit {
                "again to stop and exit"
            } else {
                "again to exit"
            },
        ),
        ExitConfirmationTrigger::CtrlC => (
            Keystroke {
                key: "c".to_owned(),
                ctrl: true,
                ..Default::default()
            },
            "again to exit",
        ),
    };

    Message::new(vec![
        MessageItem::keystroke(keystroke),
        MessageItem::text(text),
    ])
    .with_text_color(appearance.theme().ansi_fg_red())
}

fn new_conversation_keybinding_confirmation_message(
    keystroke: Keystroke,
    app: &AppContext,
) -> Message {
    let appearance = Appearance::handle(app).as_ref(app);
    Message::new(vec![
        MessageItem::keystroke(keystroke),
        MessageItem::text("again to start new conversation"),
    ])
    .with_text_color(appearance.theme().ansi_fg_magenta())
}
