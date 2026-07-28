use ui_components::{Component as _, Options as _, button};
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::elements::{
    Border, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Empty, Flex, FormattedTextElement, Hoverable, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, Stack,
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
use super::upgrade_auth_prompt::render_upgrade_auth_prompt_bar;
use crate::model::OnboardingStateModel;
use crate::slides::{layout, slide_content};
use crate::telemetry::OnboardingEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferVariant {
    HeadStart,
    ChooseHowToStart,
}

impl OfferVariant {
    pub(crate) fn title(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "You've got a head start",
            OfferVariant::ChooseHowToStart => "Choose how to start",
        }
    }

    pub(crate) fn subtitle(self) -> Option<&'static str> {
        match self {
            OfferVariant::HeadStart => {
                Some("Your account includes AI usage to help you get started.")
            }
            OfferVariant::ChooseHowToStart => None,
        }
    }

    pub(crate) fn primary_label(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "Unlock the full AI experience",
            OfferVariant::ChooseHowToStart => "Use Warp with AI",
        }
    }

    pub(crate) fn primary_description(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => {
                "Get more monthly usage, expanded cloud agent access, and collaboration features."
            }
            OfferVariant::ChooseHowToStart => {
                "Warp Agent works locally or in the cloud with frontier and OSS models. Proactively fix terminal errors, implement changes, and ship verified code."
            }
        }
    }

    pub(crate) fn secondary_label(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "Start with included AI",
            OfferVariant::ChooseHowToStart => "Set up AI later",
        }
    }

    pub(crate) fn secondary_description(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => {
                "Explore with the AI usage included with your account and upgrade to add more anytime."
            }
            OfferVariant::ChooseHowToStart => {
                "Explore the terminal, bring your own inference, or use another CLI agent. Add AI usage and features anytime."
            }
        }
    }

    pub(crate) fn included_features(self) -> &'static [&'static str] {
        match self {
            OfferVariant::HeadStart => &[
                "Limited monthly AI usage for occasional tasks",
                "Access to premium and open-source models",
                "Use the Warp Agent locally and in the cloud",
            ],
            OfferVariant::ChooseHowToStart => &[],
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
    SelectPrimary,
    SelectSetUpLater,
    Back,
    GetWarping,
    CopyUpgradeUrl,
    PasteAuthTokenFromClipboard,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OfferChoice {
    #[default]
    Primary,
    SetUpLater,
}

#[derive(Clone, Debug)]
pub enum OfferSlideEvent {
    SetUpLaterSelected { variant: OfferVariant },
    CopyUpgradeUrlRequested,
    PasteAuthTokenFromClipboardRequested,
}

pub struct OfferSlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    primary_mouse_state: MouseStateHandle,
    secondary_mouse_state: MouseStateHandle,
    back_button: button::Button,
    get_warping_button: button::Button,
    selected_choice: OfferChoice,
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
            primary_mouse_state: MouseStateHandle::default(),
            secondary_mouse_state: MouseStateHandle::default(),
            back_button: button::Button::default(),
            get_warping_button: button::Button::default(),
            selected_choice: OfferChoice::default(),
            scroll_state: ClippedScrollStateHandle::new(),
            show_auth_prompt_bar: false,
            copy_url_mouse_state: MouseStateHandle::default(),
            paste_token_mouse_state: MouseStateHandle::default(),
        }
    }

    fn variant(&self, app: &AppContext) -> Option<OfferVariant> {
        self.onboarding_state.as_ref(app).offer_variant()
    }

    fn render_content(&self, appearance: &Appearance, variant: OfferVariant) -> Box<dyn Element> {
        slide_content::onboarding_slide_content(
            vec![
                Align::new(Self::render_header(appearance, variant))
                    .left()
                    .finish(),
                self.render_options(appearance, variant),
            ],
            self.render_bottom_nav(appearance),
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
        let mut header = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(title);
        if let Some(subtitle) = variant.subtitle() {
            let subtitle =
                FormattedTextElement::from_str(subtitle, appearance.ui_font_family(), 16.)
                    .with_color(internal_colors::text_sub(
                        theme,
                        theme.background().into_solid(),
                    ))
                    .with_weight(Weight::Normal)
                    .with_alignment(TextAlignment::Left)
                    .with_line_height_ratio(1.0)
                    .finish();
            header = header.with_child(Container::new(subtitle).with_margin_top(8.).finish());
        }
        let features = variant.included_features();
        if !features.is_empty() {
            let green = theme.ansi_fg_green();
            let mut feature_list = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(10.);
            for feature in features {
                let check = ConstrainedBox::new(Box::new(
                    Icon::CheckSkinny.to_warpui_icon(Fill::Solid(green)),
                ))
                .with_width(14.)
                .with_height(14.)
                .finish();
                let text = appearance
                    .ui_builder()
                    .paragraph(*feature)
                    .with_style(UiComponentStyles {
                        font_size: Some(13.),
                        ..Default::default()
                    })
                    .build()
                    .finish();
                feature_list.add_child(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(check)
                        .with_child(Container::new(text).with_margin_left(6.).finish())
                        .finish(),
                );
            }
            header = header.with_child(
                Container::new(feature_list.finish())
                    .with_margin_top(32.)
                    .finish(),
            );
        }
        header.finish()
    }

    fn render_options(&self, appearance: &Appearance, variant: OfferVariant) -> Box<dyn Element> {
        let primary = Self::render_option_card(
            appearance,
            variant.primary_label(),
            variant.primary_description(),
            self.selected_choice == OfferChoice::Primary,
            true,
            self.primary_mouse_state.clone(),
            OfferSlideAction::SelectPrimary,
        );
        let secondary = Self::render_option_card(
            appearance,
            variant.secondary_label(),
            variant.secondary_description(),
            self.selected_choice == OfferChoice::SetUpLater,
            false,
            self.secondary_mouse_state.clone(),
            OfferSlideAction::SelectSetUpLater,
        );
        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(Container::new(primary).with_margin_bottom(12.).finish())
                .with_child(secondary)
                .finish(),
        )
        .with_margin_top(38.)
        .finish()
    }

    fn render_bottom_nav(&self, appearance: &Appearance) -> Box<dyn Element> {
        let back = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(OfferSlideAction::Back);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );
        let enter = Keystroke::parse("enter").unwrap_or_default();
        let get_warping = self.get_warping_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Get Warping".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(OfferSlideAction::GetWarping);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(back)
            .with_child(get_warping)
            .finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_option_card(
        appearance: &Appearance,
        label: &'static str,
        description: &'static str,
        selected: bool,
        recommended: bool,
        mouse_state: MouseStateHandle,
        action: OfferSlideAction,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = selected.then(|| internal_colors::accent_overlay_1(theme));
        let border = if selected {
            theme.accent()
        } else {
            Fill::Solid(internal_colors::neutral_4(theme))
        };
        let label = appearance
            .ui_builder()
            .paragraph(label)
            .with_style(UiComponentStyles {
                font_size: Some(16.),
                font_weight: Some(Weight::Semibold),
                ..Default::default()
            })
            .build()
            .finish();
        let mut header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(label);
        if recommended {
            let green = theme.ansi_fg_green();
            let badge = Container::new(
                appearance
                    .ui_builder()
                    .paragraph("Recommended")
                    .with_style(UiComponentStyles {
                        font_size: Some(12.),
                        font_color: Some(green),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_horizontal_padding(8.)
            .with_vertical_padding(3.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(11.)))
            .with_background(Fill::Solid(green).with_opacity(10))
            .finish();
            header = header.with_child(badge);
        }
        let description = appearance
            .ui_builder()
            .paragraph(description)
            .with_style(UiComponentStyles {
                font_size: Some(14.),
                font_color: Some(internal_colors::text_sub(
                    theme,
                    theme.background().into_solid(),
                )),
                ..Default::default()
            })
            .build()
            .finish();
        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header.finish())
            .with_child(Container::new(description).with_margin_top(8.).finish())
            .finish();

        Hoverable::new(mouse_state, move |_| {
            let mut card = Container::new(content)
                .with_uniform_padding(24.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_border(Border::all(1.).with_border_fill(border));
            if let Some(background) = background {
                card = card.with_background(background);
            }
            card.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    fn render_visual(&self) -> Box<dyn Element> {
        layout::onboarding_right_panel_with_bg(
            Self::VISUAL_IMAGE_PATHS[0],
            layout::FOREGROUND_LAYOUT_DEFAULT,
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
        let Some(variant) = self.variant(ctx) else {
            return;
        };
        self.send_action(variant, variant.primary_action(), ctx);
        self.show_auth_prompt_bar = true;
        self.onboarding_state.update(ctx, |model, ctx| {
            model.request_upgrade(ctx);
        });
        ctx.notify();
    }

    fn set_up_later(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(variant) = self.variant(ctx) else {
            return;
        };
        self.send_action(variant, "set_up_later", ctx);
        ctx.emit(OfferSlideEvent::SetUpLaterSelected { variant });
    }

    fn select_choice(&mut self, choice: OfferChoice, ctx: &mut ViewContext<Self>) {
        if self.selected_choice == choice {
            return;
        }
        self.selected_choice = choice;
        ctx.notify();
    }

    fn get_warping(&mut self, ctx: &mut ViewContext<Self>) {
        match self.selected_choice {
            OfferChoice::Primary => self.request_upgrade(ctx),
            OfferChoice::SetUpLater => self.set_up_later(ctx),
        }
    }

    fn back(&mut self, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            model.back(ctx);
        });
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
        let Some(variant) = self.variant(app) else {
            return Empty::new().finish();
        };
        let appearance = Appearance::as_ref(app);
        let slide = layout::static_left(
            || self.render_content(appearance, variant),
            || self.render_visual(),
        );
        if !self.show_auth_prompt_bar {
            return slide;
        }

        let auth_prompt_bar = render_upgrade_auth_prompt_bar(
            appearance,
            self.copy_url_mouse_state.clone(),
            self.paste_token_mouse_state.clone(),
            Box::new(|ctx| {
                ctx.dispatch_typed_action(OfferSlideAction::CopyUpgradeUrl);
            }),
            Box::new(|ctx| {
                ctx.dispatch_typed_action(OfferSlideAction::PasteAuthTokenFromClipboard);
            }),
        );

        Stack::new()
            .with_child(slide)
            .with_child(Align::new(auth_prompt_bar).bottom_center().finish())
            .finish()
    }
}

impl OnboardingSlide for OfferSlide {
    fn on_up(&mut self, ctx: &mut ViewContext<Self>) {
        self.select_choice(OfferChoice::Primary, ctx);
    }

    fn on_down(&mut self, ctx: &mut ViewContext<Self>) {
        self.select_choice(OfferChoice::SetUpLater, ctx);
    }
    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.get_warping(ctx);
    }
}

impl TypedActionView for OfferSlide {
    type Action = OfferSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            OfferSlideAction::SelectPrimary => self.select_choice(OfferChoice::Primary, ctx),
            OfferSlideAction::SelectSetUpLater => {
                self.select_choice(OfferChoice::SetUpLater, ctx);
            }
            OfferSlideAction::Back => self.back(ctx),
            OfferSlideAction::GetWarping => self.get_warping(ctx),
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
