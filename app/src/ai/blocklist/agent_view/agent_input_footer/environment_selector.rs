use std::sync::Arc;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    ChildAnchor, ChildView, ConstrainedBox, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Stack,
};
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::{AgentInputButtonTheme, AmbientAgentViewModel};
use crate::ai::ambient_agents::telemetry::CloudAgentTelemetryEvent;
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::cloud_environments::{
    CloudAmbientAgentEnvironment, CloudEnvironmentCatalog, CloudSelectorChoice,
    FactorySelectorCatalog, FactorySelectorRow, FactorySelectorState, environment_matches_scope,
};
use crate::appearance::Appearance;
use crate::cloud_object::CloudObjectLookup as _;
use crate::context_chips::display_menu::{
    ChipMenuType, DisplayChipMenu, FixedFooter, GenericMenuItem, PromptDisplayMenuEvent,
};
use crate::server::ids::{ServerId, SyncId};
use crate::terminal::input::{
    HandoffComposeState, HandoffComposeStateEvent, MenuPositioning, MenuPositioningProvider,
};
use crate::terminal::view::ambient_agent::AmbientAgentViewModelEvent;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ActionButtonTheme, ButtonSize};
use crate::workspaces::user_workspaces::{TeamScope, UserWorkspaces, UserWorkspacesEvent};

/// Normalizes ambient-agent and handoff environment selection state behind one API.
#[derive(Clone)]
pub(crate) enum EnvironmentSelectorTarget {
    CloudPane(ModelHandle<AmbientAgentViewModel>),
    Handoff(ModelHandle<HandoffComposeState>),
}

#[cfg(test)]
mod factory_label_tests {
    use super::*;

    #[test]
    fn duplicate_names_use_unique_alias_or_uid_to_disambiguate() {
        let row = |uid: &str, alias: Option<&str>| FactorySelectorRow {
            choice: CloudSelectorChoice::Factory {
                uid: uid.to_owned(),
                environment_uid: SyncId::ServerId(ServerId::from(12)),
                foreman_agent_uid: "foreman".to_owned(),
            },
            name: "Build".to_owned(),
            alias: alias.map(str::to_owned),
        };
        let rows = vec![
            row("factory-a", Some("east")),
            row("factory-b", Some("shared")),
            row("factory-c", Some("shared")),
        ];
        assert_eq!(factory_row_label(&rows[0], &rows), "Build · Factory (east)");
        assert_eq!(
            factory_row_label(&rows[1], &rows),
            "Build · Factory (factory-b)"
        );
        assert_eq!(
            factory_row_label(&rows[2], &rows),
            "Build · Factory (factory-c)"
        );
    }
}

fn factory_row_label(factory: &FactorySelectorRow, rows: &[FactorySelectorRow]) -> String {
    let same_name = rows
        .iter()
        .filter(|other| other.name == factory.name)
        .count()
        > 1;
    if !same_name {
        return format!("{} · Factory", factory.name);
    }
    let alias = factory.alias.as_deref().unwrap_or_default();
    let unique_alias = !alias.is_empty()
        && rows
            .iter()
            .filter(|other| other.name == factory.name && other.alias.as_deref() == Some(alias))
            .count()
            == 1;
    let qualifier = if unique_alias {
        alias.to_owned()
    } else if let CloudSelectorChoice::Factory { uid, .. } = &factory.choice {
        uid.clone()
    } else {
        String::new()
    };
    format!("{} · Factory ({qualifier})", factory.name)
}

impl EnvironmentSelectorTarget {
    fn selected_choice(&self, ctx: &AppContext) -> Option<CloudSelectorChoice> {
        match self {
            Self::CloudPane(model) => model.as_ref(ctx).selected_choice().cloned(),
            Self::Handoff(state) => state.as_ref(ctx).selected_choice().cloned(),
        }
    }

    fn selection_invalidated(&self, ctx: &AppContext) -> bool {
        match self {
            Self::CloudPane(model) => model.as_ref(ctx).selection_invalidated(),
            Self::Handoff(state) => state.as_ref(ctx).selection_invalidated(),
        }
    }

    fn set_choice(&self, choice: CloudSelectorChoice, ctx: &mut ViewContext<EnvironmentSelector>) {
        match self {
            Self::CloudPane(model) => model.update(ctx, |model, ctx| model.set_choice(choice, ctx)),
            Self::Handoff(state) => state.update(ctx, |state, ctx| state.set_choice(choice, ctx)),
        }
    }

    fn invalidate_choice(&self, ctx: &mut ViewContext<EnvironmentSelector>) {
        match self {
            Self::CloudPane(model) => model.update(ctx, |model, ctx| model.invalidate_choice(ctx)),
            Self::Handoff(state) => state.update(ctx, |state, ctx| state.invalidate_choice(ctx)),
        }
    }

    fn selected_environment_id(&self, ctx: &AppContext) -> Option<SyncId> {
        match self {
            Self::CloudPane(model) => model.as_ref(ctx).selected_environment_id().cloned(),
            Self::Handoff(state) => state.as_ref(ctx).selected_environment_id().cloned(),
        }
    }

    fn clear_environment_for_scope_change(&self, ctx: &mut ViewContext<EnvironmentSelector>) {
        match self {
            Self::CloudPane(model) => {
                model.update(ctx, |model, ctx| {
                    model.set_environment_id(None, ctx);
                });
            }
            Self::Handoff(state) => {
                state.update(ctx, |state, ctx| {
                    state.clear_environment_for_scope_change(ctx);
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
    environments: ModelHandle<CloudEnvironmentCatalog>,
    factory_catalog: ModelHandle<FactorySelectorCatalog>,
    last_scope: Option<Option<ServerId>>,
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
    choice: CloudSelectorChoice,
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
        match &self.choice {
            CloudSelectorChoice::Environment(id) => id.to_string(),
            CloudSelectorChoice::Factory { uid, .. } => format!("factory:{uid}"),
        }
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
                    if !me.is_choice_visible(&env_item.choice, ctx) {
                        me.set_menu_visibility(false, ctx);
                        return;
                    }
                    match &env_item.choice {
                        CloudSelectorChoice::Environment(id) => {
                            send_telemetry_from_ctx!(
                                CloudAgentTelemetryEvent::EnvironmentSelected {
                                    environment_id: id.into_server(),
                                },
                                ctx
                            );
                        }
                        CloudSelectorChoice::Factory { uid, .. } => {
                            send_telemetry_from_ctx!(
                                CloudAgentTelemetryEvent::FactorySelected {
                                    factory_uid: uid.clone(),
                                },
                                ctx
                            );
                        }
                    }
                    if me.is_configuring(ctx) {
                        me.target.set_choice(env_item.choice.clone(), ctx);
                        if me.factory_enabled() {
                            let scope = UserWorkspaces::as_ref(ctx).team_context_for_operation(ctx);
                            let preference = env_item.choice.preference();
                            CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                                settings.persist_cloud_selector_preference(&scope, preference, ctx);
                            });
                        }
                        if let CloudSelectorChoice::Environment(id) = &env_item.choice {
                            me.environments.update(ctx, |catalog, ctx| {
                                catalog.persist_selection(*id, ctx);
                            });
                        }
                    }
                    me.set_menu_visibility(false, ctx);
                }
            }
            PromptDisplayMenuEvent::CloseMenu => {
                me.set_menu_visibility(false, ctx);
            }
        });

        let environments = CloudEnvironmentCatalog::handle(ctx);
        let factory_catalog = FactorySelectorCatalog::handle(ctx);
        ctx.subscribe_to_model(&environments, |me, _, _, ctx| {
            me.reconcile_selection_and_refresh(ctx);
        });
        ctx.subscribe_to_model(&factory_catalog, |me, _, _, ctx| {
            me.reconcile_selection_and_refresh(ctx);
        });
        let user_workspaces = UserWorkspaces::handle(ctx);
        ctx.subscribe_to_model(&user_workspaces, |me, _, event, ctx| {
            let affects_this_window = matches!(event, UserWorkspacesEvent::TeamsChanged)
                || matches!(
                    event,
                    UserWorkspacesEvent::WindowTeamChanged { window_id }
                        if *window_id == ctx.window_id()
                );
            if affects_this_window {
                me.reconcile_selection_and_refresh(ctx);
            }
        });

        match &target {
            EnvironmentSelectorTarget::CloudPane(model) => {
                ctx.subscribe_to_model(model, |me, _, event, ctx| {
                    if let AmbientAgentViewModelEvent::EnvironmentSelected = event {
                        me.reconcile_selection_and_refresh(ctx);
                    } else {
                        me.refresh_button(ctx);
                    }
                });
            }
            EnvironmentSelectorTarget::Handoff(state) => {
                ctx.subscribe_to_model(state, |me, _, event, ctx| {
                    if matches!(event, HandoffComposeStateEvent::ActiveChanged)
                        && !me.is_configuring(ctx)
                    {
                        me.set_menu_visibility(false, ctx);
                    }
                    match event {
                        HandoffComposeStateEvent::ActiveChanged
                        | HandoffComposeStateEvent::EnvironmentSelected => {
                            me.reconcile_selection_and_refresh(ctx);
                        }
                    }
                });
            }
        }
        let mut me = Self {
            button,
            dropdown,
            environments,
            factory_catalog,
            last_scope: None,
            is_menu_open: false,
            menu_positioning_provider,
            target,
        };
        me.reconcile_selection_and_refresh(ctx);
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

    fn factory_enabled(&self) -> bool {
        FeatureFlag::CloudModeFactorySelector.is_enabled()
    }

    fn selector_state<'a>(
        &'a self,
        ctx: &'a ViewContext<Self>,
    ) -> Option<&'a FactorySelectorState> {
        let scope = UserWorkspaces::as_ref(ctx).team_context_for_operation(ctx);
        self.factory_catalog.as_ref(ctx).state_for(&scope)
    }

    fn is_choice_visible(&self, choice: &CloudSelectorChoice, ctx: &ViewContext<Self>) -> bool {
        match choice {
            CloudSelectorChoice::Environment(id) => self.is_environment_visible(*id, ctx),
            CloudSelectorChoice::Factory { uid, .. } if self.factory_enabled() => {
                matches!(self.selector_state(ctx), Some(FactorySelectorState::Ready(snapshot)) if
                    snapshot.factory(uid).is_some_and(|row| row.choice == *choice))
            }
            CloudSelectorChoice::Factory { .. } => false,
        }
    }

    fn visible_choices(&self, ctx: &ViewContext<Self>) -> Vec<EnvironmentMenuItem> {
        let selected = self.target.selected_choice(ctx);
        let mut choices = self
            .environments
            .as_ref(ctx)
            .environments()
            .iter()
            .filter(|environment| self.is_environment_visible(environment.id, ctx))
            .map(|environment| EnvironmentMenuItem {
                choice: CloudSelectorChoice::Environment(environment.id),
                name: environment.name.clone(),
                is_selected: false,
            })
            .collect::<Vec<_>>();
        if self.factory_enabled()
            && let Some(FactorySelectorState::Ready(snapshot)) = self.selector_state(ctx)
        {
            for factory in snapshot.factories() {
                choices.push(EnvironmentMenuItem {
                    choice: factory.choice.clone(),
                    name: factory_row_label(factory, snapshot.factories()),
                    is_selected: false,
                });
            }
        }
        for item in &mut choices {
            item.is_selected = selected.as_ref() == Some(&item.choice);
        }
        choices
    }

    fn highlight_selected_environment(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(selected_choice) = self.target.selected_choice(ctx) else {
            return;
        };

        let Some(index) = self
            .visible_choices(ctx)
            .iter()
            .position(|item| item.choice == selected_choice)
        else {
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
        if is_open {
            if self.factory_enabled() {
                let scope = UserWorkspaces::as_ref(ctx).team_context_for_operation(ctx);
                if matches!(
                    self.selector_state(ctx),
                    Some(FactorySelectorState::Failed) | Some(FactorySelectorState::Ready(_))
                ) {
                    self.factory_catalog
                        .update(ctx, |catalog, ctx| catalog.refresh(&scope, ctx));
                }
            }
            self.reconcile_selection_and_refresh(ctx);
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

    fn auto_select_default_environment_if_new_session(&mut self, ctx: &mut ViewContext<Self>) {
        if self.should_auto_select_default_environment(ctx) {
            self.ensure_default_selection(ctx);
        }
    }

    fn reconcile_selection_and_refresh(&mut self, ctx: &mut ViewContext<Self>) {
        if self.factory_enabled() {
            let scope = UserWorkspaces::as_ref(ctx).team_context_for_operation(ctx);
            if self
                .last_scope
                .is_some_and(|previous| previous != scope.team_uid())
                && self.target.selected_choice(ctx).is_some()
                && self.is_configuring(ctx)
            {
                self.target.invalidate_choice(ctx);
            }
            self.last_scope = Some(scope.team_uid());
            self.factory_catalog
                .update(ctx, |catalog, ctx| catalog.ensure_loaded(&scope, ctx));
        }
        self.auto_select_default_environment_if_new_session(ctx);
        self.refresh_menu(ctx);
        self.refresh_button(ctx);
        ctx.notify();
    }

    fn should_auto_select_default_environment(&self, ctx: &AppContext) -> bool {
        match &self.target {
            EnvironmentSelectorTarget::CloudPane(model) => {
                model.as_ref(ctx).is_configuring_ambient_agent()
            }
            EnvironmentSelectorTarget::Handoff(state) => state.as_ref(ctx).is_active(),
        }
    }

    /// Ensures a default environment is selected if none is currently selected.
    fn ensure_default_selection(&mut self, ctx: &mut ViewContext<Self>) {
        if self.factory_enabled() {
            let scope = UserWorkspaces::as_ref(ctx).team_context_for_operation(ctx);
            let Some(FactorySelectorState::Ready(_)) = self.selector_state(ctx) else {
                return;
            };
            if let Some(current) = self.target.selected_choice(ctx) {
                if self.is_choice_visible(&current, ctx) {
                    return;
                }
                self.target.invalidate_choice(ctx);
                return;
            }
            if self.target.selection_invalidated(ctx) {
                return;
            }
            if let Some(choice) = self
                .factory_catalog
                .as_ref(ctx)
                .preferred_choice(&scope, ctx)
            {
                self.target.set_choice(choice, ctx);
            }
            return;
        }
        let current_selection = self.target.selected_environment_id(ctx);
        if let Some(environment_id) = current_selection {
            if self.is_environment_visible(environment_id, ctx) {
                return;
            }
            self.target.clear_environment_for_scope_change(ctx);
        }

        if let Some(environment_id) = self.default_environment_id(ctx) {
            self.target
                .ensure_default_environment_id(environment_id, ctx);
        }
    }

    fn is_environment_visible(&self, environment_id: SyncId, ctx: &ViewContext<Self>) -> bool {
        if self.factory_enabled() {
            return self.visible_environment_ids(ctx).contains(&environment_id);
        }
        let scope = UserWorkspaces::as_ref(ctx).team_context_for_operation(ctx);
        CloudAmbientAgentEnvironment::get_by_id(&environment_id, ctx)
            .is_some_and(|environment| environment_matches_scope(environment, &scope, true))
    }

    fn visible_environment_ids(&self, ctx: &ViewContext<Self>) -> Vec<SyncId> {
        let scope = UserWorkspaces::as_ref(ctx).team_context_for_operation(ctx);
        if self.factory_enabled() {
            return self
                .factory_catalog
                .as_ref(ctx)
                .visible_environments(&scope, ctx)
                .into_iter()
                .map(|env| env.id)
                .collect();
        }
        self.environments
            .as_ref(ctx)
            .environments()
            .iter()
            .filter_map(|environment| {
                CloudAmbientAgentEnvironment::get_by_id(&environment.id, ctx)
                    .filter(|environment| environment_matches_scope(environment, &scope, true))
                    .map(|_| environment.id)
            })
            .collect()
    }

    fn default_environment_id(&self, ctx: &ViewContext<Self>) -> Option<SyncId> {
        let visible_environment_ids = self.visible_environment_ids(ctx);
        self.environments
            .as_ref(ctx)
            .default_environment_id(ctx)
            .filter(|environment_id| visible_environment_ids.contains(environment_id))
            .or_else(|| visible_environment_ids.first().copied())
    }

    fn refresh_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if self.factory_enabled() {
            let items = self.visible_choices(ctx);
            self.dropdown
                .update(ctx, |menu, ctx| menu.update_menu_items(items, ctx));
            if self.is_menu_open {
                self.highlight_selected_environment(ctx);
            }
            return;
        }
        let selected_id = self.target.selected_environment_id(ctx);
        let visible_environment_ids = self.visible_environment_ids(ctx);
        let menu_items = self
            .environments
            .as_ref(ctx)
            .environments()
            .iter()
            .filter(|environment| visible_environment_ids.contains(&environment.id))
            .map(|environment| {
                let is_selected = selected_id == Some(environment.id);
                EnvironmentMenuItem {
                    choice: CloudSelectorChoice::Environment(environment.id),
                    name: environment.name.clone(),
                    is_selected,
                }
            })
            .collect::<Vec<_>>();

        self.dropdown.update(ctx, |menu, ctx| {
            menu.update_menu_items(menu_items, ctx);
        });

        if self.is_menu_open {
            self.highlight_selected_environment(ctx);
        }
    }

    fn refresh_button(&mut self, ctx: &mut ViewContext<Self>) {
        let is_configuring = self.is_configuring(ctx);
        if self.factory_enabled() {
            let label = if !is_configuring {
                self.target
                    .selected_environment_id(ctx)
                    .and_then(|id| self.environments.as_ref(ctx).environment(id))
                    .map(|env| env.name.clone())
                    .unwrap_or_else(|| "Empty environment".to_owned())
            } else if let Some(item) = self
                .visible_choices(ctx)
                .into_iter()
                .find(|item| item.is_selected)
            {
                item.name
            } else if self.target.selection_invalidated(ctx) {
                "Choose a replacement".to_owned()
            } else {
                match self.selector_state(ctx) {
                    Some(FactorySelectorState::Failed) => "Retry loading environments".to_owned(),
                    _ => "Loading environments".to_owned(),
                }
            };
            self.button.update(ctx, |button, ctx| {
                button.set_label(label, ctx);
                button.set_disabled(!is_configuring, ctx);
            });
            return;
        }

        let label = if let Some(id) = self
            .target
            .selected_environment_id(ctx)
            .filter(|id| self.is_environment_visible(*id, ctx))
        {
            self.environments
                .as_ref(ctx)
                .environment(id)
                .map(|environment| environment.name.clone())
                .unwrap_or_else(|| "New environment".to_string())
        } else if is_configuring {
            "New environment".to_string()
        } else {
            "Empty environment".to_string()
        };

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
