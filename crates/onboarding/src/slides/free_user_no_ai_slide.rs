use ui_components::{button, Component as _, Options as _};
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui_core::elements::{
    Border, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DropShadow, Flex, FormattedTextElement, Hoverable, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, SizeConstraintCondition, SizeConstraintSwitch,
};
use warpui_core::fonts::Weight;
use warpui_core::keymap::Keystroke;
use warpui_core::platform::Cursor;
use warpui_core::prelude::Align;
use warpui_core::text_layout::TextAlignment;
use warpui_core::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui_core::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext,
};

use super::OnboardingSlide;
use crate::model::OnboardingStateModel;
use crate::slides::{bottom_nav, layout, slide_content};
use crate::telemetry::OnboardingEvent;
use crate::OnboardingIntention;

const SUBSCRIBE_ITEMS: &[&str] = &[
    "1,500 credits per month",
    "Access to frontier OpenAI, Anthropic, and Google models",
    "Access to Reload credits and volume-based discounts",
    "Extended cloud agents access",
    "Highest codebase indexing limits",
    "Unlimited Warp Drive objects and collaboration",
    "Private email support",
    "Unlimited cloud conversation storage",
];

#[derive(Debug, Clone)]
pub enum FreeUserNoAiSlideAction {
    SelectAgent,
    SelectTerminal,
    BackClicked,
    NextClicked,
    UpgradeClicked,
}

pub struct FreeUserNoAiSlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    agent_mouse_state: MouseStateHandle,
    classic_terminal_mouse_state: MouseStateHandle,
    subscribe_panel_mouse_state: MouseStateHandle,
    back_button: button::Button,
    next_button: button::Button,
    subscribe_nav_button: button::Button,
    scroll_state: ClippedScrollStateHandle,
}

impl FreeUserNoAiSlide {
    pub(crate) fn new(onboarding_state: ModelHandle<OnboardingStateModel>) -> Self {
        Self {
            onboarding_state,
            agent_mouse_state: MouseStateHandle::default(),
            classic_terminal_mouse_state: MouseStateHandle::default(),
            subscribe_panel_mouse_state: MouseStateHandle::default(),
            back_button: button::Button::default(),
            next_button: button::Button::default(),
            subscribe_nav_button: button::Button::default(),
            scroll_state: ClippedScrollStateHandle::new(),
        }
    }

    fn model_intention(&self, app: &AppContext) -> OnboardingIntention {
        *self.onboarding_state.as_ref(app).intention()
    }

    fn render_content(
        &self,
        appearance: &Appearance,
        selected_index: usize,
        // i.e. when window is small, the "subscribe" button replaces the "next" button instead of just
        // living on the CTA in the right pane
        subscribe_in_nav: bool,
        agent_price_badge: &str,
    ) -> Box<dyn Element> {
        let bottom_nav =
            Align::new(self.render_bottom_nav(appearance, selected_index, subscribe_in_nav))
                .finish();

        slide_content::onboarding_slide_content(
            vec![
                Align::new(self.render_header(appearance)).left().finish(),
                Align::new(self.render_options(appearance, selected_index, agent_price_badge))
                    .finish(),
            ],
            bottom_nav,
            self.scroll_state.clone(),
            appearance,
        )
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .paragraph("Let's get started.")
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_options(
        &self,
        appearance: &Appearance,
        selected_index: usize,
        agent_price_badge: &str,
    ) -> Box<dyn Element> {
        let agent_button = self.render_option_button(
            appearance,
            0,
            Icon::Code2,
            "Agent driven development with Warp's built-in agent",
            "Iterate, plan, and build with Oz: Warp's built-in agent. Available locally or in the cloud.",
            agent_price_badge.to_string(),
            true, // badge is green
            self.agent_mouse_state.clone(),
            selected_index,
            FreeUserNoAiSlideAction::SelectAgent,
        );
        let terminal_button = self.render_option_button(
            appearance,
            1,
            Icon::Terminal,
            "Classic terminal with third-party agents",
            "A modern terminal that supports third-party agents (Claude Code, Codex, Gemini CLI) and classic terminal workflows.",
            "Free".to_string(),
            false, // badge is gray
            self.classic_terminal_mouse_state.clone(),
            selected_index,
            FreeUserNoAiSlideAction::SelectTerminal,
        );

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(
                    Container::new(agent_button)
                        .with_margin_bottom(12.)
                        .finish(),
                )
                .with_child(terminal_button)
                .finish(),
        )
        .with_margin_top(32.)
        .finish()
    }

    fn render_badge(
        &self,
        appearance: &Appearance,
        text: String,
        text_color: warpui_core::color::ColorU,
        border_color: Fill,
    ) -> Box<dyn Element> {
        let label = appearance
            .ui_builder()
            .paragraph(text)
            .with_style(UiComponentStyles {
                font_size: Some(11.),
                font_weight: Some(Weight::Normal),
                font_color: Some(text_color),
                ..Default::default()
            })
            .build()
            .finish();

        Container::new(label)
            .with_horizontal_padding(8.)
            .with_vertical_padding(3.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_border(Border::all(1.).with_border_fill(border_color))
            .finish()
    }
    #[allow(clippy::too_many_arguments)]
    fn render_option_button(
        &self,
        appearance: &Appearance,
        index: usize,
        icon: Icon,
        label: &'static str,
        description: &'static str,
        badge_text: String,
        badge_green: bool,
        mouse_state: MouseStateHandle,
        selected_index: usize,
        action: FreeUserNoAiSlideAction,
    ) -> Box<dyn Element> {
        const RADIUS: f32 = 8.;

        let theme = appearance.theme();
        let is_selected = selected_index == index;

        let text_fill = if is_selected {
            internal_colors::accent_fg_strong(theme)
        } else {
            internal_colors::text_sub(theme, theme.background().into_solid()).into()
        };
        let text_color = text_fill.into_solid();
        let background = is_selected.then(|| internal_colors::accent_overlay_1(theme));
        let border_color = if is_selected {
            theme.accent()
        } else {
            Fill::Solid(internal_colors::neutral_4(theme))
        };
        let ui_font_family = appearance.ui_font_family();

        // For green badges, always use green. For others ("Free"), follow the
        // selected state so the chip looks active when the option is selected.
        let badge_color = if badge_green {
            theme.ansi_fg_green()
        } else {
            text_fill.into_solid()
        };
        let badge = self.render_badge(
            appearance,
            badge_text,
            badge_color,
            Fill::Solid(badge_color),
        );

        Hoverable::new(mouse_state, move |_| {
            let icon_el = ConstrainedBox::new(icon.to_warpui_icon(text_fill).finish())
                .with_width(20.)
                .with_height(20.)
                .finish();

            let top_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(icon_el)
                .with_child(badge)
                .finish();

            let label_el = appearance
                .ui_builder()
                .paragraph(label)
                .with_style(UiComponentStyles {
                    font_size: Some(14.),
                    font_weight: Some(Weight::Normal),
                    font_color: Some(text_color),
                    ..Default::default()
                })
                .build()
                .finish();

            let description_el = FormattedTextElement::from_str(description, ui_font_family, 12.)
                .with_color(text_color)
                .with_weight(Weight::Normal)
                .with_alignment(TextAlignment::Left)
                .with_line_height_ratio(1.4)
                .finish();

            let content = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(top_row)
                .with_child(Container::new(label_el).with_margin_top(8.).finish())
                .with_child(Container::new(description_el).with_margin_top(4.).finish())
                .finish();

            let mut container = Container::new(content)
                .with_uniform_padding(16.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(RADIUS)))
                .with_border(Border::all(1.).with_border_fill(border_color));

            if let Some(bg) = background {
                container = container.with_background(bg);
            }

            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    fn render_bottom_nav(
        &self,
        appearance: &Appearance,
        selected_index: usize,
        subscribe_in_nav: bool,
    ) -> Box<dyn Element> {
        let back_button = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(FreeUserNoAiSlideAction::BackClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let enter = Keystroke::parse("enter").unwrap_or_default();
        let next_button = if selected_index == 1 {
            self.next_button.render(
                appearance,
                button::Params {
                    content: button::Content::Label("Get Warping".into()),
                    theme: &button::themes::Primary,
                    options: button::Options {
                        keystroke: Some(enter),
                        on_click: Some(Box::new(|ctx, _app, _pos| {
                            ctx.dispatch_typed_action(FreeUserNoAiSlideAction::NextClicked);
                        })),
                        ..button::Options::default(appearance)
                    },
                },
            )
        } else if subscribe_in_nav {
            self.subscribe_nav_button.render(
                appearance,
                button::Params {
                    content: button::Content::Label("Subscribe".into()),
                    theme: &button::themes::Primary,
                    options: button::Options {
                        on_click: Some(Box::new(|ctx, _app, _pos| {
                            ctx.dispatch_typed_action(FreeUserNoAiSlideAction::UpgradeClicked);
                        })),
                        ..button::Options::default(appearance)
                    },
                },
            )
        } else {
            self.next_button.render(
                appearance,
                button::Params {
                    content: button::Content::Label("Next".into()),
                    theme: &button::themes::Primary,
                    options: button::Options {
                        disabled: true,
                        keystroke: Some(enter),
                        ..button::Options::default(appearance)
                    },
                },
            )
        };

        bottom_nav::onboarding_bottom_nav(appearance, 1, 4, Some(back_button), Some(next_button))
    }

    fn render_subscribe_panel(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_font_family = appearance.ui_font_family();

        let card_bg = theme.surface_2();
        let text_main = internal_colors::text_main(theme, internal_colors::neutral_2(theme));
        let text_sub = internal_colors::text_sub(theme, internal_colors::neutral_2(theme));

        let title = FormattedTextElement::from_str(
            "Subscribe to access agent driven development in Warp.",
            ui_font_family,
            24.,
        )
        .with_color(text_main)
        .with_weight(Weight::Medium)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.3)
        .finish();

        let mut items_col = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(8.);

        for item_text in SUBSCRIBE_ITEMS {
            let bullet = appearance
                .ui_builder()
                .paragraph(format!("\u{2022} {item_text}"))
                .with_style(UiComponentStyles {
                    font_size: Some(14.),
                    font_weight: Some(Weight::Normal),
                    font_color: Some(text_sub),
                    ..Default::default()
                })
                .build()
                .finish();
            items_col = items_col.with_child(bullet);
        }

        let fg_color = theme.foreground().into_solid();
        let subscribe_btn = Hoverable::new(
            self.subscribe_panel_mouse_state.clone(),
            move |mouse_state| {
                let bg = if mouse_state.is_clicked() {
                    internal_colors::accent_overlay_3(theme)
                } else if mouse_state.is_hovered() {
                    internal_colors::accent_overlay_4(theme)
                } else {
                    theme.accent()
                };

                let label = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        warpui_core::elements::Text::new_inline(
                            "Subscribe",
                            appearance.ui_font_family(),
                            14.,
                        )
                        .with_color(fg_color)
                        .with_style(warpui_core::fonts::Properties {
                            weight: Weight::Semibold,
                            style: warpui_core::fonts::Style::Normal,
                        })
                        .with_selectable(false)
                        .finish(),
                    )
                    .finish();

                ConstrainedBox::new(
                    Container::new(label)
                        .with_horizontal_padding(12.)
                        .with_background(bg)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                        .finish(),
                )
                .with_height(32.)
                .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(FreeUserNoAiSlideAction::UpgradeClicked);
        })
        .finish();

        let card_content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(title)
            .with_child(
                Container::new(items_col.finish())
                    .with_margin_top(20.)
                    .finish(),
            )
            .with_child(Container::new(subscribe_btn).with_margin_top(24.).finish())
            .finish();

        let card = Container::new(card_content)
            .with_uniform_padding(28.)
            .with_background(card_bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .with_drop_shadow(DropShadow::default())
            .finish();

        Container::new(Align::new(ConstrainedBox::new(card).with_max_width(420.).finish()).finish())
            .with_background(theme.surface_1())
            .finish()
    }
}

impl Entity for FreeUserNoAiSlide {
    type Event = ();
}

impl View for FreeUserNoAiSlide {
    fn ui_name() -> &'static str {
        "FreeUserNoAiSlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let intention = self.model_intention(app);
        let agent_price_badge = self.onboarding_state.as_ref(app).agent_price_badge();
        let agent_price_badge = agent_price_badge.as_str();

        let selected_index = match intention {
            OnboardingIntention::AgentDrivenDevelopment => 0,
            OnboardingIntention::Terminal => 1,
        };

        // Wide (right panel visible): greyed-out Next in nav.
        // Narrow (right panel hidden): Subscribe in nav (the only CTA visible).
        let wide = layout::static_left(
            || self.render_content(appearance, selected_index, false, agent_price_badge),
            || self.render_subscribe_panel(appearance),
        );
        let narrow = layout::static_left(
            || self.render_content(appearance, selected_index, true, agent_price_badge),
            || self.render_subscribe_panel(appearance),
        );

        SizeConstraintSwitch::new(
            wide,
            vec![(
                SizeConstraintCondition::WidthLessThan(layout::TWO_COLUMN_MIN_WIDTH),
                narrow,
            )],
        )
        .finish()
    }
}

impl OnboardingSlide for FreeUserNoAiSlide {
    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        let intention = self.model_intention(ctx);
        if matches!(intention, OnboardingIntention::Terminal) {
            self.onboarding_state
                .update(ctx, |model, ctx| model.complete(ctx));
        }
    }

    fn on_up(&mut self, ctx: &mut ViewContext<Self>) {
        let intention = self.model_intention(ctx);
        if matches!(intention, OnboardingIntention::Terminal) {
            self.onboarding_state.update(ctx, |model, ctx| {
                model.set_intention_agent_driven_development(ctx)
            });
            ctx.notify();
        }
    }

    fn on_down(&mut self, ctx: &mut ViewContext<Self>) {
        let intention = self.model_intention(ctx);
        if matches!(intention, OnboardingIntention::AgentDrivenDevelopment) {
            self.onboarding_state
                .update(ctx, |model, ctx| model.set_intention_terminal(ctx));
            ctx.notify();
        }
    }
}

impl TypedActionView for FreeUserNoAiSlide {
    type Action = FreeUserNoAiSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FreeUserNoAiSlideAction::SelectAgent => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_intention_agent_driven_development(ctx);
                });
                ctx.notify();
            }
            FreeUserNoAiSlideAction::SelectTerminal => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_intention_terminal(ctx);
                });
                ctx.notify();
            }
            FreeUserNoAiSlideAction::BackClicked => {
                self.onboarding_state
                    .update(ctx, |model, ctx| model.back(ctx));
            }
            FreeUserNoAiSlideAction::NextClicked => {
                self.onboarding_state
                    .update(ctx, |model, ctx| model.complete(ctx));
            }
            FreeUserNoAiSlideAction::UpgradeClicked => {
                send_telemetry_from_ctx!(OnboardingEvent::FreeUserNoAiUpgradeClicked, ctx);
                self.onboarding_state
                    .update(ctx, |model, ctx| model.request_upgrade(ctx));
            }
        }
    }
}
