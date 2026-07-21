use ui_components::{Component as _, Options as _, button};
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::elements::{
    ClippedScrollStateHandle, Container, CrossAxisAlignment, Flex, FormattedTextElement,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Stack,
};
use warpui_core::fonts::Weight;
use warpui_core::keymap::Keystroke;
use warpui_core::prelude::Align;
use warpui_core::text_layout::TextAlignment;
use warpui_core::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui_core::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext,
};

use super::OnboardingSlide;
use super::upgrade_auth_prompt::render_upgrade_auth_prompt_bar;
use crate::model::OnboardingStateModel;
use crate::slides::{layout, slide_content};
use crate::telemetry::OnboardingEvent;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OfferVariant {
    HeadStart,
    #[default]
    ChooseHowToStart,
}

impl OfferVariant {
    pub(crate) fn title(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "You've got a head start",
            OfferVariant::ChooseHowToStart => "Choose how to start",
        }
    }

    pub(crate) fn subtitle(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "Your account comes with some free AI",
            OfferVariant::ChooseHowToStart => {
                "Warp's agent requires a plan. Pick how you want to start"
            }
        }
    }

    pub(crate) fn primary_label(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "Get more AI",
            OfferVariant::ChooseHowToStart => "Use Warp with AI",
        }
    }

    pub(crate) fn slide_name(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "head_start",
            OfferVariant::ChooseHowToStart => "choose_how_to_start",
        }
    }

    pub(crate) fn account_class(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "free_icp",
            OfferVariant::ChooseHowToStart => "free_standard",
        }
    }

    fn primary_action(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "get_more_ai",
            OfferVariant::ChooseHowToStart => "use_warp_with_ai",
        }
    }
}

#[derive(Clone, Debug)]
pub enum OfferSlideAction {
    Primary,
    SetUpLater,
    CopyUpgradeUrl,
    PasteAuthTokenFromClipboard,
}

#[derive(Clone, Debug)]
pub enum OfferSlideEvent {
    SetUpLaterSelected { variant: OfferVariant },
    CopyUpgradeUrlRequested,
    PasteAuthTokenFromClipboardRequested,
}

pub struct OfferSlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    primary_button: button::Button,
    set_up_later_button: button::Button,
    scroll_state: ClippedScrollStateHandle,
    show_auth_prompt_bar: bool,
    copy_url_mouse_state: MouseStateHandle,
    paste_token_mouse_state: MouseStateHandle,
}

impl OfferSlide {
    pub(crate) const VISUAL_IMAGE_PATHS: &'static [&'static str] =
        &["async/png/onboarding/welcome_agent.png"];

    pub(crate) fn new(onboarding_state: ModelHandle<OnboardingStateModel>) -> Self {
        Self {
            onboarding_state,
            primary_button: button::Button::default(),
            set_up_later_button: button::Button::default(),
            scroll_state: ClippedScrollStateHandle::new(),
            show_auth_prompt_bar: false,
            copy_url_mouse_state: MouseStateHandle::default(),
            paste_token_mouse_state: MouseStateHandle::default(),
        }
    }

    fn variant(&self, app: &AppContext) -> OfferVariant {
        self.onboarding_state.as_ref(app).offer_variant()
    }

    fn render_content(&self, appearance: &Appearance, variant: OfferVariant) -> Box<dyn Element> {
        slide_content::onboarding_slide_content(
            vec![
                Align::new(Self::render_header(appearance, variant))
                    .left()
                    .finish(),
            ],
            self.render_bottom_nav(appearance, variant),
            self.scroll_state.clone(),
            appearance,
        )
    }

    fn render_header(appearance: &Appearance, variant: OfferVariant) -> Box<dyn Element> {
        let theme = appearance.theme();
        let title = appearance
            .ui_builder()
            .paragraph(variant.title())
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();
        let subtitle =
            FormattedTextElement::from_str(variant.subtitle(), appearance.ui_font_family(), 16.)
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
            .with_child(title)
            .with_child(Container::new(subtitle).with_margin_top(16.).finish())
            .finish()
    }

    fn render_bottom_nav(
        &self,
        appearance: &Appearance,
        variant: OfferVariant,
    ) -> Box<dyn Element> {
        let set_up_later = self.set_up_later_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Set up later".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(OfferSlideAction::SetUpLater);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );
        let enter = Keystroke::parse("enter").unwrap_or_default();
        let primary = self.primary_button.render(
            appearance,
            button::Params {
                content: button::Content::Label(variant.primary_label().into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(OfferSlideAction::Primary);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(set_up_later)
            .with_child(Container::new(primary).with_margin_left(4.).finish())
            .finish()
    }

    fn render_visual(&self) -> Box<dyn Element> {
        layout::onboarding_right_panel_with_bg(
            Self::VISUAL_IMAGE_PATHS[0],
            layout::FOREGROUND_LAYOUT_DEFAULT,
        )
    }

    fn render_auth_prompt_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        render_upgrade_auth_prompt_bar(
            appearance,
            self.copy_url_mouse_state.clone(),
            self.paste_token_mouse_state.clone(),
            Box::new(|ctx| {
                ctx.dispatch_typed_action(OfferSlideAction::CopyUpgradeUrl);
            }),
            Box::new(|ctx| {
                ctx.dispatch_typed_action(OfferSlideAction::PasteAuthTokenFromClipboard);
            }),
        )
    }

    fn send_action(&self, variant: OfferVariant, action: &str, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(
            OnboardingEvent::OnboardingAction {
                slide_name: variant.slide_name().to_string(),
                action: action.to_string(),
                account_class: Some(variant.account_class().to_string()),
            },
            ctx
        );
    }

    fn request_upgrade(&mut self, ctx: &mut ViewContext<Self>) {
        let variant = self.variant(ctx);
        self.send_action(variant, variant.primary_action(), ctx);
        self.show_auth_prompt_bar = true;
        self.onboarding_state.update(ctx, |model, ctx| {
            model.request_upgrade(ctx);
        });
        ctx.notify();
    }

    fn set_up_later(&mut self, ctx: &mut ViewContext<Self>) {
        let variant = self.variant(ctx);
        self.send_action(variant, "set_up_later", ctx);
        ctx.emit(OfferSlideEvent::SetUpLaterSelected { variant });
    }
}

impl Entity for OfferSlide {
    type Event = OfferSlideEvent;
}

impl View for OfferSlide {
    fn ui_name() -> &'static str {
        "OfferSlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let variant = self.variant(app);
        let slide = layout::static_left(
            || self.render_content(appearance, variant),
            || self.render_visual(),
        );
        if !self.show_auth_prompt_bar {
            return slide;
        }

        Stack::new()
            .with_child(slide)
            .with_child(
                Align::new(self.render_auth_prompt_bar(appearance))
                    .bottom_center()
                    .finish(),
            )
            .finish()
    }
}

impl OnboardingSlide for OfferSlide {
    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.request_upgrade(ctx);
    }
}

impl TypedActionView for OfferSlide {
    type Action = OfferSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            OfferSlideAction::Primary => self.request_upgrade(ctx),
            OfferSlideAction::SetUpLater => self.set_up_later(ctx),
            OfferSlideAction::CopyUpgradeUrl => {
                ctx.emit(OfferSlideEvent::CopyUpgradeUrlRequested);
            }
            OfferSlideAction::PasteAuthTokenFromClipboard => {
                ctx.emit(OfferSlideEvent::PasteAuthTokenFromClipboardRequested);
            }
        }
    }
}

#[cfg(test)]
#[path = "offer_slide_tests.rs"]
mod tests;
