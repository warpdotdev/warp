//! Inline config block rendered on plan cards when the conversation has
//! an active `OrchestrationConfigSnapshot`. Shows a "Use orchestration"
//! toggle, Cloud/Local picker, and run-wide config dropdowns.

use std::collections::HashMap;

use ai::agent::action::RunAgentsExecutionMode;
use ai::agent::orchestration_config::OrchestrationConfigStatus;
use pathfinder_geometry::vector::vec2f;
use warp_cli::agent::Harness;
use warp_core::send_telemetry_from_ctx;
use warpui::elements::{
    ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty,
    Flex, Hoverable, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::inline_action::create_environment_modal::{
    CreateEnvironmentModal, CreateEnvironmentModalEvent,
};
use crate::ai::blocklist::inline_action::host_picker::{HostPicker, HostPickerEvent};
use crate::ai::blocklist::inline_action::orchestration_controls::{
    self as oc, AuthSecretSelection, OrchestrationControlAction, OrchestrationEditState,
    OrchestrationPickerHandles,
};
use crate::ai::blocklist::telemetry::{
    AgentProposedConfigEvent, BlocklistOrchestrationTelemetryEvent, OrchestrationApprovalStatus,
    OrchestrationExecutionModeKind, OrchestrationHarnessKind, PlanConfigApprovalToggledEvent,
};
use crate::ai::blocklist::BlocklistAIHistoryEvent;
use crate::ai::connected_self_hosted_workers::{
    ConnectedSelfHostedWorkersEvent, ConnectedSelfHostedWorkersModel,
};
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::ai::harness_availability::{
    AuthSecretFetchState, HarnessAvailabilityEvent, HarnessAvailabilityModel,
};
use crate::ai::llms::{LLMPreferences, LLMPreferencesEvent};
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::workspace::WorkspaceAction;
use crate::BlocklistAIHistoryModel;

/// True when the mode is remote and `environment_id` is non-empty.
fn env_presence(execution_mode: &RunAgentsExecutionMode) -> bool {
    matches!(
        execution_mode,
        RunAgentsExecutionMode::Remote { environment_id, .. } if !environment_id.is_empty()
    )
}

/// True when the mode is remote and `worker_host` is non-empty.
fn host_presence(execution_mode: &RunAgentsExecutionMode) -> bool {
    matches!(
        execution_mode,
        RunAgentsExecutionMode::Remote { worker_host, .. } if !worker_host.is_empty()
    )
}

const CONFIG_BLOCK_HEADER: &str = "Use orchestration";
const CONFIG_BLOCK_DESCRIPTION: &str =
    "Break this work into coordinated streams with multiple agents.";
const BASE_MODEL_HELPER: &str = "The primary model all agents will use.";

// ── Action type ─────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrchestrationConfigBlockAction {
    ToggleApproval,
    ToggleDetails,
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

impl OrchestrationControlAction for OrchestrationConfigBlockAction {
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

// ── View ────────────────────────────────────────────────────────────

pub struct OrchestrationConfigBlockView {
    conversation_id: AIConversationId,
    plan_id: String,
    edit_state: OrchestrationEditState,
    pickers: OrchestrationPickerHandles<OrchestrationConfigBlockAction>,
    create_environment_modal: ViewHandle<CreateEnvironmentModal>,
    is_approved: bool,
    details_expanded: bool,
    pickers_initialized: bool,
    toggle_switch_state: SwitchStateHandle,
    details_mouse_state: MouseStateHandle,
    /// UI-only per-harness model memory so switching harnesses preserves
    /// the user's previous model selection for each harness.
    saved_model_per_harness: HashMap<String, String>,
    /// Suppresses self-triggered refresh when `apply_field_change`
    /// saves the config and the resulting event re-enters
    /// `refresh_from_model`.
    suppress_refresh: bool,
    /// One-shot guard: cancelling the auto-popped modal must not re-pop.
    /// Reset on harness / execution-mode change.
    has_auto_opened_create_modal: bool,
    /// Required before any auto-pop fires. The plan card is
    /// reconstructed on session restore with `is_approved=true`; gating
    /// on explicit interaction (toggle approval, change harness, switch
    /// mode) suppresses the modal on restore while still firing for
    /// live-session enablement.
    user_has_interacted: bool,
}

impl OrchestrationConfigBlockView {
    pub fn new(
        conversation_id: AIConversationId,
        plan_id: String,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        let snapshot_loaded = history
            .conversation(&conversation_id)
            .and_then(|conv| conv.orchestration_config_for_plan(&plan_id))
            .is_some();
        let (edit_state, is_approved) = history
            .conversation(&conversation_id)
            .and_then(|conv| {
                conv.orchestration_config_for_plan(&plan_id)
                    .map(|(config, status)| {
                        (
                            OrchestrationEditState::from_orchestration_config(config),
                            status.is_approved(),
                        )
                    })
            })
            .unwrap_or_else(|| {
                (
                    OrchestrationEditState::from_run_agents_fields(
                        "auto",
                        "oz",
                        &RunAgentsExecutionMode::Local,
                    ),
                    false,
                )
            });

        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            move |me, _, event, ctx| {
                if let BlocklistAIHistoryEvent::OrchestrationConfigUpdated {
                    conversation_id: cid,
                    ..
                } = event
                {
                    if *cid == me.conversation_id {
                        me.refresh_from_model(ctx);
                    }
                }
            },
        );

        // Repopulate the model picker when available LLMs change (Oz
        // harness only — non-Oz harnesses get their catalog from
        // HarnessAvailabilityModel, not LLMPreferences).
        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |me, _, event, ctx| {
            if let LLMPreferencesEvent::UpdatedAvailableLLMs = event {
                if let Some(handle) = &me.pickers.model_picker {
                    let is_local = !me.edit_state.execution_mode.is_remote();
                    oc::populate_model_picker_for_harness(
                        handle,
                        &me.edit_state.model_id,
                        &me.edit_state.harness_type,
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
                    if me.pickers_initialized {
                        oc::apply_created_auth_secret_if_matches(
                            &mut me.edit_state,
                            *harness,
                            name,
                            ctx,
                        );
                        oc::repopulate_all_pickers(&mut me.edit_state, &me.pickers, ctx);
                    }
                    ctx.notify();
                }
                HarnessAvailabilityEvent::Changed
                | HarnessAvailabilityEvent::AuthSecretsLoaded
                | HarnessAvailabilityEvent::AuthSecretsFetchFailed
                | HarnessAvailabilityEvent::AuthSecretDeleted { .. } => {
                    // Repopulate even on fetch failure to replace "Loading…".
                    // The Deleted event also triggers a refresh so any
                    // already-mounted picker drops the deleted entry from
                    // its menu.
                    if me.pickers_initialized {
                        oc::repopulate_all_pickers(&mut me.edit_state, &me.pickers, ctx);
                    }
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
                    if me.pickers_initialized {
                        oc::repopulate_all_pickers(&mut me.edit_state, &me.pickers, ctx);
                    }
                    ctx.notify();
                }
            },
        );
        let mut view = Self {
            conversation_id,
            plan_id,
            edit_state,
            pickers: OrchestrationPickerHandles::default(),
            create_environment_modal,
            is_approved,
            details_expanded: false,
            pickers_initialized: false,
            toggle_switch_state: SwitchStateHandle::default(),
            details_mouse_state: MouseStateHandle::default(),
            saved_model_per_harness: HashMap::new(),
            suppress_refresh: false,
            has_auto_opened_create_modal: false,
            user_has_interacted: false,
        };
        if view.is_approved {
            view.ensure_pickers(ctx);
            // Skip auto-open here: construction is also the restore code
            // path. The first user interaction (or `arm_for_fresh_dispatch`
            // from a live config update) arms the auto-open instead.
        }
        // Capture the agent's config proposal once per view instance.
        // Gated on `snapshot_loaded` so we don't fire when the view is
        // constructed with placeholder defaults (no real snapshot yet).
        if snapshot_loaded {
            view.emit_agent_proposed_config(ctx);
        }
        view
    }

    /// Marks this view as eligible for auto-pop. Called by
    /// `AIDocumentView`'s lazy-creation path when the agent dispatches a
    /// live config update. Restoration goes through eager construction
    /// and never calls this.
    pub fn arm_for_fresh_dispatch(&mut self, ctx: &mut ViewContext<Self>) {
        self.user_has_interacted = true;
        self.maybe_auto_open_create_modal(ctx);
    }

    /// Auto-pops the create-key modal once per view per harness/mode
    /// change when the harness has no loaded secrets. Also auto-expands
    /// the details panel so the modal opens with context visible behind it.
    fn maybe_auto_open_create_modal(&mut self, ctx: &mut ViewContext<Self>) {
        if self.has_auto_opened_create_modal {
            return;
        }
        // Suppress on session restoration — see `user_has_interacted` doc.
        if !self.user_has_interacted {
            return;
        }
        if !self.is_approved || !self.pickers_initialized {
            return;
        }
        if !oc::should_show_auth_secret_picker(&self.edit_state) {
            return;
        }
        if !matches!(
            self.edit_state.auth_secret_selection,
            AuthSecretSelection::Unset
        ) {
            return;
        }
        let Some(harness) = Harness::parse_orchestration_harness(&self.edit_state.harness_type)
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
        // Show the picker / validation behind the modal.
        if !self.details_expanded {
            self.details_expanded = true;
        }
        ctx.dispatch_typed_action(&WorkspaceAction::OpenCreateAuthSecretModal { harness });
        ctx.notify();
    }

    fn refresh_from_model(&mut self, ctx: &mut ViewContext<Self>) {
        if self.suppress_refresh {
            self.suppress_refresh = false;
            return;
        }
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        if let Some(conv) = history.conversation(&self.conversation_id) {
            if let Some((config, status)) = conv.orchestration_config_for_plan(&self.plan_id) {
                self.edit_state = OrchestrationEditState::from_orchestration_config(config);
                self.is_approved = status.is_approved();
                if self.pickers_initialized {
                    oc::repopulate_all_pickers(&mut self.edit_state, &self.pickers, ctx);
                }
                ctx.notify();
            }
        }
    }

    fn ensure_pickers(&mut self, ctx: &mut ViewContext<Self>) {
        if self.pickers_initialized {
            return;
        }

        let appearance = Appearance::as_ref(ctx);
        let (styles, colors) = oc::picker_styles(appearance);

        // When the agent didn't specify a model, fall back to the
        // conversation's current base model so the picker isn't blank.
        let display_model_id = if self.edit_state.model_id.trim().is_empty() {
            BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&self.conversation_id)
                .and_then(|conv| conv.latest_exchange())
                .map(|ex| ex.model_id.to_string())
                .unwrap_or_default()
        } else {
            self.edit_state.model_id.clone()
        };
        let is_local = !self.edit_state.execution_mode.is_remote();
        let model_handle = oc::new_standard_filterable_picker_dropdown(&styles, ctx);
        model_handle.update(ctx, |d, c| d.set_use_overlay_layer(true, c));
        oc::populate_model_picker_for_harness(
            &model_handle,
            &display_model_id,
            &self.edit_state.harness_type,
            is_local,
            ctx,
        );
        self.pickers.model_picker = Some(model_handle);

        let harness_handle = oc::new_standard_picker_dropdown(&colors, ctx);
        harness_handle.update(ctx, |d, c| d.set_use_overlay_layer(true, c));
        oc::populate_harness_picker(
            &harness_handle,
            &self.edit_state.harness_type,
            is_local,
            ctx,
        );
        self.pickers.harness_picker = Some(harness_handle);

        // When restoring a Remote config with empty host or
        // environment, fill defaults so the pickers aren't blank.
        // If the config is approved, persist the defaults so the
        // stored config used by auto-launch has concrete values.
        let (needs_host, needs_env) = match &self.edit_state.execution_mode {
            RunAgentsExecutionMode::Remote {
                worker_host,
                environment_id,
                ..
            } => (worker_host.is_empty(), environment_id.is_empty()),
            RunAgentsExecutionMode::Local => (false, false),
        };
        let mut filled_defaults = false;
        if needs_host {
            // Prefer the workspace default (or the dev env-var override)
            // over the bare "warp" fallback so self-hosted teams see
            // their default pre-selected. Mirrors the Oz webapp's
            // `HostSelector` initial-selection behavior.
            let default_host = oc::resolve_default_host_slug(ctx)
                .unwrap_or_else(|| oc::ORCHESTRATION_WARP_WORKER_HOST.to_string());
            self.edit_state.set_worker_host(default_host);
            filled_defaults = true;
        }
        if needs_env {
            if let Some(default_env) = oc::resolve_default_environment_id(ctx) {
                self.edit_state.set_environment_id(default_env);
                filled_defaults = true;
            }
        }
        if filled_defaults && self.is_approved {
            self.apply_field_change(ctx);
        }
        let initial_env = match &self.edit_state.execution_mode {
            RunAgentsExecutionMode::Remote { environment_id, .. } => environment_id.as_str(),
            RunAgentsExecutionMode::Local => "",
        };
        let env_handle = oc::create_environment_picker(initial_env, &styles, ctx);
        env_handle.update(ctx, |d, c| d.set_use_overlay_layer(true, c));
        self.pickers.environment_picker = Some(env_handle);

        let initial_host = match &self.edit_state.execution_mode {
            RunAgentsExecutionMode::Remote { worker_host, .. } => worker_host.as_str(),
            RunAgentsExecutionMode::Local => oc::ORCHESTRATION_WARP_WORKER_HOST,
        };
        let host_handle = ctx.add_typed_action_view(HostPicker::new);
        // Paint the open menu in the overlay layer so it doesn't get covered
        // by sibling pickers, matching the other pickers in this view.
        host_handle.update(ctx, |picker, picker_ctx| {
            picker.set_use_overlay_layer(true, picker_ctx);
        });
        oc::populate_host_picker(&host_handle, initial_host, ctx);
        ctx.subscribe_to_view(&host_handle, |_me, _, event, ctx| match event {
            HostPickerEvent::Opened => {
                ConnectedSelfHostedWorkersModel::handle(ctx).update(ctx, |model, ctx| {
                    model.refresh(ctx);
                });
            }
            HostPickerEvent::HostChanged { slug } => {
                ctx.dispatch_typed_action(&OrchestrationConfigBlockAction::WorkerHostChanged {
                    worker_host: slug.clone(),
                });
            }
            HostPickerEvent::Closed => {}
        });
        self.pickers.host_picker = Some(host_handle);

        // Seed the auth secret from persisted per-harness settings before
        // building the picker so the dropdown shows the last selection.
        // The full selection resolver honors an explicit `Inherit` choice
        // (which isn't carried on the OrchestrationConfig wire payload).
        if matches!(
            self.edit_state.auth_secret_selection,
            AuthSecretSelection::Unset
        ) {
            self.edit_state.auth_secret_selection =
                oc::resolve_auth_secret_selection_for_harness(&self.edit_state.harness_type, ctx);
        }
        let auth_secret_handle = oc::new_standard_picker_dropdown(&colors, ctx);
        auth_secret_handle.update(ctx, |d, c| d.set_use_overlay_layer(true, c));
        oc::populate_auth_secret_picker_for_harness(
            &auth_secret_handle,
            &self.edit_state.auth_secret_selection,
            &self.edit_state.harness_type,
            ctx,
        );
        self.pickers.auth_secret_picker = Some(auth_secret_handle);

        self.pickers_initialized = true;
        oc::sync_picker_selections(&self.edit_state, &self.pickers, ctx);
    }

    fn open_create_environment_modal(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(environment_picker) = &self.pickers.environment_picker {
            environment_picker.update(ctx, |dropdown, ctx| dropdown.close(ctx));
        }
        self.create_environment_modal.update(ctx, |modal, ctx| {
            modal.show(ctx);
        });
        ctx.notify();
    }

    fn select_created_environment(&mut self, environment_id: String, ctx: &mut ViewContext<Self>) {
        self.edit_state.set_environment_id(environment_id.clone());
        if let Some(environment_picker) = &self.pickers.environment_picker {
            oc::populate_environment_picker(environment_picker, &environment_id, ctx);
        }
        self.apply_field_change(ctx);
        ctx.notify();
    }

    fn apply_field_change(&mut self, ctx: &mut ViewContext<Self>) {
        self.suppress_refresh = true;
        let config = self.edit_state.to_orchestration_config();
        let status = if self.is_approved {
            OrchestrationConfigStatus::Approved
        } else {
            OrchestrationConfigStatus::Disapproved
        };
        let conversation_id = self.conversation_id;
        let plan_id = self.plan_id.clone();
        AIDocumentModel::handle(ctx).update(ctx, |model, ctx| {
            model.set_orchestration_config_for_plan(conversation_id, plan_id, config, status, ctx);
        });
    }
}

impl Entity for OrchestrationConfigBlockView {
    type Event = ();
}

impl View for OrchestrationConfigBlockView {
    fn ui_name() -> &'static str {
        "OrchestrationConfigBlockView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // Header row: "Use orchestration" + pill toggle switch
        let header_label = Text::new(
            CONFIG_BLOCK_HEADER.to_string(),
            appearance.ui_font_family(),
            16.,
        )
        .with_color(blended_colors::text_main(theme, theme.background()))
        .with_style(Properties::default().weight(Weight::Bold))
        .finish();

        let is_on = self.is_approved;
        let ui_builder = appearance.ui_builder();

        let header_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(warpui::elements::Expanded::new(1.0, header_label).finish())
            .with_child(
                ui_builder
                    .switch(self.toggle_switch_state.clone())
                    .check(is_on)
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(OrchestrationConfigBlockAction::ToggleApproval);
                    })
                    .finish(),
            )
            .finish();
        column.add_child(header_row);

        // Description
        let description = Text::new(
            CONFIG_BLOCK_DESCRIPTION.to_string(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(blended_colors::text_main(theme, theme.background()))
        .finish();
        column.add_child(Container::new(description).with_margin_top(8.).finish());

        // "View details" row + expandable controls (only when approved)
        if self.is_approved {
            // Divider
            let divider = Container::new(
                ConstrainedBox::new(Empty::new().finish())
                    .with_height(1.)
                    .finish(),
            )
            .with_background_color(theme.surface_2().into_solid())
            .finish();
            column.add_child(Container::new(divider).with_margin_top(8.).finish());

            // "View details" link row
            let chevron_icon = if self.details_expanded {
                warp_core::ui::Icon::ChevronDown
            } else {
                warp_core::ui::Icon::ChevronRight
            };
            let disabled_text_color = blended_colors::text_disabled(theme, theme.background());
            let details_text = Text::new(
                "View details".to_string(),
                appearance.ui_font_family(),
                appearance.monospace_font_size() + 1.,
            )
            .with_color(disabled_text_color)
            .finish();
            let chevron = ConstrainedBox::new(
                chevron_icon
                    .to_warpui_icon(warp_core::ui::theme::Fill::Solid(disabled_text_color))
                    .finish(),
            )
            .with_width(14.)
            .with_height(14.)
            .finish();
            let details_link = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(2.)
                .with_child(details_text)
                .with_child(chevron)
                .finish();
            let details_link_hoverable =
                Hoverable::new(self.details_mouse_state.clone(), move |_| details_link)
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(OrchestrationConfigBlockAction::ToggleDetails);
                    })
                    .with_cursor(Cursor::PointingHand)
                    .finish();
            let details_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(details_link_hoverable)
                .finish();
            column.add_child(Container::new(details_row).with_margin_top(8.).finish());

            // Expanded controls
            if self.details_expanded {
                // Cloud / Local mode toggle (full width)
                let active_seg_bg =
                    warp_core::ui::theme::color::internal_colors::accent_overlay_2(theme);
                column.add_child(
                    Container::new(oc::render_mode_toggle(
                        self.edit_state.execution_mode.is_remote(),
                        &self.pickers,
                        appearance,
                        Some(active_seg_bg),
                        true,
                    ))
                    .with_margin_top(12.)
                    .finish(),
                );

                // Pickers stacked vertically
                column.add_child(oc::render_picker_row_with_layout(
                    &self.edit_state,
                    &self.pickers,
                    appearance,
                    true,
                ));

                // Helper text
                let helper = Text::new(
                    BASE_MODEL_HELPER.to_string(),
                    appearance.ui_font_family(),
                    appearance.monospace_font_size() - 1.,
                )
                .with_color(blended_colors::text_disabled(theme, theme.background()))
                .finish();
                column.add_child(Container::new(helper).with_margin_top(4.).finish());

                // Validation
                if let Some(reason) = oc::accept_disabled_reason_with_auth(&self.edit_state, app) {
                    column.add_child(oc::render_validation_error(
                        reason,
                        theme.ui_error_color(),
                        appearance,
                    ));
                } else if let Some(message) =
                    oc::empty_env_recommendation_message(&self.edit_state.execution_mode, app)
                {
                    column.add_child(oc::render_validation_error(
                        message,
                        theme.ui_warning_color(),
                        appearance,
                    ));
                }
            }
        }

        // Outer container with accent styling per Figma
        let card = Container::new(column.finish())
            .with_uniform_padding(12.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_background(warp_core::ui::theme::color::internal_colors::accent_overlay_1(theme))
            .with_border(warpui::elements::Border::all(1.).with_border_fill(theme.accent()))
            .finish();

        let mut stack = Stack::new().with_child(card);
        if self.create_environment_modal.as_ref(app).is_visible() {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.create_environment_modal).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }
        stack.finish()
    }
}

impl TypedActionView for OrchestrationConfigBlockView {
    type Action = OrchestrationConfigBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            OrchestrationConfigBlockAction::ToggleApproval => {
                self.is_approved = !self.is_approved;
                if self.is_approved && !self.pickers_initialized {
                    self.ensure_pickers(ctx);
                }
                let status = if self.is_approved {
                    OrchestrationApprovalStatus::Approved
                } else {
                    OrchestrationApprovalStatus::Disapproved
                };
                self.emit_plan_config_approval_toggled(status, ctx);
                self.apply_field_change(ctx);
                // First moment the picker exists — arm and evaluate.
                if self.is_approved {
                    self.user_has_interacted = true;
                    self.maybe_auto_open_create_modal(ctx);
                }
                ctx.notify();
            }
            OrchestrationConfigBlockAction::ToggleDetails => {
                self.details_expanded = !self.details_expanded;
                if self.details_expanded && !self.pickers_initialized {
                    self.ensure_pickers(ctx);
                }
                ctx.notify();
            }
            OrchestrationConfigBlockAction::ExecutionModeToggled { is_remote } => {
                let conversation_id = self.conversation_id;
                oc::apply_execution_mode_change(
                    &mut self.edit_state,
                    &self.pickers,
                    *is_remote,
                    |ctx| {
                        BlocklistAIHistoryModel::as_ref(ctx)
                            .conversation(&conversation_id)
                            .and_then(|conv| conv.latest_exchange())
                            .map(|ex| ex.model_id.to_string())
                    },
                    ctx,
                );
                self.apply_field_change(ctx);
                // Local → Cloud can newly reveal the auth picker.
                self.user_has_interacted = true;
                self.has_auto_opened_create_modal = false;
                self.maybe_auto_open_create_modal(ctx);
                ctx.notify();
            }
            OrchestrationConfigBlockAction::ModelChanged { model_id } => {
                self.edit_state.model_id = model_id.clone();
                self.apply_field_change(ctx);
                ctx.notify();
            }
            OrchestrationConfigBlockAction::HarnessChanged { harness_type } => {
                let conversation_id = self.conversation_id;
                oc::apply_harness_change(
                    &mut self.edit_state,
                    &mut self.saved_model_per_harness,
                    &self.pickers,
                    harness_type,
                    |ctx| {
                        BlocklistAIHistoryModel::as_ref(ctx)
                            .conversation(&conversation_id)
                            .and_then(|conv| conv.latest_exchange())
                            .map(|ex| ex.model_id.to_string())
                    },
                    ctx,
                );
                self.apply_field_change(ctx);
                // Per-harness state reset — re-arm for the new harness.
                self.user_has_interacted = true;
                self.has_auto_opened_create_modal = false;
                self.maybe_auto_open_create_modal(ctx);
                ctx.notify();
            }
            OrchestrationConfigBlockAction::EnvironmentChanged { environment_id } => {
                self.edit_state.set_environment_id(environment_id.clone());
                oc::persist_environment_selection(environment_id, ctx);
                self.apply_field_change(ctx);
                ctx.notify();
            }
            OrchestrationConfigBlockAction::CreateEnvironmentRequested => {
                self.open_create_environment_modal(ctx);
            }
            OrchestrationConfigBlockAction::WorkerHostChanged { worker_host } => {
                self.edit_state.set_worker_host(worker_host.clone());
                oc::persist_host_selection(worker_host, ctx);
                self.apply_field_change(ctx);
                ctx.notify();
            }
            OrchestrationConfigBlockAction::AuthSecretChanged { auth_secret_name } => {
                // No `apply_field_change`: secrets are user-scoped and
                // persisted side-channel, not baked into `OrchestrationConfig`.
                oc::apply_auth_secret_change(
                    &mut self.edit_state,
                    &self.pickers,
                    auth_secret_name.clone(),
                    ctx,
                );
                ctx.notify();
            }
            OrchestrationConfigBlockAction::CreateNewAuthSecretRequested => {
                oc::apply_create_new_auth_secret_requested(&mut self.edit_state, ctx);
                if let Some(harness) =
                    Harness::parse_orchestration_harness(&self.edit_state.harness_type)
                {
                    ctx.dispatch_typed_action(&WorkspaceAction::OpenCreateAuthSecretModal {
                        harness,
                    });
                }
                ctx.notify();
            }
        }
    }
}

impl OrchestrationConfigBlockView {
    fn emit_plan_config_approval_toggled(
        &self,
        status: OrchestrationApprovalStatus,
        ctx: &mut ViewContext<Self>,
    ) {
        send_telemetry_from_ctx!(
            BlocklistOrchestrationTelemetryEvent::PlanConfigApprovalToggled(
                PlanConfigApprovalToggledEvent {
                    conversation_id: self.conversation_id,
                    plan_id: (!self.plan_id.is_empty()).then(|| self.plan_id.clone()),
                    status,
                    execution_mode: OrchestrationExecutionModeKind::from_run_agents(
                        &self.edit_state.execution_mode,
                    ),
                    harness: OrchestrationHarnessKind::from_str(&self.edit_state.harness_type),
                    has_model: !self.edit_state.model_id.trim().is_empty(),
                    has_environment: env_presence(&self.edit_state.execution_mode),
                    has_worker_host: host_presence(&self.edit_state.execution_mode),
                    has_auth_secret: self.edit_state.auth_secret_name().is_some(),
                }
            ),
            ctx
        );
    }

    fn emit_agent_proposed_config(&self, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(
            BlocklistOrchestrationTelemetryEvent::AgentProposedConfig(AgentProposedConfigEvent {
                conversation_id: self.conversation_id,
                plan_id: (!self.plan_id.is_empty()).then(|| self.plan_id.clone()),
                harness: OrchestrationHarnessKind::from_str(&self.edit_state.harness_type),
                execution_mode: OrchestrationExecutionModeKind::from_run_agents(
                    &self.edit_state.execution_mode,
                ),
                has_model: !self.edit_state.model_id.trim().is_empty(),
                has_environment: env_presence(&self.edit_state.execution_mode),
                has_worker_host: host_presence(&self.edit_state.execution_mode),
            }),
            ctx
        );
    }
}
