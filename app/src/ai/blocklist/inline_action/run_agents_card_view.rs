//! Inline view for the orchestrate (`RunAgents`) confirmation card.
//!
//! Each card is a `View` keyed by `AIAgentActionId`, embedded by
//! `AIBlock` via `ChildView`. Keybindings and Accept dispatch live on
//! the view; only `RejectRequested` flows back to the parent.
use std::collections::HashMap;
use std::rc::Rc;

use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
use ai::agent::action_result::{RunAgentsAgentOutcomeKind, RunAgentsResult};
use ai::agent::orchestration_config::{OrchestrationConfig, OrchestrationConfigStatus};
use ai::skills::SkillReference;
use pathfinder_geometry::vector::vec2f;
use warp_core::send_telemetry_from_ctx;
use warpui::elements::{
    Border, ChildAnchor, ChildView, Container, CornerRadius, CrossAxisAlignment, Empty, Flex,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack, Text, Wrap,
};
use warpui::keymap::FixedBinding;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{icons, AIAgentActionId, AIAgentActionResultType};
use crate::ai::blocklist::action_model::{
    AIActionStatus, BlocklistAIActionEvent, BlocklistAIActionModel, RunAgentsExecutor,
    RunAgentsExecutorEvent, RunAgentsSpawningSnapshot,
};
use crate::ai::blocklist::agent_view::orchestration_pill_bar::render_static_agent_pill;
use crate::ai::blocklist::block::model::AIBlockModel;
use crate::ai::blocklist::block::view_impl::WithContentItemSpacing;
use crate::ai::blocklist::block::AIBlock;
use crate::ai::blocklist::inline_action::create_environment_modal::{
    CreateEnvironmentModal, CreateEnvironmentModalEvent,
};
use crate::ai::blocklist::inline_action::host_picker::{HostPicker, HostPickerEvent};
use crate::ai::blocklist::inline_action::inline_action_header::{HeaderConfig, InteractionMode};
use crate::ai::blocklist::inline_action::inline_action_icons;
use crate::ai::blocklist::inline_action::orchestration_controls::{
    self as oc, AuthSecretSelection, OrchestrationControlAction, OrchestrationPickerHandles,
};
use crate::ai::blocklist::inline_action::requested_action::{
    render_requested_action_row_for_text, CTRL_C_KEYSTROKE, ENTER_KEYSTROKE,
};
use crate::ai::blocklist::telemetry::{
    orchestration_modified_field, BlocklistOrchestrationTelemetryEvent,
    OrchestrationApprovalStatus, OrchestrationEnteredEvent, OrchestrationEntrySource,
    OrchestrationExecutionModeKind, OrchestrationHarnessKind, RunAgentsCardDecision,
    RunAgentsCardDecisionEvent,
};
use crate::ai::connected_self_hosted_workers::{
    ConnectedSelfHostedWorkersEvent, ConnectedSelfHostedWorkersModel,
};
use crate::ai::harness_availability::{
    AuthSecretFetchState, HarnessAvailabilityEvent, HarnessAvailabilityModel,
};
use crate::ai::llms::{LLMPreferences, LLMPreferencesEvent};
use crate::appearance::Appearance;
use crate::menu::{Event as MenuEvent, Menu, MenuItemFields, MenuVariant};
use crate::report_error;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ButtonSize, KeystrokeSource, NakedTheme};
use crate::view_components::compactible_action_button::{
    CompactibleActionButton, RenderCompactibleActionButton, MEDIUM_SIZE_SWITCH_THRESHOLD,
};
use crate::view_components::compactible_split_action_button::CompactibleSplitActionButton;
use crate::view_components::dropdown::DropdownEvent;
use crate::view_components::{FilterableDropdownEvent, FilterableDropdownOrientation};

const RUN_AGENTS_CARD_TITLE: &str = "Can I start additional agents for this task?";

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            RunAgentsCardViewAction::Accept,
            id!(RunAgentsCardView::ui_name()),
        ),
        FixedBinding::new(
            "numpadenter",
            RunAgentsCardViewAction::Accept,
            id!(RunAgentsCardView::ui_name()),
        ),
        FixedBinding::new(
            "ctrl-c",
            RunAgentsCardViewAction::Reject,
            id!(RunAgentsCardView::ui_name()),
        ),
    ]);
}

/// Per-action edit state for the orchestrate confirmation card.
/// Delegates run-wide config fields to `oc::OrchestrationEditState`
/// and adds card-specific fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunAgentsEditState {
    pub orch: oc::OrchestrationEditState,
    pub agent_run_configs: Vec<RunAgentsAgentRunConfig>,
    pub base_prompt: String,
    pub summary: String,
    /// Run-wide skills propagated to each child at dispatch.
    pub skills: Vec<SkillReference>,
    /// The plan that this RunAgents call is executing for.
    pub plan_id: String,
}

impl RunAgentsEditState {
    pub fn from_request(req: &RunAgentsRequest) -> Self {
        let mut orch = oc::OrchestrationEditState::from_run_agents_fields(
            &req.model_id,
            &req.harness_type,
            &req.execution_mode,
        );
        // Carry the request's auth secret across the round trip. Absence
        // becomes `Unset`; the picker re-resolves from persisted settings.
        orch.auth_secret_selection =
            AuthSecretSelection::from_optional_name(req.harness_auth_secret_name.clone());
        if matches!(req.execution_mode, RunAgentsExecutionMode::Local) {
            orch.sanitize_for_local_execution();
        }
        Self {
            orch,
            agent_run_configs: req.agent_run_configs.clone(),
            base_prompt: req.base_prompt.clone(),
            summary: req.summary.clone(),
            skills: req.skills.clone(),
            plan_id: req.plan_id.clone(),
        }
    }

    pub fn to_request(&self) -> RunAgentsRequest {
        RunAgentsRequest {
            summary: self.summary.clone(),
            base_prompt: self.base_prompt.clone(),
            skills: self.skills.clone(),
            model_id: self.orch.model_id.clone(),
            harness_type: self.orch.harness_type.clone(),
            execution_mode: self.orch.execution_mode.clone(),
            agent_run_configs: self.agent_run_configs.clone(),
            plan_id: self.plan_id.clone(),
            harness_auth_secret_name: self.orch.auth_secret_name().map(str::to_string),
        }
    }
}

impl OrchestrationControlAction for RunAgentsCardViewAction {
    fn execution_mode_toggled(is_remote: bool) -> Self {
        Self::ExecutionModeToggled { is_remote }
    }
    fn model_changed(model_id: String) -> Self {
        Self::ModelChanged { model_id }
    }
    fn harness_changed(harness_type: String) -> Self {
        Self::HarnessChanged { harness_type }
    }
    fn environment_changed(environment_id: String) -> Self {
        Self::EnvironmentChanged { environment_id }
    }
    fn create_environment_requested() -> Self {
        Self::CreateEnvironmentRequested
    }
    fn auth_secret_changed(auth_secret_name: Option<String>) -> Self {
        Self::AuthSecretChanged { auth_secret_name }
    }
    fn create_new_auth_secret_requested() -> Self {
        Self::CreateNewAuthSecretRequested
    }
}

/// Per-action UI handles for the confirmation card.
#[derive(Default, Clone)]
struct RunAgentsCardHandles {
    reject_button: Option<CompactibleActionButton>,
    accept_button: Option<CompactibleSplitActionButton>,
    pickers: OrchestrationPickerHandles<RunAgentsCardViewAction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunAgentsCardViewAction {
    Accept,
    AcceptWithoutOrchestration,
    ToggleAcceptMenu,
    Reject,
    ExecutionModeToggled {
        is_remote: bool,
    },
    ModelChanged {
        model_id: String,
    },
    HarnessChanged {
        harness_type: String,
    },
    EnvironmentChanged {
        environment_id: String,
    },
    CreateEnvironmentRequested,
    WorkerHostChanged {
        worker_host: String,
    },
    AuthSecretChanged {
        auth_secret_name: Option<String>,
    },
    /// User picked the "New API key…" item; opens the workspace create modal.
    CreateNewAuthSecretRequested,
}

#[derive(Clone, Debug)]
pub enum RunAgentsCardViewEvent {
    RejectRequested,
}

pub struct RunAgentsCardView {
    action_id: AIAgentActionId,
    state: RunAgentsEditState,
    handles: RunAgentsCardHandles,
    spawning: Option<RunAgentsSpawningSnapshot>,
    /// Retained for interactive defaults and telemetry about plan-sourced
    /// orchestration state.
    active_config: Option<(OrchestrationConfig, OrchestrationConfigStatus)>,

    // Split-button accept menu state
    is_accept_menu_open: bool,
    accept_menu: ViewHandle<Menu<RunAgentsCardViewAction>>,
    position_id_prefix: String,

    action_model: ModelHandle<BlocklistAIActionModel>,
    block_model: Rc<dyn AIBlockModel<View = AIBlock>>,
    /// UI-only per-harness model memory so switching harnesses preserves
    /// the user's previous model selection for each harness.
    saved_model_per_harness: HashMap<String, String>,
    create_environment_modal: ViewHandle<CreateEnvironmentModal>,
    /// Snapshot of the latest raw `RunAgentsRequest` from the LLM
    /// stream. Used at decision time to diff the run-wide config
    /// fields the user changed before accepting.
    original_tool_call_request: RunAgentsRequest,
    /// Guards `OrchestrationEntered` against double-fires on re-renders.
    entered_event_emitted: bool,
    /// Guards the terminal decision event against double-fires.
    decision_event_emitted: bool,
    /// One-shot guard: cancelling the auto-popped modal must not re-pop.
    /// Reset on harness / execution-mode change.
    has_auto_opened_create_modal: bool,
}

/// Resolves UI-only interactive defaults on edit state that has
/// already had config-inherited fields resolved. These defaults are
/// for the picker display and should NOT run before auto-launch
/// matching.
///
/// 1. Defaults the Oz model to the conversation's base model.
/// 2. Defaults Remote worker_host to "warp".
/// 3. Defaults a Remote environment from settings / recency.
fn resolve_interactive_defaults(
    state: &mut RunAgentsEditState,
    block_model: &dyn AIBlockModel<View = AIBlock>,
    ctx: &AppContext,
) {
    if state.orch.model_id.is_empty() {
        let harness =
            warp_cli::agent::Harness::parse_orchestration_harness(&state.orch.harness_type);
        if matches!(harness, Some(warp_cli::agent::Harness::Oz) | None) {
            if let Some(base) = block_model.base_model(ctx).map(|id| id.to_string()) {
                state.orch.model_id = base;
            }
        }
    }
    if let RunAgentsExecutionMode::Remote {
        environment_id,
        worker_host,
        ..
    } = &state.orch.execution_mode
    {
        let needs_host = worker_host.is_empty();
        let needs_env = environment_id.is_empty();
        if needs_host {
            // Prefer the workspace default (or the dev env-var override)
            // over the bare "warp" fallback so self-hosted teams see
            // their default pre-selected. Mirrors the Oz webapp's
            // `HostSelector` initial-selection behavior.
            let default_host = oc::resolve_default_host_slug(ctx)
                .unwrap_or_else(|| oc::ORCHESTRATION_WARP_WORKER_HOST.to_string());
            state.orch.set_worker_host(default_host);
        }
        if needs_env {
            if let Some(default_env) = oc::resolve_default_environment_id(ctx) {
                state.orch.set_environment_id(default_env);
            }
        }
    }
}
impl RunAgentsCardView {
    pub fn new(
        action_id: AIAgentActionId,
        request: &RunAgentsRequest,
        active_config: Option<(OrchestrationConfig, OrchestrationConfigStatus)>,
        action_model: ModelHandle<BlocklistAIActionModel>,
        run_agents_executor: ModelHandle<RunAgentsExecutor>,
        block_model: Rc<dyn AIBlockModel<View = AIBlock>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let state = RunAgentsEditState::from_request(request);
        // Snapshot the raw incoming request so we can diff against the
        // edited state at Accept time.
        let original_tool_call_request = request.clone();

        let reject_keystroke = CTRL_C_KEYSTROKE.clone();
        let accept_keystroke = ENTER_KEYSTROKE.clone();

        let reject_button = CompactibleActionButton::new(
            "Reject".to_string(),
            Some(KeystrokeSource::Fixed(reject_keystroke)),
            ButtonSize::Small,
            RunAgentsCardViewAction::Reject,
            Icon::X,
            std::sync::Arc::new(NakedTheme),
            ctx,
        );
        let position_id_prefix = format!("{action_id:?}");
        let accept_button = CompactibleSplitActionButton::new(
            "Accept".to_string(),
            Some(KeystrokeSource::Fixed(accept_keystroke)),
            ButtonSize::Small,
            RunAgentsCardViewAction::Accept,
            RunAgentsCardViewAction::ToggleAcceptMenu,
            Icon::Check,
            true,
            Some(Self::get_position_id_for_accept_split_button(
                &position_id_prefix,
            )),
            ctx,
        );

        let accept_menu = ctx.add_typed_action_view(|ctx| {
            let theme = Appearance::as_ref(ctx).theme();
            Menu::new()
                .with_menu_variant(MenuVariant::Fixed)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .prevent_interaction_with_other_elements()
        });
        ctx.subscribe_to_view(&accept_menu, |me, _menu, event, ctx| match event {
            MenuEvent::Close { .. } => {
                me.is_accept_menu_open = false;
                ctx.notify();
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        let action_id_for_subscription = action_id.clone();
        ctx.subscribe_to_model(&run_agents_executor, move |me, _, event, ctx| match event {
            RunAgentsExecutorEvent::SpawningStarted {
                action_id,
                snapshot,
            } if action_id == &action_id_for_subscription => {
                me.spawning = Some(*snapshot);
                ctx.notify();
            }
            RunAgentsExecutorEvent::SpawningFinished { action_id }
                if action_id == &action_id_for_subscription =>
            {
                me.spawning = None;
                ctx.notify();
            }
            RunAgentsExecutorEvent::SpawningStarted { .. }
            | RunAgentsExecutorEvent::SpawningFinished { .. } => {}
        });

        // Re-render when this action finishes or becomes blocked.
        let action_id_for_action_events = action_id.clone();
        ctx.subscribe_to_model(&action_model, move |me, _, event, ctx| match event {
            BlocklistAIActionEvent::FinishedAction { action_id, .. }
                if action_id == &action_id_for_action_events =>
            {
                ctx.notify();
            }
            BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(action_id)
                if action_id == &action_id_for_action_events =>
            {
                // Normal case: streaming is complete and the action is
                // ready for user confirmation. Re-render so the card
                // transitions from the "Configuring agents..." placeholder
                // to the full confirmation UI.
                resolve_interactive_defaults(&mut me.state, &*me.block_model, ctx);
                oc::repopulate_all_pickers(&mut me.state.orch, &me.handles.pickers, ctx);
                me.refresh_accept_button_state(ctx);
                me.maybe_auto_open_create_modal(ctx);
                if let Some(conversation_id) = me.block_model.conversation_id(ctx) {
                    me.emit_orchestration_entered_once(conversation_id, ctx);
                }
                ctx.notify();
            }
            _ => {}
        });

        // Repopulate the model picker when available Warp LLMs change.
        // Only relevant for Oz harness — non-Oz harnesses get their
        // model catalog from HarnessAvailabilityModel, not LLMPreferences.
        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |me, _, event, ctx| {
            if let LLMPreferencesEvent::UpdatedAvailableLLMs = event {
                if let Some(handle) = &me.handles.pickers.model_picker {
                    let is_local = !me.state.orch.execution_mode.is_remote();
                    oc::populate_model_picker_for_harness(
                        handle,
                        &me.state.orch.model_id,
                        &me.state.orch.harness_type,
                        is_local,
                        ctx,
                    );
                }
            }
        });

        // Repopulate pickers when the server-provided harness list,
        // harness model catalogs, or per-harness auth secrets change.
        // Without an `AuthSecretsLoaded` handler the picker stays on
        // "Loading…" forever after the lazy fetch completes.
        ctx.subscribe_to_model(
            &HarnessAvailabilityModel::handle(ctx),
            |me, _, event, ctx| match event {
                HarnessAvailabilityEvent::AuthSecretCreated { harness, name } => {
                    // Adopt the new secret before repopulating the picker.
                    oc::apply_created_auth_secret_if_matches(
                        &mut me.state.orch,
                        *harness,
                        name,
                        ctx,
                    );
                    oc::repopulate_all_pickers(&mut me.state.orch, &me.handles.pickers, ctx);
                    me.refresh_accept_button_state(ctx);
                    ctx.notify();
                }
                HarnessAvailabilityEvent::Changed
                | HarnessAvailabilityEvent::AuthSecretsLoaded
                | HarnessAvailabilityEvent::AuthSecretsFetchFailed
                | HarnessAvailabilityEvent::AuthSecretDeleted { .. } => {
                    // Repopulate even on fetch failure to replace "Loading…".
                    // Deleted events also force a repopulate so this card
                    // stops surfacing the deleted secret as an option.
                    oc::repopulate_all_pickers(&mut me.state.orch, &me.handles.pickers, ctx);
                    me.refresh_accept_button_state(ctx);
                    me.maybe_auto_open_create_modal(ctx);
                    ctx.notify();
                }
                HarnessAvailabilityEvent::AuthSecretCreationFailed { .. }
                | HarnessAvailabilityEvent::AuthSecretDeletionFailed { .. } => {}
            },
        );

        let create_environment_modal = ctx.add_typed_action_view(CreateEnvironmentModal::new);
        ctx.subscribe_to_view(&create_environment_modal, |me, _, event, ctx| match event {
            CreateEnvironmentModalEvent::Created { environment_id } => {
                me.select_created_environment(environment_id.clone(), ctx);
            }
            CreateEnvironmentModalEvent::Cancelled => {
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(
            &ConnectedSelfHostedWorkersModel::handle(ctx),
            |me, _, event, ctx| match event {
                ConnectedSelfHostedWorkersEvent::Changed => {
                    oc::repopulate_all_pickers(&mut me.state.orch, &me.handles.pickers, ctx);
                    me.refresh_accept_button_state(ctx);
                    ctx.notify();
                }
            },
        );
        // When auto_launched is true, execution is deferred to the
        // ActionBlockedOnUserConfirmation subscription above — the action
        // hasn't been queued in pending_actions yet at construction time.
        let mut view = Self {
            action_id,
            state,
            handles: RunAgentsCardHandles {
                reject_button: Some(reject_button),
                accept_button: Some(accept_button),
                ..Default::default()
            },
            spawning: None,
            active_config,
            is_accept_menu_open: false,
            accept_menu,
            position_id_prefix,
            action_model,
            block_model,
            saved_model_per_harness: HashMap::new(),
            create_environment_modal,
            original_tool_call_request,
            entered_event_emitted: false,
            decision_event_emitted: false,
            has_auto_opened_create_modal: false,
        };

        view.ensure_pickers(ctx);
        view.refresh_accept_button_state(ctx);
        // No-ops if secrets are still in flight; the `AuthSecretsLoaded`
        // subscription will retry once they resolve.
        view.maybe_auto_open_create_modal(ctx);

        view
    }

    /// Re-sync edit state from the latest streaming request.
    pub fn update_request(&mut self, request: &RunAgentsRequest, ctx: &mut ViewContext<Self>) {
        if self.spawning.is_some() {
            return;
        }
        // Keep the raw-tool-call snapshot in sync with the latest
        // streamed chunk.
        self.original_tool_call_request = request.clone();
        let mut new_state = RunAgentsEditState::from_request(request);
        // Resolve empty fields from the active config (same as in new()).
        if let Some((config, status)) = &self.active_config {
            if status.is_approved() {
                new_state.orch.resolve_from_config(config);
            }
        }
        if new_state.orch.model_id.is_empty() {
            let harness =
                warp_cli::agent::Harness::parse_orchestration_harness(&new_state.orch.harness_type);
            if matches!(harness, Some(warp_cli::agent::Harness::Oz) | None) {
                if let Some(base) = self.block_model.base_model(ctx).map(|id| id.to_string()) {
                    new_state.orch.model_id = base;
                }
            }
        }
        // Re-seed an Unset selection from persisted per-harness settings,
        // honoring an explicit `Inherit` choice for this harness.
        if matches!(
            new_state.orch.auth_secret_selection,
            AuthSecretSelection::Unset
        ) {
            new_state.orch.auth_secret_selection =
                oc::resolve_auth_secret_selection_for_harness(&new_state.orch.harness_type, ctx);
        }
        if self.state != new_state {
            let harness_or_model_changed = self.state.orch.harness_type
                != new_state.orch.harness_type
                || self.state.orch.model_id != new_state.orch.model_id
                || self.state.orch.execution_mode != new_state.orch.execution_mode;
            self.state = new_state;
            if harness_or_model_changed {
                // Repopulate pickers and re-arm auto-open for the newly-
                // streamed harness.
                oc::repopulate_all_pickers(&mut self.state.orch, &self.handles.pickers, ctx);
                self.has_auto_opened_create_modal = false;
            }
            self.refresh_accept_button_state(ctx);
            self.maybe_auto_open_create_modal(ctx);
            ctx.notify();
        }
    }

    /// Validates and dispatches the resolved request.
    pub fn accept(&mut self, ctx: &mut ViewContext<Self>) {
        self.handle_accept(ctx);
    }

    fn handle_accept(&mut self, ctx: &mut ViewContext<Self>) {
        if self.spawning.is_some() {
            return;
        }
        if let Some(reason) = oc::accept_disabled_reason_with_auth(&self.state.orch, ctx) {
            log::warn!("RunAgentsCardView: refusing Accept because action is disabled: {reason}");
            return;
        }
        let request = self.state.to_request();
        self.emit_decision(RunAgentsCardDecision::Accept, ctx);
        let action_id = self.action_id.clone();
        self.action_model.update(ctx, |action_model, action_ctx| {
            action_model.execute_run_agents(&action_id, request, action_ctx);
        });
    }

    /// Emits `OrchestrationEntered::RunAgentsCardShown` at most once
    /// per card instance.
    fn emit_orchestration_entered_once(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.entered_event_emitted {
            return;
        }
        self.entered_event_emitted = true;
        send_telemetry_from_ctx!(
            BlocklistOrchestrationTelemetryEvent::OrchestrationEntered(OrchestrationEnteredEvent {
                conversation_id,
                plan_id: (!self.state.plan_id.is_empty()).then(|| self.state.plan_id.clone()),
                entry_source: OrchestrationEntrySource::RunAgentsCardShown,
            }),
            ctx
        );
    }

    /// Emits `RunAgentsCardDecision` at most once per card instance.
    fn emit_decision(&mut self, decision: RunAgentsCardDecision, ctx: &mut ViewContext<Self>) {
        if self.decision_event_emitted {
            return;
        }
        self.decision_event_emitted = true;
        let Some(conversation_id) = self.block_model.conversation_id(ctx) else {
            return;
        };
        let modified_fields_from_tool_call =
            diverged_orch_fields(&self.state.orch, &self.original_tool_call_request);
        let (had_active_config, active_config_status, modified_fields_from_active_config) =
            match &self.active_config {
                Some((cfg, status)) => {
                    let status_enum = if status.is_approved() {
                        Some(OrchestrationApprovalStatus::Approved)
                    } else if status.is_disapproved() {
                        Some(OrchestrationApprovalStatus::Disapproved)
                    } else {
                        None
                    };
                    let diff = if status.is_approved() {
                        diverged_orch_fields_against_config(&self.state.orch, cfg)
                    } else {
                        Vec::new()
                    };
                    (true, status_enum, diff)
                }
                None => (false, None, Vec::new()),
            };
        send_telemetry_from_ctx!(
            BlocklistOrchestrationTelemetryEvent::RunAgentsCardDecision(
                RunAgentsCardDecisionEvent {
                    conversation_id,
                    plan_id: (!self.state.plan_id.is_empty()).then(|| self.state.plan_id.clone()),
                    decision,
                    agent_count: self.state.agent_run_configs.len(),
                    harness: OrchestrationHarnessKind::from_str(&self.state.orch.harness_type),
                    execution_mode: OrchestrationExecutionModeKind::from_run_agents(
                        &self.state.orch.execution_mode,
                    ),
                    modified_fields_from_tool_call,
                    modified_fields_from_active_config,
                    had_active_config,
                    active_config_status,
                }
            ),
            ctx
        );
    }

    /// Auto-pops the create-key modal once per card per harness/mode
    /// change when the harness has no loaded secrets and selection is
    /// `Unset`. Cancelling leaves the picker on "+ New API key…"; the
    /// user can reopen the modal by clicking that item.
    fn maybe_auto_open_create_modal(&mut self, ctx: &mut ViewContext<Self>) {
        if self.has_auto_opened_create_modal {
            return;
        }
        // Skip non-interactive card states (render short-circuits to a
        // status-only card; the user can't act on a popped modal).
        if self.spawning.is_some() {
            return;
        }
        if self.block_model.is_restored() {
            return;
        }
        if matches!(
            self.action_model
                .as_ref(ctx)
                .get_action_status(&self.action_id),
            Some(AIActionStatus::Finished(_)) | Some(AIActionStatus::RunningAsync)
        ) {
            return;
        }
        if !oc::should_show_auth_secret_picker(&self.state.orch) {
            return;
        }
        if !matches!(
            self.state.orch.auth_secret_selection,
            AuthSecretSelection::Unset
        ) {
            return;
        }
        let Some(harness) =
            warp_cli::agent::Harness::parse_orchestration_harness(&self.state.orch.harness_type)
        else {
            return;
        };
        // Only auto-open on `Loaded([])`. Other fetch states are
        // ambiguous; the `AuthSecretsLoaded` subscription will retry.
        let has_zero_loaded = matches!(
            HarnessAvailabilityModel::as_ref(ctx).auth_secrets_for(harness),
            AuthSecretFetchState::Loaded(secrets) if secrets.is_empty()
        );
        if !has_zero_loaded {
            return;
        }
        self.has_auto_opened_create_modal = true;
        ctx.dispatch_typed_action(
            &crate::workspace::WorkspaceAction::OpenCreateAuthSecretModal { harness },
        );
    }

    /// Re-derives the Accept button's `disabled` + tooltip from the gate.
    /// Call after every code path that mutates `self.state.orch`.
    fn refresh_accept_button_state(&mut self, ctx: &mut ViewContext<Self>) {
        let reason = oc::accept_disabled_reason_with_auth(&self.state.orch, ctx);
        let Some(mut accept) = self.handles.accept_button.clone() else {
            return;
        };
        accept.set_disabled(reason.is_some(), ctx);
        // Tooltip explains why the button is disabled; falls back to "Accept".
        accept.set_tooltip(reason.or_else(|| Some("Accept".to_string())), ctx);
        self.handles.accept_button = Some(accept);
    }

    /// Construct the picker dropdown views (idempotent).
    fn ensure_pickers(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let (styles, colors) = oc::picker_styles(appearance);

        let initial_model_id_default = self
            .block_model
            .base_model(ctx)
            .map(|id| id.to_string())
            .unwrap_or_default();
        let state = &self.state;

        if self.handles.pickers.model_picker.is_none() {
            let initial_model_id = if state.orch.model_id.trim().is_empty() {
                initial_model_id_default.clone()
            } else {
                state.orch.model_id.clone()
            };
            let is_local = !state.orch.execution_mode.is_remote();
            let handle = oc::new_standard_filterable_picker_dropdown(&styles, ctx);
            Self::set_upward_filterable_menu_position(&handle, ctx);
            oc::populate_model_picker_for_harness(
                &handle,
                &initial_model_id,
                &state.orch.harness_type,
                is_local,
                ctx,
            );
            Self::subscribe_filterable_picker_close(&handle, ctx);
            self.handles.pickers.model_picker = Some(handle);
        }

        if self.handles.pickers.harness_picker.is_none() {
            let handle = oc::new_standard_picker_dropdown(&colors, ctx);
            Self::set_upward_menu_position(&handle, ctx);
            oc::populate_harness_picker(
                &handle,
                &state.orch.harness_type,
                !state.orch.execution_mode.is_remote(),
                ctx,
            );
            Self::subscribe_picker_close(&handle, ctx);
            self.handles.pickers.harness_picker = Some(handle);
        }

        if self.handles.pickers.environment_picker.is_none() {
            let initial_env = match &state.orch.execution_mode {
                RunAgentsExecutionMode::Remote { environment_id, .. } => environment_id.as_str(),
                RunAgentsExecutionMode::Local => "",
            };
            let handle = oc::create_environment_picker(initial_env, &styles, ctx);
            handle.update(ctx, |d, _| {
                d.set_orientation(FilterableDropdownOrientation::Up)
            });
            ctx.subscribe_to_view(&handle, |me, _, event, ctx| {
                if let FilterableDropdownEvent::Close = event {
                    me.refocus_after_picker_close(ctx);
                }
            });
            self.handles.pickers.environment_picker = Some(handle);
        }

        if self.handles.pickers.host_picker.is_none() {
            let initial_host = match &state.orch.execution_mode {
                RunAgentsExecutionMode::Remote { worker_host, .. } => worker_host.as_str(),
                RunAgentsExecutionMode::Local => oc::ORCHESTRATION_WARP_WORKER_HOST,
            };
            let handle = ctx.add_typed_action_view(HostPicker::new);
            // Open upward so the menu doesn't overlap pickers below it,
            // matching the other dropdowns in this card.
            handle.update(ctx, |picker, picker_ctx| {
                picker.set_menu_position(
                    warpui::elements::PositionedElementAnchor::TopLeft,
                    warpui::elements::ChildAnchor::BottomLeft,
                    picker_ctx,
                );
            });
            oc::populate_host_picker(&handle, initial_host, ctx);
            ctx.subscribe_to_view(&handle, |me, _, event, ctx| match event {
                HostPickerEvent::Opened => {
                    ConnectedSelfHostedWorkersModel::handle(ctx).update(ctx, |model, ctx| {
                        model.refresh(ctx);
                    });
                }
                HostPickerEvent::HostChanged { slug } => {
                    ctx.dispatch_typed_action(&RunAgentsCardViewAction::WorkerHostChanged {
                        worker_host: slug.clone(),
                    });
                }
                HostPickerEvent::Closed => {
                    me.refocus_after_picker_close(ctx);
                }
            });
            self.handles.pickers.host_picker = Some(handle);
        }

        if self.handles.pickers.auth_secret_picker.is_none() {
            // Seed from the request's secret name first; otherwise fall
            // back to the persisted per-harness selection so the picker
            // matches what cloud-mode would show. Honors an explicit
            // `Inherit` choice for this harness.
            if matches!(
                self.state.orch.auth_secret_selection,
                AuthSecretSelection::Unset
            ) {
                self.state.orch.auth_secret_selection =
                    oc::resolve_auth_secret_selection_for_harness(
                        &self.state.orch.harness_type,
                        ctx,
                    );
            }
            let selection = self.state.orch.auth_secret_selection.clone();
            let harness_type = self.state.orch.harness_type.clone();
            let handle = oc::new_standard_picker_dropdown(&colors, ctx);
            Self::set_upward_menu_position(&handle, ctx);
            oc::populate_auth_secret_picker_for_harness(&handle, &selection, &harness_type, ctx);
            Self::subscribe_picker_close(&handle, ctx);
            self.handles.pickers.auth_secret_picker = Some(handle);
        }

        self.sync_picker_selections(ctx);
    }

    /// Opens the dropdown menu above the trigger to avoid overlapping
    /// the input box. Only used by the confirmation card — the plan
    /// config card renders higher up where downward menus are fine.
    fn set_upward_menu_position(
        dropdown_handle: &ViewHandle<
            crate::view_components::dropdown::Dropdown<RunAgentsCardViewAction>,
        >,
        ctx: &mut ViewContext<Self>,
    ) {
        dropdown_handle.update(ctx, |dropdown, ctx| {
            dropdown.set_menu_position(
                warpui::elements::PositionedElementAnchor::TopLeft,
                warpui::elements::ChildAnchor::BottomLeft,
                ctx,
            );
        });
    }

    fn set_upward_filterable_menu_position(
        dropdown_handle: &ViewHandle<
            crate::view_components::FilterableDropdown<RunAgentsCardViewAction>,
        >,
        ctx: &mut ViewContext<Self>,
    ) {
        dropdown_handle.update(ctx, |dropdown, _| {
            dropdown.set_orientation(FilterableDropdownOrientation::Up)
        });
    }

    fn subscribe_picker_close(
        dropdown_handle: &ViewHandle<
            crate::view_components::dropdown::Dropdown<RunAgentsCardViewAction>,
        >,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.subscribe_to_view(dropdown_handle, move |me, _, event, ctx| {
            if let DropdownEvent::Close = event {
                me.refocus_after_picker_close(ctx);
            }
        });
    }

    fn subscribe_filterable_picker_close(
        dropdown_handle: &ViewHandle<
            crate::view_components::FilterableDropdown<RunAgentsCardViewAction>,
        >,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.subscribe_to_view(dropdown_handle, move |me, _, event, ctx| {
            if let FilterableDropdownEvent::Close = event {
                me.refocus_after_picker_close(ctx);
            }
        });
    }

    fn refocus_after_picker_close(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn sync_picker_selections(&mut self, ctx: &mut ViewContext<Self>) {
        oc::sync_picker_selections(&self.state.orch, &self.handles.pickers, ctx);
    }

    fn open_create_environment_modal(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(environment_picker) = &self.handles.pickers.environment_picker {
            environment_picker.update(ctx, |dropdown, ctx| dropdown.close(ctx));
        }
        self.create_environment_modal.update(ctx, |modal, ctx| {
            modal.show(ctx);
        });
        ctx.notify();
    }

    fn select_created_environment(&mut self, environment_id: String, ctx: &mut ViewContext<Self>) {
        self.state.orch.set_environment_id(environment_id.clone());
        if let Some(environment_picker) = &self.handles.pickers.environment_picker {
            oc::populate_environment_picker(environment_picker, &environment_id, ctx);
        }
        ctx.notify();
    }

    fn toggle_accept_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_accept_menu_open = !self.is_accept_menu_open;
        if self.is_accept_menu_open {
            let item = MenuItemFields::new_with_label("Accept w/o orchestration", "")
                .with_on_select_action(RunAgentsCardViewAction::AcceptWithoutOrchestration)
                .into_item();
            self.accept_menu.update(ctx, |menu, ctx| {
                menu.set_items(vec![item], ctx);
            });
            self.accept_menu
                .update(ctx, |menu, ctx| menu.set_selected_by_index(0, ctx));
            ctx.focus(&self.accept_menu);
        }
        ctx.notify();
    }

    fn get_position_id_for_accept_split_button(prefix: &str) -> String {
        format!("RunAgentsCardView-{prefix}-accept-split")
    }
}

impl Entity for RunAgentsCardView {
    type Event = RunAgentsCardViewEvent;
}

impl View for RunAgentsCardView {
    fn ui_name() -> &'static str {
        "RunAgentsCardView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let status = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action_id);

        if let Some(AIActionStatus::Finished(result)) = &status {
            if let AIAgentActionResultType::RunAgents(orchestrate_result) = &result.result {
                return render_terminal_state(orchestrate_result, appearance, app);
            }
            report_error!(
                "Unexpected action result type for orchestrate",
                extra: { "result_type" => ?result.result }
            );
            return Empty::new().finish();
        }

        // In-flight dispatch: check both spawning snapshot and action
        // status because the event arrives one tick after the status.
        if let Some(snapshot) = &self.spawning {
            return render_spawning_card(snapshot, appearance, app);
        }
        if matches!(status, Some(AIActionStatus::RunningAsync)) {
            let snapshot = RunAgentsSpawningSnapshot {
                agent_count: self.state.agent_run_configs.len(),
            };
            return render_spawning_card(&snapshot, appearance, app);
        }

        // Restored-from-history: dispatch state is lost, render as
        // Cancelled. Must be checked before the streaming gate below,
        // because restored blocks have no pending action status.
        if self.block_model.is_restored() {
            return render_status_only_card(
                "Spawn agents cancelled".to_string(),
                appearance,
                StatusKind::Cancelled,
                app,
            );
        }

        // Still streaming: show "Configuring agents..." placeholder until
        // the action reaches Blocked status (i.e., streaming is complete
        // and the action is queued for user confirmation).
        if !matches!(status, Some(AIActionStatus::Blocked)) {
            return render_status_only_card(
                "Configuring agents\u{2026}".to_string(),
                appearance,
                StatusKind::Spawning,
                app,
            );
        }

        let is_blocked = matches!(status, Some(AIActionStatus::Blocked));
        let card = render_confirmation_card(&self.state, &self.handles, is_blocked, app);

        let mut root_stack = Stack::new();
        root_stack.add_child(card);

        if self.is_accept_menu_open {
            root_stack.add_positioned_child(
                ChildView::new(&self.accept_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    Self::get_position_id_for_accept_split_button(&self.position_id_prefix),
                    vec2f(0., 8.),
                    warpui::elements::PositionedElementOffsetBounds::WindowByPosition,
                    warpui::elements::PositionedElementAnchor::BottomRight,
                    warpui::elements::ChildAnchor::TopRight,
                ),
            );
        }

        if self.create_environment_modal.as_ref(app).is_visible() {
            root_stack.add_positioned_overlay_child(
                ChildView::new(&self.create_environment_modal).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }

        root_stack.finish()
    }
}

impl TypedActionView for RunAgentsCardView {
    type Action = RunAgentsCardViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            RunAgentsCardViewAction::Accept => {
                self.handle_accept(ctx);
            }
            RunAgentsCardViewAction::AcceptWithoutOrchestration => {
                self.emit_decision(RunAgentsCardDecision::AcceptWithoutOrchestration, ctx);
                let action_id = self.action_id.clone();
                self.action_model.update(ctx, |action_model, action_ctx| {
                    action_model.deny_run_agents(&action_id, String::new(), action_ctx);
                });
            }
            RunAgentsCardViewAction::ToggleAcceptMenu => {
                self.toggle_accept_menu(ctx);
            }
            RunAgentsCardViewAction::Reject => {
                self.emit_decision(RunAgentsCardDecision::Reject, ctx);
                ctx.emit(RunAgentsCardViewEvent::RejectRequested);
            }
            RunAgentsCardViewAction::ExecutionModeToggled { is_remote } => {
                let block_model = self.block_model.clone();
                oc::apply_execution_mode_change(
                    &mut self.state.orch,
                    &self.handles.pickers,
                    *is_remote,
                    |ctx| block_model.base_model(ctx).map(|id| id.to_string()),
                    ctx,
                );
                // Mode change can newly reveal the auth picker (Local
                // → Cloud) — give the user a fresh auto-open prompt.
                self.has_auto_opened_create_modal = false;
                self.refresh_accept_button_state(ctx);
                self.maybe_auto_open_create_modal(ctx);
                ctx.notify();
            }
            RunAgentsCardViewAction::ModelChanged { model_id } => {
                self.state.orch.model_id = model_id.clone();
                self.refresh_accept_button_state(ctx);
                ctx.notify();
            }
            RunAgentsCardViewAction::HarnessChanged { harness_type } => {
                let block_model = self.block_model.clone();
                oc::apply_harness_change(
                    &mut self.state.orch,
                    &mut self.saved_model_per_harness,
                    &self.handles.pickers,
                    harness_type,
                    |ctx| block_model.base_model(ctx).map(|id| id.to_string()),
                    ctx,
                );
                // Harness change resets per-harness selection state, so
                // give the new harness a fresh auto-open prompt.
                self.has_auto_opened_create_modal = false;
                self.refresh_accept_button_state(ctx);
                self.maybe_auto_open_create_modal(ctx);
                ctx.notify();
            }
            RunAgentsCardViewAction::EnvironmentChanged { environment_id } => {
                self.state.orch.set_environment_id(environment_id.clone());
                oc::persist_environment_selection(environment_id, ctx);
                self.refresh_accept_button_state(ctx);
                ctx.notify();
            }
            RunAgentsCardViewAction::CreateEnvironmentRequested => {
                self.open_create_environment_modal(ctx);
            }
            RunAgentsCardViewAction::WorkerHostChanged { worker_host } => {
                self.state.orch.set_worker_host(worker_host.clone());
                oc::persist_host_selection(worker_host, ctx);
                self.refresh_accept_button_state(ctx);
                ctx.notify();
            }
            RunAgentsCardViewAction::AuthSecretChanged { auth_secret_name } => {
                oc::apply_auth_secret_change(
                    &mut self.state.orch,
                    &self.handles.pickers,
                    auth_secret_name.clone(),
                    ctx,
                );
                self.refresh_accept_button_state(ctx);
                ctx.notify();
            }
            RunAgentsCardViewAction::CreateNewAuthSecretRequested => {
                oc::apply_create_new_auth_secret_requested(&mut self.state.orch, ctx);
                if let Some(harness) = warp_cli::agent::Harness::parse_orchestration_harness(
                    &self.state.orch.harness_type,
                ) {
                    ctx.dispatch_typed_action(
                        &crate::workspace::WorkspaceAction::OpenCreateAuthSecretModal { harness },
                    );
                }
                self.refresh_accept_button_state(ctx);
                ctx.notify();
            }
        }
    }
}

/// Field names from [`orchestration_modified_field`] that differ
/// between the user-edited `state` and the LLM's original
/// `RunAgentsRequest`.
fn diverged_orch_fields(
    state: &oc::OrchestrationEditState,
    original: &RunAgentsRequest,
) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if state.model_id != original.model_id {
        fields.push(orchestration_modified_field::MODEL_ID);
    }
    if state.harness_type != original.harness_type {
        fields.push(orchestration_modified_field::HARNESS);
    }
    let state_remote = state.execution_mode.is_remote();
    let original_remote = original.execution_mode.is_remote();
    if state_remote != original_remote {
        fields.push(orchestration_modified_field::EXECUTION_MODE);
    } else if let (
        RunAgentsExecutionMode::Remote {
            environment_id: state_env,
            worker_host: state_host,
            ..
        },
        RunAgentsExecutionMode::Remote {
            environment_id: orig_env,
            worker_host: orig_host,
            ..
        },
    ) = (&state.execution_mode, &original.execution_mode)
    {
        if state_env != orig_env {
            fields.push(orchestration_modified_field::ENVIRONMENT_ID);
        }
        if state_host != orig_host {
            fields.push(orchestration_modified_field::WORKER_HOST);
        }
    }
    if state.auth_secret_name() != original.harness_auth_secret_name.as_deref() {
        fields.push(orchestration_modified_field::AUTH_SECRET);
    }
    fields
}

/// Same shape as [`diverged_orch_fields`] but compares against an
/// approved `OrchestrationConfig`. auth_secret is omitted: managed
/// secrets are per-user, not stored on the config.
fn diverged_orch_fields_against_config(
    state: &oc::OrchestrationEditState,
    config: &OrchestrationConfig,
) -> Vec<&'static str> {
    use ai::agent::orchestration_config::OrchestrationExecutionMode;
    let mut fields = Vec::new();
    if state.model_id != config.model_id {
        fields.push(orchestration_modified_field::MODEL_ID);
    }
    if state.harness_type != config.harness_type {
        fields.push(orchestration_modified_field::HARNESS);
    }
    let state_remote = state.execution_mode.is_remote();
    let config_remote = matches!(
        config.execution_mode,
        OrchestrationExecutionMode::Remote { .. }
    );
    if state_remote != config_remote {
        fields.push(orchestration_modified_field::EXECUTION_MODE);
    } else if let (
        RunAgentsExecutionMode::Remote {
            environment_id: state_env,
            worker_host: state_host,
            ..
        },
        OrchestrationExecutionMode::Remote {
            environment_id: cfg_env,
            worker_host: cfg_host,
        },
    ) = (&state.execution_mode, &config.execution_mode)
    {
        if state_env != cfg_env {
            fields.push(orchestration_modified_field::ENVIRONMENT_ID);
        }
        if state_host != cfg_host {
            fields.push(orchestration_modified_field::WORKER_HOST);
        }
    }
    fields
}

fn render_confirmation_card(
    state: &RunAgentsEditState,
    handles: &RunAgentsCardHandles,
    is_blocked: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let header = render_header(handles, app);
    let body = render_body(state, app);

    let mut content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(header)
        .with_child(body);

    content.add_child(render_editor(state, handles, app));

    let border_color = if is_blocked {
        theme.accent()
    } else {
        theme.surface_2()
    };

    Container::new(content.finish())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_border(Border::all(1.).with_border_fill(border_color))
        .finish()
        .with_content_item_spacing()
        .finish()
}

fn render_header(handles: &RunAgentsCardHandles, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let mut config = HeaderConfig::new(RUN_AGENTS_CARD_TITLE, app)
        .with_icon(icons::yellow_stop_icon(appearance))
        .with_corner_radius_override(CornerRadius::with_top(Radius::Pixels(8.)));

    if let (Some(reject), Some(accept)) = (
        handles.reject_button.as_ref(),
        handles.accept_button.as_ref(),
    ) {
        let action_buttons: Vec<Rc<dyn RenderCompactibleActionButton>> =
            vec![Rc::new(reject.clone()), Rc::new(accept.clone())];
        config = config.with_interaction_mode(InteractionMode::ActionButtons {
            action_buttons,
            size_switch_threshold: MEDIUM_SIZE_SWITCH_THRESHOLD,
        });
    }

    config.render(app)
}

fn render_body(state: &RunAgentsEditState, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    column.add_child(render_summary(state, appearance));
    column.add_child(render_agents_section(state, app));

    Container::new(column.finish())
        .with_horizontal_padding(16.)
        .with_vertical_padding(12.)
        .with_background_color(theme.background().into_solid())
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
        .finish()
}

fn render_summary(state: &RunAgentsEditState, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let summary = if state.summary.trim().is_empty() {
        format!(
            "Spawn {} agent(s) to address this task.",
            state.agent_run_configs.len()
        )
    } else {
        state.summary.clone()
    };
    let summary_text = Text::new(
        summary,
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(blended_colors::text_main(theme, theme.background()))
    .with_selectable(true)
    .finish();

    Container::new(summary_text)
        .with_margin_bottom(12.)
        .finish()
}

fn render_agents_section(state: &RunAgentsEditState, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let label = Text::new(
        format!("Agents ({})", state.agent_run_configs.len()),
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 1.,
    )
    .with_color(blended_colors::text_disabled(theme, theme.background()))
    .finish();

    let pills_row = Wrap::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(4.)
        .with_run_spacing(4.)
        .with_children(
            state
                .agent_run_configs
                .iter()
                .map(|cfg| render_static_agent_pill(&cfg.name, app)),
        )
        .finish();

    Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(Container::new(label).with_margin_bottom(6.).finish())
        .with_child(pills_row)
        .finish()
}

fn render_terminal_state(
    result: &RunAgentsResult,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let (label, kind) = format_terminal_state(result);
    render_status_only_card(label, appearance, kind, app)
}

pub(crate) fn format_terminal_state(result: &RunAgentsResult) -> (String, StatusKind) {
    match result {
        RunAgentsResult::Launched { agents, .. } => {
            let total = agents.len();
            let launched = agents
                .iter()
                .filter(|a| matches!(a.kind, RunAgentsAgentOutcomeKind::Launched { .. }))
                .count();
            if launched == total {
                let label = if total == 1 {
                    "Spawned 1 agent".to_string()
                } else {
                    format!("Spawned {total} agents")
                };
                (label, StatusKind::Success)
            } else if launched == 0 {
                // Every child failed to launch: surface a terminal failure
                // rather than the in-progress-looking mixed state.
                let label = if total == 1 {
                    "Failed to spawn agent".to_string()
                } else {
                    format!("Failed to spawn {total} agents")
                };
                (label, StatusKind::Failure)
            } else {
                (
                    format!("Spawned {launched} of {total} agents"),
                    StatusKind::Mixed,
                )
            }
        }
        RunAgentsResult::Denied { reason } => {
            let body = if reason.is_empty() {
                "Orchestration is currently disabled. Re-enable on the plan card to launch."
                    .to_string()
            } else {
                format!(
                    "Orchestration is currently disabled. Re-enable on the plan card to launch. ({reason})"
                )
            };
            (body, StatusKind::Cancelled)
        }
        RunAgentsResult::Failure { error } => {
            let label = if error.is_empty() {
                "Failed to start orchestration".to_string()
            } else {
                format!("Failed to start orchestration: {error}")
            };
            (label, StatusKind::Failure)
        }
        RunAgentsResult::Cancelled => ("Spawn agents cancelled".to_string(), StatusKind::Cancelled),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum StatusKind {
    Spawning,
    Success,
    Mixed,
    Failure,
    Cancelled,
}

fn render_spawning_card(
    snapshot: &RunAgentsSpawningSnapshot,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let total = snapshot.agent_count;
    let label = if total == 1 {
        "Spawning 1 agent\u{2026}".to_string()
    } else {
        format!("Spawning {total} agents\u{2026}")
    };
    render_status_only_card(label, appearance, StatusKind::Spawning, app)
}

fn render_status_only_card(
    label: String,
    appearance: &Appearance,
    kind: StatusKind,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let icon = match kind {
        StatusKind::Spawning => icons::yellow_running_icon(appearance).finish(),
        // Partial success is terminal, so use a static warning glyph rather
        // than the in-progress-looking running circle.
        StatusKind::Mixed => inline_action_icons::warning_icon(appearance).finish(),
        StatusKind::Success => inline_action_icons::green_check_icon(appearance).finish(),
        StatusKind::Failure => inline_action_icons::red_x_icon(appearance).finish(),
        StatusKind::Cancelled => inline_action_icons::cancelled_icon(appearance).finish(),
    };
    let row = render_requested_action_row_for_text(
        label.into(),
        appearance.ui_font_family(),
        Some(icon),
        None,
        false,
        false,
        app,
    );
    Container::new(row)
        .with_background_color(blended_colors::neutral_2(theme))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .finish()
        .with_agent_output_item_spacing(app)
        .finish()
}

fn render_editor(
    state: &RunAgentsEditState,
    handles: &RunAgentsCardHandles,
    app: &AppContext,
) -> Box<dyn Element> {
    use warpui::elements::ConstrainedBox;
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    let divider = Container::new(
        ConstrainedBox::new(Empty::new().finish())
            .with_height(1.)
            .finish(),
    )
    .with_background_color(theme.surface_2().into_solid())
    .finish();
    column.add_child(divider);

    column.add_child(
        Container::new(oc::render_mode_toggle(
            state.orch.execution_mode.is_remote(),
            &handles.pickers,
            appearance,
            None,
            false,
        ))
        .with_margin_top(12.)
        .finish(),
    );
    column.add_child(oc::render_picker_row(
        &state.orch,
        &handles.pickers,
        appearance,
    ));

    if let Some(reason) = oc::accept_disabled_reason_with_auth(&state.orch, app) {
        column.add_child(oc::render_validation_error(
            reason,
            theme.ui_error_color(),
            appearance,
        ));
    } else if let Some(message) =
        oc::empty_env_recommendation_message(&state.orch.execution_mode, app)
    {
        column.add_child(oc::render_validation_error(
            message,
            theme.ui_warning_color(),
            appearance,
        ));
    }

    Container::new(column.finish())
        .with_horizontal_padding(16.)
        .with_padding_bottom(12.)
        .with_background_color(theme.background().into_solid())
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
        .finish()
}

#[cfg(test)]
#[path = "run_agents_card_view_tests.rs"]
mod tests;
