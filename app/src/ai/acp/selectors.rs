use std::sync::Arc;

use pathfinder_geometry::vector::vec2f;
use warp_cli::agent::Harness;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ChildAnchor, ChildView, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Stack,
};
use warpui::{
    Action, AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use super::harness_picker::{
    LocalAcpHarnessModel, LocalAcpHarnessModelEvent, LocalAcpModelDiscoveryStatus,
};
use super::{registry};
use crate::ai::blocklist::agent_view::agent_input_footer::AgentInputButtonTheme;
use crate::ai::harness_display;
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields, MenuVariant};
use crate::terminal::input::{MenuPositioning, MenuPositioningProvider};
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize};

const ITEM_FONT_SIZE: f32 = 14.;
const ITEM_ICON_SIZE: f32 = 16.;
const ITEM_VERTICAL_PADDING: f32 = 4.;
const MENU_CONTENT_VERTICAL_PADDING: f32 = 4.;
const MENU_HORIZONTAL_PADDING: f32 = 16.;
const MENU_MAX_HEIGHT: f32 = 220.;
const MENU_WIDTH: f32 = 260.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LocalAcpHarnessSelectorAction {
    ToggleMenu,
    SelectHarness(Harness),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LocalAcpModelSelectorAction {
    ToggleMenu,
    SelectDefault,
    SelectModel(String),
}

pub(crate) enum LocalAcpSelectorEvent {
    MenuVisibilityChanged { open: bool },
}

pub(crate) struct LocalAcpHarnessSelector {
    button: ViewHandle<ActionButton>,
    menu: ViewHandle<Menu<LocalAcpHarnessSelectorAction>>,
    is_menu_open: bool,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    state: ModelHandle<LocalAcpHarnessModel>,
}

impl LocalAcpHarnessSelector {
    pub(crate) fn new(
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", AgentInputButtonTheme)
                .with_size(ButtonSize::AgentInputButton)
                .with_tooltip("Choose ACP agent")
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(LocalAcpHarnessSelectorAction::ToggleMenu);
                })
        });

        let menu = ctx.add_typed_action_view(|_ctx| selector_menu());
        ctx.subscribe_to_view(&menu, |me, _, event, ctx| match event {
            MenuEvent::Close { .. } => me.set_menu_visibility(false, ctx),
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        let state = LocalAcpHarnessModel::handle(ctx);
        ctx.subscribe_to_model(&state, |me, _, event, ctx| match event {
            LocalAcpHarnessModelEvent::SelectionChanged => {
                me.refresh_button(ctx);
                me.refresh_menu(ctx);
            }
        });

        let mut me = Self {
            button,
            menu,
            is_menu_open: false,
            menu_positioning_provider,
            state,
        };
        me.refresh_button(ctx);
        me.refresh_menu(ctx);
        me
    }

    pub(crate) fn open_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.set_menu_visibility(true, ctx);
    }

    pub(crate) fn is_menu_open(&self) -> bool {
        self.is_menu_open
    }

    fn set_menu_visibility(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        if self.is_menu_open == is_open {
            return;
        }
        self.is_menu_open = is_open;
        ctx.emit(LocalAcpSelectorEvent::MenuVisibilityChanged { open: is_open });
        ctx.notify();
    }

    fn refresh_button(&mut self, ctx: &mut ViewContext<Self>) {
        let selected_harness = self.state.as_ref(ctx).selected_harness();
        self.button.update(ctx, |button, ctx| {
            button.set_label(harness_display::display_name(selected_harness), ctx);
            button.set_icon(Some(harness_display::icon_for(selected_harness)), ctx);
        });
    }

    fn refresh_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let hover_background = hover_background(ctx);
        let selected_harness = self.state.as_ref(ctx).selected_harness();
        let items: Vec<MenuItem<LocalAcpHarnessSelectorAction>> = registry::agent_specs()
            .iter()
            .map(|spec| {
                MenuItem::Item(
                    MenuItemFields::new(harness_display::display_name(spec.harness))
                        .with_icon(harness_display::icon_for(spec.harness))
                        .with_icon_size_override(ITEM_ICON_SIZE)
                        .with_font_size_override(ITEM_FONT_SIZE)
                        .with_padding_override(ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
                        .with_override_hover_background_color(hover_background)
                        .with_on_select_action(LocalAcpHarnessSelectorAction::SelectHarness(
                            spec.harness,
                        )),
                )
            })
            .collect();

        self.menu.update(ctx, |menu, ctx| {
            menu.set_border(Some(menu_border(ctx)));
            menu.set_items(items, ctx);
            menu.set_selected_by_action(
                &LocalAcpHarnessSelectorAction::SelectHarness(selected_harness),
                ctx,
            );
        });
    }

    fn menu_positioning(&self, app: &AppContext) -> OffsetPositioning {
        selector_menu_positioning(&self.menu_positioning_provider, app)
    }
}

impl Entity for LocalAcpHarnessSelector {
    type Event = LocalAcpSelectorEvent;
}

impl TypedActionView for LocalAcpHarnessSelector {
    type Action = LocalAcpHarnessSelectorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            LocalAcpHarnessSelectorAction::ToggleMenu => {
                self.set_menu_visibility(!self.is_menu_open, ctx);
            }
            LocalAcpHarnessSelectorAction::SelectHarness(harness) => {
                self.state
                    .update(ctx, |state, ctx| state.select_harness(*harness, ctx));
                self.set_menu_visibility(false, ctx);
            }
        }
    }
}

impl View for LocalAcpHarnessSelector {
    fn ui_name() -> &'static str {
        "LocalAcpHarnessSelector"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new();
        stack.add_child(ChildView::new(&self.button).finish());
        if self.is_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.menu).finish(),
                self.menu_positioning(app),
            );
        }
        stack.finish()
    }
}

pub(crate) struct LocalAcpModelSelector {
    button: ViewHandle<ActionButton>,
    menu: ViewHandle<Menu<LocalAcpModelSelectorAction>>,
    is_menu_open: bool,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    state: ModelHandle<LocalAcpHarnessModel>,
}

impl LocalAcpModelSelector {
    pub(crate) fn new(
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", AgentInputButtonTheme)
                .with_size(ButtonSize::AgentInputButton)
                .with_tooltip("Choose ACP model")
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(LocalAcpModelSelectorAction::ToggleMenu);
                })
        });

        let menu = ctx.add_typed_action_view(|_ctx| selector_menu());
        ctx.subscribe_to_view(&menu, |me, _, event, ctx| match event {
            MenuEvent::Close { .. } => me.set_menu_visibility(false, ctx),
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        let state = LocalAcpHarnessModel::handle(ctx);
        ctx.subscribe_to_model(&state, |me, _, event, ctx| match event {
            LocalAcpHarnessModelEvent::SelectionChanged => {
                me.refresh_button(ctx);
                me.refresh_menu(ctx);
            }
        });

        let mut me = Self {
            button,
            menu,
            is_menu_open: false,
            menu_positioning_provider,
            state,
        };
        me.refresh_button(ctx);
        me.refresh_menu(ctx);
        me
    }

    pub(crate) fn open_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.set_menu_visibility(true, ctx);
    }

    pub(crate) fn is_menu_open(&self) -> bool {
        self.is_menu_open
    }

    fn set_menu_visibility(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        if self.is_menu_open == is_open {
            return;
        }
        self.is_menu_open = is_open;
        ctx.emit(LocalAcpSelectorEvent::MenuVisibilityChanged { open: is_open });
        ctx.notify();
    }

    fn refresh_button(&mut self, ctx: &mut ViewContext<Self>) {
        let label = self.state.as_ref(ctx).selected_model_label();
        self.button.update(ctx, |button, ctx| {
            button.set_label(label, ctx);
            button.set_icon(Some(Icon::AgentMode), ctx);
        });
    }

    fn refresh_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let hover_background = hover_background(ctx);
        let selected_model = self.state.as_ref(ctx).selected_model_id_owned();
        let selected_harness = self.state.as_ref(ctx).selected_harness();
        let discovery_status = self
            .state
            .as_ref(ctx)
            .model_discovery_status(selected_harness);

        let mut items = vec![MenuItem::Item(
            MenuItemFields::new("default")
                .with_icon(Icon::AgentMode)
                .with_icon_size_override(ITEM_ICON_SIZE)
                .with_font_size_override(ITEM_FONT_SIZE)
                .with_padding_override(ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
                .with_override_hover_background_color(hover_background)
                .with_on_select_action(LocalAcpModelSelectorAction::SelectDefault),
        )];

        items.extend(
            self.state
                .as_ref(ctx)
                .models_for_harness(selected_harness)
                .into_iter()
                .map(|model| {
                    MenuItem::Item(
                        MenuItemFields::new(model.name)
                            .with_icon(Icon::AgentMode)
                            .with_icon_size_override(ITEM_ICON_SIZE)
                            .with_font_size_override(ITEM_FONT_SIZE)
                            .with_padding_override(ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
                            .with_override_hover_background_color(hover_background)
                            .with_on_select_action(LocalAcpModelSelectorAction::SelectModel(
                                model.id,
                            )),
                    )
                }),
        );

        if items.len() == 1 {
            let status_label = match discovery_status {
                LocalAcpModelDiscoveryStatus::Idle => Some("Open to discover models".to_string()),
                LocalAcpModelDiscoveryStatus::Loading => Some("Discovering models...".to_string()),
                LocalAcpModelDiscoveryStatus::Loaded => {
                    Some("No model options exposed".to_string())
                }
                LocalAcpModelDiscoveryStatus::Failed(error) => {
                    Some(format!("Model discovery failed: {error}"))
                }
            };
            if let Some(status_label) = status_label {
                items.push(MenuItem::Item(
                    MenuItemFields::new(status_label)
                        .with_icon(Icon::AgentMode)
                        .with_icon_size_override(ITEM_ICON_SIZE)
                        .with_font_size_override(ITEM_FONT_SIZE)
                        .with_padding_override(ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
                        .with_disabled(true),
                ));
            }
        }

        let selected_action = selected_model
            .map(LocalAcpModelSelectorAction::SelectModel)
            .unwrap_or(LocalAcpModelSelectorAction::SelectDefault);

        self.menu.update(ctx, |menu, ctx| {
            menu.set_border(Some(menu_border(ctx)));
            menu.set_items(items, ctx);
            menu.set_selected_by_action(&selected_action, ctx);
        });
    }

    fn menu_positioning(&self, app: &AppContext) -> OffsetPositioning {
        selector_menu_positioning(&self.menu_positioning_provider, app)
    }

    fn ensure_models_discovered(&self, ctx: &mut ViewContext<Self>) {
        let harness = self.state.as_ref(ctx).selected_harness();
        self.state.update(ctx, |state, ctx| {
            state.ensure_models_discovered(harness, ctx);
        });
    }
}

impl Entity for LocalAcpModelSelector {
    type Event = LocalAcpSelectorEvent;
}

impl TypedActionView for LocalAcpModelSelector {
    type Action = LocalAcpModelSelectorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            LocalAcpModelSelectorAction::ToggleMenu => {
                self.ensure_models_discovered(ctx);
                self.set_menu_visibility(!self.is_menu_open, ctx);
            }
            LocalAcpModelSelectorAction::SelectDefault => {
                self.state
                    .update(ctx, |state, ctx| state.select_model_id(None, ctx));
                self.set_menu_visibility(false, ctx);
            }
            LocalAcpModelSelectorAction::SelectModel(model_id) => {
                self.state.update(ctx, |state, ctx| {
                    state.select_model_id(Some(model_id.clone()), ctx)
                });
                self.set_menu_visibility(false, ctx);
            }
        }
    }
}

impl View for LocalAcpModelSelector {
    fn ui_name() -> &'static str {
        "LocalAcpModelSelector"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new();
        stack.add_child(ChildView::new(&self.button).finish());
        if self.is_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.menu).finish(),
                self.menu_positioning(app),
            );
        }
        stack.finish()
    }
}

fn selector_menu<A: Action + Clone + 'static>() -> Menu<A> {
    let mut menu = Menu::new()
        .with_width(MENU_WIDTH)
        .with_drop_shadow()
        .with_menu_variant(MenuVariant::scrollable())
        .prevent_interaction_with_other_elements();
    menu.set_content_padding_overrides(
        Some(MENU_CONTENT_VERTICAL_PADDING),
        Some(MENU_CONTENT_VERTICAL_PADDING),
    );
    menu.set_height(MENU_MAX_HEIGHT);
    menu
}

fn menu_border(app: &AppContext) -> Border {
    Border::all(1.).with_border_color(internal_colors::neutral_4(Appearance::as_ref(app).theme()))
}

fn hover_background(app: &AppContext) -> Fill {
    internal_colors::fg_overlay_2(Appearance::as_ref(app).theme())
}

fn selector_menu_positioning(
    provider: &Arc<dyn MenuPositioningProvider>,
    app: &AppContext,
) -> OffsetPositioning {
    match provider.menu_position(app) {
        MenuPositioning::BelowInputBox => OffsetPositioning::offset_from_parent(
            vec2f(0., 4.),
            ParentOffsetBounds::WindowByPosition,
            ParentAnchor::BottomRight,
            ChildAnchor::TopRight,
        ),
        MenuPositioning::AboveInputBox => OffsetPositioning::offset_from_parent(
            vec2f(0., -4.),
            ParentOffsetBounds::WindowByPosition,
            ParentAnchor::TopRight,
            ChildAnchor::BottomRight,
        ),
    }
}
