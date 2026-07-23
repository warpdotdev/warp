use asset_macro::bundled_or_fetched_asset;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use thousands::Separable;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warp_graphql::billing::StripeSubscriptionPlan;
use warpui::elements::{
    Align, CacheOption, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DropShadow, Expanded, Flex, FormattedTextElement, HighlightedHyperlink, Image,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Stack,
};
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::auth::AuthStateProvider;
use crate::pricing::PricingInfoModel;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::CustomerType;
use crate::{TelemetryEvent, send_telemetry_from_ctx};

const MODAL_WIDTH: f32 = 360.;
const MODAL_HEIGHT: f32 = 532.;
const COMPACT_MODAL_HEIGHT: f32 = 360.;
const HEADER_HEIGHT: f32 = 92.;
const BUTTON_DIAMETER: f32 = 20.;
const BILLING_AND_USAGE_URL: &str = "warp://settings/billing_and_usage";

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum CloudAgentCapacityModalVariant {
    #[default]
    ConcurrentLimit,
    OutOfCredits,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        CloudAgentCapacityModalAction::Close,
        id!("CloudAgentCapacityModal"),
    )]);
}

#[derive(Default)]
struct StateHandles {
    close_button: MouseStateHandle,
    upgrade_button: MouseStateHandle,
}

pub struct CloudAgentCapacityModal {
    state_handles: StateHandles,
    variant: CloudAgentCapacityModalVariant,
}

impl CloudAgentCapacityModal {
    pub fn new() -> Self {
        CloudAgentCapacityModal {
            state_handles: Default::default(),
            variant: CloudAgentCapacityModalVariant::default(),
        }
    }

    pub fn set_variant(&mut self, variant: CloudAgentCapacityModalVariant) {
        self.variant = variant;
    }

    fn get_upgrade_url(ctx: &ViewContext<Self>) -> Option<String> {
        let auth_state = AuthStateProvider::handle(ctx).as_ref(ctx).get();
        if let Some(team) = UserWorkspaces::handle(ctx).as_ref(ctx).current_team() {
            return Some(UserWorkspaces::upgrade_link_for_team(team.uid));
        }

        let user_id = auth_state.user_id().unwrap_or_default();
        Some(UserWorkspaces::upgrade_link(user_id))
    }

    fn can_upgrade(customer_type: CustomerType, variant: CloudAgentCapacityModalVariant) -> bool {
        match variant {
            CloudAgentCapacityModalVariant::ConcurrentLimit => !matches!(
                customer_type,
                CustomerType::Business | CustomerType::Enterprise
            ),
            CloudAgentCapacityModalVariant::OutOfCredits => {
                matches!(customer_type, CustomerType::Free | CustomerType::Unknown)
            }
        }
    }

    fn should_show_cta(
        customer_type: CustomerType,
        variant: CloudAgentCapacityModalVariant,
    ) -> bool {
        matches!(variant, CloudAgentCapacityModalVariant::OutOfCredits)
            || Self::can_upgrade(customer_type, variant)
    }

    fn cta_url(&self, ctx: &ViewContext<Self>) -> Option<String> {
        let customer_type = UserWorkspaces::handle(ctx)
            .as_ref(ctx)
            .current_workspace()
            .map(|workspace| workspace.billing_metadata.customer_type)
            .unwrap_or(CustomerType::Free);
        if !Self::should_show_cta(customer_type, self.variant) {
            return None;
        }
        if Self::can_upgrade(customer_type, self.variant) {
            Self::get_upgrade_url(ctx)
        } else {
            Some(BILLING_AND_USAGE_URL.to_string())
        }
    }

    fn render_content(&self, customer_type: CustomerType, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::handle(app).as_ref(app);
        let theme = appearance.theme();
        let neutral_bg = blended_colors::neutral_1(theme);
        let (title_text, mut explanation_text) = match self.variant {
            CloudAgentCapacityModalVariant::ConcurrentLimit => (
                "Concurrent cloud agent limit reached",
                "This cloud run is queued because your team has reached the maximum number of concurrent cloud agents. It will start automatically when another cloud run finishes.".to_string(),
            ),
            CloudAgentCapacityModalVariant::OutOfCredits => (
                "You're out of AI credits",
                "This cloud run stopped because your team has used all available AI credits for the current billing period.".to_string(),
            ),
        };

        // Title
        let title = FormattedTextElement::from_str(title_text, appearance.ui_font_family(), 24.)
            .with_color(blended_colors::text_main(theme, neutral_bg))
            .with_weight(Weight::Bold)
            .finish();

        // Explanation.
        let can_upgrade = Self::can_upgrade(customer_type, self.variant);
        let show_cta = Self::should_show_cta(customer_type, self.variant);
        if can_upgrade {
            let upgrade_suffix = match self.variant {
                CloudAgentCapacityModalVariant::ConcurrentLimit => {
                    " Upgrade your plan for more concurrent cloud agents."
                }
                CloudAgentCapacityModalVariant::OutOfCredits => {
                    " Upgrade your plan to continue running cloud agents."
                }
            };
            explanation_text.push_str(upgrade_suffix);
        }
        let subtitle =
            FormattedTextElement::from_str(explanation_text, appearance.ui_font_family(), 14.)
                .with_color(blended_colors::text_sub(theme, neutral_bg))
                .finish();

        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(Container::new(title).with_margin_bottom(12.).finish())
            .with_child(Container::new(subtitle).with_margin_bottom(16.).finish());

        if can_upgrade {
            let (target_plan, agent_multiplier, extra_benefits) = match customer_type {
                CustomerType::Build | CustomerType::BuildMax => {
                    (StripeSubscriptionPlan::BuildBusiness, "2x", vec!["SSO"])
                }
                // Free tier or a legacy plan.
                _ => (StripeSubscriptionPlan::Build, "5x", vec![]),
            };

            let plan_pricing = PricingInfoModel::handle(app)
                .as_ref(app)
                .plan_pricing(&target_plan);

            // Pricing text based on plan type and actual pricing
            let pricing_text = if customer_type == CustomerType::Free {
                if let Some(pricing) = plan_pricing {
                    let price = pricing.yearly_plan_price_per_month_usd_cents / 100;
                    format!(
                        "Paid plans start at ${price}/month and include everything in your free trial plus:"
                    )
                } else {
                    "Paid plans include everything in your free trial plus:".to_string()
                }
            } else if let Some(pricing) = plan_pricing {
                let price = pricing.yearly_plan_price_per_month_usd_cents / 100;
                format!(
                    "The Business plan starts at ${price}/month and includes everything on your current plan plus:"
                )
            } else {
                "The Business plan includes everything on your current plan plus:".to_string()
            };

            let pricing = FormattedTextElement::new(
                FormattedText::new([FormattedTextLine::Line(vec![
                    FormattedTextFragment::plain_text(pricing_text),
                ])]),
                14.,
                appearance.ui_font_family(),
                appearance.ui_font_family(),
                blended_colors::text_sub(theme, neutral_bg),
                HighlightedHyperlink::default(),
            )
            .finish();

            // Credits text from plan pricing
            let credits_text = if let Some(limit) = plan_pricing.and_then(|plan| plan.request_limit)
            {
                format!("{} AI credits per month", limit.separate_with_commas())
            } else {
                "Extended AI credits per month".to_string()
            };

            // Benefits list based on plan type
            let mut benefits = vec![
                format!("{} the number of concurrent cloud agents", agent_multiplier),
                credits_text,
                "Bring your own API key".to_string(),
            ];
            for extra in extra_benefits {
                benefits.push(extra.to_string());
            }

            let mut benefits_column =
                Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Start);

            for benefit in benefits {
                let benefit_formatted = FormattedText::new([FormattedTextLine::Line(vec![
                    FormattedTextFragment::plain_text(benefit),
                ])]);
                benefits_column.add_child(
                    Container::new(
                        Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(
                                Container::new(
                                    ConstrainedBox::new(
                                        Icon::CheckCircleBroken
                                            .to_warpui_icon(Fill::Solid(theme.ansi_fg_green()))
                                            .finish(),
                                    )
                                    .with_width(14.)
                                    .with_height(14.)
                                    .finish(),
                                )
                                .with_margin_right(4.)
                                .finish(),
                            )
                            .with_child(
                                FormattedTextElement::new(
                                    benefit_formatted,
                                    14.,
                                    appearance.ui_font_family(),
                                    appearance.ui_font_family(),
                                    blended_colors::text_sub(theme, neutral_bg),
                                    HighlightedHyperlink::default(),
                                )
                                .finish(),
                            )
                            .finish(),
                    )
                    .with_margin_bottom(8.)
                    .finish(),
                );
            }
            content.add_child(Container::new(pricing).with_margin_bottom(8.).finish());
            content.add_child(benefits_column.finish());
        }

        let content = content.finish();
        let cta_button = if show_cta {
            let cta_button_label = if can_upgrade {
                "Upgrade plan"
            } else {
                "Open billing"
            };
            Some(
                appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Accent,
                        self.state_handles.upgrade_button.clone(),
                    )
                    .with_style(UiComponentStyles {
                        font_size: Some(14.),
                        height: Some(32.),
                        width: Some(296.),
                        ..Default::default()
                    })
                    .with_centered_text_label(cta_button_label.to_string())
                    .build()
                    .with_cursor(Cursor::PointingHand)
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(CloudAgentCapacityModalAction::Upgrade)
                    })
                    .finish(),
            )
        } else {
            None
        };

        // Main content layout
        let layout = if let Some(cta_button) = cta_button {
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(content)
                .with_child(Align::new(cta_button).bottom_left().finish())
                .finish()
        } else {
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(content)
                .finish()
        };
        Container::new(layout).with_uniform_padding(32.).finish()
    }

    fn render_header() -> Box<dyn Element> {
        ConstrainedBox::new(
            Image::new(
                bundled_or_fetched_asset!("png/concurrency_limit_header.png"),
                CacheOption::BySize,
            )
            .cover()
            .with_corner_radius(CornerRadius::with_top(Radius::Pixels(10.)))
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .with_height(HEADER_HEIGHT)
        .finish()
    }
}

impl Entity for CloudAgentCapacityModal {
    type Event = CloudAgentCapacityModalEvent;
}

impl View for CloudAgentCapacityModal {
    fn ui_name() -> &'static str {
        "CloudAgentCapacityModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::handle(app).as_ref(app);
        let theme = appearance.theme();

        let close_button = appearance
            .ui_builder()
            .close_button(BUTTON_DIAMETER, self.state_handles.close_button.clone())
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(CloudAgentCapacityModalAction::Close))
            .finish();

        let customer_type = UserWorkspaces::as_ref(app)
            .current_workspace()
            .map(|workspace| workspace.billing_metadata.customer_type)
            .unwrap_or(CustomerType::Free);
        let can_upgrade = Self::can_upgrade(customer_type, self.variant);

        let mut modal = Stack::new();
        modal.add_child(
            Container::new(
                ConstrainedBox::new(
                    Flex::column()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_child(Self::render_header())
                        .with_child(
                            Expanded::new(1., self.render_content(customer_type, app)).finish(),
                        )
                        .finish(),
                )
                .with_width(MODAL_WIDTH)
                .with_height(if can_upgrade {
                    MODAL_HEIGHT
                } else {
                    COMPACT_MODAL_HEIGHT
                })
                .finish(),
            )
            .with_background_color(blended_colors::neutral_1(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
            .with_drop_shadow(DropShadow::default())
            .finish(),
        );
        modal.add_positioned_child(
            close_button,
            OffsetPositioning::offset_from_parent(
                vec2f(-8., 8.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );

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

        // Semi-transparent backdrop overlay
        Container::new(Align::new(stack.finish()).finish())
            .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
            .finish()
    }
}

impl TypedActionView for CloudAgentCapacityModal {
    type Action = CloudAgentCapacityModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CloudAgentCapacityModalAction::Close => {
                send_telemetry_from_ctx!(TelemetryEvent::CloudAgentCapacityModalDismissed, ctx);
                ctx.emit(CloudAgentCapacityModalEvent::Close);
            }
            CloudAgentCapacityModalAction::Upgrade => {
                if let Some(upgrade_url) = self.cta_url(ctx) {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::CloudAgentCapacityModalUpgradeClicked,
                        ctx
                    );
                    ctx.open_url(&upgrade_url);
                    ctx.emit(CloudAgentCapacityModalEvent::Close);
                }
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum CloudAgentCapacityModalEvent {
    Close,
}

#[derive(Clone, Debug)]
pub enum CloudAgentCapacityModalAction {
    Close,
    Upgrade,
}
