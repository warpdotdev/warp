use crate::pricing::{PricingInfoModel, PricingInfoModelEvent};
use crate::terminal::general_settings::GeneralSettings;
use crate::ui_components::blended_colors;
use crate::view_components::{Dropdown, DropdownEvent, DropdownItem, ToastFlavor};
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
use crate::workspaces::workspace::CustomerType;
use asset_macro::bundled_or_fetched_asset;
use itertools::Itertools;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use settings::Setting as _;
use thousands::Separable;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warp_graphql::billing::{AddonCreditsOption, StripeSubscriptionPlan};
use warpui::elements::{
    Align, Border, CacheOption, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, DropShadow, Flex, FormattedTextElement, HighlightedHyperlink, Image,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack,
};
use warpui::fonts::{FamilyId, Weight};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

const BUTTON_DIAMETER: f32 = 20.;
const DROPDOWN_WIDTH: f32 = 160.;
const MODAL_HEIGHT: f32 = 540.;
const MODAL_WIDTH: f32 = 876.;
const LEFT_PANEL_WIDTH: f32 = 333.;
const CORNER_RADIUS: f32 = 20.;
const PANEL_PADDING: f32 = 24.;

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum BuildPlanMigrationModalViewAction {
    SelectReloadDenomination(usize),
    // true => "enabled", false => "disabled"
    EnableAutoReloadToggled(bool),
    GetStartedClicked,
    Close,
    OpenUrl(&'static str),
}

#[derive(Default)]
struct StateHandles {
    close_button: MouseStateHandle,
    upgrade_button: MouseStateHandle,
    auto_reload_checkbox: MouseStateHandle,
}

pub struct BuildPlanMigrationModal {
    state_handles: StateHandles,
    selected_addon_credits_option: usize,
    addon_credits_options: Vec<AddonCreditsOption>,
    is_updating: bool,
    is_dropdown_expanded: bool,
    auto_reload_enabled: bool,
    reload_denominations_dropdown: ViewHandle<Dropdown<BuildPlanMigrationModalViewAction>>,
}

impl BuildPlanMigrationModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &PricingInfoModel::handle(ctx),
            |me, _, event, ctx| match event {
                PricingInfoModelEvent::PricingInfoUpdated => {
                    me.update_addon_credits_options(ctx);
                    ctx.notify();
                }
            },
        );

        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _handle, event, ctx| {
            me.handle_workspaces_event(event, ctx);
        });

        let reload_denominations_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(DROPDOWN_WIDTH);
            dropdown.set_menu_width(DROPDOWN_WIDTH, ctx);
            dropdown
        });

        ctx.subscribe_to_view(
            &reload_denominations_dropdown,
            |me, _, event, ctx| match event {
                DropdownEvent::ToggleExpanded => {
                    me.is_dropdown_expanded = !me.is_dropdown_expanded;
                    ctx.notify();
                }
                DropdownEvent::Close => {
                    me.is_dropdown_expanded = false;
                    ctx.notify();
                }
            },
        );

        let mut me = BuildPlanMigrationModal {
            state_handles: Default::default(),
            selected_addon_credits_option: 0,
            addon_credits_options: Default::default(),
            is_updating: false,
            is_dropdown_expanded: false,
            auto_reload_enabled: false,
            reload_denominations_dropdown,
        };
        me.update_addon_credits_options(ctx);
        me.refresh_addon_credits_settings(ctx);
        me
    }

    fn refresh_addon_credits_settings(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(workspace) = UserWorkspaces::as_ref(ctx).current_workspace() else {
            return;
        };
        let addon_credits_settings = &workspace.settings.addon_credits_settings;
        self.auto_reload_enabled = addon_credits_settings.auto_reload_enabled;
        self.selected_addon_credits_option = addon_credits_settings
            .selected_auto_reload_credit_denomination
            .and_then(|amount| {
                self.addon_credits_options
                    .iter()
                    .find_position(|option| option.credits == amount)
            })
            .map_or(0, |pair| pair.0);
        // Update dropdown to reflect the refreshed selection
        self.reload_denominations_dropdown
            .update(ctx, |dropdown, ctx| {
                dropdown.set_selected_by_index(self.selected_addon_credits_option, ctx);
            });
    }

    fn update_addon_credits_options(&mut self, ctx: &mut ViewContext<Self>) {
        self.addon_credits_options = PricingInfoModel::as_ref(ctx)
            .addon_credits_options()
            .map(|opts| opts.to_vec())
            .unwrap_or_default();
        // Sync the selected denomination after options are updated
        self.sync_selected_denomination(ctx);
        // Populate dropdown after syncing selection so it shows the correct item
        self.populate_reload_denomination_dropdown(ctx);
    }

    fn sync_selected_denomination(&mut self, ctx: &ViewContext<Self>) {
        let Some(workspace) = UserWorkspaces::as_ref(ctx).current_workspace() else {
            return;
        };

        let addon_credits_settings = &workspace.settings.addon_credits_settings;
        // Sync the auto-reload enabled flag
        self.auto_reload_enabled = addon_credits_settings.auto_reload_enabled;

        if let Some(selected_amount) =
            addon_credits_settings.selected_auto_reload_credit_denomination
        {
            // Find the index of the option that matches the selected amount
            if let Some((index, _)) = self
                .addon_credits_options
                .iter()
                .enumerate()
                .find(|(_, option)| option.credits == selected_amount)
            {
                self.selected_addon_credits_option = index;
            }
        }
    }

    fn handle_workspaces_event(
        &mut self,
        event: &UserWorkspacesEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess => {
                if self.is_updating {
                    // Close modal on success when we initiated the update
                    self.is_updating = false;
                    self.update_addon_credits_options(ctx);
                    Self::mark_modal_dismissed(ctx);
                    ctx.emit(BuildPlanMigrationModalEvent::Close);
                } else {
                    // External update - refresh our state to stay in sync
                    self.refresh_addon_credits_settings(ctx);
                    ctx.notify();
                }
            }
            UserWorkspacesEvent::UpdateWorkspaceSettingsRejected(_err) => {
                self.is_updating = false;
                ctx.emit(BuildPlanMigrationModalEvent::ShowToast {
                    message: "Failed to enable auto-reload. Please try updating your settings in Billing & usage.".to_string(),
                    flavor: ToastFlavor::Error,
                });
                ctx.notify();
            }
            _ => {}
        }
    }

    fn mark_modal_dismissed(ctx: &mut ViewContext<Self>) {
        let general_settings = GeneralSettings::handle(ctx);
        general_settings.update(ctx, |settings, ctx| {
            if let Err(e) = settings
                .build_plan_migration_modal_dismissed
                .set_value(true, ctx)
            {
                log::warn!("Failed to set build plan migration modal dismissed setting: {e}");
            }
        });
    }

    fn populate_reload_denomination_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        self.reload_denominations_dropdown
            .update(ctx, |dropdown, ctx| {
                dropdown.set_items(
                    self.addon_credits_options
                        .iter()
                        .enumerate()
                        .map(|(i, option)| {
                            DropdownItem::new(
                                format!(
                                    "${} / {} credits",
                                    option.price_usd_cents / 100,
                                    option.credits.separate_with_commas(),
                                ),
                                BuildPlanMigrationModalViewAction::SelectReloadDenomination(i),
                            )
                        })
                        .collect(),
                    ctx,
                );
                // Set the selected item to match the current selection
                dropdown.set_selected_by_index(self.selected_addon_credits_option, ctx);
            });
        ctx.notify();
    }

    fn render_auto_reload_controls(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let check_color = theme.background().into_solid();

        let auto_reload_enabled = self.auto_reload_enabled;
        let checkbox = appearance
            .ui_builder()
            .checkbox(
                self.state_handles.auto_reload_checkbox.clone(),
                Some(appearance.ui_font_size()),
            )
            .check(auto_reload_enabled)
            .with_style(UiComponentStyles {
                font_color: Some(check_color),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(
                    BuildPlanMigrationModalViewAction::EnableAutoReloadToggled(
                        !auto_reload_enabled,
                    ),
                )
            })
            .finish();

        let label = FormattedTextElement::from_str(
            crate::i18n::tr_static(app, "Auto-reload"),
            appearance.ui_font_family(),
            12.,
        )
        .with_color(blended_colors::text_sub(
            theme,
            blended_colors::neutral_4(theme),
        ))
        .finish();

        let checkbox_row = Flex::row()
            .with_child(checkbox)
            .with_child(label)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();

        let dropdown = if self.auto_reload_enabled {
            ChildView::new(&self.reload_denominations_dropdown).finish()
        } else {
            // Match dropdown height to prevent layout shift (dropdown is typically ~28-32px)
            ConstrainedBox::new(warpui::elements::Empty::new().finish())
                .with_width(DROPDOWN_WIDTH)
                .with_height(28.)
                .finish()
        };

        Flex::row()
            .with_child(
                Container::new(checkbox_row)
                    .with_vertical_margin(8.)
                    .with_margin_right(4.)
                    .finish(),
            )
            .with_child(dropdown)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .finish()
    }

    fn render_get_started_button(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let button_text = if self.is_updating {
            crate::i18n::tr_static(app, "Saving...").to_string()
        } else {
            crate::i18n::tr_static(app, "Get Started").to_string()
        };

        let button_font_color = self.is_updating.then_some(
            appearance
                .theme()
                .disabled_text_color(appearance.theme().surface_3())
                .into(),
        );
        let button_bg_color = self
            .is_updating
            .then_some(appearance.theme().surface_3().into());
        let button_border = self
            .is_updating
            .then_some(ColorU::transparent_black().into());

        let mut button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.state_handles.upgrade_button.clone(),
            )
            .with_style(UiComponentStyles {
                font_size: Some(12.),
                height: Some(28.),
                width: Some(96.),
                font_color: button_font_color,
                background: button_bg_color,
                border_color: button_border,
                ..Default::default()
            })
            .with_centered_text_label(button_text)
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(BuildPlanMigrationModalViewAction::GetStartedClicked)
            });

        if self.is_updating {
            button = button.disable();
        }
        button.finish()
    }

    fn render_right_panel_content(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let title = Self::create_text(
            crate::i18n::tr_static(app, "Use auto-reload to never miss a beat.").to_string(),
            appearance.ui_font_family(),
            16.,
            blended_colors::text_main(theme, blended_colors::neutral_2(theme)),
            Some(Weight::Bold),
        );

        let description = Self::create_text(
            crate::i18n::tr_static(app, "Auto-reload will automatically purchase credits at your selected rate when your account balance reaches 100 credits. Your monthly spend limit is set at your legacy plan's monthly cost and can be updated in Settings > Billing & usage.").to_string(),
            appearance.ui_font_family(),
            14.,
            blended_colors::text_sub(theme, blended_colors::neutral_4(theme)),
            None,
        );

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(
                Shrinkable::new(
                    1.,
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(Container::new(title).with_margin_bottom(12.).finish())
                        .with_child(Container::new(description).with_margin_bottom(16.).finish())
                        .finish(),
                )
                .finish(),
            )
            .with_child(
                Container::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_child(self.render_auto_reload_controls(appearance, app))
                        .with_child(self.render_get_started_button(appearance, app))
                        .finish(),
                )
                .finish(),
            )
            .finish()
    }

    fn render_right_panel(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let image = ConstrainedBox::new(
            Image::new(
                bundled_or_fetched_asset!("png/build_spiral.png"),
                CacheOption::BySize,
            )
            .cover()
            .with_corner_radius(CornerRadius::with_top_right(Radius::Pixels(CORNER_RADIUS)))
            .finish(),
        )
        .with_width(543.)
        .with_height(335.)
        .finish();

        let content_panel = Shrinkable::new(
            1.,
            Container::new(self.render_right_panel_content(appearance, app))
                .with_uniform_padding(PANEL_PADDING)
                .with_background(blended_colors::neutral_2(theme))
                .with_border(Border::left(1.).with_border_color(blended_colors::neutral_4(theme)))
                .with_corner_radius(CornerRadius::with_bottom_right(Radius::Pixels(
                    CORNER_RADIUS,
                )))
                .finish(),
        )
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(image)
            .with_child(content_panel)
            .finish()
    }
}

impl Entity for BuildPlanMigrationModal {
    type Event = BuildPlanMigrationModalEvent;
}

const BULLET_WIDTH: f32 = 12.;

impl BuildPlanMigrationModal {
    fn create_bullet_item(
        text: String,
        font_family: FamilyId,
        font_size: f32,
        color: ColorU,
    ) -> Box<dyn Element> {
        let bullet = FormattedTextElement::from_str("•", font_family, font_size)
            .with_color(color)
            .with_weight(Weight::Bold)
            .finish();

        let text_content = FormattedTextElement::from_str(text, font_family, font_size)
            .with_color(color)
            .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                ConstrainedBox::new(bullet)
                    .with_width(BULLET_WIDTH)
                    .finish(),
            )
            .with_child(Shrinkable::new(1., text_content).finish())
            .finish()
    }
}

impl BuildPlanMigrationModal {
    fn create_text(
        text: String,
        font_family: FamilyId,
        font_size: f32,
        color: ColorU,
        weight: Option<Weight>,
    ) -> Box<dyn Element> {
        let mut element =
            FormattedTextElement::from_str(text, font_family, font_size).with_color(color);
        if let Some(weight) = weight {
            element = element.with_weight(weight);
        }
        element.finish()
    }

    fn render_left_panel(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        let text_color = blended_colors::text_sub(theme, blended_colors::neutral_2(theme));

        // Check if any service agreement has type Business
        let is_business = UserWorkspaces::as_ref(app)
            .current_workspace()
            .map(|workspace| workspace.billing_metadata.customer_type == CustomerType::Business)
            .unwrap_or(false);

        let plan_pricing = PricingInfoModel::as_ref(app).plan_pricing(if is_business {
            &StripeSubscriptionPlan::BuildBusiness
        } else {
            &StripeSubscriptionPlan::Build
        });
        let base_credits_limit = plan_pricing.and_then(|p| p.request_limit).unwrap_or(1500);
        // (monthly price cents, monthly price cents for annual)
        let base_plan_prices = plan_pricing
            .map(|p| {
                (
                    p.monthly_plan_price_per_month_usd_cents,
                    p.yearly_plan_price_per_month_usd_cents,
                )
            })
            .unwrap_or((2000, 1800));

        let title_text = if is_business {
            "Welcome to the New Business Plan"
        } else {
            "Welcome to Warp Build"
        };

        let title = Self::create_text(
            title_text.to_string(),
            font_family,
            24.,
            blended_colors::text_main(theme, blended_colors::neutral_2(theme)),
            Some(Weight::Bold),
        );

        let intro_text = if is_business {
            "Your workspace has been updated to the new Warp Business Plan as the legacy Business plan is sunset."
        } else {
            "Your workspace has been updated to the Warp Build Plan as the legacy Pro, Turbo, and Lightspeed plans are sunset."
        };

        let intro = Self::create_text(intro_text.to_string(), font_family, 14., text_color, None);

        let pricing_header = Self::create_text(
            if is_business {
                "The new Business plan is a primarily usage-based plan, starting at:"
            } else {
                "Warp Build is a primarily usage-based plan, starting at:"
            }
            .to_string(),
            font_family,
            14.,
            text_color,
            None,
        );

        let price_monthly = Self::create_bullet_item(
            format!("${} per user per month", base_plan_prices.0 / 100),
            font_family,
            14.,
            text_color,
        );

        let price_annual = Self::create_bullet_item(
            format!(
                "${} per user per month for annual plans",
                base_plan_prices.1 / 100
            ),
            font_family,
            14.,
            text_color,
        );

        let features_header = Self::create_text(
            if is_business {
                "The new Business plan comes with:"
            } else {
                "Build comes with:"
            }
            .to_string(),
            font_family,
            14.,
            text_color,
            None,
        );

        let base_credits = Self::create_bullet_item(
            format!(
                "{} base credits per month",
                base_credits_limit.separate_with_commas()
            ),
            font_family,
            14.,
            text_color,
        );

        let reload_credits = Self::create_bullet_item(
            "Access to Reload credits and volume-based discounts".to_string(),
            font_family,
            14.,
            text_color,
        );

        let byok = Self::create_bullet_item(
            "Bring your own API key".to_string(),
            font_family,
            14.,
            text_color,
        );

        let mut features_list = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(base_credits)
            .with_child(reload_credits)
            .with_child(byok);

        if is_business {
            let sso = Self::create_bullet_item(
                "SAML-based SSO".to_string(),
                font_family,
                14.,
                text_color,
            );
            features_list.add_child(sso);

            let zdr = Self::create_bullet_item(
                "Automatically enforced team-wide Zero Data Retention".to_string(),
                font_family,
                14.,
                text_color,
            );
            features_list.add_child(zdr);
        }

        let and_more =
            Self::create_bullet_item("And more...".to_string(), font_family, 14., text_color);
        features_list.add_child(and_more);

        let learn_more_fragments = vec![
            FormattedTextFragment::plain_text("Learn more on our "),
            FormattedTextFragment::hyperlink("pricing page", "https://www.warp.dev/pricing"),
            FormattedTextFragment::plain_text("."),
        ];
        let learn_more = Container::new(
            FormattedTextElement::new(
                FormattedText::new([FormattedTextLine::Line(learn_more_fragments)]),
                14.,
                font_family,
                font_family,
                text_color,
                HighlightedHyperlink::default(),
            )
            .with_hyperlink_font_color(appearance.theme().accent().into_solid())
            .register_default_click_handlers(|_url, ctx, _| {
                ctx.dispatch_typed_action(BuildPlanMigrationModalViewAction::OpenUrl(
                    "https://www.warp.dev/pricing",
                ));
            })
            .finish(),
        )
        .finish();

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(Container::new(title).with_margin_bottom(12.).finish())
                .with_child(Container::new(intro).with_margin_bottom(16.).finish())
                .with_child(
                    Container::new(pricing_header)
                        .with_margin_bottom(8.)
                        .finish(),
                )
                .with_child(price_monthly)
                .with_child(
                    Container::new(price_annual)
                        .with_margin_bottom(16.)
                        .finish(),
                )
                .with_child(
                    Container::new(features_header)
                        .with_margin_bottom(8.)
                        .finish(),
                )
                .with_child(
                    Container::new(features_list.finish())
                        .with_margin_bottom(16.)
                        .finish(),
                )
                .with_child(Container::new(learn_more).finish())
                .finish(),
        )
        .with_background_color(blended_colors::neutral_1(theme))
        .with_corner_radius(CornerRadius::with_left(Radius::Pixels(CORNER_RADIUS)))
        .with_uniform_padding(PANEL_PADDING)
        .finish()
    }

    fn render_close_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .close_button(BUTTON_DIAMETER, self.state_handles.close_button.clone())
            .with_style(UiComponentStyles {
                font_color: Some(ColorU::white()),
                ..Default::default()
            })
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(BuildPlanMigrationModalViewAction::Close)
            })
            .finish()
    }
}

impl View for BuildPlanMigrationModal {
    fn ui_name() -> &'static str {
        "BuildPlanMigrationModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let left_panel = self.render_left_panel(appearance, app);
        let close_button = self.render_close_button(appearance);

        let right_panel_width = MODAL_WIDTH - LEFT_PANEL_WIDTH;
        let mut modal = Stack::new();
        modal.add_child(
            Container::new(
                ConstrainedBox::new(
                    Flex::row()
                        .with_child(
                            ConstrainedBox::new(left_panel)
                                .with_width(LEFT_PANEL_WIDTH)
                                .with_height(MODAL_HEIGHT)
                                .finish(),
                        )
                        .with_child(
                            ConstrainedBox::new(self.render_right_panel(app))
                                .with_width(right_panel_width)
                                .with_height(MODAL_HEIGHT)
                                .finish(),
                        )
                        .finish(),
                )
                .with_width(MODAL_WIDTH)
                .with_height(MODAL_HEIGHT)
                .finish(),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_drop_shadow(DropShadow::default())
            .finish(),
        );
        modal.add_positioned_child(
            close_button,
            OffsetPositioning::offset_from_parent(
                vec2f(-14., 14.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );

        // Stack needed so that modal can get bounds information,
        // specifically to ensure no overlap with the window's traffic lights
        let mut stack = Stack::new();
        stack.add_positioned_child(
            modal.finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );

        Container::new(Align::new(stack.finish()).finish())
            .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
            .finish()
    }
}

impl TypedActionView for BuildPlanMigrationModal {
    type Action = BuildPlanMigrationModalViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BuildPlanMigrationModalViewAction::SelectReloadDenomination(index) => {
                self.selected_addon_credits_option = *index;
                ctx.notify();
            }
            BuildPlanMigrationModalViewAction::GetStartedClicked => {
                // Get current team UID and workspace data
                let workspaces = UserWorkspaces::as_ref(ctx);
                let Some(team_uid) = workspaces.current_team_uid() else {
                    ctx.emit(BuildPlanMigrationModalEvent::ShowToast {
                        message: "Oops, something went wrong; your team data could not be found."
                            .to_string(),
                        flavor: ToastFlavor::Error,
                    });
                    return;
                };

                // Get current monthly spend limit before any mutable borrows
                let current_monthly_spend_limit = workspaces
                    .current_workspace()
                    .and_then(|ws| ws.settings.addon_credits_settings.max_monthly_spend_cents);

                // Set loading state
                self.is_updating = true;
                ctx.notify();

                // Determine selected denomination (only if auto-reload is enabled)
                let selected_denomination = if self.auto_reload_enabled {
                    self.addon_credits_options
                        .get(self.selected_addon_credits_option)
                        .map(|option| option.credits)
                } else {
                    None
                };

                // Determine if we need to update the monthly spend limit
                // If the selected denomination price is greater than the current limit, increase the limit
                let new_monthly_spend_limit = if self.auto_reload_enabled {
                    self.addon_credits_options
                        .get(self.selected_addon_credits_option)
                        .and_then(|option| {
                            let selected_price = option.price_usd_cents;
                            match current_monthly_spend_limit {
                                Some(current_limit) if selected_price > current_limit => {
                                    Some(selected_price)
                                }
                                None => Some(selected_price),
                                _ => None,
                            }
                        })
                } else {
                    None
                };

                // Call API to update auto-reload settings
                UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                    user_workspaces.update_addon_credits_settings(
                        team_uid,
                        Some(self.auto_reload_enabled),
                        new_monthly_spend_limit,
                        selected_denomination,
                        ctx,
                    );
                });
            }
            BuildPlanMigrationModalViewAction::Close => {
                Self::mark_modal_dismissed(ctx);
                ctx.emit(BuildPlanMigrationModalEvent::Close);
            }
            BuildPlanMigrationModalViewAction::EnableAutoReloadToggled(enabled) => {
                self.auto_reload_enabled = *enabled;
                ctx.notify();
            }
            BuildPlanMigrationModalViewAction::OpenUrl(url) => {
                ctx.open_url(url);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum BuildPlanMigrationModalEvent {
    Close,
    ShowToast {
        message: String,
        flavor: ToastFlavor,
    },
}
