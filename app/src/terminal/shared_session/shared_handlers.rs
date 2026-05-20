use std::cell::Cell;
use std::rc::Rc;

use input_classifier::InputType;
use session_sharing_protocol::common::{
    CLIAgentSessionState, InputMode, InputType as ProtocolInputType, SelectedAgentModel,
    SelectedConversation, ServerConversationToken, UniversalDeveloperInputContextUpdate,
};
use warp_core::features::FeatureFlag;
use warpui::{AppContext, ModelHandle, SingletonEntity, WeakViewHandle};

use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewEntryOrigin};
use crate::ai::blocklist::{BlocklistAIContextModel, BlocklistAIHistoryModel, InputConfig};
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputEntrypoint, CLIAgentInputState, CLIAgentRichInputCloseReason, CLIAgentSession,
    CLIAgentSessionContext, CLIAgentSessionStatus, CLIAgentSessionsModel,
};
use crate::terminal::CLIAgent;
use crate::terminal::TerminalView;

/// Handles updating the local LLM preferences when a selected agent model update is received.
/// This function is shared between the viewer and sharer to ensure consistent behavior.
pub(crate) fn apply_selected_agent_model_update(
    terminal_view_id: warpui::EntityId,
    selected_model: &SelectedAgentModel,
    _guard: &ActiveRemoteUpdate,
    ctx: &mut AppContext,
) {
    let model_id = LLMId::from(selected_model.model_id().to_owned());

    // Check if this is already our current model - if so, skip the update to avoid loops
    let llm_prefs = LLMPreferences::as_ref(ctx);
    let current_model_id = llm_prefs
        .get_active_base_model(ctx, Some(terminal_view_id))
        .id
        .clone();
    if current_model_id == model_id {
        return;
    }

    // Check if the model is available to the viewer. If not, skip the update.
    // This handles cases where the viewer and sharer have different model permissions.
    let model_is_available = llm_prefs
        .get_base_llm_choices_for_agent_mode(ctx)
        .any(|info| info.id == model_id);
    if !model_is_available {
        log::warn!("Skipping shared-session model update - {model_id} is unknown");
        return;
    }

    log::info!("Selecting base agent model {model_id} (from session sharing update)");

    // Update the local LLMPreferences to match the selected model
    LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
        prefs.update_preferred_agent_mode_llm(&model_id, terminal_view_id, ctx);
    });
}

/// Handles updating the local input mode when an input mode update is received.
/// This function is shared between the viewer and sharer to ensure consistent behavior.
pub(crate) fn apply_input_mode_update(
    weak_view_handle: &WeakViewHandle<TerminalView>,
    input_mode: &InputMode,
    _guard: &ActiveRemoteUpdate,
    ctx: &mut AppContext,
) {
    let Some(view) = weak_view_handle.upgrade(ctx) else {
        return;
    };

    // When AgentView is enabled, we only apply input mode updates when in an active agent view.
    // Outside of agent view, input mode changes are not relevant.
    if FeatureFlag::AgentView.is_enabled() {
        let agent_view_controller = view.as_ref(ctx).agent_view_controller().clone();
        if !agent_view_controller.as_ref(ctx).is_active() {
            return;
        }
    }

    let client_input_type = match input_mode.input_type {
        ProtocolInputType::Shell => InputType::Shell,
        ProtocolInputType::AI => InputType::AI,
    };
    let new_config = InputConfig {
        input_type: client_input_type,
        is_locked: input_mode.is_locked,
    };

    // Skip update if nothing would change
    let current_config = view.as_ref(ctx).input_config(ctx);
    if current_config == new_config {
        return;
    }

    view.update(ctx, |terminal_view, ctx| {
        terminal_view.apply_external_input_mode_update(new_config, ctx);
    });
}

/// Handles updating the local auto-approve setting when an update is received.
/// This function is shared between the viewer and sharer to ensure consistent behavior.
pub(crate) fn apply_auto_approve_agent_actions_update(
    weak_view_handle: &WeakViewHandle<TerminalView>,
    auto_approve: bool,
    _guard: &ActiveRemoteUpdate,
    ctx: &mut AppContext,
) {
    let Some(view) = weak_view_handle.upgrade(ctx) else {
        return;
    };

    view.update(ctx, |view, ctx| {
        let ai_context_model = view.ai_context_model().clone();
        ai_context_model.update(ctx, |context_model, ctx| {
            let current_mode = context_model.pending_query_autoexecute_override(ctx);
            let is_on = current_mode.is_autoexecute_any_action();

            // Skip if we're already in the desired state to avoid feedback loops.
            if is_on == auto_approve {
                return;
            }

            context_model.toggle_pending_query_autoexecute(ctx);
        });
    });
}

/// Handles updating the local selected conversation when a selected conversation update is received.
/// This function is shared between the viewer and sharer to ensure consistent behavior.
pub(crate) fn apply_selected_conversation_update(
    weak_view_handle: &WeakViewHandle<TerminalView>,
    selected_conversation: &SelectedConversation,
    _guard: &ActiveRemoteUpdate,
    ctx: &mut AppContext,
) {
    let Some(view) = weak_view_handle.upgrade(ctx) else {
        return;
    };

    // In shared ambient agent sessions, we can temporarily receive "none/new" selected_conversation
    // updates (e.g. before a server conversation token exists).
    //
    // If we already have an active local conversation selected (typically created/selected by the
    // incoming shared-session init event), treating these updates as authoritative can create an
    // extra empty conversation on the viewer.
    //
    // To avoid that, ignore "none/new" updates once there is already an active *empty* conversation.
    if view.as_ref(ctx).is_shared_ambient_agent_session()
        && matches!(
            selected_conversation,
            SelectedConversation::NewConversation | SelectedConversation::NoConversation
        )
    {
        let active_conversation_id = if FeatureFlag::AgentView.is_enabled() {
            view.as_ref(ctx)
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
        } else {
            view.as_ref(ctx)
                .ai_context_model()
                .as_ref(ctx)
                .selected_conversation_id(ctx)
        };

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let has_empty_active_conversation = active_conversation_id
            .as_ref()
            .and_then(|conversation_id| history_model.as_ref(ctx).conversation(conversation_id))
            .is_some_and(|c| c.exchange_count() == 0);

        if has_empty_active_conversation {
            return;
        }
    }

    match selected_conversation {
        SelectedConversation::ExistingConversation(server_conversation_token) => {
            // Convert server token to local conversation ID using the AI controller
            let ai_controller = view.as_ref(ctx).ai_controller().clone();
            let conversation_id = ai_controller.update(ctx, |controller, ctx| {
                controller.find_existing_conversation_by_server_token(
                    &server_conversation_token.as_uuid().to_string(),
                    ctx,
                )
            });

            if let Some(conversation_id) = conversation_id {
                // Update the context model with the selected conversation
                view.update(ctx, |view, ctx| {
                    view.ai_context_model().update(ctx, |context_model, ctx| {
                        // Only update if different to avoid feedback loop
                        if context_model.selected_conversation_id(ctx) != Some(conversation_id) {
                            context_model.set_pending_query_state_for_existing_conversation(
                                conversation_id,
                                AgentViewEntryOrigin::SharedSessionSelection,
                                ctx,
                            );
                        }
                    });
                });
            }
        }
        SelectedConversation::NewConversation => {
            // Start new conversation in agent view
            let agent_view_controller = view.as_ref(ctx).agent_view_controller().clone();
            view.update(ctx, |view, ctx| {
                view.ai_context_model().update(ctx, |context_model, ctx| {
                    if FeatureFlag::AgentView.is_enabled() {
                        // Check if we're already in an empty agent view to avoid feedback loop.
                        let agent_view_state = agent_view_controller.as_ref(ctx).agent_view_state();
                        if let Some(conversation_id) = agent_view_state.active_conversation_id() {
                            let history_model = BlocklistAIHistoryModel::handle(ctx);
                            let is_empty = history_model
                                .as_ref(ctx)
                                .conversation(&conversation_id)
                                .is_none_or(|c| c.exchange_count() == 0);
                            if is_empty {
                                // Already in an empty agent view - no need to start another new one
                                return;
                            }
                        }
                    } else {
                        // Check if state is already None to avoid feedback loop
                        if context_model.selected_conversation_id(ctx).is_none() {
                            return;
                        }
                    }
                    context_model.set_pending_query_state_for_new_conversation(
                        AgentViewEntryOrigin::SharedSessionSelection,
                        ctx,
                    );
                });
            });
        }
        SelectedConversation::NoConversation => {
            let agent_view_controller = view.as_ref(ctx).agent_view_controller().clone();
            view.update(ctx, |view, ctx| {
                view.ai_context_model().update(ctx, |context_model, ctx| {
                    if FeatureFlag::AgentView.is_enabled() {
                        // Only exit if currently in agent view to avoid feedback loop
                        if agent_view_controller.as_ref(ctx).is_active() {
                            agent_view_controller.update(ctx, |controller, ctx| {
                                controller.exit_agent_view(ctx);
                            });
                        }
                    } else {
                        // For non-agent view users, we treat NoConversation the same as new conversation.
                        if context_model.selected_conversation_id(ctx).is_some() {
                            context_model.set_pending_query_state_for_new_conversation(
                                AgentViewEntryOrigin::SharedSessionSelection,
                                ctx,
                            );
                        }
                    }
                });
            });
        }
    }
}

/// Build a selected_conversation update based on the current view state.
/// Routes to the appropriate implementation based on whether AgentView is enabled.
/// Returns None if the update should not be sent (e.g., selected conversation has no server token yet).
pub(crate) fn build_selected_conversation_update(
    agent_view_controller: &ModelHandle<AgentViewController>,
    context_model: &ModelHandle<BlocklistAIContextModel>,
    ctx: &mut AppContext,
) -> Option<UniversalDeveloperInputContextUpdate> {
    if FeatureFlag::AgentView.is_enabled() {
        build_selected_conversation_update_agent_view_enabled(
            agent_view_controller,
            &BlocklistAIHistoryModel::handle(ctx),
            ctx,
        )
    } else {
        build_selected_conversation_update_agent_view_disabled(
            context_model,
            &BlocklistAIHistoryModel::handle(ctx),
            ctx,
        )
    }
}

fn build_selected_conversation_update_agent_view_disabled(
    ai_context_model: &ModelHandle<BlocklistAIContextModel>,
    history_model: &ModelHandle<BlocklistAIHistoryModel>,
    ctx: &mut AppContext,
) -> Option<UniversalDeveloperInputContextUpdate> {
    let selected_conversation_id = ai_context_model.as_ref(ctx).selected_conversation_id(ctx);
    let server_token_opt: Option<ServerConversationToken> =
        selected_conversation_id.and_then(|conversation_id| {
            history_model
                .as_ref(ctx)
                .conversation(&conversation_id)
                .and_then(|conversation| conversation.server_conversation_token().cloned())
                .and_then(|token| token.try_into().ok())
        });

    // Only send update if starting new (None) or token is present
    let should_send = selected_conversation_id.is_none() || server_token_opt.is_some();
    if !should_send {
        return None;
    }

    Some(UniversalDeveloperInputContextUpdate {
        selected_conversation: Some(SelectedConversation::new(server_token_opt)),
        ..Default::default()
    })
}

fn build_selected_conversation_update_agent_view_enabled(
    agent_view_controller: &ModelHandle<AgentViewController>,
    history_model: &ModelHandle<BlocklistAIHistoryModel>,
    ctx: &mut AppContext,
) -> Option<UniversalDeveloperInputContextUpdate> {
    let agent_view_state = agent_view_controller.as_ref(ctx).agent_view_state();

    let selected_conversation = if !agent_view_state.is_active() {
        SelectedConversation::NoConversation
    } else if let Some(conversation_id) = agent_view_state.active_conversation_id() {
        let conversation = history_model.as_ref(ctx).conversation(&conversation_id);
        let server_token_opt = conversation
            .and_then(|c| c.server_conversation_token().cloned())
            .and_then(|token| token.try_into().ok());

        if let Some(server_token) = server_token_opt {
            SelectedConversation::ExistingConversation(server_token)
        } else {
            // If the conversation has content but no token yet, skip this update. Otherwise we'd send
            // NewConversation now and ExistingConversation moments later when the token
            // arrives, causing the second update to sometimes be overwritten by an echo of the first update
            // (and leading to a weird state where the viewer sends a query and is then briefly entered into an empty agent view).
            let is_empty = conversation.is_none_or(|c| c.exchange_count() == 0);
            if is_empty {
                SelectedConversation::NewConversation
            } else {
                return None;
            }
        }
    } else {
        SelectedConversation::NewConversation
    };

    Some(UniversalDeveloperInputContextUpdate {
        selected_conversation: Some(selected_conversation),
        ..Default::default()
    })
}

/// Applies CLI agent session + rich-input state from the remote side.
/// Creates/removes the session and opens/closes rich input based on
/// the given `CLIAgentSessionState`.
pub(crate) fn apply_cli_agent_state_update(
    weak_view_handle: &WeakViewHandle<TerminalView>,
    cli_agent_session: &CLIAgentSessionState,
    _guard: &ActiveRemoteUpdate,
    ctx: &mut AppContext,
) {
    let Some(view) = weak_view_handle.upgrade(ctx) else {
        return;
    };
    let view_id = view.id();

    match cli_agent_session {
        CLIAgentSessionState::Active {
            cli_agent,
            is_rich_input_open,
        } => {
            let agent = CLIAgent::from_serialized_name(cli_agent);

            // Create the agent session if it does not exist.
            let already_exists = CLIAgentSessionsModel::as_ref(ctx)
                .session(view_id)
                .is_some_and(|s| s.agent == agent);
            if !already_exists {
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions_model, ctx| {
                    sessions_model.set_session(
                        view_id,
                        CLIAgentSession {
                            agent,
                            status: CLIAgentSessionStatus::InProgress,
                            session_context: CLIAgentSessionContext::default(),
                            input_state: CLIAgentInputState::Closed,
                            listener: None,
                            plugin_version: None,
                            remote_host: None,
                            draft_text: None,
                            custom_command_prefix: None,
                            // Viewer input is managed by the sync protocol,
                            // not local status-change auto-toggle.
                            should_auto_toggle_input: false,
                        },
                        ctx,
                    );
                });

                view.update(ctx, |view, ctx| {
                    view.apply_cli_agent_footer_visibility(true, ctx);
                });
            }

            // For cloud agent sessions with non-Oz harnesses, auto-open rich
            // input when creating a new CLI agent session so the viewer gets the
            // composer immediately (byte-sharing has roundtrip lag without it).
            let effective_rich_input_open =
                if !already_exists && view.as_ref(ctx).is_shared_ambient_agent_session() {
                    true
                } else {
                    *is_rich_input_open
                };

            // Update the rich input state.
            let currently_open = CLIAgentSessionsModel::as_ref(ctx).is_input_open(view_id);
            if currently_open != effective_rich_input_open {
                view.update(ctx, |view, ctx| {
                    if effective_rich_input_open {
                        view.open_cli_agent_rich_input(
                            CLIAgentInputEntrypoint::SharedSessionSync,
                            ctx,
                        );
                    } else {
                        view.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::Other, ctx);
                    }
                });
            }

            view.update(ctx, |view, ctx| {
                view.sync_agent_view_for_shared_third_party_viewer(ctx);
            });
        }
        CLIAgentSessionState::Inactive => {
            // Session cleanup is handled by BlockCompleted events on the
            // viewer side, so no explicit teardown is needed here.
        }
    }
}

// ---------------------------------------------------------------------------
// Echo-suppression for remote session-sharing context updates.
//
// When a participant (viewer or sharer) receives a
// `UniversalDeveloperInputContextUpdate` from the remote side, the `apply_*`
// helpers above update local state which fires model events. Those events are
// observed by broadcast subscribers that would normally send the value *back*
// over the network, creating an echo loop.
//
// To prevent this, each side creates a `RemoteUpdateGuard` and:
//   1. Clones it into every broadcast subscriber, which calls
//      `guard.should_broadcast()` and skips when `false`.
//   2. Wraps incoming `apply_*` calls with `guard.start_remote_update()`,
//      which returns an `ActiveRemoteUpdate` RAII token that suppresses
//      broadcasts for the duration of the synchronous update.
//
// When adding a **new** field to `UniversalDeveloperInputContextUpdate`:
//   - Check `guard.should_broadcast()` in the new broadcast subscriber.
//   - Ensure the new `apply_*` call sits inside the existing
//     `ActiveRemoteUpdate` scope in the incoming handler.
// ---------------------------------------------------------------------------

/// Shared guard that tracks whether we are currently applying a remote
/// session-sharing context update.
#[derive(Clone)]
pub(crate) struct RemoteUpdateGuard {
    inner: Rc<Cell<bool>>,
}

impl RemoteUpdateGuard {
    /// Creates a new guard, initially not suppressing broadcasts.
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(Cell::new(false)),
        }
    }

    /// Returns `true` when a context update originated locally and should be
    /// broadcast to the remote side. Returns `false` when we are in the middle
    /// of applying a remote update (i.e. the echo should be suppressed).
    pub(crate) fn should_broadcast(&self) -> bool {
        !self.inner.get()
    }

    /// Returns an RAII token that suppresses outgoing broadcasts until dropped.
    /// Wrap all `apply_*` calls for incoming remote updates in this so that
    /// the synchronous event dispatch sees the guard as active.
    pub(crate) fn start_remote_update(&self) -> ActiveRemoteUpdate {
        debug_assert!(
            !self.inner.get(),
            "RemoteUpdateGuard::start_remote_update called while already active"
        );
        self.inner.set(true);
        ActiveRemoteUpdate {
            inner: self.inner.clone(),
        }
    }
}

/// RAII token that suppresses outgoing broadcasts while held.
pub(crate) struct ActiveRemoteUpdate {
    inner: Rc<Cell<bool>>,
}

impl Drop for ActiveRemoteUpdate {
    fn drop(&mut self) {
        self.inner.set(false);
    }
}
