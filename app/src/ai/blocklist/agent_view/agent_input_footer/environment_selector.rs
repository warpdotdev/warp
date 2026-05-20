use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use settings::Setting;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::Fill;
use warpui::{
    elements::{
        ChildAnchor, ChildView, ConstrainedBox, OffsetPositioning, ParentAnchor, ParentElement,
        ParentOffsetBounds, Stack,
    },
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use std::sync::Arc;

use crate::{
    ai::ambient_agents::telemetry::CloudAgentTelemetryEvent,
    ai::{
        cloud_agent_settings::CloudAgentSettings, cloud_environments::CloudAmbientAgentEnvironment,
    },
    appearance::Appearance,
    cloud_object::model::{generic_string_model::StringModel, persistence::CloudModel},
    context_chips::display_menu::{
        ChipMenuType, DisplayChipMenu, FixedFooter, GenericMenuItem, PromptDisplayMenuEvent,
    },
    report_if_error,
    server::ids::SyncId,
    terminal::input::{
        HandoffComposeState, HandoffComposeStateEvent, MenuPositioning, MenuPositioningProvider,
    },
    terminal::view::ambient_agent::AmbientAgentViewModelEvent,
    ui_components::icons::Icon,
    view_components::action_button::{ActionButton, ActionButtonTheme, ButtonSize},
};

use super::{AgentInputButtonTheme, AmbientAgentViewModel};

/// Normalizes ambient-agent and handoff environment selection state behind one API.
#[derive(Clone)]
pub(crate) enum EnvironmentSelectorTarget {
    CloudPane(ModelHandle<AmbientAgentViewModel>),
    Handoff(ModelHandle<HandoffComposeState>),
}

impl EnvironmentSelectorTarget {
    fn selected_environment_id(&self, ctx: &AppContext) -> Option<SyncId> {
        match self {
            Self::CloudPane(model) => model.as_ref(ctx).selected_environment_id().cloned(),
            Self::Handoff(state) => state.as_ref(ctx).selected_environment_id().cloned(),
        }
    }

    fn set_environment_id(
        &self,
        environment_id: Option<SyncId>,
        is_explicit: bool,
        ctx: &mut ViewContext<EnvironmentSelector>,
    ) {
        match self {
            Self::CloudPane(model) => {
                model.update(ctx, |model, ctx| {
                    model.set_environment_id(environment_id, ctx);
                });
            }
            Self::Handoff(state) => {
                state.update(ctx, |state, ctx| {
                    state.set_environment_id(environment_id, is_explicit, ctx);
                });
            }
        }
    }

    fn ensure_default_environment_id(
        &self,
        environment_id: SyncId,
        ctx: &mut ViewContext<EnvironmentSelector>,
    ) {
        match self {
            Self::CloudPane(model) => {
                model.update(ctx, |model, ctx| {
                    model.set_environment_id(Some(environment_id), ctx);
                });
            }
            Self::Handoff(state) => {
                state.update(ctx, |state, ctx| {
                    state.ensure_default_environment_id(environment_id, ctx);
                });
            }
        }
    }

    fn is_configuring(&self, ctx: &AppContext) -> bool {
        match self {
            Self::CloudPane(model) => model.as_ref(ctx).is_configuring_ambient_agent(),
            Self::Handoff(state) => state.as_ref(ctx).is_active(),
        }
    }
}

/// A selector component for choosing an ambient agent environment.
pub struct EnvironmentSelector {
    button: ViewHandle<ActionButton>,
    dropdown: ViewHandle<DisplayChipMenu>,
    is_menu_open: bool,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    target: EnvironmentSelectorTarget,
}

pub enum EnvironmentSelectorEvent {
    MenuVisibilityChanged { open: bool },
    OpenEnvironmentManagementPane,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentSelectorAction {
    ToggleMenu,
}

/// Menu item for an environment in the selector.
#[derive(Debug, Clone)]
struct EnvironmentMenuItem {
    id: SyncId,
    name: String,
    is_selected: bool,
}

const ENV_MENU_CHECK_ICON_SIZE: f32 = 16.;

impl GenericMenuItem for EnvironmentMenuItem {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> String {
        self.name.clone()
    }

    fn icon(&self, _app: &AppContext) -> Option<Icon> {
        None
    }

    fn action_data(&self) -> String {
        self.id.to_string()
    }

    fn right_side_element(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if !self.is_selected {
            return None;
        }
        let theme = Appearance::as_ref(app).theme();
        let color = theme.main_text_color(theme.surface_2()).into_solid();
        Some(
            ConstrainedBox::new(Icon::Check.to_warpui_icon(Fill::Solid(color)).finish())
                .with_width(ENV_MENU_CHECK_ICON_SIZE)
                .with_height(ENV_MENU_CHECK_ICON_SIZE)
                .finish(),
        )
    }
}

/// Menu item for the "New Environment" footer option.
#[derive(Debug, Clone)]
struct NewEnvironmentMenuItem;

impl GenericMenuItem for NewEnvironmentMenuItem {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> String {
        "New environment".to_string()
    }

    fn icon(&self, _app: &AppContext) -> Option<Icon> {
        Some(Icon::Plus)
    }

    fn action_data(&self) -> String {
        "new_environment".to_string()
    }
}

pub(crate) fn sort_environments_by_recency(environments: &mut [CloudAmbientAgentEnvironment]) {
    environments.sort_by(|a, b| {
        // Sort by last-used timestamp descending (most recent first), then by display name ascending
        b.metadata
            .last_task_run_ts
            .cmp(&a.metadata.last_task_run_ts)
            .then_with(|| {
                a.model()
                    .string_model
                    .name
                    .to_lowercase()
                    .cmp(&b.model().string_model.name.to_lowercase())
            })
    });
}

impl EnvironmentSelector {
    pub fn new(
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        target: EnvironmentSelectorTarget,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", AgentInputButtonTheme)
                .with_icon(Icon::Globe4)
                .with_tooltip("Choose an environment")
                .with_size(ButtonSize::AgentInputButton)
                .with_disabled_theme(DisabledTheme)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(EnvironmentSelectorAction::ToggleMenu);
                })
        });

        let dropdown = ctx.add_typed_action_view(move |ctx| {
            DisplayChipMenu::new(
                Vec::<EnvironmentMenuItem>::new(),
                Some(FixedFooter::new(Arc::new(NewEnvironmentMenuItem))),
                ChipMenuType::Environments,
                ctx,
            )
        });

        ctx.subscribe_to_view(&dropdown, |me, _, event, ctx| match event {
            PromptDisplayMenuEvent::MenuAction(generic_event) => {
                // Check if this is the "New Environment" footer action
                if generic_event
                    .action_item
                    .as_any()
                    .downcast_ref::<NewEnvironmentMenuItem>()
                    .is_some()
                {
                    send_telemetry_from_ctx!(
                        CloudAgentTelemetryEvent::OpenedEnvironmentManagementPane,
                        ctx
                    );
                    me.set_menu_visibility(false, ctx);
                    ctx.emit(EnvironmentSelectorEvent::OpenEnvironmentManagementPane);
                    return;
                }

                // Otherwise, it's an environment selection.
                if let Some(env_item) = generic_event
                    .action_item
                    .as_any()
                    .downcast_ref::<EnvironmentMenuItem>()
                {
                    send_telemetry_from_ctx!(
                        CloudAgentTelemetryEvent::EnvironmentSelected {
                            environment_id: env_item.id.into_server(),
                        },
                        ctx
                    );
                    if me.is_configuring(ctx) {
                        me.target.set_environment_id(Some(env_item.id), true, ctx);
                        // Persist the selection to settings for next time.
                        me.save_selected_environment_to_settings(env_item.id, ctx);
                    }
                    me.set_menu_visibility(false, ctx);
                }
            }
            PromptDisplayMenuEvent::CloseMenu => {
                me.set_menu_visibility(false, ctx);
            }
        });

        // Subscribe to CloudModel to refresh when environments are added/removed.
        ctx.subscribe_to_model(&CloudModel::handle(ctx), |me, _, _, ctx| {
            me.ensure_default_selection(ctx);
            me.refresh_menu(ctx);
            me.refresh_button(ctx);
            ctx.notify();
        });

        match &target {
            EnvironmentSelectorTarget::CloudPane(model) => {
                ctx.subscribe_to_model(model, |me, _, event, ctx| {
                    if let AmbientAgentViewModelEvent::EnvironmentSelected = event {
                        me.refresh_menu(ctx);
                    }
                    me.refresh_button(ctx);
                });
            }
            EnvironmentSelectorTarget::Handoff(state) => {
                ctx.subscribe_to_model(state, |me, _, event, ctx| {
                    match event {
                        HandoffComposeStateEvent::ActiveChanged => {
                            if !me.is_configuring(ctx) {
                                me.set_menu_visibility(false, ctx);
                            }
                            me.ensure_default_selection(ctx);
                            me.refresh_menu(ctx);
                        }
                        HandoffComposeStateEvent::EnvironmentSelected => {
                            me.refresh_menu(ctx);
                        }
                    }
                    me.refresh_button(ctx);
                });
            }
        }
        let mut me = Self {
            button,
            dropdown,
            is_menu_open: false,
            menu_positioning_provider,
            target,
        };
        me.refresh_menu(ctx);
        me.refresh_button(ctx);
        me.ensure_default_selection(ctx);
        me
    }

    pub fn is_menu_open(&self) -> bool {
        self.is_menu_open
    }

    pub fn open_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_configuring(ctx) {
            return;
        }
        self.set_menu_visibility(true, ctx);
    }

    fn is_configuring(&self, ctx: &AppContext) -> bool {
        self.target.is_configuring(ctx)
    }

    fn highlight_selected_environment(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(selected_id) = self.target.selected_environment_id(ctx) else {
            return;
        };

        let mut environments = CloudAmbientAgentEnvironment::get_all(ctx);
        sort_environments_by_recency(&mut environments);
        let Some(index) = environments.iter().position(|env| env.id == selected_id) else {
            return;
        };

        self.dropdown.update(ctx, |menu, ctx| {
            menu.select_index(index, ctx);
        });
    }

    pub(super) fn set_menu_visibility(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        if self.is_menu_open == is_open {
            return;
        }

        self.is_menu_open = is_open;
        if is_open {
            send_telemetry_from_ctx!(CloudAgentTelemetryEvent::EnvironmentSelectorOpened, ctx);
            ctx.focus(&self.dropdown);
            self.highlight_selected_environment(ctx);
        }
        ctx.emit(EnvironmentSelectorEvent::MenuVisibilityChanged { open: is_open });
        ctx.notify();
    }

    /// Ensures a default environment is selected if none is currently selected.
    fn ensure_default_selection(&mut self, ctx: &mut ViewContext<Self>) {
        let current_selection = self.target.selected_environment_id(ctx);
        if current_selection.is_some() {
            return;
        }

        // First, try to restore the user's last selected environment from settings.
        if let Some(env_id) = self.get_saved_environment_from_settings(ctx) {
            // Verify the environment still exists.
            if CloudAmbientAgentEnvironment::get_by_id(&env_id, ctx).is_some() {
                self.target.ensure_default_environment_id(env_id, ctx);
                return;
            }
        }

        // Fall back to auto-selecting the most recently used environment.
        let mut environments = CloudAmbientAgentEnvironment::get_all(ctx);
        sort_environments_by_recency(&mut environments);
        if let Some(first_env) = environments.first() {
            self.target.ensure_default_environment_id(first_env.id, ctx);
        }
    }

    /// Retrieves the last selected environment ID from settings.
    fn get_saved_environment_from_settings(&self, ctx: &ViewContext<Self>) -> Option<SyncId> {
        *CloudAgentSettings::as_ref(ctx)
            .last_selected_environment_id
            .value()
    }

    /// Saves the selected environment ID to settings.
    fn save_selected_environment_to_settings(&self, env_id: SyncId, ctx: &mut ViewContext<Self>) {
        CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings
                .last_selected_environment_id
                .set_value(Some(env_id), ctx));
        });
    }

    fn refresh_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let mut environments = CloudAmbientAgentEnvironment::get_all(ctx);
        sort_environments_by_recency(&mut environments);

        let selected_id = self.target.selected_environment_id(ctx);

        let menu_items: Vec<EnvironmentMenuItem> = environments
            .iter()
            .map(|env| {
                let is_selected = selected_id.as_ref() == Some(&env.id);
                EnvironmentMenuItem {
                    id: env.id,
                    name: env.model().string_model.display_name(),
                    is_selected,
                }
            })
            .collect();

        self.dropdown.update(ctx, |menu, ctx| {
            menu.update_menu_items(menu_items, ctx);
        });

        if self.is_menu_open {
            self.highlight_selected_environment(ctx);
        }
    }

    fn refresh_button(&mut self, ctx: &mut ViewContext<Self>) {
        let label = if let Some(id) = self.target.selected_environment_id(ctx) {
            CloudAmbientAgentEnvironment::get_by_id(&id, ctx)
                .map(|env| env.model().string_model.display_name())
                .unwrap_or_else(|| "New environment".to_string())
        } else {
            "New environment".to_string()
        };

        let is_configuring = self.is_configuring(ctx);

        self.button.update(ctx, |button, ctx| {
            button.set_label(label, ctx);
            button.set_tooltip(
                if is_configuring {
                    Some("Choose an environment")
                } else {
                    Some("Agent environment")
                },
                ctx,
            );
            button.set_disabled(!is_configuring, ctx);
        });
    }

    fn get_menu_positioning(&self, app: &AppContext) -> OffsetPositioning {
        match self.menu_positioning_provider.menu_position(app) {
            MenuPositioning::BelowInputBox => OffsetPositioning::offset_from_parent(
                vec2f(0., 4.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::BottomLeft,
                ChildAnchor::TopLeft,
            ),
            MenuPositioning::AboveInputBox => OffsetPositioning::offset_from_parent(
                vec2f(0., -4.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::BottomLeft,
            ),
        }
    }
}

impl TypedActionView for EnvironmentSelector {
    type Action = EnvironmentSelectorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            EnvironmentSelectorAction::ToggleMenu => {
                if self.is_configuring(ctx) {
                    self.set_menu_visibility(!self.is_menu_open, ctx);
                }
            }
        }
    }
}

impl View for EnvironmentSelector {
    fn ui_name() -> &'static str {
        "EnvironmentSelector"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new();
        stack.add_child(ChildView::new(&self.button).finish());

        if self.is_menu_open {
            let menu = ChildView::new(&self.dropdown).finish();
            let positioning = self.get_menu_positioning(app);
            stack.add_positioned_overlay_child(menu, positioning);
        }

        stack.finish()
    }
}

impl Entity for EnvironmentSelector {
    type Event = EnvironmentSelectorEvent;
}

struct DisabledTheme;

impl ActionButtonTheme for DisabledTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        AgentInputButtonTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        _hovered: bool,
        background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        // `background` may be a translucent overlay fill; compute disabled text color against an
        // effective solid background to avoid washing out the label.
        let base_bg = appearance.theme().surface_1();
        let effective_bg = match background {
            Some(overlay) => base_bg.blend(&overlay),
            None => base_bg,
        };

        appearance
            .theme()
            .disabled_text_color(effective_bg)
            .into_solid()
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        AgentInputButtonTheme.border(appearance)
    }

    fn should_opt_out_of_contrast_adjustment(&self) -> bool {
        AgentInputButtonTheme.should_opt_out_of_contrast_adjustment()
    }
}
