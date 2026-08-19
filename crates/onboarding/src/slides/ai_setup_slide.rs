use ui_components::{Component as _, Options as _, button};
use warp_core::ui::Icon;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::color::internal_colors;
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
use crate::model::{AiSetupChoice, OnboardingStateModel};
use crate::slides::{bottom_nav, layout, slide_content};

/// Checklist shown on the "Use Warp agent" card.
const WARP_AGENT_FEATURES: &[&str] = &[
    "Best harness for terminal tasks and agentic coding",
    "Frontier models from OpenAI, Anthropic, and Google",
    "Model routing across frontier and open-weight models",
    "Multi-agent orchestration",
];

#[derive(Debug, Clone)]
pub enum AiSetupSlideAction {
    SelectWarpAgent,
    SelectThirdParty,
    BackClicked,
    NextClicked,
}

/// The "Choose your AI setup" slide (DES-816 V3), shown on the AI-first path for
/// users enrolled in the FREE_AI_REMOVAL experiment arm. Forks between the Warp
/// agent (paid-plan path) and third-party agents (works on Free).
pub struct AiSetupSlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    warp_agent_mouse_state: MouseStateHandle,
    third_party_mouse_state: MouseStateHandle,
    back_button: button::Button,
    next_button: button::Button,
    scroll_state: ClippedScrollStateHandle,
}

impl AiSetupSlide {
    pub(crate) fn new(onboarding_state: ModelHandle<OnboardingStateModel>) -> Self {
        Self {
            onboarding_state,
            warp_agent_mouse_state: MouseStateHandle::default(),
            third_party_mouse_state: MouseStateHandle::default(),
            back_button: button::Button::default(),
            next_button: button::Button::default(),
            scroll_state: ClippedScrollStateHandle::new(),
        }
    }

    // The final DES-816 visual exports have not landed yet, so the right panel
    // reuses existing bundled assets that match each choice: the agent
    // experience for "Use Warp agent" and the CLI-agent toolbar for
    // "Use third party agents".
    pub(crate) const VISUAL_IMAGE_PATHS: &'static [&'static str] = &[
        "async/png/onboarding/welcome_agent.png",
        "async/png/onboarding/thirdparty_toolbar_enabled_vertical.png",
    ];

    fn choice(&self, app: &AppContext) -> AiSetupChoice {
        self.onboarding_state.as_ref(app).ai_setup_choice()
    }

    fn render_content(
        &self,
        appearance: &Appearance,
        choice: AiSetupChoice,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let bottom_nav = Align::new(self.render_bottom_nav(appearance, app)).finish();

        slide_content::onboarding_slide_content(
            vec![
                Align::new(self.render_header(appearance)).left().finish(),
                Align::new(self.render_options(appearance, choice)).finish(),
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
            .paragraph("Choose your AI setup")
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();

        let subtitle = FormattedTextElement::from_str(
            "Choose if you'd like to use Warp Agent or third party agents.",
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

    fn render_options(&self, appearance: &Appearance, choice: AiSetupChoice) -> Box<dyn Element> {
        let warp_agent_card = self.render_warp_agent_card(
            appearance,
            matches!(choice, AiSetupChoice::WarpAgent),
            self.warp_agent_mouse_state.clone(),
        );

        let third_party_card = self.render_third_party_card(
            appearance,
            matches!(choice, AiSetupChoice::ThirdParty),
            self.third_party_mouse_state.clone(),
        );

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(
                    Container::new(warp_agent_card)
                        .with_margin_bottom(12.)
                        .finish(),
                )
                .with_child(third_party_card)
                .finish(),
        )
        .with_margin_top(38.)
        .finish()
    }

    /// Shared chrome for an AI-setup option card. Applies the selected/unselected
    /// background + border + rounded corners, wires up hover/click, and emits the
    /// provided select action.
    fn render_card_chrome(
        appearance: &Appearance,
        is_selected: bool,
        mouse_state: MouseStateHandle,
        select_action: AiSetupSlideAction,
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
            ctx.dispatch_typed_action(select_action.clone());
        })
        .finish()
    }

    fn render_warp_agent_card(
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

        let header_row = {
            let label = appearance
                .ui_builder()
                .paragraph("Use Warp Agent")
                .with_style(UiComponentStyles {
                    font_size: Some(16.),
                    font_weight: Some(Weight::Semibold),
                    font_color: Some(label_color),
                    ..Default::default()
                })
                .build()
                .finish();

            let badge = {
                let green = theme.ansi_fg_green();
                let badge_text = appearance
                    .ui_builder()
                    .paragraph("Access more models")
                    .with_style(UiComponentStyles {
                        font_size: Some(12.),
                        font_weight: Some(Weight::Normal),
                        font_color: Some(green),
                        ..Default::default()
                    })
                    .build()
                    .finish();
                Container::new(badge_text)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(11.)))
                    .with_border(Border::all(1.).with_border_fill(Fill::Solid(green)))
                    .with_horizontal_padding(8.)
                    .with_vertical_padding(3.)
                    .finish()
            };

            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(label)
                .with_child(badge)
                .finish()
        };

        let description = FormattedTextElement::from_str(
            "State of the art agent harness deeply integrated into the terminal.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(description_color)
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.2)
        .finish();

        let checklist = {
            // When the card is selected, use the theme's green to match the
            // "Blended ANSI/green_fg" token in the design.
            let check_fill = if is_selected {
                Fill::Solid(theme.ansi_fg_green())
            } else {
                Fill::Solid(label_color)
            };
            let mut col = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Start);
            for &item in WARP_AGENT_FEATURES {
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
                        font_color: Some(label_color),
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

        Self::render_card_chrome(
            appearance,
            is_selected,
            mouse_state,
            AiSetupSlideAction::SelectWarpAgent,
            content,
        )
    }

    fn render_third_party_card(
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
            .paragraph("Use third party agents")
            .with_style(UiComponentStyles {
                font_size: Some(16.),
                font_weight: Some(Weight::Semibold),
                font_color: Some(text_color),
                ..Default::default()
            })
            .build()
            .finish();

        let description = FormattedTextElement::from_str(
            "Use agents like Claude Code, Codex, and Gemini.",
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
            .with_child(label)
            .with_child(Container::new(description).with_margin_top(12.).finish())
            .finish();

        Self::render_card_chrome(
            appearance,
            is_selected,
            mouse_state,
            AiSetupSlideAction::SelectThirdParty,
            content,
        )
    }

    fn render_bottom_nav(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let back_button = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(AiSetupSlideAction::BackClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let enter = Keystroke::parse("enter").unwrap_or_default();
        let next_button = self.next_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Next".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(AiSetupSlideAction::NextClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let (step_index, step_count) = self.onboarding_state.as_ref(app).progress();
        bottom_nav::onboarding_bottom_nav(
            appearance,
            step_index,
            step_count,
            Some(back_button),
            Some(next_button),
        )
    }

    fn render_visual(&self, choice: AiSetupChoice) -> Box<dyn Element> {
        match choice {
            AiSetupChoice::WarpAgent => layout::onboarding_right_panel_with_bg(
                Self::VISUAL_IMAGE_PATHS[0],
                layout::FOREGROUND_LAYOUT_DEFAULT,
            ),
            AiSetupChoice::ThirdParty => layout::onboarding_right_panel_with_bg(
                Self::VISUAL_IMAGE_PATHS[1],
                layout::FOREGROUND_LAYOUT_THIRD_PARTY,
            ),
        }
    }
}

impl Entity for AiSetupSlide {
    type Event = ();
}

impl View for AiSetupSlide {
    fn ui_name() -> &'static str {
        "AiSetupSlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let choice = self.choice(app);

        // Background is rendered by the parent onboarding view (including background images).
        layout::static_left(
            || self.render_content(appearance, choice, app),
            || self.render_visual(choice),
        )
    }
}

impl AiSetupSlide {
    fn select_choice(&mut self, choice: AiSetupChoice, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            model.set_ai_setup_choice(choice, ctx);
        });
        ctx.notify();
    }

    fn next(&mut self, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            model.next(ctx);
        });
    }
}

impl OnboardingSlide for AiSetupSlide {
    fn on_up(&mut self, ctx: &mut ViewContext<Self>) {
        self.select_choice(AiSetupChoice::WarpAgent, ctx);
    }

    fn on_down(&mut self, ctx: &mut ViewContext<Self>) {
        self.select_choice(AiSetupChoice::ThirdParty, ctx);
    }

    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.next(ctx);
    }
}

impl TypedActionView for AiSetupSlide {
    type Action = AiSetupSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AiSetupSlideAction::SelectWarpAgent => {
                self.select_choice(AiSetupChoice::WarpAgent, ctx);
            }
            AiSetupSlideAction::SelectThirdParty => {
                self.select_choice(AiSetupChoice::ThirdParty, ctx);
            }
            AiSetupSlideAction::BackClicked => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.back(ctx);
                });
            }
            AiSetupSlideAction::NextClicked => {
                self.next(ctx);
            }
        }
    }
}
