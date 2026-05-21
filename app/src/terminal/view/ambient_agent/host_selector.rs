use std::sync::Arc;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use settings::Setting as _;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ChildAnchor, ChildView, OffsetPositioning, ParentAnchor, ParentElement as _,
    ParentOffsetBounds, Stack,
};
use warpui::fonts::{Properties, Weight};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::blocklist::inline_action::orchestration_controls::ORCHESTRATION_WARP_WORKER_HOST;
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::connected_self_hosted_workers::ConnectedSelfHostedWorkersModel;
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields};
use crate::report_if_error;
use crate::terminal::input::{MenuPositioning, MenuPositioningProvider};
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, TooltipAlignment,
};

const HEADER_FONT_SIZE: f32 = 12.;

const ITEM_FONT_SIZE: f32 = 14.;

const MENU_HORIZONTAL_PADDING: f32 = 16.;

const ITEM_VERTICAL_PADDING: f32 = 8.;

const HEADER_VERTICAL_PADDING: f32 = 6.;

const MENU_WIDTH: f32 = 208.;

const BUTTON_TOOLTIP: &str = "Execution host";

const MENU_HEADER_LABEL: &str = "Execution host";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Host {
    Warp,
    SelfHosted { slug: String },
}

impl Host {
    fn display_name(&self) -> &str {
        match self {
            Host::Warp => "Warp",
            Host::SelfHosted { slug } => slug.as_str(),
        }
    }

    /// Returns the value to send as `worker_host` in the config snapshot.
    pub fn worker_host_value(&self) -> Option<String> {
        match self {
            Host::Warp => Some(ORCHESTRATION_WARP_WORKER_HOST.to_string()),
            Host::SelfHosted { slug } => Some(slug.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostSelectorAction {
    ToggleMenu,
    SelectHost(Host),
}

pub enum HostSelectorEvent {
    MenuVisibilityChanged { open: bool },
    HostSelected,
}

pub struct HostSelector {
    button: ViewHandle<ActionButton>,
    menu: ViewHandle<Menu<HostSelectorAction>>,
    is_menu_open: bool,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    selected: Host,
    /// The configured default self-hosted host, if any.
    default_host: Option<Host>,
}

impl HostSelector {
    pub fn new(
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Initial selection. Reading `selected.display_name()` below ensures the
        // field is exercised at construction time (not just written to on
        // `SelectHost`), so it stays out of clippy's `field is never read`
        // warning while still serving as the source of truth for the label.
        let selected = Host::Warp;
        let initial_label = selected.display_name().to_string();

        let button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new(initial_label, NakedHeaderButtonTheme)
                .with_size(ButtonSize::AgentInputButton)
                .with_menu(true)
                .with_tooltip(BUTTON_TOOLTIP)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(HostSelectorAction::ToggleMenu);
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

        ctx.subscribe_to_model(&Appearance::handle(ctx), |me, _, _, ctx| {
            me.refresh_menu(ctx);
        });
        ctx.subscribe_to_model(
            &ConnectedSelfHostedWorkersModel::handle(ctx),
            |me, _, _, ctx| {
                me.refresh_menu(ctx);
            },
        );

        let mut me = Self {
            button,
            menu,
            is_menu_open: false,
            menu_positioning_provider,
            selected,
            default_host: None,
        };
        // Restore the last selected host from settings.
        if let Some(saved_slug) = CloudAgentSettings::as_ref(ctx)
            .last_selected_host
            .value()
            .as_deref()
        {
            let restored = if saved_slug == ORCHESTRATION_WARP_WORKER_HOST {
                Host::Warp
            } else {
                Host::SelfHosted {
                    slug: saved_slug.to_string(),
                }
            };
            me.selected = restored;
            let label = me.selected.display_name().to_string();
            me.button.update(ctx, |button, ctx| {
                button.set_label(label, ctx);
            });
        }
        me.refresh_menu(ctx);
        me
    }

    pub fn is_menu_open(&self) -> bool {
        self.is_menu_open
    }

    pub fn has_default_host(&self) -> bool {
        self.default_host.is_some()
    }

    pub fn selected(&self) -> &Host {
        &self.selected
    }

    pub fn set_default_host(&mut self, slug: String, ctx: &mut ViewContext<Self>) {
        let host = Host::SelfHosted { slug };
        self.default_host = Some(host.clone());

        // If the user has a saved selection, preserve it instead of
        // unconditionally overwriting with the default.
        let saved_slug = CloudAgentSettings::as_ref(ctx)
            .last_selected_host
            .value()
            .clone();
        if saved_slug.is_some() {
            // The constructor already applied the saved selection;
            // just refresh the menu so the new default appears as an option.
            self.refresh_menu(ctx);
            return;
        }

        // No saved preference — use the default host.
        let label = host.display_name().to_string();
        self.selected = host;
        self.button.update(ctx, |button, ctx| {
            button.set_label(label, ctx);
        });
        self.refresh_menu(ctx);
    }

    /// Programmatically opens the host selector popover. No-op if already open.
    pub fn open_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.set_menu_visibility(true, ctx);
    }

    /// Highlights the currently-selected host in the menu. Called when the menu transitions
    /// from closed to open so the user has a clear starting point for arrow-key navigation
    /// instead of an unselected list.
    fn highlight_selected_host(&mut self, ctx: &mut ViewContext<Self>) {
        let selected_action = HostSelectorAction::SelectHost(self.selected.clone());
        self.menu.update(ctx, |menu, ctx| {
            menu.set_selected_by_action(&selected_action, ctx);
        });
    }

    fn set_menu_visibility(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        if self.is_menu_open == is_open {
            return;
        }
        self.is_menu_open = is_open;
        if is_open {
            ConnectedSelfHostedWorkersModel::handle(ctx).update(ctx, |model, ctx| {
                model.refresh(ctx);
            });
            ctx.focus(&self.menu);
            self.highlight_selected_host(ctx);
        }
        ctx.emit(HostSelectorEvent::MenuVisibilityChanged { open: is_open });
        ctx.notify();
    }

    fn refresh_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        let hover_background: Fill = internal_colors::neutral_4(theme).into();
        let header_text_color = theme.disabled_text_color(theme.surface_2()).into_solid();
        let border = Border::all(1.).with_border_fill(theme.outline());
        let items = build_menu_items(
            hover_background,
            header_text_color,
            self.default_host.as_ref(),
            &self.selected,
            ctx,
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

fn build_menu_items(
    hover_background: Fill,
    header_text_color: ColorU,
    default_host: Option<&Host>,
    selected: &Host,
    ctx: &mut ViewContext<HostSelector>,
) -> Vec<MenuItem<HostSelectorAction>> {
    let header = MenuItem::Header {
        fields: MenuItemFields::new(MENU_HEADER_LABEL)
            .with_font_size_override(HEADER_FONT_SIZE)
            .with_override_text_color(header_text_color)
            .with_padding_override(HEADER_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
            .with_no_interaction_on_hover(),
        clickable: false,
        right_side_fields: None,
    };

    let item_for = |host: Host| {
        let label = host.display_name().to_string();
        MenuItem::Item(
            MenuItemFields::new(label)
                .with_font_size_override(ITEM_FONT_SIZE)
                .with_padding_override(ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
                .with_override_hover_background_color(hover_background)
                .with_on_select_action(HostSelectorAction::SelectHost(host)),
        )
    };

    let mut items = vec![header];
    if let Some(host) = default_host {
        items.push(item_for(host.clone()));
    }
    items.push(item_for(Host::Warp));
    let default_slug = match default_host {
        Some(Host::SelfHosted { slug }) => Some(slug.as_str()),
        Some(Host::Warp) | None => None,
    };
    let mut connected_hosts = ConnectedSelfHostedWorkersModel::as_ref(ctx)
        .worker_hosts_excluding(default_slug)
        .into_iter()
        .collect::<Vec<_>>();
    if let Host::SelfHosted { slug } = selected {
        if default_slug != Some(slug.as_str()) {
            connected_hosts.push(slug.clone());
        }
    }
    connected_hosts.sort();
    connected_hosts.dedup();
    for host in connected_hosts {
        items.push(item_for(Host::SelfHosted { slug: host }));
    }
    items
}

impl Entity for HostSelector {
    type Event = HostSelectorEvent;
}

impl TypedActionView for HostSelector {
    type Action = HostSelectorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            HostSelectorAction::ToggleMenu => {
                let new_state = !self.is_menu_open;
                self.set_menu_visibility(new_state, ctx);
            }
            HostSelectorAction::SelectHost(host) => {
                self.selected = host.clone();
                let label = self.selected.display_name().to_string();
                self.button.update(ctx, |button, ctx| {
                    button.set_label(label.clone(), ctx);
                });
                // Persist the selection to settings for next time.
                if let Some(slug) = host.worker_host_value() {
                    CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings.last_selected_host.set_value(Some(slug), ctx));
                    });
                }
                ctx.emit(HostSelectorEvent::HostSelected);
                self.set_menu_visibility(false, ctx);
            }
        }
    }
}

impl View for HostSelector {
    fn ui_name() -> &'static str {
        "HostSelector"
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

#[derive(Clone, Copy)]
pub struct NakedHeaderButtonTheme;

impl ActionButtonTheme for NakedHeaderButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(internal_colors::fg_overlay_1(appearance.theme()))
        } else {
            None
        }
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance
            .theme()
            .sub_text_color(appearance.theme().surface_1())
            .into_solid()
    }

    fn border(&self, _appearance: &Appearance) -> Option<ColorU> {
        None
    }

    fn should_opt_out_of_contrast_adjustment(&self) -> bool {
        true
    }

    fn font_properties(&self) -> Option<Properties> {
        Some(Properties {
            weight: Weight::Semibold,
            ..Default::default()
        })
    }
}
