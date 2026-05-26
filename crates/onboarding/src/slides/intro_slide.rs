use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use ui_components::{button, Component as _, Options as _};
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warpui::elements::shimmering_text::{
    ShimmerConfig, ShimmeringTextElement, ShimmeringTextStateHandle,
};
use warpui::elements::{
    Align, ChildAnchor, ConstrainedBox, Container, CrossAxisAlignment, Flex, FormattedTextElement,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Stack,
};
use warpui::keymap::Keystroke;
use warpui::text_layout::TextAlignment;
use warpui::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext,
};

use super::OnboardingSlide;
use crate::model::OnboardingStateModel;
use crate::OnboardingEvent;

#[derive(Clone, Debug)]
pub enum IntroSlideEvent {
    LoginRequested,
}

#[derive(Clone, Debug)]
pub enum IntroSlideAction {
    GetStartedClicked,
    LoginClicked,
}

pub struct IntroSlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    get_started_button: button::Button,
    shimmering_title_handle: ShimmeringTextStateHandle,
    login_mouse_state: MouseStateHandle,
}

impl IntroSlide {
    pub(crate) fn new(onboarding_state: ModelHandle<OnboardingStateModel>) -> Self {
        Self {
            onboarding_state,
            get_started_button: button::Button::default(),
            shimmering_title_handle: ShimmeringTextStateHandle::new(),
            login_mouse_state: MouseStateHandle::default(),
        }
    }
}

impl Entity for IntroSlide {
    type Event = IntroSlideEvent;
}

impl View for IntroSlide {
    fn ui_name() -> &'static str {
        "IntroSlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let content = self.render_centered_content(appearance);
        let constrained = ConstrainedBox::new(content).with_max_width(421.).finish();
        // Background is rendered by the parent onboarding view (including background images).
        let centered = Container::new(Align::new(constrained).finish()).finish();

        let sub_text_color = internal_colors::text_sub(theme, theme.background().into_solid());
        let ui_builder = appearance.ui_builder();
        let disclaimer_styles = UiComponentStyles {
            font_color: Some(sub_text_color),
            font_size: Some(12.),
            ..Default::default()
        };

        let login_row = Flex::row()
            .with_child(
                ui_builder
                    .span("Already have an account? ")
                    .with_style(disclaimer_styles)
                    .build()
                    .finish(),
            )
            .with_child(
                ui_builder
                    .link(
                        "Log in".into(),
                        None,
                        Some(Box::new(|ctx| {
                            ctx.dispatch_typed_action(IntroSlideAction::LoginClicked);
                        })),
                        self.login_mouse_state.clone(),
                    )
                    .soft_wrap(false)
                    .with_style(UiComponentStyles {
                        font_size: Some(12.),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish();

        let mut stack = Stack::new();
        stack.add_child(centered);
        stack.add_positioned_child(
            login_row,
            OffsetPositioning::offset_from_parent(
                vec2f(0., -28.),
                ParentOffsetBounds::ParentBySize,
                ParentAnchor::BottomMiddle,
                ChildAnchor::BottomMiddle,
            ),
        );
        stack.finish()
    }
}

impl IntroSlide {
    fn get_started_clicked(&mut self, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(OnboardingEvent::GetStartedClicked, ctx);

        self.onboarding_state.update(ctx, |model, ctx| {
            model.next(ctx);
        });
    }
}

impl OnboardingSlide for IntroSlide {
    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.get_started_clicked(ctx);
    }
}

impl IntroSlide {
    fn render_centered_content(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let logo_fill = internal_colors::fg_overlay_4(theme);
        let logo = ConstrainedBox::new(Icon::WarpLogoLight.to_warpui_icon(logo_fill).finish())
            .with_width(64.)
            .with_height(64.)
            .finish();

        let base_color: ColorU = internal_colors::fg_overlay_4(theme).into();
        let shimmer_color: ColorU = theme.foreground().into();
        let title = ShimmeringTextElement::new(
            "Welcome to Warp",
            appearance.ui_font_family(),
            32.,
            base_color,
            shimmer_color,
            ShimmerConfig::default(),
            self.shimmering_title_handle.clone(),
        )
        .finish();

        let subtitle_color = internal_colors::text_sub(theme, theme.background().into_solid());
        let subtitle = FormattedTextElement::from_str(
            "A modern terminal with state of the art agents built in.",
            appearance.ui_font_family(),
            16.,
        )
        .with_color(subtitle_color)
        .with_alignment(TextAlignment::Center)
        .with_line_height_ratio(1.0)
        .finish();

        let enter = Keystroke::parse("enter").unwrap_or_default();
        let get_started_button = self.get_started_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Get started".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(IntroSlideAction::GetStartedClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(logo)
            .with_child(title)
            .with_child(Container::new(subtitle).with_margin_top(12.).finish())
            .with_child(
                Container::new(get_started_button)
                    .with_margin_top(24.)
                    .finish(),
            )
            .finish()
    }
}

impl TypedActionView for IntroSlide {
    type Action = IntroSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            IntroSlideAction::GetStartedClicked => {
                self.get_started_clicked(ctx);
            }
            IntroSlideAction::LoginClicked => {
                send_telemetry_from_ctx!(OnboardingEvent::WelcomeLoginClicked, ctx);
                ctx.emit(IntroSlideEvent::LoginRequested);
            }
        }
    }
}
