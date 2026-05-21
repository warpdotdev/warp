use crate::ai::{AIRequestUsageModel, AIRequestUsageModelEvent};
use crate::auth::AuthStateProvider;
use crate::pricing::{PricingInfoModel, PricingInfoModelEvent};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::TelemetryEvent;
use asset_macro::bundled_or_fetched_asset;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use thousands::Separable;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::{Fill, WarpTheme};
use warp_graphql::billing::{PlanPricing, StripeSubscriptionPlan};
use warpui::elements::{
    Align, Border, CacheOption, ChildAnchor, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, DropShadow, Flex, FormattedTextElement, HighlightedHyperlink, Image,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Stack,
};
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

const BUTTON_DIAMETER: f32 = 20.;
const MODAL_HEIGHT: f32 = 440.;
const LEFT_PANEL_WIDTH: f32 = 360.;
const RIGHT_PANEL_WIDTH: f32 = 360.;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        FreeTierLimitHitModalAction::Close,
        id!("FreeTierLimitHitModal"),
    )]);
}

#[derive(Default)]
struct StateHandles {
    close_button: MouseStateHandle,
    upgrade_button: MouseStateHandle,
}

pub struct FreeTierLimitHitModal {
    state_handles: StateHandles,
}

impl FreeTierLimitHitModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &PricingInfoModel::handle(ctx),
            |_, _, event, ctx| match event {
                PricingInfoModelEvent::PricingInfoUpdated => {
                    ctx.unsubscribe_to_model(&PricingInfoModel::handle(ctx));
                    ctx.notify();
                }
            },
        );

        ctx.subscribe_to_model(
            &AIRequestUsageModel::handle(ctx),
            |_, _, event, ctx| match event {
                AIRequestUsageModelEvent::RequestUsageUpdated => {
                    ctx.emit(FreeTierLimitHitModalEvent::MaybeOpen);
                }
                AIRequestUsageModelEvent::RequestBonusRefunded { .. } => {}
            },
        );

        FreeTierLimitHitModal {
            state_handles: Default::default(),
        }
    }

    fn get_upgrade_url(ctx: &ViewContext<Self>) -> String {
        let auth_state = AuthStateProvider::handle(ctx).as_ref(ctx).get();
        if let Some(team) = UserWorkspaces::handle(ctx).as_ref(ctx).current_team() {
            UserWorkspaces::upgrade_link_for_team(team.uid)
        } else {
            let user_id = auth_state.user_id().unwrap_or_default();
            UserWorkspaces::upgrade_link(user_id)
        }
    }

    fn get_build_plan_details(app: &AppContext) -> Option<&PlanPricing> {
        let pricing_model = PricingInfoModel::handle(app).as_ref(app);
        pricing_model.plan_pricing(&StripeSubscriptionPlan::Build)
    }

    fn render_checklist_item_dynamic(
        text: String,
        appearance: &Appearance,
        theme: &WarpTheme,
    ) -> Box<dyn Element> {
        let formatted_text = FormattedText::new([FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text(text),
        ])]);
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
                    formatted_text,
                    14.,
                    appearance.ui_font_family(),
                    appearance.ui_font_family(),
                    blended_colors::text_sub(theme, blended_colors::neutral_1(theme)),
                    HighlightedHyperlink::default(),
                )
                .finish(),
            )
            .finish()
    }

    fn render_left_panel(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::handle(app).as_ref(app);
        let theme = appearance.theme();

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_child(
                            Container::new(
                                FormattedTextElement::from_str(
                                    "You’re out of credits",
                                    appearance.ui_font_family(),
                                    24.,
                                )
                                .with_color(blended_colors::text_main(
                                    theme,
                                    blended_colors::neutral_1(theme),
                                ))
                                .with_weight(Weight::Bold)
                                .finish(),
                            )
                            .with_margin_bottom(12.)
                            .finish(),
                        )
                        .with_child(
                            Container::new(
                                FormattedTextElement::from_str(
                                    "To continue using AI, please upgrade your plan.",
                                    appearance.ui_font_family(),
                                    14.,
                                )
                                .with_color(blended_colors::text_sub(
                                    theme,
                                    blended_colors::neutral_1(theme),
                                ))
                                .finish(),
                            )
                            .with_margin_bottom(16.)
                            .finish(),
                        )
                        .with_child(
                            Container::new({
                                let benefits_text = if let Some(plan) = Self::get_build_plan_details(app) {
                                    let price = plan.monthly_plan_price_per_month_usd_cents / 100;
                                    format!("The Build plan is ${price}/month which includes everything in the free tier plus:")
                                } else {
                                    "The Build plan includes everything in the free tier plus:".to_string()
                                };
                                let formatted_text = FormattedText::new([FormattedTextLine::Line(vec![
                                    FormattedTextFragment::plain_text(benefits_text),
                                ])]);
                                FormattedTextElement::new(
                                    formatted_text,
                                    14.,
                                    appearance.ui_font_family(),
                                    appearance.ui_font_family(),
                                    blended_colors::text_sub(theme, blended_colors::neutral_1(theme)),
                                    HighlightedHyperlink::default(),
                                )
                                .finish()
                            })
                            .with_margin_bottom(8.)
                            .finish(),
                        )
                        .with_child(
                            Container::new({
                                let credits_text = if let Some(plan) = Self::get_build_plan_details(app) {
                                    let limit = plan.request_limit.unwrap_or(1500);
                                    format!("{} Credits per month", limit.separate_with_commas())
                                } else {
                                    "Extended Credits per month".to_string()
                                };
                                Self::render_checklist_item_dynamic(credits_text, appearance, theme)
                            })
                            .with_margin_bottom(8.)
                            .finish(),
                        )
                        .with_child(
                            Container::new(
                                Self::render_checklist_item_dynamic(
                                    "Access to frontier OpenAI, Anthropic, and Google models".to_string(),
                                    appearance,
                                    theme,
                                )
                            )
                            .with_margin_bottom(8.)
                            .finish(),
                        )
                        .with_child(
                            Container::new({
                                let formatted_text = FormattedText::new([FormattedTextLine::Line(vec![
                                    FormattedTextFragment::plain_text("Access to "),
                                    FormattedTextFragment::hyperlink(
                                        "Reload Credits".to_string(),
                                        "https://docs.warp.dev/support-and-community/plans-and-billing/add-on-credits".to_string(),
                                    ),
                                ])]);
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
                                            formatted_text,
                                            14.,
                                            appearance.ui_font_family(),
                                            appearance.ui_font_family(),
                                            blended_colors::text_sub(theme, blended_colors::neutral_1(theme)),
                                            HighlightedHyperlink::default(),
                                        )
                                        .register_default_click_handlers(|url, ctx, _| {
                                            ctx.dispatch_typed_action(FreeTierLimitHitModalAction::OpenUrl(url.url.clone()));
                                        })
                                        .finish(),
                                    )
                                    .finish()
                            })
                            .with_margin_bottom(8.)
                            .finish(),
                        )
                        .with_child(
                            Container::new({
                                let formatted_text = FormattedText::new([FormattedTextLine::Line(vec![
                                    FormattedTextFragment::hyperlink(
                                        "Extended cloud agents access".to_string(),
                                        "https://www.warp.dev/oz".to_string(),
                                    ),
                                ])]);
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
                                            formatted_text,
                                            14.,
                                            appearance.ui_font_family(),
                                            appearance.ui_font_family(),
                                            blended_colors::text_sub(theme, blended_colors::neutral_1(theme)),
                                            HighlightedHyperlink::default(),
                                        )
                                        .register_default_click_handlers(|url, ctx, _| {
                                            ctx.dispatch_typed_action(FreeTierLimitHitModalAction::OpenUrl(url.url.clone()));
                                        })
                                        .finish(),
                                    )
                                    .finish()
                            })
                            .finish(),
                        )
                        .finish(),
                )
                .with_child(
                    Align::new(
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
                            .with_centered_text_label(
                                crate::i18n::tr_static(app, "Upgrade plan").to_string(),
                            )
                            .build()
                            .with_cursor(Cursor::PointingHand)
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(FreeTierLimitHitModalAction::OpenUpgrade)
                            })
                            .finish(),
                    )
                    .bottom_left()
                    .finish(),
                )
                .finish(),
        )
        .with_background_color(blended_colors::neutral_1(theme))
        .with_corner_radius(CornerRadius::with_left(Radius::Pixels(10.)))
        .with_uniform_padding(32.)
        .finish()
    }

    fn render_right_panel(&self) -> Box<dyn Element> {
        ConstrainedBox::new(
            Image::new(
                bundled_or_fetched_asset!("png/free_tier_to_build.png"),
                CacheOption::BySize,
            )
            .with_corner_radius(CornerRadius::with_right(Radius::Pixels(10.)))
            .finish(),
        )
        .with_width(RIGHT_PANEL_WIDTH)
        .with_height(MODAL_HEIGHT)
        .finish()
    }
}

impl Entity for FreeTierLimitHitModal {
    type Event = FreeTierLimitHitModalEvent;
}

impl View for FreeTierLimitHitModal {
    fn ui_name() -> &'static str {
        "FreeTierLimitHitModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::handle(app).as_ref(app);
        let theme = appearance.theme();

        let close_button = appearance
            .ui_builder()
            .close_button(BUTTON_DIAMETER, self.state_handles.close_button.clone())
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(FreeTierLimitHitModalAction::Close))
            .finish();

        let mut modal = Stack::new();
        modal.add_child(
            Container::new(
                ConstrainedBox::new(
                    Flex::row()
                        .with_child(
                            ConstrainedBox::new(self.render_left_panel(app))
                                .with_width(LEFT_PANEL_WIDTH)
                                .with_height(MODAL_HEIGHT)
                                .finish(),
                        )
                        .with_child(self.render_right_panel())
                        .finish(),
                )
                .with_width(LEFT_PANEL_WIDTH + RIGHT_PANEL_WIDTH)
                .with_height(MODAL_HEIGHT)
                .finish(),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
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

        Container::new(Align::new(stack.finish()).finish())
            .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
            .finish()
    }
}

impl TypedActionView for FreeTierLimitHitModal {
    type Action = FreeTierLimitHitModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FreeTierLimitHitModalAction::Close => {
                ctx.emit(FreeTierLimitHitModalEvent::Close);

                send_telemetry_from_ctx!(TelemetryEvent::FreeTierLimitHitInterstitialClosed, ctx);
            }
            FreeTierLimitHitModalAction::OpenUpgrade => {
                let upgrade_url = Self::get_upgrade_url(ctx);
                ctx.open_url(&upgrade_url);
                ctx.emit(FreeTierLimitHitModalEvent::Close);

                send_telemetry_from_ctx!(
                    TelemetryEvent::FreeTierLimitHitInterstitialUpgradeButtonClicked,
                    ctx
                );
            }
            FreeTierLimitHitModalAction::OpenUrl(url) => {
                ctx.open_url(url);
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum FreeTierLimitHitModalEvent {
    MaybeOpen,
    Close,
}

#[derive(Clone, Debug)]
pub enum FreeTierLimitHitModalAction {
    Close,
    OpenUpgrade,
    OpenUrl(String),
}
