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
    accept_disabled_reason_with_auth, api_key_snapshot, empty_env_recommendation_message,
    environment_snapshot, harness_snapshot, host_snapshot, location_snapshot, model_snapshot,
    persist_environment_selection, persist_host_selection,
    resolve_auth_secret_selection_for_harness, resolve_default_environment_id,
    resolve_default_host_slug, should_show_auth_secret_picker, AIActionStatus, AIAgentAction,
    AIAgentActionId, AIAgentActionType, AuthSecretSelection, BlocklistAIActionEvent,
    BlocklistAIActionModel, Harness, HarnessAvailabilityEvent, HarnessAvailabilityModel,
    LLMPreferences, LLMPreferencesEvent, OptionSnapshot, OrchestrationConfig,
    OrchestrationConfigState, OrchestrationConfigStatus, OrchestrationEditState,
    RunAgentsExecutionMode, RunAgentsExecutor, RunAgentsExecutorEvent, RunAgentsRequest,
    RunAgentsSpawningSnapshot, ORCHESTRATION_WARP_WORKER_HOST,
};
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    Modifier, TuiChildView, TuiContainer, TuiElement, TuiFlex, TuiParentElement, TuiText,
};
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, FixedBinding};
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::agent_block_sections::render_fallback_tool_call_section;
use crate::agent_identity::{assign_agent_identity_indices, AgentIdentity};
use crate::keybindings::TUI_BINDING_GROUP;
use crate::option_selector::{OptionSelectorPage, TuiOptionSelector, TuiOptionSelectorEvent};
use crate::tui_builder::TuiUiBuilder;

const ORCHESTRATION_BLOCK_TITLE: &str = "Can I start additional agents for this task?";

/// Keymap-context flag set while the acceptance card is active.
const ACCEPTANCE_CONTEXT_FLAG: &str = "TuiOrchestrationBlockAcceptance";
/// Keymap-context flag set while a configuration page is active.
const CONFIGURING_CONTEXT_FLAG: &str = "TuiOrchestrationBlockConfiguring";

/// Row ids emitted by `location_snapshot`.
const LOCATION_CLOUD_ID: &str = "cloud";

/// Registers the card's keybindings. Called once at TUI
/// startup from `keybindings::init`. All bindings are fixed and scoped to
/// the card's keymap context, so they only fire while a card is focused.
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
        FixedBinding::new(
            "enter",
            TuiOrchestrationBlockAction::ConfirmSelection,
            configuring(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "numpadenter",
            TuiOrchestrationBlockAction::ConfirmSelection,
            configuring(),
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

/// Builds the dispatched request from the card fields and the edited
/// run-wide state, exactly as the GUI's `RunAgentsEditState::to_request`
/// does (auth via `auth_secret_name()`, `computer_use_enabled` preserved
/// through the cloned execution mode).
fn build_request(fields: &RunAgentsRequest, state: &OrchestrationConfigState) -> RunAgentsRequest {
    RunAgentsRequest {
        summary: fields.summary.clone(),
        base_prompt: fields.base_prompt.clone(),
        skills: fields.skills.clone(),
        model_id: state.model_id.clone(),
        harness_type: state.harness_type.clone(),
        execution_mode: state.execution_mode.clone(),
        agent_run_configs: fields.agent_run_configs.clone(),
        plan_id: fields.plan_id.clone(),
        harness_auth_secret_name: state.auth_secret_name().map(str::to_string),
    }
}

/// One single-field configuration page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigPage {
    Location,
    Harness,
    ApiKey,
    Host,
    Environment,
    Model,
}

/// External orchestration behavior used by the block.
trait OrchestrationBlockController {
    /// Returns the current lifecycle status for the action.
    fn action_status(
        &self,
        action_id: &AIAgentActionId,
        ctx: &AppContext,
    ) -> Option<AIActionStatus>;

    /// Builds the current option snapshot for a configuration page.
    fn snapshot_for_page(
        &self,
        page: ConfigPage,
        state: &OrchestrationConfigState,
        ctx: &AppContext,
    ) -> OptionSnapshot;

    /// Commits one page selection into the edit state.
    fn apply_page_selection(
        &self,
        page: ConfigPage,
        id: &str,
        edit_state: &mut OrchestrationEditState,
        fallback_base_model_id: Option<String>,
        ctx: &mut AppContext,
    );

    /// Returns the reason acceptance is currently unavailable.
    fn accept_disabled_reason(
        &self,
        state: &OrchestrationConfigState,
        ctx: &AppContext,
    ) -> Option<String>;

    /// Dispatches the edited orchestration request.
    fn execute(&self, action_id: &AIAgentActionId, request: RunAgentsRequest, ctx: &mut AppContext);
}

/// Production controller backed by the shared orchestration models.
struct ModelOrchestrationBlockController {
    action_model: ModelHandle<BlocklistAIActionModel>,
}

impl OrchestrationBlockController for ModelOrchestrationBlockController {
    fn action_status(
        &self,
        action_id: &AIAgentActionId,
        ctx: &AppContext,
    ) -> Option<AIActionStatus> {
        self.action_model.as_ref(ctx).get_action_status(action_id)
    }

    fn snapshot_for_page(
        &self,
        page: ConfigPage,
        state: &OrchestrationConfigState,
        ctx: &AppContext,
    ) -> OptionSnapshot {
        match page {
            ConfigPage::Location => location_snapshot(state, ctx),
            ConfigPage::Harness => harness_snapshot(state, ctx),
            ConfigPage::ApiKey => api_key_snapshot(state, ctx),
            ConfigPage::Host => host_snapshot(state, ctx),
            ConfigPage::Environment => environment_snapshot(state, ctx),
            ConfigPage::Model => model_snapshot(state, ctx),
        }
    }

    fn apply_page_selection(
        &self,
        page: ConfigPage,
        id: &str,
        edit_state: &mut OrchestrationEditState,
        fallback_base_model_id: Option<String>,
        ctx: &mut AppContext,
    ) {
        match page {
            ConfigPage::Location => {
                edit_state
                    .orchestration_config_state
                    .apply_execution_mode_change(
                        id == LOCATION_CLOUD_ID,
                        fallback_base_model_id,
                        ctx,
                    );
            }
            ConfigPage::Harness => {
                edit_state.apply_harness_change(id, fallback_base_model_id, ctx);
            }
            ConfigPage::ApiKey => {
                let name = (!id.is_empty()).then(|| id.to_string());
                edit_state
                    .orchestration_config_state
                    .apply_auth_secret_change(name, ctx);
            }
            ConfigPage::Host => {
                edit_state
                    .orchestration_config_state
                    .set_worker_host(id.to_string());
                persist_host_selection(id, ctx);
            }
            ConfigPage::Environment => {
                edit_state
                    .orchestration_config_state
                    .set_environment_id(id.to_string());
                persist_environment_selection(id, ctx);
            }
            ConfigPage::Model => {
                edit_state.orchestration_config_state.model_id = id.to_string();
            }
        }
    }

    fn accept_disabled_reason(
        &self,
        state: &OrchestrationConfigState,
        ctx: &AppContext,
    ) -> Option<String> {
        accept_disabled_reason_with_auth(state, ctx)
    }

    fn execute(
        &self,
        action_id: &AIAgentActionId,
        request: RunAgentsRequest,
        ctx: &mut AppContext,
    ) {
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.execute_run_agents(action_id, request, ctx);
        });
    }
}

impl ConfigPage {
    /// The page's question, with agent/agents chosen from the request.
    fn question(self, agent_count: usize) -> String {
        let agent = if agent_count == 1 { "agent" } else { "agents" };
        match self {
            Self::Location => format!("Where should the {agent} run?"),
            Self::Harness => format!("Which harness should the {agent} use?"),
            Self::ApiKey => format!("Which API key should the {agent} use?"),
            Self::Host => format!("Which host should run the {agent}?"),
            Self::Environment => format!("Which environment should the {agent} use?"),
            Self::Model => format!("Which model should the {agent} use?"),
        }
    }

    /// Whether this page opts into the selector's pinned search editor.
    fn is_searchable(self) -> bool {
        matches!(self, Self::Model)
    }
}

/// Whether the card shows the acceptance summary or a configuration page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CardMode {
    Acceptance,
    Configuring { page: ConfigPage },
}
/// Page direction requested by an arrow-key confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PageNavigation {
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
    ConfirmSelection,
    CommitAndPreviousPage,
    CommitAndNextPage,
    NextPage,
    Back,
    Reject,
}

/// The TUI orchestration confirmation block. See the module docs.
pub(crate) struct TuiOrchestrationBlock {
    action_id: AIAgentActionId,
    /// The latest streamed tool call, kept in sync by
    /// [`Self::update_request`]; terminal/streaming states render from it
    /// through the shared fallback tool-call presentation.
    action: AIAgentAction,
    orchestration_edit_state: OrchestrationEditState,
    /// Card fields carried through editing into the dispatched request.
    request_fields: RunAgentsRequest,
    mode: CardMode,
    selector: ViewHandle<TuiOptionSelector>,
    /// Arrow direction to apply after the selector confirms its current
    /// value. Enter leaves this unset and follows the normal forward flow.
    confirmation_navigation: Option<PageNavigation>,
    controller: Rc<dyn OrchestrationBlockController>,
    /// Approved/disapproved plan config used to resolve inherited fields.
    active_config: Option<(OrchestrationConfig, OrchestrationConfigStatus)>,
    /// The conversation's base model, used as the Oz model fallback.
    fallback_base_model_id: Option<String>,
    /// Whether the block was restored from history (non-interactive).
    is_restored: bool,
    spawning: Option<RunAgentsSpawningSnapshot>,
    /// Set once the request is accepted or rejected.
    decided: bool,
    /// Validation reason shown inline after a blocked Accept.
    accept_error: Option<String>,
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

        // Live catalog changes revalidate the edit state and refresh the
        // active page.
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
        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |me, _, event, ctx| {
            if let LLMPreferencesEvent::UpdatedAvailableLLMs = event {
                me.orchestration_edit_state
                    .orchestration_config_state
                    .revalidate_after_catalog_change(ctx);
                me.refresh_active_page(ctx);
                ctx.notify();
            }
        });
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
        Self {
            action_id: action.id.clone(),
            action,
            orchestration_edit_state: OrchestrationEditState::new(Self::config_state_from_request(
                request,
                active_config.as_ref(),
            )),
            request_fields: request.clone(),
            mode: CardMode::Acceptance,
            selector,
            confirmation_navigation: None,
            controller,
            active_config,
            fallback_base_model_id,
            is_restored,
            spawning: None,
            decided: false,
            accept_error: None,
            identity_palette,
        }
    }

    /// Constructs an interactive block around a test controller.
    #[cfg(test)]
    fn new_for_test(
        action: AIAgentAction,
        request: &RunAgentsRequest,
        controller: Rc<dyn OrchestrationBlockController>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::from_parts(
            action,
            request,
            None,
            controller,
            Some("auto".to_string()),
            false,
            Vec::new(),
            ctx,
        )
    }

    /// Seeds the run-wide edit state from the streamed request, resolving
    /// empty fields from an approved plan config (state parity with the
    /// GUI card's `RunAgentsEditState::from_request` + `resolve_from_config`).
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
        if matches!(request.execution_mode, RunAgentsExecutionMode::Local) {
            // Re-applying Local sanitizes product-disabled local harnesses.
            state.toggle_execution_mode_to_remote(false);
        }
        if let Some((config, status)) = active_config {
            if status.is_approved() {
                state.resolve_from_config(config);
            }
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
            if matches!(harness, Some(Harness::Oz) | None) {
                if let Some(base) = &self.fallback_base_model_id {
                    state.model_id = base.clone();
                }
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
            if needs_env {
                if let Some(default_env) = resolve_default_environment_id(ctx) {
                    state.set_environment_id(default_env);
                }
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

    /// Whether this card is the active blocking interaction: interactive in
    /// Acceptance/Configuring while the action awaits confirmation, and
    /// false once accepted, rejected, spawning, finished, or restored.
    pub(crate) fn wants_focus(&self, ctx: &AppContext) -> bool {
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
            field_label: "Edit agent configuration".to_string(),
            position: (position, sequence.len()),
            prompt: page.question(self.request_fields.agent_run_configs.len()),
            snapshot: self.snapshot_for_page(page, ctx),
            searchable: page.is_searchable(),
        };
        self.selector.update(ctx, |selector, ctx| {
            selector.set_page(selector_page, ctx);
        });
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
            self.mode = CardMode::Acceptance;
            ctx.notify();
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

    /// Applies a confirmed selection to the edit state and
    /// advances to the next applicable page.
    fn handle_page_confirmed(&mut self, id: &str, ctx: &mut ViewContext<Self>) {
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

    /// Completes a page confirmation using an arrow's requested direction,
    /// or the normal Enter behavior when no arrow direction is pending.
    fn finish_page_confirmation(&mut self, page: ConfigPage, ctx: &mut ViewContext<Self>) {
        match self.confirmation_navigation.take() {
            Some(navigation) => self.navigate_after_confirmation(page, navigation, ctx),
            None => self.advance_after(page, ctx),
        }
    }

    /// Advances past `page` in the (freshly recomputed) sequence, returning
    /// to the acceptance card after the final page.
    fn advance_after(&mut self, page: ConfigPage, ctx: &mut ViewContext<Self>) {
        let sequence =
            Self::page_sequence(&self.orchestration_edit_state.orchestration_config_state);
        let next = sequence
            .iter()
            .position(|p| *p == page)
            .and_then(|index| sequence.get(index + 1))
            .copied();
        match next {
            Some(next) => self.open_page(next, ctx),
            None => {
                self.mode = CardMode::Acceptance;
                ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
                ctx.notify();
            }
        }
    }

    /// Moves after committing `page`, recomputing the dynamic sequence so
    /// choices such as Local navigate within the newly applicable pages.
    fn navigate_after_confirmation(
        &mut self,
        page: ConfigPage,
        navigation: PageNavigation,
        ctx: &mut ViewContext<Self>,
    ) {
        let sequence =
            Self::page_sequence(&self.orchestration_edit_state.orchestration_config_state);
        let Some(index) = sequence.iter().position(|candidate| *candidate == page) else {
            self.mode = CardMode::Acceptance;
            ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
            ctx.notify();
            return;
        };
        let target = match navigation {
            PageNavigation::Previous => index
                .checked_sub(1)
                .and_then(|index| sequence.get(index))
                .copied(),
            PageNavigation::Next => sequence.get(index + 1).copied(),
        }
        .unwrap_or(page);
        self.open_page(target, ctx);
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
                let id = id.clone();
                self.handle_page_confirmed(&id, ctx);
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
            TuiOptionSelectorEvent::RetryRequested => {
                self.confirmation_navigation = None;
                self.ensure_auth_secrets_fetched(ctx);
                self.refresh_active_page(ctx);
            }
            TuiOptionSelectorEvent::Dismissed => {
                self.confirmation_navigation = None;
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
        if self.decided || self.spawning.is_some() || !self.wants_focus(ctx) {
            return;
        }
        if let Some(reason) = self.controller.accept_disabled_reason(
            &self.orchestration_edit_state.orchestration_config_state,
            ctx,
        ) {
            self.accept_error = Some(reason);
            ctx.notify();
            return;
        }
        self.decided = true;
        self.accept_error = None;
        self.mode = CardMode::Acceptance;
        let request = self.to_request();
        let action_id = self.action_id.clone();
        self.controller.execute(&action_id, request, ctx);
        ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
        ctx.notify();
    }

    /// Reject: resolves the request as rejected exactly once,
    /// from the acceptance card or any configuration page.
    fn handle_reject(&mut self, ctx: &mut ViewContext<Self>) {
        if self.decided || self.spawning.is_some() || !self.wants_focus(ctx) {
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
        if !self.wants_focus(ctx) {
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
        self.confirmation_navigation = None;
        let consumed = self
            .selector
            .update(ctx, |selector, ctx| selector.handle_back(ctx));
        if consumed {
            return;
        }
        if matches!(self.mode, CardMode::Configuring { .. }) {
            self.mode = CardMode::Acceptance;
            ctx.emit(TuiOrchestrationBlockEvent::BlockingStateChanged);
            ctx.notify();
        }
    }

    /// Confirms the selector's selected option (Enter).
    fn handle_confirm_selection(&mut self, ctx: &mut ViewContext<Self>) {
        self.confirmation_navigation = None;
        self.selector.update(ctx, |selector, ctx| {
            selector.confirm_selected(ctx);
        });
    }
    /// Confirms the selector's selected option, then navigates in the arrow's
    /// direction once the selector emits its confirmation event.
    fn handle_arrow_navigation(&mut self, navigation: PageNavigation, ctx: &mut ViewContext<Self>) {
        self.confirmation_navigation = Some(navigation);
        self.selector.update(ctx, |selector, ctx| {
            selector.confirm_selected(ctx);
        });
    }

    // ── Rendering ───────────────────────────────────────────────────

    /// Deterministic identities for the proposed agents, from the pinned
    /// palette.
    fn agent_identities(&self) -> Vec<&AgentIdentity> {
        let names = self
            .request_fields
            .agent_run_configs
            .iter()
            .map(|config| config.name.as_str());
        assign_agent_identity_indices(names, self.identity_palette.len())
            .into_iter()
            .filter_map(|index| self.identity_palette.get(index))
            .collect()
    }

    /// The harness display label for the current selection.
    fn harness_label(&self, ctx: &AppContext) -> String {
        match Harness::parse_orchestration_harness(
            &self
                .orchestration_edit_state
                .orchestration_config_state
                .harness_type,
        ) {
            Some(harness) => HarnessAvailabilityModel::as_ref(ctx)
                .display_name_for(harness)
                .to_string(),
            None => "Warp".to_string(),
        }
    }

    /// The display label for an id within a snapshot, falling back to the id.
    fn label_for_id(snapshot: &OptionSnapshot, id: &str, fallback: &str) -> String {
        snapshot
            .rows
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.label.clone())
            .unwrap_or_else(|| {
                if id.is_empty() {
                    fallback.to_string()
                } else {
                    id.to_string()
                }
            })
    }

    /// The wrapping agent-identity line: every proposed agent's glyph and
    /// name in its identity color, separated by muted bullets.
    fn render_agent_identity_line(&self, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        let mut spans: Vec<(String, _)> = Vec::new();
        for (index, (config, identity)) in self
            .request_fields
            .agent_run_configs
            .iter()
            .zip(self.agent_identities())
            .enumerate()
        {
            if index > 0 {
                spans.push(("  •  ".to_string(), builder.muted_text_style()));
            }
            spans.push((format!("{} ", identity.glyph), identity.style));
            spans.push((
                config.name.clone(),
                identity.style.add_modifier(Modifier::BOLD),
            ));
        }
        TuiText::from_spans(spans).finish()
    }

    /// The wrapping inline `Label: value` metadata line with muted bullet
    /// separators and bold values.
    fn render_metadata_line(
        &self,
        app: &AppContext,
        builder: &TuiUiBuilder,
    ) -> Box<dyn TuiElement> {
        let state = &self.orchestration_edit_state.orchestration_config_state;
        let is_remote = state.execution_mode.is_remote();
        let mut entries: Vec<(&str, String)> = vec![(
            "Location",
            if is_remote { "Cloud" } else { "Local" }.to_string(),
        )];
        entries.push(("Harness", self.harness_label(app)));
        if is_remote {
            if should_show_auth_secret_picker(state) {
                let api_key = match &state.auth_secret_selection {
                    AuthSecretSelection::Named(name) => name.clone(),
                    AuthSecretSelection::Inherit => "Skip (advanced)".to_string(),
                    AuthSecretSelection::Unset | AuthSecretSelection::CreatingNew => {
                        "Select an API key".to_string()
                    }
                };
                entries.push(("API key", api_key));
            }
            let host = match &state.execution_mode {
                RunAgentsExecutionMode::Remote { worker_host, .. }
                    if !worker_host.trim().is_empty() =>
                {
                    worker_host.clone()
                }
                RunAgentsExecutionMode::Remote { .. } | RunAgentsExecutionMode::Local => {
                    ORCHESTRATION_WARP_WORKER_HOST.to_string()
                }
            };
            entries.push(("Host", host));
            let environment_id = match &state.execution_mode {
                RunAgentsExecutionMode::Remote { environment_id, .. } => environment_id.clone(),
                RunAgentsExecutionMode::Local => String::new(),
            };
            entries.push((
                "Environment",
                Self::label_for_id(
                    &environment_snapshot(state, app),
                    &environment_id,
                    "Empty environment",
                ),
            ));
        }
        entries.push((
            "Model",
            Self::label_for_id(
                &model_snapshot(state, app),
                &state.model_id,
                "Default model",
            ),
        ));

        let mut spans: Vec<(String, _)> = Vec::new();
        for (index, (label, value)) in entries.into_iter().enumerate() {
            if index > 0 {
                spans.push(("  •  ".to_string(), builder.muted_text_style()));
            }
            spans.push((format!("{label}: "), builder.primary_text_style()));
            spans.push((value, builder.orchestration_selected_value_style()));
        }
        TuiText::from_spans(spans).finish()
    }

    /// The acceptance card body: the agent list and the inline run-wide
    /// configuration values. The request summary is not repeated here; it
    /// streams into the transcript above the card.
    fn render_acceptance(&self, app: &AppContext, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        let state = &self.orchestration_edit_state.orchestration_config_state;
        let mut column = TuiFlex::column();

        column.add_child(
            TuiText::new(format!(
                "Agents ({}):",
                self.request_fields.agent_run_configs.len()
            ))
            .with_style(builder.primary_text_style())
            .truncate()
            .finish(),
        );
        column.add_child(self.render_agent_identity_line(builder));
        column.add_child(TuiText::new(" ").finish());
        column.add_child(self.render_metadata_line(app, builder));

        // Inline validation or the soft empty-env nudge.
        if let Some(error) = &self.accept_error {
            column.add_child(
                TuiText::new(error.clone())
                    .with_style(builder.error_text_style())
                    .finish(),
            );
        } else if let Some(message) = empty_env_recommendation_message(&state.execution_mode, app) {
            column.add_child(
                TuiText::new(message)
                    .with_style(builder.attention_glyph_style())
                    .truncate()
                    .finish(),
            );
        }

        column.finish()
    }
    /// The persistent title row shared by acceptance and configuration.
    fn render_title(&self, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        TuiText::from_spans([
            ("■ ".to_string(), builder.attention_glyph_style()),
            (
                ORCHESTRATION_BLOCK_TITLE.to_string(),
                builder.primary_text_style(),
            ),
        ])
        .finish()
    }

    /// The configuring body: the active selector page.
    fn render_configuring(&self) -> Box<dyn TuiElement> {
        TuiChildView::new(&self.selector).finish()
    }

    /// Key hints shown below (not inside) the tinted card.
    fn render_footer(&self, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        let spans = match self.mode {
            CardMode::Acceptance => vec![
                ("Enter ".to_string(), builder.primary_text_style()),
                ("to accept  ".to_string(), builder.muted_text_style()),
                ("Ctrl + E".to_string(), builder.primary_text_style()),
                (" to edit ".to_string(), builder.muted_text_style()),
                ("Ctrl + C".to_string(), builder.primary_text_style()),
                (" to reject".to_string(), builder.muted_text_style()),
            ],
            CardMode::Configuring { .. } => vec![
                ("Enter ".to_string(), builder.primary_text_style()),
                ("to accept  ".to_string(), builder.muted_text_style()),
                ("Tab or ← →".to_string(), builder.primary_text_style()),
                (" to navigate  ".to_string(), builder.muted_text_style()),
                ("Esc ".to_string(), builder.primary_text_style()),
                ("to go back".to_string(), builder.muted_text_style()),
            ],
        };
        TuiText::from_spans(spans).finish()
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
        let status = self.controller.action_status(&self.action_id, app);

        // Terminal, spawning, restored, and still-streaming states reuse the
        // shared fallback tool-call row and its `tool_call_labels` copy.
        let interactive = !self.is_restored
            && self.spawning.is_none()
            && matches!(status, Some(AIActionStatus::Blocked));
        if !interactive {
            return render_fallback_tool_call_section(
                &self.action,
                status.as_ref(),
                false,
                None,
                app,
            );
        }

        let builder = TuiUiBuilder::from_app(app);
        // The header row carries a stronger tint than the body, per the
        // design's stacked header overlays.
        let header = TuiContainer::new(self.render_title(&builder))
            .with_background(builder.orchestration_header_background())
            .with_padding_x(1)
            .finish();
        let body = match self.mode {
            CardMode::Acceptance => self.render_acceptance(app, &builder),
            CardMode::Configuring { .. } => self.render_configuring(),
        };
        let body = TuiContainer::new(body)
            .with_background(builder.orchestration_surface_background())
            .with_padding_x(3)
            .with_padding_y(1)
            .finish();
        // Stretch so the tinted header and body fill the full row width
        // rather than sizing to their text content.
        TuiFlex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(header)
            .child(body)
            .child(
                TuiContainer::new(self.render_footer(&builder))
                    .with_padding_top(1)
                    .finish(),
            )
            .finish()
    }
}

impl TypedActionView for TuiOrchestrationBlock {
    type Action = TuiOrchestrationBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiOrchestrationBlockAction::Accept => self.handle_accept(ctx),
            TuiOrchestrationBlockAction::Configure => self.handle_configure(ctx),
            TuiOrchestrationBlockAction::ConfirmSelection => self.handle_confirm_selection(ctx),
            TuiOrchestrationBlockAction::CommitAndPreviousPage => {
                self.handle_arrow_navigation(PageNavigation::Previous, ctx)
            }
            TuiOrchestrationBlockAction::CommitAndNextPage => {
                self.handle_arrow_navigation(PageNavigation::Next, ctx)
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
