//! [`TuiOrchestrationBlock`]: the TUI permission and configuration card for a
//! `RunAgents` request.
//!
//! The card has two interactive modes: an acceptance card summarizing the
//! request and its run-wide configuration, and a configuring mode that walks
//! a dynamic sequence of single-field pages rendered by
//! [`TuiOptionSelector`]. Accept dispatches the edited request through the
//! shared [`BlocklistAIActionModel::execute_run_agents`] path; Reject emits
//! [`TuiOrchestrationBlockEvent::RejectRequested`], which the owning
//! [`crate::agent_block::TuiAIBlock`] maps to action cancellation. Terminal,
//! spawning, streaming, and restored states reuse the existing fallback
//! tool-call presentation and its `tool_call_labels` copy.

use std::rc::Rc;

use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionId, AIAgentActionType, AuthSecretSelection,
    BlocklistAIActionEvent, BlocklistAIActionModel, Harness, HarnessAvailabilityEvent,
    HarnessAvailabilityModel, LLMPreferences, LLMPreferencesEvent, ORCHESTRATION_WARP_WORKER_HOST,
    OptionSnapshot, OrchestrationConfig, OrchestrationConfigState, OrchestrationConfigStatus,
    OrchestrationEditState, RunAgentsExecutionMode, RunAgentsExecutor, RunAgentsExecutorEvent,
    RunAgentsRequest, RunAgentsSpawningSnapshot, persist_host_selection,
    resolve_auth_secret_selection_for_harness, resolve_default_environment_id,
    resolve_default_host_slug, should_show_auth_secret_picker,
};
use warpui::SingletonEntity;
use warpui_core::elements::tui::TuiElement;
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, FixedBinding};
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};
mod configuration;
mod render;

use configuration::{
    ConfigPage, ModelOrchestrationBlockController, OrchestrationBlockController, build_request,
};

use crate::keybindings::TUI_BINDING_GROUP;
use crate::option_selector::{
    OptionSelectorHeader, OptionSelectorPage, TuiOptionSelector, TuiOptionSelectorEvent,
};
use crate::orchestrated_agent_identity_styling::AgentIdentity;
use crate::tui_builder::TuiUiBuilder;

const ORCHESTRATION_BLOCK_TITLE: &str = "Can I start additional agents for this task?";

/// Keymap-context flag set while the acceptance card is active.
const ACCEPTANCE_CONTEXT_FLAG: &str = "TuiOrchestrationBlockAcceptance";
/// Keymap-context flag set while a configuration page is active.
const CONFIGURING_CONTEXT_FLAG: &str = "TuiOrchestrationBlockConfiguring";

/// Registers fixed card keybindings scoped to the active card mode.
pub(crate) fn init(app: &mut AppContext) {
    let acceptance = || id!(TuiOrchestrationBlock::ui_name()) & id!(ACCEPTANCE_CONTEXT_FLAG);
    let configuring = || id!(TuiOrchestrationBlock::ui_name()) & id!(CONFIGURING_CONTEXT_FLAG);
    app.register_fixed_bindings([
        FixedBinding::new("enter", TuiOrchestrationBlockAction::Accept, acceptance())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "numpadenter",
            TuiOrchestrationBlockAction::Accept,
            acceptance(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "ctrl-e",
            TuiOrchestrationBlockAction::Configure,
            acceptance(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("escape", TuiOrchestrationBlockAction::Back, configuring())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "left",
            TuiOrchestrationBlockAction::CommitAndPreviousPage,
            configuring(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "right",
            TuiOrchestrationBlockAction::CommitAndNextPage,
            configuring(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("tab", TuiOrchestrationBlockAction::NextPage, configuring())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "ctrl-c",
            TuiOrchestrationBlockAction::Reject,
            id!(TuiOrchestrationBlock::ui_name()),
        )
        .with_group(TUI_BINDING_GROUP),
    ]);
}

/// The card's active interactive presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CardMode {
    Acceptance,
    Configuring { page: ConfigPage },
}

/// Direction to navigate after the selector confirms the current page.
/// Arrow actions retain this until the selector emits its confirmation event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PageConfirmationNavigation {
    Previous,
    Next,
}

/// Events emitted to the owning agent block.
#[derive(Clone, Debug)]
pub(crate) enum TuiOrchestrationBlockEvent {
    /// The user rejected the request; the block cancels the action.
    RejectRequested,
    /// The card's blocking/focus state may have changed; ancestors re-derive
    /// the active blocker and re-measure the card.
    BlockingStateChanged,
    /// The active selector page changed intrinsic height.
    LayoutInvalidated,
}

/// Typed actions bound to the card's keybindings.
#[derive(Clone, Debug)]
pub(crate) enum TuiOrchestrationBlockAction {
    Accept,
    Configure,
    CommitAndPreviousPage,
    CommitAndNextPage,
    NextPage,
    Back,
    Reject,
}

/// The TUI orchestration confirmation block. See the module docs.
pub(crate) struct TuiOrchestrationBlock {
    // Request and action identity.
    action_id: AIAgentActionId,
    /// The latest streamed tool call, kept in sync by
    /// [`Self::update_request`]; terminal/streaming states render from it
    /// through the shared fallback tool-call presentation.
    action: AIAgentAction,
    /// Card fields carried through editing into the dispatched request.
    request_fields: RunAgentsRequest,
    /// Approved/disapproved plan config used to resolve inherited fields.
    active_config: Option<(OrchestrationConfig, OrchestrationConfigStatus)>,
    /// The conversation's base model, used as the Oz model fallback.
    fallback_base_model_id: Option<String>,
    /// Whether the block was restored from history (non-interactive).
    is_restored: bool,

    // Interactive card state.
    orchestration_edit_state: OrchestrationEditState,
    mode: CardMode,
    selector: ViewHandle<TuiOptionSelector>,
    /// Arrow direction awaiting the selector's confirmation event.
    pending_page_navigation: Option<PageConfirmationNavigation>,
    /// Validation reason shown inline after a blocked Accept.
    accept_error: Option<String>,

    // Execution state.
    controller: Rc<dyn OrchestrationBlockController>,
    spawning: Option<RunAgentsSpawningSnapshot>,
    /// Set once the request is accepted or rejected.
    decided: bool,
    /// Identity palette pinned at construction so identities stay stable
    /// across re-renders, edits, and theme switches.
    identity_palette: Vec<AgentIdentity>,
}

impl TuiOrchestrationBlock {
    /// Creates a block for one pending `RunAgents` action and wires its model
    /// subscriptions.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        action: AIAgentAction,
        request: &RunAgentsRequest,
        active_config: Option<(OrchestrationConfig, OrchestrationConfigStatus)>,
        action_model: ModelHandle<BlocklistAIActionModel>,
        run_agents_executor: ModelHandle<RunAgentsExecutor>,
        fallback_base_model_id: Option<String>,
        is_restored: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let action_id = action.id.clone();
        let action_id_for_executor = action_id.clone();
        // Spawning events replace the confirmation UI with launch progress
        // and release its input-blocking focus while agents start.
        ctx.subscribe_to_model(&run_agents_executor, move |me, _, event, ctx| match event {
            RunAgentsExecutorEvent::SpawningStarted {
                action_id,
                snapshot,
            } if action_id == &action_id_for_executor => {
                me.spawning = Some(*snapshot);
                me.mode = CardMode::Acceptance;
                ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
                ctx.notify();
            }
            RunAgentsExecutorEvent::SpawningFinished { action_id }
                if action_id == &action_id_for_executor =>
            {
                me.spawning = None;
                ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
                ctx.notify();
            }
            RunAgentsExecutorEvent::SpawningStarted { .. }
            | RunAgentsExecutorEvent::SpawningFinished { .. } => {}
        });

        let action_id_for_actions = action_id.clone();
        // Action lifecycle events enable the card once streaming finishes and
        // release its focus when this RunAgents action reaches a terminal state.
        ctx.subscribe_to_model(&action_model, move |me, _, event, ctx| match event {
            BlocklistAIActionEvent::FinishedAction { action_id, .. }
                if action_id == &action_id_for_actions =>
            {
                me.mode = CardMode::Acceptance;
                ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
                ctx.notify();
            }
            BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(action_id)
                if action_id == &action_id_for_actions =>
            {
                // Streaming completed: the card transitions from the
                // "Configuring agents…" placeholder to the interactive
                // acceptance card, so resolve display defaults now.
                me.resolve_interactive_defaults(ctx);
                ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
                ctx.notify();
            }
            _ => {}
        });

        // Harness and auth-secret catalog changes can invalidate selections,
        // so revalidate the edit state and rebuild the active page.
        ctx.subscribe_to_model(
            &HarnessAvailabilityModel::handle(ctx),
            |me, _, event, ctx| match event {
                HarnessAvailabilityEvent::Changed
                | HarnessAvailabilityEvent::AuthSecretsLoaded
                | HarnessAvailabilityEvent::AuthSecretsFetchFailed
                | HarnessAvailabilityEvent::AuthSecretCreated { .. }
                | HarnessAvailabilityEvent::AuthSecretDeleted { .. } => {
                    me.orchestration_edit_state
                        .orchestration_config_state
                        .revalidate_after_catalog_change(ctx);
                    me.refresh_active_page(ctx);
                    ctx.notify();
                }
                HarnessAvailabilityEvent::AuthSecretCreationFailed { .. }
                | HarnessAvailabilityEvent::AuthSecretDeletionFailed { .. } => {}
            },
        );

        // Model catalog updates can invalidate the selected model, so
        // revalidate the edit state and rebuild the active model page.
        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |me, _, event, ctx| {
            if let LLMPreferencesEvent::UpdatedAvailableLLMs = event {
                me.orchestration_edit_state
                    .orchestration_config_state
                    .revalidate_after_catalog_change(ctx);
                me.refresh_active_page(ctx);
                ctx.notify();
            }
        });

        // Connected worker changes alter the remote host choices shown on
        // the active page.
        ctx.subscribe_to_model(
            &warp::tui_export::ConnectedSelfHostedWorkersModel::handle(ctx),
            |me, _, event, ctx| {
                let warp::tui_export::ConnectedSelfHostedWorkersEvent::Changed = event;
                me.refresh_active_page(ctx);
                ctx.notify();
            },
        );

        let controller = Rc::new(ModelOrchestrationBlockController { action_model });
        let identity_palette = TuiUiBuilder::from_app(ctx).agent_identity_palette();
        let mut view = Self::from_parts(
            action,
            request,
            active_config,
            controller,
            fallback_base_model_id,
            is_restored,
            identity_palette,
            ctx,
        );
        view.resolve_interactive_defaults(ctx);
        view
    }

    /// Constructs the block from injected external behavior.
    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        action: AIAgentAction,
        request: &RunAgentsRequest,
        active_config: Option<(OrchestrationConfig, OrchestrationConfigStatus)>,
        controller: Rc<dyn OrchestrationBlockController>,
        fallback_base_model_id: Option<String>,
        is_restored: bool,
        identity_palette: Vec<AgentIdentity>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let selector = ctx.add_typed_action_tui_view(TuiOptionSelector::new);
        ctx.subscribe_to_view(&selector, |me, _, event, ctx| {
            me.handle_selector_event(event, ctx);
        });
        let orchestration_edit_state = OrchestrationEditState::new(
            Self::config_state_from_request(request, active_config.as_ref()),
        );
        Self {
            action_id: action.id.clone(),
            action,
            request_fields: request.clone(),
            active_config,
            fallback_base_model_id,
            is_restored,
            orchestration_edit_state,
            mode: CardMode::Acceptance,
            selector,
            pending_page_navigation: None,
            accept_error: None,
            controller,
            spawning: None,
            decided: false,
            identity_palette,
        }
    }

    /// Seeds the run-wide edit state from the streamed request. An approved
    /// plan config unconditionally supplies the executor-owned fields;
    /// otherwise the TUI's Local mode is normalized to its Oz-only policy.
    fn config_state_from_request(
        request: &RunAgentsRequest,
        active_config: Option<&(OrchestrationConfig, OrchestrationConfigStatus)>,
    ) -> OrchestrationConfigState {
        let mut state = OrchestrationConfigState::from_run_agents_fields(
            Some(&request.model_id),
            Some(&request.harness_type),
            &request.execution_mode,
        );
        // Carry the request's auth secret across the round trip. Absence
        // becomes `Unset`; defaults re-resolve from persisted settings.
        state.auth_secret_selection =
            AuthSecretSelection::from_optional_name(request.harness_auth_secret_name.clone());
        let approved_config = active_config
            .filter(|(_, status)| status.is_approved())
            .map(|(config, _)| config);
        if let Some(config) = approved_config {
            state.override_from_approved_config(config);
        } else {
            configuration::normalize_tui_local_harness(&mut state);
        }
        state
    }

    /// Resolves UI-only display defaults, mirroring the GUI card's
    /// `resolve_interactive_defaults`: the Oz model falls back to the
    /// conversation base model, a Remote run pre-fills the default host and
    /// environment, and an `Unset` auth selection re-seeds from persisted
    /// per-harness settings.
    fn resolve_interactive_defaults(&mut self, ctx: &AppContext) {
        let state = &mut self.orchestration_edit_state.orchestration_config_state;
        if state.model_id.is_empty() {
            let harness = Harness::parse_orchestration_harness(&state.harness_type);
            if matches!(harness, Some(Harness::Oz) | None)
                && let Some(base) = &self.fallback_base_model_id
            {
                state.model_id = base.clone();
            }
        }
        if let RunAgentsExecutionMode::Remote {
            environment_id,
            worker_host,
            ..
        } = &state.execution_mode
        {
            let needs_host = worker_host.is_empty();
            let needs_env = environment_id.is_empty();
            if needs_host {
                let default_host = resolve_default_host_slug(ctx)
                    .unwrap_or_else(|| ORCHESTRATION_WARP_WORKER_HOST.to_string());
                state.set_worker_host(default_host);
            }
            if needs_env && let Some(default_env) = resolve_default_environment_id(ctx) {
                state.set_environment_id(default_env);
            }
        }
        if matches!(state.auth_secret_selection, AuthSecretSelection::Unset) {
            state.auth_secret_selection =
                resolve_auth_secret_selection_for_harness(&state.harness_type, ctx);
        }
    }

    /// Re-syncs edit state from the latest streaming request chunk
    /// (mirroring the GUI card's `update_request`).
    pub(crate) fn update_request(
        &mut self,
        request: &RunAgentsRequest,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.spawning.is_some() || self.decided {
            return;
        }
        self.action.action = AIAgentActionType::RunAgents(request.clone());
        let new_state = Self::config_state_from_request(request, self.active_config.as_ref());
        let changed = self.request_fields != *request
            || self.orchestration_edit_state.orchestration_config_state != new_state;
        if !changed {
            return;
        }
        self.request_fields = request.clone();
        self.orchestration_edit_state = OrchestrationEditState::new(new_state);
        self.resolve_interactive_defaults(ctx);
        self.refresh_active_page(ctx);
        ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
        ctx.notify();
    }

    /// Whether this card still awaits a user decision.
    pub(super) fn is_awaiting_confirmation(&self, ctx: &AppContext) -> bool {
        if self.decided || self.spawning.is_some() || self.is_restored {
            return false;
        }

        matches!(
            self.controller.action_status(&self.action_id, ctx),
            Some(AIActionStatus::Blocked)
        )
    }

    /// The dynamic page sequence for the current edit state.
    fn page_sequence(state: &OrchestrationConfigState) -> Vec<ConfigPage> {
        if state.execution_mode.is_remote() {
            let mut pages = vec![ConfigPage::Location, ConfigPage::Harness];
            if should_show_auth_secret_picker(state) {
                pages.push(ConfigPage::ApiKey);
            }
            pages.extend([ConfigPage::Host, ConfigPage::Environment, ConfigPage::Model]);
            pages
        } else {
            vec![ConfigPage::Location, ConfigPage::Model]
        }
    }

    /// Builds the option snapshot for `page` from the shared builders.
    fn snapshot_for_page(&self, page: ConfigPage, ctx: &AppContext) -> OptionSnapshot {
        self.controller.snapshot_for_page(
            page,
            &self.orchestration_edit_state.orchestration_config_state,
            ctx,
        )
    }

    /// Opens `page`: swaps the selector to its page fields, and
    /// lazily fetches auth secrets for the API-key page (the same lazy fetch
    /// the GUI triggers on picker population).
    fn open_page(&mut self, page: ConfigPage, ctx: &mut ViewContext<Self>) {
        self.mode = CardMode::Configuring { page };
        self.accept_error = None;
        if matches!(page, ConfigPage::ApiKey) {
            self.ensure_auth_secrets_fetched(ctx);
        }
        let sequence =
            Self::page_sequence(&self.orchestration_edit_state.orchestration_config_state);
        let position = sequence.iter().position(|p| *p == page).unwrap_or(0) + 1;
        let selector_page = OptionSelectorPage {
            header: Some(OptionSelectorHeader {
                field_label: "Edit agent configuration".to_string(),
                position: (position, sequence.len()),
                prompt: page.question(self.request_fields.agent_run_configs.len()),
            }),
            snapshot: self.snapshot_for_page(page, ctx),
            searchable: page.is_searchable(),
        };
        self.selector.update(ctx, |selector, ctx| {
            selector.set_page(selector_page, ctx);
        });
        ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
        ctx.notify();
    }

    /// Returns from configuration to the interactive acceptance card.
    fn return_to_acceptance(&mut self, ctx: &mut ViewContext<Self>) {
        self.mode = CardMode::Acceptance;
        self.pending_page_navigation = None;
        ctx.focus_self();
        ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
        ctx.notify();
    }

    /// Refreshes the active page's snapshot in place after a catalog or
    /// state change, updating the header so the dynamic page count stays
    /// current.
    fn refresh_active_page(&mut self, ctx: &mut ViewContext<Self>) {
        let CardMode::Configuring { page } = self.mode else {
            return;
        };
        let sequence =
            Self::page_sequence(&self.orchestration_edit_state.orchestration_config_state);
        if !sequence.contains(&page) {
            // The active page no longer applies (e.g. auth page removed by a
            // catalog change); fall back to the acceptance card.
            self.return_to_acceptance(ctx);
            return;
        }
        let snapshot = self.snapshot_for_page(page, ctx);
        self.selector.update(ctx, |selector, ctx| {
            selector.refresh_snapshot(snapshot, ctx);
        });
    }

    /// Triggers the lazy per-harness auth-secret fetch (also the Retry path
    /// for a `Failed` API-key page).
    fn ensure_auth_secrets_fetched(&self, ctx: &mut ViewContext<Self>) {
        let Some(harness) = Harness::parse_orchestration_harness(
            &self
                .orchestration_edit_state
                .orchestration_config_state
                .harness_type,
        ) else {
            return;
        };
        HarnessAvailabilityModel::handle(ctx).update(ctx, |availability, ctx| {
            availability.ensure_auth_secrets_fetched(harness, ctx);
        });
    }

    /// Navigates after confirmation using an arrow's requested direction, or
    /// advances normally for Enter.
    fn finish_page_confirmation(&mut self, page: ConfigPage, ctx: &mut ViewContext<Self>) {
        let sequence =
            Self::page_sequence(&self.orchestration_edit_state.orchestration_config_state);
        let Some(index) = sequence.iter().position(|candidate| *candidate == page) else {
            self.return_to_acceptance(ctx);
            return;
        };
        let navigation = self.pending_page_navigation.take();
        let target = match navigation {
            Some(PageConfirmationNavigation::Previous) => index
                .checked_sub(1)
                .and_then(|index| sequence.get(index))
                .copied(),
            Some(PageConfirmationNavigation::Next) | None => sequence.get(index + 1).copied(),
        };
        match target {
            Some(target) => self.open_page(target, ctx),
            None if navigation.is_some() => self.open_page(page, ctx),
            None => self.return_to_acceptance(ctx),
        }
    }

    /// Moves to the next page without applying the current selection.
    fn navigate_page(&mut self, forward: bool, ctx: &mut ViewContext<Self>) {
        let CardMode::Configuring { page } = self.mode else {
            return;
        };
        let sequence =
            Self::page_sequence(&self.orchestration_edit_state.orchestration_config_state);
        let Some(index) = sequence.iter().position(|candidate| *candidate == page) else {
            return;
        };
        let target = if forward {
            sequence.get(index + 1)
        } else {
            index.checked_sub(1).and_then(|index| sequence.get(index))
        };
        if let Some(target) = target.copied() {
            self.open_page(target, ctx);
        }
    }

    /// Routes selector events for the active page.
    fn handle_selector_event(
        &mut self,
        event: &TuiOptionSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiOptionSelectorEvent::Confirmed { id } => {
                let CardMode::Configuring { page } = self.mode else {
                    return;
                };
                self.controller.apply_page_selection(
                    page,
                    id,
                    &mut self.orchestration_edit_state,
                    self.fallback_base_model_id.clone(),
                    ctx,
                );
                self.finish_page_confirmation(page, ctx);
            }
            TuiOptionSelectorEvent::CustomTextSubmitted { value } => {
                if let CardMode::Configuring {
                    page: ConfigPage::Host,
                } = self.mode
                {
                    self.orchestration_edit_state
                        .orchestration_config_state
                        .set_worker_host(value.clone());
                    persist_host_selection(value, ctx);
                    self.finish_page_confirmation(ConfigPage::Host, ctx);
                }
            }
            TuiOptionSelectorEvent::CustomTextOpened => {}
            TuiOptionSelectorEvent::RetryRequested => {
                self.pending_page_navigation = None;
                self.ensure_auth_secrets_fetched(ctx);
                self.refresh_active_page(ctx);
            }
            TuiOptionSelectorEvent::Dismissed => {
                self.pending_page_navigation = None;
                self.handle_back(ctx);
            }
            TuiOptionSelectorEvent::LayoutInvalidated => {
                ctx.emit(TuiOrchestrationBlockEvent::LayoutInvalidated);
            }
        }
    }

    /// Builds the dispatched request from this card's fields and edited
    /// run-wide state; see [`build_request`].
    fn to_request(&self) -> RunAgentsRequest {
        build_request(
            &self.request_fields,
            &self.orchestration_edit_state.orchestration_config_state,
        )
    }

    /// Accept: validates with the shared gate; a blocked
    /// accept renders the reason inline and stays active, a valid one
    /// dispatches the edited request through `execute_run_agents`.
    fn handle_accept(&mut self, ctx: &mut ViewContext<Self>) {
        if self.decided || self.spawning.is_some() || !self.is_awaiting_confirmation(ctx) {
            return;
        }
        let request = self.to_request();
        let action_id = self.action_id.clone();
        if let Err(reason) = self.controller.accept(
            &action_id,
            request,
            &self.orchestration_edit_state.orchestration_config_state,
            ctx,
        ) {
            self.accept_error = Some(reason);
            ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
            ctx.notify();
            return;
        }
        self.decided = true;
        self.accept_error = None;
        self.mode = CardMode::Acceptance;
        ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
        ctx.notify();
    }

    /// Reject: resolves the request as rejected exactly once,
    /// from the acceptance card or any configuration page.
    fn handle_reject(&mut self, ctx: &mut ViewContext<Self>) {
        if self.decided || self.spawning.is_some() || !self.is_awaiting_confirmation(ctx) {
            return;
        }
        self.decided = true;
        self.mode = CardMode::Acceptance;
        ctx.emit(TuiOrchestrationBlockEvent::RejectRequested);
        ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
        ctx.notify();
    }

    /// Opens configuration on the first page.
    fn handle_configure(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_awaiting_confirmation(ctx) {
            return;
        }
        let first = Self::page_sequence(&self.orchestration_edit_state.orchestration_config_state)
            .first()
            .copied();
        if let Some(first) = first {
            self.open_page(first, ctx);
        }
    }

    /// Escape from configuration: completed pages keep their confirmed
    /// selections; the current page's unconfirmed selection is discarded.
    /// Active custom-text editing unwinds first.
    fn handle_back(&mut self, ctx: &mut ViewContext<Self>) {
        self.pending_page_navigation = None;
        let consumed = self
            .selector
            .update(ctx, |selector, ctx| selector.handle_back(ctx));
        if consumed {
            return;
        }
        if matches!(self.mode, CardMode::Configuring { .. }) {
            self.return_to_acceptance(ctx);
        }
    }

    /// Confirms the selection, then applies the requested arrow navigation.
    fn handle_arrow_navigation(
        &mut self,
        navigation: PageConfirmationNavigation,
        ctx: &mut ViewContext<Self>,
    ) {
        self.pending_page_navigation = Some(navigation);
        let confirmation_started = self
            .selector
            .update(ctx, |selector, ctx| selector.confirm_selected(ctx));
        if !confirmation_started {
            self.pending_page_navigation = None;
        }
    }
}

impl Entity for TuiOrchestrationBlock {
    type Event = TuiOrchestrationBlockEvent;
}

impl TuiView for TuiOrchestrationBlock {
    fn ui_name() -> &'static str {
        "TuiOrchestrationBlock"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        vec![self.selector.id()]
    }

    fn keymap_context(&self, _ctx: &AppContext) -> keymap::Context {
        let mut context = keymap::Context::default();
        context.set.insert(Self::ui_name());
        match self.mode {
            CardMode::Acceptance => context.set.insert(ACCEPTANCE_CONTEXT_FLAG),
            CardMode::Configuring { .. } => context.set.insert(CONFIGURING_CONTEXT_FLAG),
        };
        context
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        render::render(self, app)
    }
}

impl TypedActionView for TuiOrchestrationBlock {
    type Action = TuiOrchestrationBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiOrchestrationBlockAction::Accept => self.handle_accept(ctx),
            TuiOrchestrationBlockAction::Configure => self.handle_configure(ctx),
            TuiOrchestrationBlockAction::CommitAndPreviousPage => {
                self.handle_arrow_navigation(PageConfirmationNavigation::Previous, ctx)
            }
            TuiOrchestrationBlockAction::CommitAndNextPage => {
                self.handle_arrow_navigation(PageConfirmationNavigation::Next, ctx)
            }
            TuiOrchestrationBlockAction::NextPage => self.navigate_page(true, ctx),
            TuiOrchestrationBlockAction::Back => self.handle_back(ctx),
            TuiOrchestrationBlockAction::Reject => self.handle_reject(ctx),
        }
    }
}

#[cfg(test)]
#[path = "orchestration_block_tests.rs"]
mod tests;
