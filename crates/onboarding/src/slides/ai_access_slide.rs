use ui_components::{Component as _, Options as _, button};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::elements::{
    Border, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Flex, FormattedTextElement, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, Stack,
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
use crate::model::{AiAccessChoice, OnboardingAuthState, OnboardingStateModel};
use crate::slides::{bottom_nav, layout, slide_content};

#[derive(Debug, Clone)]
pub enum AiAccessSlideAction {
    SelectSubscription,
    SelectSetUpLater,
    CopyUpgradeUrlClicked,
    PasteAuthTokenFromClipboardClicked,
    BackClicked,
    NextClicked,
}

/// Emitted to the parent onboarding view so the (app-crate) upgrade fallback
/// actions can be handled at the root level — the onboarding crate can't
/// reference them directly.
#[derive(Debug, Clone)]
pub enum AiAccessSlideEvent {
    CopyUpgradeUrlRequested,
    PasteAuthTokenFromClipboardRequested,
}

/// The "Choose how to access AI" slide (Warp Agent path). Forks between a paid
/// subscription and a "Set up later" option that lets the user explore Warp's
/// built-in AI before committing to a plan.
pub struct AiAccessSlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    subscription_mouse_state: MouseStateHandle,
    set_up_later_mouse_state: MouseStateHandle,
    back_button: button::Button,
    next_button: button::Button,
    scroll_state: ClippedScrollStateHandle,
    show_auth_prompt_bar: bool,
    copy_url_mouse_state: MouseStateHandle,
    paste_token_mouse_state: MouseStateHandle,
}

impl AiAccessSlide {
    pub(crate) fn new(onboarding_state: ModelHandle<OnboardingStateModel>) -> Self {
        Self {
            onboarding_state,
            subscription_mouse_state: MouseStateHandle::default(),
            set_up_later_mouse_state: MouseStateHandle::default(),
            back_button: button::Button::default(),
            next_button: button::Button::default(),
            scroll_state: ClippedScrollStateHandle::new(),
            show_auth_prompt_bar: false,
            copy_url_mouse_state: MouseStateHandle::default(),
            paste_token_mouse_state: MouseStateHandle::default(),
        }
    }

    // The final DES-816 visual exports have not landed yet, so the right panel
    // reuses the existing bundled agent welcome image.
    pub(crate) const VISUAL_IMAGE_PATHS: &'static [&'static str] =
        &["async/png/onboarding/welcome_agent.png"];

    fn choice(&self, app: &AppContext) -> AiAccessChoice {
        self.onboarding_state.as_ref(app).ai_access_choice()
    }

    fn render_content(
        &self,
        appearance: &Appearance,
        choice: AiAccessChoice,
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

        let title = appearance
            .ui_builder()
            .paragraph("Get AI access")
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();

        let subtitle = FormattedTextElement::from_str(
            "Save with a recurring plan, or explore Warp's AI before committing.",
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
            .with_child(title)
            .with_child(Container::new(subtitle).with_margin_top(16.).finish())
            .finish()
    }

    fn render_options(&self, appearance: &Appearance, choice: AiAccessChoice) -> Box<dyn Element> {
        let subscription_card = self
            .render_subscription_card(appearance, matches!(choice, AiAccessChoice::Subscription));

        let set_up_later_card =
            self.render_set_up_later_card(appearance, matches!(choice, AiAccessChoice::SetUpLater));

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(
                    Container::new(subscription_card)
                        .with_margin_bottom(12.)
                        .finish(),
                )
                .with_child(set_up_later_card)
                .finish(),
        )
        .with_margin_top(38.)
        .finish()
    }

    /// Shared chrome for an option card: selected/unselected background + border,
    /// hover/click to select.
    fn render_card_chrome(
        appearance: &Appearance,
        is_selected: bool,
        mouse_state: MouseStateHandle,
        select_action: AiAccessSlideAction,
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

    fn render_subscription_card(
        &self,
        appearance: &Appearance,
        is_selected: bool,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg_solid = theme.background().into_solid();
        let label_color = if is_selected {
            internal_colors::text_main(theme, bg_solid)
        } else {
            internal_colors::text_sub(theme, bg_solid)
        };
        let description_color = internal_colors::text_sub(theme, bg_solid);

        let label = appearance
            .ui_builder()
            .paragraph("Subscription")
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
                .paragraph("Best value")
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

        let header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(label)
            .with_child(badge)
            .finish();

        let description = FormattedTextElement::from_str(
            "Starting at $18 / mo, available with monthly or annual plans. Includes base credits, \
             frontier models, cloud agents, collaboration, and more.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(description_color)
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.2)
        .finish();

        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(header_row)
            .with_child(Container::new(description).with_margin_top(12.).finish())
            .finish();

        Self::render_card_chrome(
            appearance,
            is_selected,
            self.subscription_mouse_state.clone(),
            AiAccessSlideAction::SelectSubscription,
            content,
        )
    }

    fn render_set_up_later_card(
        &self,
        appearance: &Appearance,
        is_selected: bool,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg_solid = theme.background().into_solid();
        let label_color = if is_selected {
            internal_colors::text_main(theme, bg_solid)
        } else {
            internal_colors::text_sub(theme, bg_solid)
        };
        let description_color = internal_colors::text_sub(theme, bg_solid);

        let label = appearance
            .ui_builder()
            .paragraph("Set up later")
            .with_style(UiComponentStyles {
                font_size: Some(16.),
                font_weight: Some(Weight::Semibold),
                font_color: Some(label_color),
                ..Default::default()
            })
            .build()
            .finish();

        let description = FormattedTextElement::from_str(
            "Explore Warp's built-in AI features before committing to a plan, or bring your own \
             inference.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(description_color)
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.2)
        .finish();

        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(label)
            .with_child(Container::new(description).with_margin_top(12.).finish())
            .finish();

        Self::render_card_chrome(
            appearance,
            is_selected,
            self.set_up_later_mouse_state.clone(),
            AiAccessSlideAction::SelectSetUpLater,
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
                        ctx.dispatch_typed_action(AiAccessSlideAction::BackClicked);
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
                        ctx.dispatch_typed_action(AiAccessSlideAction::NextClicked);
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

    fn render_visual(&self) -> Box<dyn Element> {
        layout::onboarding_right_panel_with_bg(
            Self::VISUAL_IMAGE_PATHS[0],
            layout::FOREGROUND_LAYOUT_DEFAULT,
        )
    }

    /// Full-width bar pinned below the slide's two-column layout. Shown after
    /// the user picks Subscription and clicks "Next", so they can fall back to
    /// copying the upgrade URL (or pasting the returned auth token) if the
    /// browser didn't launch automatically.
    fn render_auth_prompt_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        const BAR_HEIGHT: f32 = 40.;
        const ICON_SIZE: f32 = 14.;
        const FONT_SIZE: f32 = 12.;

        let theme = appearance.theme();
        let bar_bg = theme.surface_1();
        let bar_bg_solid = bar_bg.into_solid();
        let text_color = internal_colors::text_sub(theme, bar_bg_solid);
        let ui_builder = appearance.ui_builder();

        let text_styles = UiComponentStyles {
            font_color: Some(text_color),
            font_size: Some(FONT_SIZE),
            ..Default::default()
        };
        let link_styles = UiComponentStyles {
            font_size: Some(FONT_SIZE),
            ..Default::default()
        };

        let icon = ConstrainedBox::new(Box::new(
            Icon::AlertCircle.to_warpui_icon(Fill::Solid(text_color)),
        ))
        .with_width(ICON_SIZE)
        .with_height(ICON_SIZE)
        .finish();

        let copy_url_link = ui_builder
            .link(
                "copy the URL".into(),
                None,
                Some(Box::new(|ctx| {
                    ctx.dispatch_typed_action(AiAccessSlideAction::CopyUpgradeUrlClicked);
                })),
                self.copy_url_mouse_state.clone(),
            )
            .soft_wrap(false)
            .with_style(link_styles)
            .build()
            .finish();

        let paste_token_link = ui_builder
            .link(
                "Click here".into(),
                None,
                Some(Box::new(|ctx| {
                    ctx.dispatch_typed_action(
                        AiAccessSlideAction::PasteAuthTokenFromClipboardClicked,
                    );
                })),
                self.paste_token_mouse_state.clone(),
            )
            .soft_wrap(false)
            .with_style(link_styles)
            .build()
            .finish();

        let text_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(icon)
            .with_child(
                Container::new(
                    ui_builder
                        .span("If your browser hasn't launched, ")
                        .with_style(text_styles)
                        .build()
                        .finish(),
                )
                .with_margin_left(8.)
                .finish(),
            )
            .with_child(copy_url_link)
            .with_child(
                ui_builder
                    .span(" and open the page manually. ")
                    .with_style(text_styles)
                    .build()
                    .finish(),
            )
            .with_child(paste_token_link)
            .with_child(
                ui_builder
                    .span(" to paste your token from the browser.")
                    .with_style(text_styles)
                    .build()
                    .finish(),
            )
            .finish();

        let row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(text_row)
            .finish();

        ConstrainedBox::new(
            Container::new(row)
                .with_background(bar_bg)
                .with_border(Border::top(1.).with_border_color(internal_colors::neutral_4(theme)))
                .with_horizontal_padding(16.)
                .finish(),
        )
        .with_min_height(BAR_HEIGHT)
        .finish()
    }
}

impl Entity for AiAccessSlide {
    type Event = AiAccessSlideEvent;
}

impl View for AiAccessSlide {
    fn ui_name() -> &'static str {
        "AiAccessSlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let choice = self.choice(app);

        let slide = layout::static_left(
            || self.render_content(appearance, choice, app),
            || self.render_visual(),
        );

        // Overlay the fallback bar at the bottom (rather than adding it to the
        // column) so the slide layout stays stable whether or not it's shown.
        let show_bar = self.show_auth_prompt_bar
            && !matches!(
                self.onboarding_state.as_ref(app).auth_state(),
                OnboardingAuthState::PayingUser,
            );
        if !show_bar {
            return slide;
        }

        let mut stack = Stack::new();
        stack.add_child(slide);
        stack.add_child(
            Align::new(self.render_auth_prompt_bar(appearance))
                .bottom_center()
                .finish(),
        );
        stack.finish()
    }
}

impl AiAccessSlide {
    fn select_choice(&mut self, choice: AiAccessChoice, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            model.set_ai_access_choice(choice, ctx);
        });
        ctx.notify();
    }

    fn next(&mut self, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            model.next(ctx);
        });
    }

    /// Primary "Next" action. On the subscription path this advances when the
    /// user already has a plan, otherwise it launches checkout in the browser
    /// (the slide auto-advances once billing flips to a paying plan). The "Set
    /// up later" path always advances.
    fn advance_or_upgrade(&mut self, ctx: &mut ViewContext<Self>) {
        match self.choice(ctx) {
            AiAccessChoice::Subscription => {
                if matches!(
                    self.onboarding_state.as_ref(ctx).auth_state(),
                    OnboardingAuthState::PayingUser
                ) {
                    self.next(ctx);
                } else {
                    // Surface the manual-fallback bar in case the browser
                    // doesn't launch; it's hidden again once billing flips to
                    // PayingUser.
                    self.show_auth_prompt_bar = true;
                    self.onboarding_state.update(ctx, |model, ctx| {
                        model.request_upgrade(ctx);
                    });
                    ctx.notify();
                }
            }
            AiAccessChoice::SetUpLater => self.next(ctx),
        }
    }
}

impl OnboardingSlide for AiAccessSlide {
    fn on_up(&mut self, ctx: &mut ViewContext<Self>) {
        self.select_choice(AiAccessChoice::Subscription, ctx);
    }

    fn on_down(&mut self, ctx: &mut ViewContext<Self>) {
        self.select_choice(AiAccessChoice::SetUpLater, ctx);
    }

    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.advance_or_upgrade(ctx);
    }
}

impl TypedActionView for AiAccessSlide {
    type Action = AiAccessSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AiAccessSlideAction::SelectSubscription => {
                self.select_choice(AiAccessChoice::Subscription, ctx);
            }
            AiAccessSlideAction::SelectSetUpLater => {
                self.select_choice(AiAccessChoice::SetUpLater, ctx);
            }
            AiAccessSlideAction::CopyUpgradeUrlClicked => {
                ctx.emit(AiAccessSlideEvent::CopyUpgradeUrlRequested);
            }
            AiAccessSlideAction::PasteAuthTokenFromClipboardClicked => {
                ctx.emit(AiAccessSlideEvent::PasteAuthTokenFromClipboardRequested);
            }
            AiAccessSlideAction::BackClicked => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.back(ctx);
                });
            }
            AiAccessSlideAction::NextClicked => {
                self.advance_or_upgrade(ctx);
            }
        }
    }
}
