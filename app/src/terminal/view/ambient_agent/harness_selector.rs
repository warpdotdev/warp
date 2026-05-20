//! Harness selector: an "options menu" (`ActionButton` + generic `Menu<A>`) shown
//! in a row above the cloud mode input that lets the user switch between the Oz
//! and Claude Code harnesses.

use std::sync::Arc;

use pathfinder_geometry::vector::vec2f;
use warpui::{
    elements::{
        Border, ChildAnchor, ChildView, OffsetPositioning, ParentAnchor, ParentElement as _,
        ParentOffsetBounds, Stack,
    },
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use warp_cli::agent::Harness;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;

use settings::Setting as _;

use crate::ai::blocklist::agent_view::agent_input_footer::AgentInputButtonTheme;
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::harness_display::{brand_color, icon_for};
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields};
use crate::report_if_error;
use crate::terminal::input::{MenuPositioning, MenuPositioningProvider};
use crate::terminal::view::ambient_agent::AmbientAgentViewModel;
use crate::view_components::action_button::{ActionButton, ActionButtonTheme, ButtonSize};

/// Font size for the header row (Figma: 12px).
const HEADER_FONT_SIZE: f32 = 12.;

/// Font size for harness item rows (Figma: 14px).
const ITEM_FONT_SIZE: f32 = 14.;

/// Horizontal padding inside menu rows (Figma: 16px).
const MENU_HORIZONTAL_PADDING: f32 = 16.;

/// Vertical padding on harness rows (Figma: 8px top & bottom).
const ITEM_VERTICAL_PADDING: f32 = 8.;

/// Vertical padding on the header row. Figma uses asymmetric `8px top / 4px
/// bottom`; `MenuItemFields::with_padding_override` only supports a single
/// vertical value, so we approximate with the average (6px).
const HEADER_VERTICAL_PADDING: f32 = 6.;

/// Width of the dropdown panel in logical pixels.
const MENU_WIDTH: f32 = 208.;

/// Leading-icon size for harness item rows in logical pixels. Slightly larger
/// than the default `ui_font_size()` to give the logos more visual presence.
const ITEM_ICON_SIZE: f32 = 16.;

/// Tooltip string for the closed-state button.
const BUTTON_TOOLTIP: &str = "Agent harness";

/// Label rendered at the top of the dropdown.
const MENU_HEADER_LABEL: &str = "Agent harness";

/// Actions dispatched by the [`HarnessSelector`].
#[derive(Clone, Debug, PartialEq)]
pub enum HarnessSelectorAction {
    /// Toggle the visibility of the dropdown menu.
    ToggleMenu,
    /// The user picked a harness from the dropdown.
    SelectHarness(Harness),
}

/// Events emitted by the [`HarnessSelector`].
pub enum HarnessSelectorEvent {
    /// The dropdown visibility changed.
    MenuVisibilityChanged { open: bool },
}

/// A dropdown selector for choosing which execution harness to run cloud agent
/// prompts with.
pub struct HarnessSelector {
    button: ViewHandle<ActionButton>,
    menu: ViewHandle<Menu<HarnessSelectorAction>>,
    is_menu_open: bool,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    ambient_agent_model: ModelHandle<AmbientAgentViewModel>,
}

impl HarnessSelector {
    pub fn new(
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        ambient_agent_model: ModelHandle<AmbientAgentViewModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", AgentInputButtonTheme)
                .with_size(ButtonSize::AgentInputButton)
                .with_menu(true)
                .with_disabled_theme(AgentInputButtonTheme)
                .with_tooltip(BUTTON_TOOLTIP)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(HarnessSelectorAction::ToggleMenu);
                })
        });

        let menu = ctx.add_typed_action_view(|_ctx| {
            Menu::new()
                .with_width(MENU_WIDTH)
                .with_drop_shadow()
                .prevent_interaction_with_other_elements()
        });

        ctx.subscribe_to_view(&menu, |me, _, event, ctx| match event {
            MenuEvent::Close { .. } => {
                me.set_menu_visibility(false, ctx);
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        ctx.subscribe_to_model(&ambient_agent_model, |me, _, _, ctx| {
            me.refresh_button(ctx);
            me.refresh_menu(ctx);
        });

        ctx.subscribe_to_model(&Appearance::handle(ctx), |me, _, _, ctx| {
            me.refresh_menu(ctx);
        });

        ctx.subscribe_to_model(
            &HarnessAvailabilityModel::handle(ctx),
            |me, _, _event, ctx| {
                me.refresh_menu(ctx);
                me.refresh_button(ctx);
            },
        );

        let mut me = Self {
            button,
            menu,
            is_menu_open: false,
            menu_positioning_provider,
            ambient_agent_model,
        };
        // Restore the last selected harness from settings if none is explicitly set.
        if let Some(saved) = CloudAgentSettings::as_ref(ctx)
            .last_selected_harness
            .value()
            .as_deref()
        {
            if let Some(harness) = Harness::from_config_name(saved) {
                if HarnessAvailabilityModel::as_ref(ctx).is_harness_enabled(harness) {
                    me.ambient_agent_model.update(ctx, |model, ctx| {
                        model.set_harness(harness, ctx);
                    });
                }
            }
        }
        me.refresh_button(ctx);
        me.refresh_menu(ctx);
        me
    }

    pub fn is_menu_open(&self) -> bool {
        self.is_menu_open
    }

    /// Programmatically opens the harness selector popover. No-op if already open.
    pub fn open_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_locked_to_oz(ctx) {
            return;
        }
        self.set_menu_visibility(true, ctx);
    }

    /// Highlights the currently-selected harness in the menu. Called when the menu
    /// transitions from closed to open so the user has a clear starting point for arrow-key
    /// navigation instead of an unselected list.
    fn highlight_selected_harness(&mut self, ctx: &mut ViewContext<Self>) {
        let harness = self.ambient_agent_model.as_ref(ctx).selected_harness();
        let selected_action = HarnessSelectorAction::SelectHarness(harness);
        self.menu.update(ctx, |menu, ctx| {
            menu.set_selected_by_action(&selected_action, ctx);
        });
    }

    pub fn set_button_theme<T>(&self, theme: T, ctx: &mut ViewContext<Self>)
    where
        T: ActionButtonTheme + Clone + 'static,
    {
        self.button.update(ctx, |button, ctx| {
            button.set_theme(theme.clone(), ctx);
            button.set_disabled_theme(theme, ctx);
        });
    }

    fn set_menu_visibility(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        let is_open = is_open && !self.is_locked_to_oz(ctx);
        if self.is_menu_open == is_open {
            return;
        }
        self.is_menu_open = is_open;
        if is_open {
            ctx.focus(&self.menu);
            self.highlight_selected_harness(ctx);
        }
        ctx.emit(HarnessSelectorEvent::MenuVisibilityChanged { open: is_open });
        ctx.notify();
    }

    fn is_locked_to_oz(&self, app: &AppContext) -> bool {
        self.ambient_agent_model
            .as_ref(app)
            .is_local_to_cloud_handoff()
    }

    fn refresh_button(&mut self, ctx: &mut ViewContext<Self>) {
        let is_locked_to_oz = self.is_locked_to_oz(ctx);
        let harness = self.ambient_agent_model.as_ref(ctx).selected_harness();
        let label = HarnessAvailabilityModel::as_ref(ctx)
            .display_name_for(harness)
            .to_string();
        let icon = icon_for(harness);
        self.button.update(ctx, |button, ctx| {
            button.set_label(label, ctx);
            button.set_icon(Some(icon), ctx);
            button.set_has_menu(!is_locked_to_oz, ctx);
            button.set_disabled(is_locked_to_oz, ctx);
            button.set_tooltip(
                Some(if is_locked_to_oz {
                    "This conversation is with the Warp Agent, so the cloud handoff will also use Warp"
                } else {
                    BUTTON_TOOLTIP
                }),
                ctx,
            );
        });
        if is_locked_to_oz {
            self.set_menu_visibility(false, ctx);
        }
    }

    fn refresh_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        let hover_background: Fill = internal_colors::neutral_4(theme).into();
        let header_text_color = theme.disabled_text_color(theme.surface_2()).into_solid();
        let disabled_text_color = theme.disabled_text_color(theme.surface_2()).into_solid();
        let border = Border::all(1.).with_border_fill(theme.outline());
        let availability_model = HarnessAvailabilityModel::as_ref(ctx);
        let items = build_menu_items(
            availability_model,
            hover_background,
            header_text_color,
            disabled_text_color,
        );
        self.menu.update(ctx, |menu, ctx| {
            menu.set_border(Some(border));
            menu.set_items(items, ctx);
        });
    }

    fn menu_positioning(&self, app: &AppContext) -> OffsetPositioning {
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

/// Builds the menu items from harness availability data.
fn build_menu_items(
    availability: &HarnessAvailabilityModel,
    hover_background: Fill,
    header_text_color: pathfinder_color::ColorU,
    disabled_text_color: pathfinder_color::ColorU,
) -> Vec<MenuItem<HarnessSelectorAction>> {
    let header = MenuItem::Header {
        fields: MenuItemFields::new(MENU_HEADER_LABEL)
            .with_font_size_override(HEADER_FONT_SIZE)
            .with_override_text_color(header_text_color)
            .with_padding_override(HEADER_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
            .with_no_interaction_on_hover(),
        clickable: false,
        right_side_fields: None,
    };

    let mut items = vec![header];

    for entry in availability.available_harnesses() {
        let harness = entry.harness;
        let is_disabled = !entry.enabled;
        let display_name = availability.display_name_for(harness).to_string();
        let mut fields = MenuItemFields::new(display_name)
            .with_icon(icon_for(harness))
            .with_icon_size_override(ITEM_ICON_SIZE)
            .with_font_size_override(ITEM_FONT_SIZE)
            .with_padding_override(ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
            .with_override_hover_background_color(hover_background)
            .with_on_select_action(HarnessSelectorAction::SelectHarness(harness));
        if let Some(color) = brand_color(harness) {
            fields = fields.with_override_icon_color(Fill::from(color));
        }
        if is_disabled {
            fields = fields
                .with_disabled(true)
                .with_override_text_color(disabled_text_color)
                .with_tooltip("Disabled by your administrator");
        }
        items.push(MenuItem::Item(fields));
    }

    items
}

impl Entity for HarnessSelector {
    type Event = HarnessSelectorEvent;
}

impl TypedActionView for HarnessSelector {
    type Action = HarnessSelectorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            HarnessSelectorAction::ToggleMenu => {
                if self.is_locked_to_oz(ctx) {
                    self.set_menu_visibility(false, ctx);
                    return;
                }
                let new_state = !self.is_menu_open;
                self.set_menu_visibility(new_state, ctx);
            }
            HarnessSelectorAction::SelectHarness(harness) => {
                if self.is_locked_to_oz(ctx) {
                    self.set_menu_visibility(false, ctx);
                    return;
                }
                let harness = *harness;
                self.ambient_agent_model.update(ctx, |model, ctx| {
                    model.set_harness(harness, ctx);
                });
                // Persist the selection to settings for next time.
                CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .last_selected_harness
                        .set_value(Some(harness.config_name().to_string()), ctx));
                });
                self.set_menu_visibility(false, ctx);
            }
        }
    }
}

impl View for HarnessSelector {
    fn ui_name() -> &'static str {
        "HarnessSelector"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new();
        stack.add_child(ChildView::new(&self.button).finish());

        if self.is_menu_open {
            let positioning = self.menu_positioning(app);
            stack.add_positioned_overlay_child(ChildView::new(&self.menu).finish(), positioning);
        }

        stack.finish()
    }
}
