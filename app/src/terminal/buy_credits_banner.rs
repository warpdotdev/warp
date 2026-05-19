use std::sync::Arc;

use enclose::enclose;
use itertools::Itertools as _;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::Icon;
use warp_graphql::billing::AddonCreditsOption;
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, DropShadow, Expanded, Flex, FormattedTextElement, HighlightedHyperlink,
    Hoverable, Icon as WarpUiIcon, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement as _, ParentOffsetBounds, Radius, Shrinkable,
    SizeConstraintCondition, SizeConstraintSwitch, Stack, Text,
};
use warpui::fonts::Weight;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent as _, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity as _, View, ViewContext, ViewHandle};

use crate::ai::request_usage_model::{
    AIRequestUsageModel, AIRequestUsageModelEvent, BuyCreditsBannerDisplayState,
};
use crate::auth::AuthStateProvider;
use crate::features::FeatureFlag;
use crate::menu::MenuItemFields;
use crate::pricing::{PricingInfoModel, PricingInfoModelEvent};
use crate::send_telemetry_from_ctx;
use crate::server::ids::ServerId;
use crate::server::telemetry::{OutOfCreditsBannerAction, TelemetryEvent};
use crate::settings_view::create_discount_badge;
use crate::view_components::Dropdown;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
use warp_graphql::error::BudgetExceededError;

#[derive(Default)]
struct MouseStates {
    buy_button: MouseStateHandle,
    close_button: MouseStateHandle,
    manage_billing_button: MouseStateHandle,
    auto_reload_checkbox: MouseStateHandle,
    auto_reload_info_icon: MouseStateHandle,
}

pub struct BuyCreditsBanner {
    mouse_states: MouseStates,
    denomination_dropdown: ViewHandle<Dropdown<Action>>,
    addon_credits_options: Vec<AddonCreditsOption>,
    selected_denomination_index: usize,
    purchase_addon_credits_loading: bool,
    should_display_banner: bool,
    billing_settings_hyperlink: HighlightedHyperlink,
    is_denomination_dropdown_open: bool,
    auto_reload_enabled: bool,
    banner_auto_reload_update_in_flight: bool,
}

impl BuyCreditsBanner {
    pub fn is_denomination_dropdown_open(&self, _app: &AppContext) -> bool {
        self.is_denomination_dropdown_open
    }

    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&PricingInfoModel::handle(ctx), |me, _handle, event, ctx| {
            #[allow(irrefutable_let_patterns)]
            if let PricingInfoModelEvent::PricingInfoUpdated = event {
                me.update_addon_credits_options(ctx);
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(
            &AIRequestUsageModel::handle(ctx),
            |_me, _handle, event, ctx| {
                if let AIRequestUsageModelEvent::RequestUsageUpdated = event {
                    ctx.notify();
                }
            },
        );

        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _handle, event, ctx| {
            me.handle_workspaces_event(event, ctx);
        });

        let denomination_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx).with_drop_shadow();
            dropdown.set_top_bar_max_width(170.);
            dropdown.set_menu_width(225., ctx);
            dropdown
        });

        ctx.subscribe_to_view(&denomination_dropdown, |me, _, event, ctx| match event {
            crate::view_components::DropdownEvent::ToggleExpanded => {
                me.is_denomination_dropdown_open = true;
                ctx.notify();
            }
            crate::view_components::DropdownEvent::Close => {
                me.is_denomination_dropdown_open = false;
                ctx.emit(BuyCreditsBannerEvent::RefocusInput);
                ctx.notify();
            }
        });

        let mut me = Self {
            mouse_states: Default::default(),
            denomination_dropdown,
            addon_credits_options: Default::default(),
            selected_denomination_index: 0,
            purchase_addon_credits_loading: false,
            should_display_banner: false,
            billing_settings_hyperlink: HighlightedHyperlink::default(),
            is_denomination_dropdown_open: false,
            auto_reload_enabled: false,
            banner_auto_reload_update_in_flight: false,
        };
        me.update_addon_credits_options(ctx);
        me
    }

    fn handle_workspaces_event(
        &mut self,
        event: &UserWorkspacesEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            UserWorkspacesEvent::PurchaseAddonCreditsSuccess => {
                self.purchase_addon_credits_loading = false;

                let banner_toggle_flag_enabled =
                    FeatureFlag::BuildPlanAutoReloadBannerToggle.is_enabled();
                let post_purchase_modal_flag_enabled =
                    FeatureFlag::BuildPlanAutoReloadPostPurchaseModal.is_enabled();

                let selected_credits = self
                    .addon_credits_options
                    .get(self.selected_denomination_index)
                    .map(|option| option.credits);
                let has_admin_permissions = {
                    let auth_state = AuthStateProvider::as_ref(ctx).get();
                    let current_team = UserWorkspaces::as_ref(ctx).current_team();
                    auth_state
                        .user_email()
                        .zip(current_team)
                        .map(|(email, team)| team.has_admin_permissions(&email))
                        .unwrap_or_default()
                };

                // Things we always do:
                // - emit telemetry
                // - hide banner
                send_telemetry_from_ctx!(
                    TelemetryEvent::OutOfCreditsBannerClosed {
                        action: OutOfCreditsBannerAction::CreditsPurchased,
                        selected_credits,
                        auto_reload_checkbox_enabled: self.auto_reload_enabled,
                        banner_toggle_flag_enabled,
                        post_purchase_modal_flag_enabled,
                    },
                    ctx
                );

                self.should_display_banner = false;
                AIRequestUsageModel::handle(ctx).update(ctx, |ai_request_usage_model, ctx| {
                    ai_request_usage_model.dismiss_buy_credits_banner(ctx);
                });

                // Experiment-specific behavior:
                // - Banner toggle flow: optionally enable auto-reload immediately.
                // - Post-purchase modal flow: show the modal.
                if banner_toggle_flag_enabled {
                    if has_admin_permissions && self.auto_reload_enabled {
                        self.banner_auto_reload_update_in_flight = true;

                        if let Some(team_uid) = UserWorkspaces::as_ref(ctx).current_team_uid() {
                            UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                                user_workspaces.update_addon_credits_settings(
                                    team_uid,
                                    Some(true),
                                    None, // Don't change monthly spend limit
                                    selected_credits,
                                    ctx,
                                );
                            });
                        }
                    }
                } else if has_admin_permissions && post_purchase_modal_flag_enabled {
                    // Default selection in the modal should match the denomination the user clicked "buy" on.
                    ctx.emit(BuyCreditsBannerEvent::OpenAutoReloadModal {
                        purchased_credits: selected_credits.unwrap_or(0),
                    });
                }

                ctx.notify();
            }
            UserWorkspacesEvent::PurchaseAddonCreditsRejected(err) => {
                self.purchase_addon_credits_loading = false;
                self.banner_auto_reload_update_in_flight = false;

                if err.downcast_ref::<BudgetExceededError>().is_some() {
                    self.should_display_banner = true;
                } else {
                    AIRequestUsageModel::handle(ctx).update(ctx, |ai_request_usage_model, ctx| {
                        ai_request_usage_model.dismiss_buy_credits_banner(ctx);
                    });
                }
                ctx.notify();
            }
            UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess => {
                if self.banner_auto_reload_update_in_flight {
                    self.banner_auto_reload_update_in_flight = false;
                    ctx.notify();
                }
            }
            UserWorkspacesEvent::UpdateWorkspaceSettingsRejected(_) => {
                if self.banner_auto_reload_update_in_flight {
                    self.banner_auto_reload_update_in_flight = false;
                    ctx.emit(BuyCreditsBannerEvent::ShowAutoReloadError {
                        error_message: "Failed to enable auto-reload for your team. Please try again in Settings > Billing and Usage.",
                    });
                    ctx.notify();
                }
            }
            _ => {}
        }
    }

    fn render_auto_reload_checkbox(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let check_color = theme.background().into_solid();
        let auto_reload_enabled = self.auto_reload_enabled;

        let checkbox = appearance
            .ui_builder()
            .checkbox(
                self.mouse_states.auto_reload_checkbox.clone(),
                Some(appearance.ui_font_size()),
            )
            .check(auto_reload_enabled)
            .with_style(UiComponentStyles {
                font_color: Some(check_color),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::ToggleAutoReload);
            })
            .finish();

        let sub_text_color = theme.sub_text_color(theme.surface_1());

        let label = Text::new_inline("Auto reload", appearance.ui_font_family(), 12.)
            .with_color(sub_text_color.into())
            .finish();

        // Get the selected amount for the tooltip
        let selected_credits = self
            .addon_credits_options
            .get(self.selected_denomination_index)
            .map(|option| option.credits)
            .unwrap_or(0);

        let tooltip_text = format!(
            "When enabled, auto reload will purchase {} credits when your credit balance gets low",
            selected_credits
        );

        // Create info icon with a custom sub_text_color & mouse cursor (i.e. as opposed to using IconWithTooltip)
        let ui_builder = appearance.ui_builder();
        let info_icon = Hoverable::new(
            self.mouse_states.auto_reload_info_icon.clone(),
            move |state| {
                let info_icon_element = Container::new(
                    ConstrainedBox::new(
                        WarpUiIcon::new("bundled/svg/info.svg", sub_text_color).finish(),
                    )
                    .with_width(13.)
                    .with_height(13.)
                    .finish(),
                )
                .finish();

                let mut stack = Stack::new().with_child(info_icon_element);
                if state.is_hovered() {
                    let tool_tip = ui_builder.tool_tip(tooltip_text.clone()).build();
                    stack.add_positioned_child(
                        tool_tip.finish(),
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., -3.),
                            ParentOffsetBounds::Unbounded,
                            ParentAnchor::TopMiddle,
                            ChildAnchor::BottomMiddle,
                        ),
                    );
                }
                stack.finish()
            },
        )
        .finish();

        Flex::row()
            .with_child(Container::new(checkbox).with_margin_right(4.).finish())
            .with_child(label)
            .with_child(
                Container::new(info_icon)
                    .with_margin_left(4.)
                    .with_margin_bottom(1.)
                    .finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish()
    }

    fn update_addon_credits_options(&mut self, ctx: &mut ViewContext<Self>) {
        self.addon_credits_options = PricingInfoModel::as_ref(ctx)
            .addon_credits_options()
            .map(|opts| opts.to_vec())
            .unwrap_or_default();

        let base_rate = self
            .addon_credits_options
            .first()
            .map_or(0., |option| option.rate());
        let items = self
            .addon_credits_options
            .iter()
            .enumerate()
            .map(|(index, option)| {
                let primary_text = format!(
                    "${:.0} / {} credits",
                    option.price_usd_cents as f32 / 100.,
                    option.credits
                );
                let discount_percent = if base_rate > 0.0 {
                    let actual_rate = option.rate();
                    ((base_rate - actual_rate) / base_rate * 100.0).round() as u32
                } else {
                    0
                };
                if discount_percent > 0 {
                    MenuItemFields::new_with_custom_label(
                        Arc::new(enclose!((primary_text) move |is_selected, is_hovered, appearance, _| {
                            let text_color = appearance.theme().main_text_color(
                                if is_selected || is_hovered {
                                    appearance.theme().accent()
                                } else {
                                    appearance.theme().surface_1()
                                }
                            );
                            let main_text = Text::new_inline(
                                primary_text.clone(),
                                appearance.ui_font_family(),
                                appearance.ui_font_size(),
                            )
                            .with_color(text_color.into())
                            .finish();

                            let discount_badge = create_discount_badge(discount_percent, appearance);

                            Flex::row()
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                                .with_main_axis_size(MainAxisSize::Max)
                                .with_child(main_text)
                                .with_child(discount_badge)
                                .finish()
                        })),
                        Some(primary_text)
                    )
                    .with_on_select_action(Action::SelectDenomination(index).into())
                    .into_item()
                } else {
                    MenuItemFields::new(primary_text.clone())
                        .with_on_select_action(Action::SelectDenomination(index).into())
                        .into_item()
                }
            })
            .collect_vec();
        self.denomination_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_rich_items(items, ctx);
            dropdown.set_selected_by_index(0, ctx);
        });
    }

    fn render_auto_reload_blocked(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let alert_icon = Container::new(
            ConstrainedBox::new(
                Icon::AlertCircle
                    .to_warpui_icon(theme.foreground())
                    .finish(),
            )
            .with_height(16.)
            .with_width(16.)
            .finish(),
        )
        .with_margin_right(8.)
        .finish();

        let auth_state = AuthStateProvider::as_ref(app).get();
        let current_team = UserWorkspaces::as_ref(app).current_team();
        let has_admin_permissions = auth_state
            .user_email()
            .zip(current_team)
            .map(|(email, team)| team.has_admin_permissions(&email))
            .unwrap_or_default();

        // Banner text with title and description based on admin status
        let banner_description = if has_admin_permissions {
            "Your monthly spend limit has been reached. Increase it to continue."
        } else {
            "Contact a team admin to increase monthly limit."
        };

        let banner_text = Flex::column()
            .with_children([
                appearance
                    .ui_builder()
                    .paragraph("Monthly limit reached")
                    .with_style(UiComponentStyles {
                        font_size: Some(14.),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
                appearance
                    .ui_builder()
                    .paragraph(banner_description)
                    .with_style(UiComponentStyles {
                        font_color: Some(theme.sub_text_color(theme.surface_1()).into()),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            ])
            .finish();

        let close_button = appearance
            .ui_builder()
            .close_button(16., self.mouse_states.close_button.clone())
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::Close))
            .finish();

        let mut content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children([
                alert_icon,
                Shrinkable::new(1., Align::new(banner_text).left().finish()).finish(),
            ]);

        // Only show manage billing button for admins
        if has_admin_permissions {
            let manage_billing_button = appearance
                .ui_builder()
                .button(
                    ButtonVariant::Accent,
                    self.mouse_states.manage_billing_button.clone(),
                )
                .with_style(UiComponentStyles {
                    font_weight: Some(Weight::Semibold),
                    padding: Some(Coords {
                        top: 6.,
                        bottom: 6.,
                        left: 8.,
                        right: 8.,
                    }),
                    ..Default::default()
                })
                .with_text_label("Manage billing".to_string())
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(Action::ManageBilling);
                })
                .finish();

            content.add_child(
                Container::new(manage_billing_button)
                    .with_margin_right(8.)
                    .finish(),
            );
        }

        content.add_child(close_button);

        Container::new(content.finish())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_background_color(theme.surface_1().into())
            .with_horizontal_padding(16.)
            .with_vertical_padding(12.)
            .with_horizontal_margin(8.)
            .with_drop_shadow(DropShadow::new_with_standard_offset_and_spread(
                ColorU::new(0, 0, 0, 48),
            ))
            .finish()
    }

    fn render_out_of_credits(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        // When the auto-reload checkbox is present, we need more horizontal space before we can
        // fit everything on one row.
        const STACKED_LAYOUT_MAX_WIDTH_WITH_AUTO_RELOAD: f32 = 520.;
        const STACKED_LAYOUT_MAX_WIDTH_WITHOUT_AUTO_RELOAD: f32 = 400.;

        let theme = appearance.theme();

        let make_alert_icon = |margin_bottom: f32| {
            Container::new(
                ConstrainedBox::new(
                    Icon::AlertCircle
                        .to_warpui_icon(theme.foreground())
                        .finish(),
                )
                .with_height(16.)
                .with_width(16.)
                .finish(),
            )
            .with_margin_right(8.)
            .with_margin_bottom(margin_bottom)
            .finish()
        };

        let auth_state = AuthStateProvider::as_ref(app).get();
        let current_team = UserWorkspaces::as_ref(app).current_team();
        let has_admin_permissions = auth_state
            .user_email()
            .zip(current_team)
            .map(|(email, team)| team.has_admin_permissions(&email))
            .unwrap_or_default();
        let delinquent_due_to_payment_issue = current_team
            .is_some_and(|team| team.billing_metadata.is_delinquent_due_to_payment_issue());
        let auto_reload_banner_toggle_ff =
            FeatureFlag::BuildPlanAutoReloadBannerToggle.is_enabled();

        // Check if user has reached their monthly addon credits limit
        let current_workspace = UserWorkspaces::as_ref(app).current_workspace();
        let is_at_monthly_limit = current_workspace
            .map(|workspace| workspace.is_at_addon_credits_monthly_limit())
            .unwrap_or(false);

        // Check if the selected purchase would reach/exceed the monthly limit
        let selected_option = self
            .addon_credits_options
            .get(self.selected_denomination_index);
        let would_purchase_exceed_limit = current_workspace
            .zip(selected_option)
            .map(|(workspace, option)| {
                workspace.would_addon_purchase_reach_limit(option.price_usd_cents)
            })
            .unwrap_or(false);

        let make_banner_text = || {
            let mut banner_text_children = vec![appearance
                .ui_builder()
                .paragraph("Out of credits")
                .with_style(UiComponentStyles {
                    font_size: Some(14.),
                    ..Default::default()
                })
                .build()
                .finish()];

            // Show different message based on whether purchase would exceed limit
            if is_at_monthly_limit || would_purchase_exceed_limit {
                // Create formatted text with clickable hyperlink
                let warning_text_fragments = vec![
                    FormattedTextFragment::plain_text(
                        "Purchasing these credits would take you over your monthly spend limit. ",
                    ),
                    FormattedTextFragment::hyperlink_action("Increase it", Action::ManageBilling),
                    FormattedTextFragment::plain_text(" to continue."),
                ];

                let formatted_warning = FormattedTextElement::new(
                    FormattedText::new([FormattedTextLine::Line(warning_text_fragments)]),
                    appearance.ui_font_size(),
                    appearance.ui_font_family(),
                    appearance.ui_font_family(),
                    theme.sub_text_color(theme.surface_1()).into(),
                    self.billing_settings_hyperlink.clone(),
                )
                .with_hyperlink_font_color(theme.accent().into_solid())
                .register_default_click_handlers_with_action_support(
                    |hyperlink_lens, event, _ctx| match hyperlink_lens {
                        warpui::elements::HyperlinkLens::Url(_url) => {}
                        warpui::elements::HyperlinkLens::Action(action_ref) => {
                            if let Some(action) = action_ref.as_any().downcast_ref::<Action>() {
                                event.dispatch_typed_action(action.clone());
                            }
                        }
                    },
                )
                .finish();

                banner_text_children.push(formatted_warning);
            } else {
                // Default message when not at limit
                let banner_description = if has_admin_permissions {
                    "Add more credits to your account to continue using Oz agents."
                } else {
                    "Contact a team admin to purchase more credits to continue."
                };

                banner_text_children.push(
                    appearance
                        .ui_builder()
                        .paragraph(banner_description)
                        .with_style(UiComponentStyles {
                            font_color: Some(theme.sub_text_color(theme.surface_1()).into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                );
            }

            let banner_text =
                Container::new(Flex::column().with_children(banner_text_children).finish())
                    .with_margin_right(8.)
                    .finish();

            Expanded::new(
                1.,
                Align::new(ConstrainedBox::new(banner_text).finish())
                    .left()
                    .finish(),
            )
            .finish()
        };

        let make_buy_button = || {
            let team_uid = current_team.map(|team| team.uid);

            let buy_button_disabled = self.purchase_addon_credits_loading
                || delinquent_due_to_payment_issue
                || is_at_monthly_limit
                || would_purchase_exceed_limit;

            let button_text = if self.purchase_addon_credits_loading {
                "Buying…".to_string()
            } else {
                "Buy".to_string()
            };

            let button_font_color = buy_button_disabled.then_some(
                appearance
                    .theme()
                    .disabled_text_color(appearance.theme().surface_3())
                    .into(),
            );
            let button_bg_color =
                buy_button_disabled.then_some(appearance.theme().surface_3().into());
            let button_border = buy_button_disabled.then_some(ColorU::transparent_black().into());

            let mut buy_button = appearance
                .ui_builder()
                .button(ButtonVariant::Accent, self.mouse_states.buy_button.clone())
                .with_style(UiComponentStyles {
                    font_weight: Some(Weight::Semibold),
                    padding: Some(Coords {
                        top: 6.,
                        bottom: 6.,
                        left: 8.,
                        right: 8.,
                    }),
                    font_color: button_font_color,
                    background: button_bg_color,
                    border_color: button_border,
                    ..Default::default()
                })
                .with_text_label(button_text)
                .build()
                .on_click(move |ctx, _, _| {
                    if let Some(team_uid) = team_uid {
                        ctx.dispatch_typed_action(Action::PurchaseAddonCredits { team_uid });
                    }
                });

            if buy_button_disabled {
                buy_button = buy_button.disable();
            }

            buy_button.finish()
        };

        let make_close_button = || {
            Container::new(
                appearance
                    .ui_builder()
                    .close_button(16., self.mouse_states.close_button.clone())
                    .build()
                    .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::Close))
                    .finish(),
            )
            .with_margin_left(8.)
            .finish()
        };

        let make_admin_controls_children = || {
            let denomination_dropdown = ChildView::new(&self.denomination_dropdown).finish();
            let buy_button = make_buy_button();

            let mut children = Vec::new();

            if auto_reload_banner_toggle_ff {
                children.push(
                    Container::new(self.render_auto_reload_checkbox(appearance))
                        .with_margin_right(8.)
                        .finish(),
                );
            }

            children.extend([
                Container::new(denomination_dropdown)
                    .with_margin_right(4.)
                    .finish(),
                Container::new(buy_button).finish(),
            ]);

            children
        };

        let make_admin_controls_row = || {
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children(make_admin_controls_children())
                .finish()
        };

        let make_row_layout = || {
            let mut content = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children([make_alert_icon(12.), make_banner_text()]);

            if has_admin_permissions {
                content.add_children(make_admin_controls_children());
            }

            content.add_child(make_close_button());
            content.finish()
        };

        let make_stacked_layout = || {
            let top_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children([make_alert_icon(0.), make_banner_text(), make_close_button()])
                .finish();

            let bottom_row = Container::new(make_admin_controls_row())
                .with_margin_top(8.)
                .finish();

            Flex::column().with_children([top_row, bottom_row]).finish()
        };

        let content = if has_admin_permissions {
            let stacked_breakpoint = if auto_reload_banner_toggle_ff {
                STACKED_LAYOUT_MAX_WIDTH_WITH_AUTO_RELOAD
            } else {
                STACKED_LAYOUT_MAX_WIDTH_WITHOUT_AUTO_RELOAD
            };

            SizeConstraintSwitch::new(
                make_row_layout(),
                vec![(
                    SizeConstraintCondition::WidthLessThan(stacked_breakpoint),
                    make_stacked_layout(),
                )],
            )
            .finish()
        } else {
            make_row_layout()
        };

        // The dropdown adds some margin so we need to use less padding.
        let vertical_padding = if has_admin_permissions { 6. } else { 12. };

        Container::new(content)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_background_color(theme.surface_1().into())
            .with_horizontal_padding(16.)
            .with_vertical_padding(vertical_padding)
            .with_horizontal_margin(8.)
            .with_drop_shadow(DropShadow::default())
            .finish()
    }
}

#[derive(Clone, Debug)]
pub enum BuyCreditsBannerEvent {
    OpenBillingAndUsage,
    RefocusInput,
    OpenAutoReloadModal { purchased_credits: i32 },
    ShowAutoReloadError { error_message: &'static str },
}

impl Entity for BuyCreditsBanner {
    type Event = BuyCreditsBannerEvent;
}

impl View for BuyCreditsBanner {
    fn ui_name() -> &'static str {
        "BuyCreditsBanner"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let ai_request_usage = AIRequestUsageModel::as_ref(app);

        // Override with spend limit error if set (from failed purchase attempt)
        let display_state = if self.should_display_banner {
            BuyCreditsBannerDisplayState::MonthlyLimitReached
        } else {
            ai_request_usage.compute_buy_addon_credits_banner_display_state(app)
        };

        match display_state {
            BuyCreditsBannerDisplayState::Hidden => {
                Container::new(warpui::elements::Empty::new().finish()).finish()
            }
            BuyCreditsBannerDisplayState::OutOfCredits => {
                self.render_out_of_credits(appearance, app)
            }
            BuyCreditsBannerDisplayState::MonthlyLimitReached => {
                self.render_auto_reload_blocked(appearance, app)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    SelectDenomination(usize),
    Close,
    PurchaseAddonCredits { team_uid: ServerId },
    ManageBilling,
    ToggleAutoReload,
}

impl warpui::TypedActionView for BuyCreditsBanner {
    type Action = Action;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Action::SelectDenomination(index) => {
                self.selected_denomination_index = *index;
                ctx.notify();
            }
            Action::Close => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::OutOfCreditsBannerClosed {
                        action: OutOfCreditsBannerAction::Dismissed,
                        selected_credits: None,
                        auto_reload_checkbox_enabled: self.auto_reload_enabled,
                        banner_toggle_flag_enabled: FeatureFlag::BuildPlanAutoReloadBannerToggle
                            .is_enabled(),
                        post_purchase_modal_flag_enabled:
                            FeatureFlag::BuildPlanAutoReloadPostPurchaseModal.is_enabled(),
                    },
                    ctx
                );

                self.should_display_banner = false;
                AIRequestUsageModel::handle(ctx).update(ctx, |model, ctx| {
                    model.dismiss_buy_credits_banner(ctx);
                });
                ctx.notify();
            }
            Action::ManageBilling => {
                ctx.emit(BuyCreditsBannerEvent::OpenBillingAndUsage);
                self.should_display_banner = false;
                AIRequestUsageModel::handle(ctx).update(ctx, |model, ctx| {
                    model.dismiss_buy_credits_banner(ctx);
                });
                ctx.notify();
            }
            Action::PurchaseAddonCredits { team_uid } => {
                if let Some(option) = self
                    .addon_credits_options
                    .get(self.selected_denomination_index)
                {
                    let credits = option.credits;
                    self.purchase_addon_credits_loading = true;
                    UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                        user_workspaces.purchase_addon_credits(*team_uid, credits, ctx);
                    });
                    ctx.notify();
                }
            }
            Action::ToggleAutoReload => {
                self.auto_reload_enabled = !self.auto_reload_enabled;
                ctx.notify();
            }
        }
    }
}
