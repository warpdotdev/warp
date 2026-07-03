use ui_components::{button, Component as _, Options as _};
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui_core::elements::{
    Border, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Flex, FormattedTextElement, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius,
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
use crate::model::{NoAiConfirmationSource, OnboardingStateModel};
use crate::slides::{bottom_nav, layout, slide_content};
use crate::visuals::{intention_terminal_visual, intention_visual};
use crate::{OnboardingIntention, AI_FEATURES};

#[derive(Debug, Clone)]
pub enum IntentionSlideAction {
    SelectOption { index: usize },
    BackClicked,
    NextClicked,
}

pub struct IntentionSlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    agent_driven_development_mouse_state: MouseStateHandle,
    classic_terminal_mouse_state: MouseStateHandle,
    back_button: button::Button,
    next_button: button::Button,
    scroll_state: ClippedScrollStateHandle,
}

impl IntentionSlide {
    pub(crate) fn new(onboarding_state: ModelHandle<OnboardingStateModel>) -> Self {
        Self {
            onboarding_state,
            agent_driven_development_mouse_state: MouseStateHandle::default(),
            classic_terminal_mouse_state: MouseStateHandle::default(),
            back_button: button::Button::default(),
            next_button: button::Button::default(),
            scroll_state: ClippedScrollStateHandle::new(),
        }
    }

    fn model_intention(&self, app: &AppContext) -> OnboardingIntention {
        *self.onboarding_state.as_ref(app).intention()
    }

    fn render_content(&self, appearance: &Appearance, selected_index: usize) -> Box<dyn Element> {
        let bottom_nav = Align::new(self.render_bottom_nav(appearance, selected_index)).finish();

        slide_content::onboarding_slide_content(
            vec![
                Align::new(self.render_header(appearance)).left().finish(),
                Align::new(self.render_options(appearance, selected_index)).finish(),
            ],
            bottom_nav,
            self.scroll_state.clone(),
            appearance,
        )
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let logo_fill = internal_colors::fg_overlay_4(theme);
        let logo = ConstrainedBox::new(Icon::WarpLogoLight.to_warpui_icon(logo_fill).finish())
            .with_width(64.)
            .with_height(64.)
            .finish();

        let title = appearance
            .ui_builder()
            .paragraph("Welcome to Warp")
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();

        let subtitle = FormattedTextElement::from_str(
            "How do you want to work?",
            appearance.ui_font_family(),
            16.,
        )
        .with_color(internal_colors::text_sub(
            theme,
            theme.background().into_solid(),
        ))
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.0)
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            // Offset icon built in padding to left align icon with title.
            .with_child(Container::new(logo).with_margin_left(-7.).finish())
            .with_child(Container::new(title).with_margin_top(11.).finish())
            .with_child(Container::new(subtitle).with_margin_top(16.).finish())
            .finish()
    }

    fn render_options(&self, appearance: &Appearance, selected_index: usize) -> Box<dyn Element> {
        let agent_card = self.render_agent_card(
            appearance,
            selected_index == 0,
            self.agent_driven_development_mouse_state.clone(),
        );

        let terminal_card = self.render_terminal_card(
            appearance,
            selected_index == 1,
            self.classic_terminal_mouse_state.clone(),
        );

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(Container::new(agent_card).with_margin_bottom(12.).finish())
                .with_child(terminal_card)
                .finish(),
        )
        .with_margin_top(38.)
        .finish()
    }

    /// Shared chrome for an intention-slide option card. Applies the selected/unselected
    /// background + border + rounded corners, wires up hover/click, and emits the
    /// `SelectOption` action for the provided `index`.
    fn render_card_chrome(
        appearance: &Appearance,
        is_selected: bool,
        index: usize,
        mouse_state: MouseStateHandle,
        content: Box<dyn Element>,
    ) -> Box<dyn Element> {
        const RADIUS: f32 = 8.;

        let theme = appearance.theme();
        let background = if is_selected {
            Some(internal_colors::accent_overlay_1(theme))
        } else {
            None
        };
        let border_color = if is_selected {
            theme.accent()
        } else {
            Fill::Solid(internal_colors::neutral_4(theme))
        };

        Hoverable::new(mouse_state, move |_| {
            let mut container = Container::new(content)
                .with_uniform_padding(24.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(RADIUS)))
                .with_border(Border::all(1.).with_border_fill(border_color));
            if let Some(bg) = background {
                container = container.with_background(bg);
            }
            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(IntentionSlideAction::SelectOption { index });
        })
        .finish()
    }

    fn render_agent_card(
        &self,
        appearance: &Appearance,
        is_selected: bool,
        mouse_state: MouseStateHandle,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg_solid = theme.background().into_solid();
        let label_color = if is_selected {
            internal_colors::text_main(theme, bg_solid)
        } else {
            internal_colors::text_sub(theme, bg_solid)
        };
        let description_color = internal_colors::text_sub(theme, bg_solid);
        let checklist_color = label_color;
        let icon_fill = Fill::Solid(label_color);

        let header_row = {
            let label = appearance
                .ui_builder()
                .paragraph("Build faster with agents")
                .with_style(UiComponentStyles {
                    font_size: Some(16.),
                    font_weight: Some(Weight::Semibold),
                    font_color: Some(label_color),
                    ..Default::default()
                })
                .build()
                .finish();

            let mut icon_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center);
            for (i, icon) in [Icon::Oz, Icon::ClaudeLogo, Icon::OpenAILogo]
                .iter()
                .enumerate()
            {
                let el = ConstrainedBox::new(icon.to_warpui_icon(icon_fill).finish())
                    .with_width(16.)
                    .with_height(16.)
                    .finish();
                icon_row = if i == 0 {
                    icon_row.with_child(el)
                } else {
                    icon_row.with_child(Container::new(el).with_margin_left(8.).finish())
                };
            }

            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(label)
                .with_child(icon_row.finish())
                .finish()
        };

        let description = FormattedTextElement::from_str(
            "Get AI features to accelerate terminal and agent-driven workflows:",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(description_color)
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.2)
        .finish();

        let checklist = {
            let items = AI_FEATURES;
            // When the agent card is selected, use the theme's green to match the
            // "Blended ANSI/green_fg" token in the design.
            let check_fill = if is_selected {
                Fill::Solid(theme.ansi_fg_green())
            } else {
                Fill::Solid(checklist_color)
            };
            let mut col = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Start);
            for &item in items {
                let icon_el = ConstrainedBox::new(Icon::Check.to_warpui_icon(check_fill).finish())
                    .with_width(16.)
                    .with_height(16.)
                    .finish();
                let text_el = appearance
                    .ui_builder()
                    .paragraph(item.to_string())
                    .with_style(UiComponentStyles {
                        font_size: Some(14.),
                        font_weight: Some(Weight::Normal),
                        font_color: Some(checklist_color),
                        ..Default::default()
                    })
                    .build()
                    .finish();
                let row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(icon_el)
                    .with_child(Container::new(text_el).with_margin_left(8.).finish())
                    .finish();
                col = col.with_child(
                    Container::new(row)
                        .with_padding_top(4.)
                        .with_padding_bottom(4.)
                        .finish(),
                );
            }
            col.finish()
        };

        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header_row)
            .with_child(Container::new(description).with_margin_top(12.).finish())
            .with_child(Container::new(checklist).with_margin_top(12.).finish())
            .finish();

        Self::render_card_chrome(appearance, is_selected, 0, mouse_state, content)
    }

    fn render_terminal_card(
        &self,
        appearance: &Appearance,
        is_selected: bool,
        mouse_state: MouseStateHandle,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg_solid = theme.background().into_solid();
        let text_color = if is_selected {
            internal_colors::text_main(theme, bg_solid)
        } else {
            internal_colors::text_sub(theme, bg_solid)
        };

        let label = appearance
            .ui_builder()
            .paragraph("Just use the terminal")
            .with_style(UiComponentStyles {
                font_size: Some(16.),
                font_weight: Some(Weight::Semibold),
                font_color: Some(text_color),
                ..Default::default()
            })
            .build()
            .finish();

        let badge = {
            let badge_text = appearance
                .ui_builder()
                .paragraph("No AI features")
                .with_style(UiComponentStyles {
                    font_size: Some(12.),
                    font_weight: Some(Weight::Semibold),
                    font_color: Some(text_color),
                    ..Default::default()
                })
                .build()
                .finish();
            Container::new(badge_text)
                .with_background(internal_colors::fg_overlay_2(theme))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
                .with_horizontal_padding(4.)
                .with_vertical_padding(2.)
                .finish()
        };

        let header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(label)
            .with_child(badge)
            .finish();

        let description = FormattedTextElement::from_str(
            "A modern terminal optimized for speed, context, and control without AI.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(text_color)
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.2)
        .finish();

        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header_row)
            .with_child(Container::new(description).with_margin_top(12.).finish())
            .finish();

        Self::render_card_chrome(appearance, is_selected, 1, mouse_state, content)
    }

    fn render_bottom_nav(
        &self,
        appearance: &Appearance,
        selected_index: usize,
    ) -> Box<dyn Element> {
        let back_button = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(IntentionSlideAction::BackClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let new_settings_modes = FeatureFlag::OpenWarpNewSettingsModes.is_enabled();
        let next_text = if !new_settings_modes && selected_index == 1 {
            "Get Warping"
        } else {
            "Next"
        };
        let enter = Keystroke::parse("enter").unwrap_or_default();
        let next_button = self.next_button.render(
            appearance,
            button::Params {
                content: button::Content::Label(next_text.into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(IntentionSlideAction::NextClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let is_terminal = selected_index == 1;
        let (step_index, step_count) = if new_settings_modes {
            if is_terminal {
                (0, 4)
            } else {
                (0, 5)
            }
        } else {
            (1, 4)
        };
        bottom_nav::onboarding_bottom_nav(
            appearance,
            step_index,
            step_count,
            Some(back_button),
            Some(next_button),
        )
    }

    /// All onboarding image paths used by the intention slide visual.
    pub(crate) const VISUAL_IMAGE_PATHS: &'static [&'static str] = &[
        "async/png/onboarding/welcome_agent.png",
        "async/png/onboarding/welcome_terminal.png",
    ];

    fn render_visual(&self, appearance: &Appearance, selected_index: usize) -> Box<dyn Element> {
        let theme = appearance.theme();

        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            let path = if selected_index == 1 {
                Self::VISUAL_IMAGE_PATHS[1]
            } else {
                Self::VISUAL_IMAGE_PATHS[0]
            };
            layout::onboarding_right_panel_with_bg(path, layout::FOREGROUND_LAYOUT_DEFAULT)
        } else {
            let panel_background = internal_colors::neutral_2(theme);
            let neutral = internal_colors::neutral_4(theme);
            let neutral_highlight = internal_colors::neutral_6(theme);
            let accent = internal_colors::accent(theme);

            let visual = if selected_index == 1 {
                intention_terminal_visual(
                    panel_background,
                    neutral,
                    neutral_highlight,
                    accent.into_solid(),
                )
            } else {
                let blue = theme.ansi_fg_blue();
                let green = theme.ansi_fg_green();
                let yellow = theme.ansi_fg_yellow();
                intention_visual(panel_background, neutral, blue, green, yellow)
            };

            Container::new(visual)
                .with_background_color(internal_colors::neutral_1(theme))
                .finish()
        }
    }
}

impl Entity for IntentionSlide {
    type Event = ();
}

impl View for IntentionSlide {
    fn ui_name() -> &'static str {
        "IntentionSlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let intention = self.model_intention(app);

        let selected_index = match intention {
            OnboardingIntention::AgentDrivenDevelopment => 0,
            OnboardingIntention::Terminal => 1,
        };

        // Background is rendered by the parent onboarding view (including background images).
        layout::static_left(
            || self.render_content(appearance, selected_index),
            || self.render_visual(appearance, selected_index),
        )
    }
}

impl IntentionSlide {
    fn select_option(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| match index {
            0 => model.set_intention_agent_driven_development(ctx),
            1 => model.set_intention_terminal(ctx),
            _ => {}
        });
        ctx.notify();
    }

    fn next(&mut self, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
                match model.intention() {
                    // "Just use the terminal" confirms leaving AI behind before advancing.
                    OnboardingIntention::Terminal => {
                        model.request_no_ai_confirmation(NoAiConfirmationSource::Intention, ctx);
                    }
                    // Agent intention routes to the next step (the AI-setup fork).
                    OnboardingIntention::AgentDrivenDevelopment => model.next(ctx),
                }
            } else {
                match model.intention() {
                    OnboardingIntention::Terminal => {
                        model.complete(ctx);
                    }
                    OnboardingIntention::AgentDrivenDevelopment => {
                        model.next(ctx);
                    }
                }
            }
        });
    }
}

impl OnboardingSlide for IntentionSlide {
    fn on_up(&mut self, ctx: &mut ViewContext<Self>) {
        let selected_index: usize = match self.model_intention(ctx) {
            OnboardingIntention::AgentDrivenDevelopment => 0,
            OnboardingIntention::Terminal => 1,
        };

        self.select_option(selected_index.saturating_sub(1), ctx);
    }

    fn on_down(&mut self, ctx: &mut ViewContext<Self>) {
        let selected_index: usize = match self.model_intention(ctx) {
            OnboardingIntention::AgentDrivenDevelopment => 0,
            OnboardingIntention::Terminal => 1,
        };

        self.select_option((selected_index + 1).min(1), ctx);
    }

    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.next(ctx);
    }
}

impl TypedActionView for IntentionSlide {
    type Action = IntentionSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            IntentionSlideAction::SelectOption { index } => {
                self.select_option(*index, ctx);
            }
            IntentionSlideAction::BackClicked => {
                let onboarding_state = self.onboarding_state.clone();
                onboarding_state.update(ctx, |model, ctx| {
                    model.back(ctx);
                });
            }
            IntentionSlideAction::NextClicked => {
                self.next(ctx);
            }
        }
    }
}
