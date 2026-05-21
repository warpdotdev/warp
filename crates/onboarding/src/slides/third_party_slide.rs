use super::toggle_card::{render_toggle_card, ToggleCardSpec};
use super::OnboardingSlide;
use crate::model::{OnboardingStateEvent, OnboardingStateModel};
use crate::slides::{bottom_nav, layout, slide_content};
use crate::OnboardingIntention;

use ui_components::{button, Component as _, Options as _};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::prelude::Align;
use warpui::{
    elements::{
        ClippedScrollStateHandle, Container, CrossAxisAlignment, Flex, FormattedTextElement,
        MainAxisSize, MouseStateHandle, ParentElement,
    },
    fonts::Weight,
    keymap::Keystroke,
    text_layout::TextAlignment,
    ui_components::components::{UiComponent as _, UiComponentStyles},
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext,
};

/// Which setting card is currently expanded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingCard {
    CliToolbar,
    Notifications,
}

#[derive(Debug, Clone)]
pub enum ThirdPartySlideAction {
    SelectSettingCard { card: SettingCard },
    SetCliAgentToolbarEnabled { enabled: bool },
    SetShowAgentNotifications { enabled: bool },
    BackClicked,
    NextClicked,
}

pub struct ThirdPartySlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    selected_setting: Option<SettingCard>,
    cli_toolbar_card_mouse_state: MouseStateHandle,
    notifications_card_mouse_state: MouseStateHandle,
    cli_toolbar_seg_left_mouse: MouseStateHandle,
    cli_toolbar_seg_right_mouse: MouseStateHandle,
    notifications_seg_left_mouse: MouseStateHandle,
    notifications_seg_right_mouse: MouseStateHandle,
    back_button: button::Button,
    next_button: button::Button,
    scroll_state: ClippedScrollStateHandle,
}

impl ThirdPartySlide {
    pub(crate) fn new(
        onboarding_state: ModelHandle<OnboardingStateModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&onboarding_state, |_me, _model, event, ctx| {
            if matches!(event, OnboardingStateEvent::IntentionChanged) {
                ctx.notify();
            }
        });

        Self {
            onboarding_state,
            selected_setting: None,
            cli_toolbar_card_mouse_state: MouseStateHandle::default(),
            notifications_card_mouse_state: MouseStateHandle::default(),
            cli_toolbar_seg_left_mouse: MouseStateHandle::default(),
            cli_toolbar_seg_right_mouse: MouseStateHandle::default(),
            notifications_seg_left_mouse: MouseStateHandle::default(),
            notifications_seg_right_mouse: MouseStateHandle::default(),
            back_button: button::Button::default(),
            next_button: button::Button::default(),
            scroll_state: ClippedScrollStateHandle::new(),
        }
    }

    /// All onboarding image paths used by this slide's visual.
    pub(crate) const VISUAL_IMAGE_PATHS: &'static [&'static str] = &[
        "async/png/onboarding/thirdparty_toolbar_enabled_vertical.png",
        "async/png/onboarding/thirdparty_toolbar_enabled_horizontal.png",
        "async/png/onboarding/thirdparty_toolbar_disabled_vertical.png",
        "async/png/onboarding/thirdparty_toolbar_disabled_horizontal.png",
        "async/png/onboarding/thirdparty_notifications_enabled.png",
        "async/png/onboarding/thirdparty_notifications_disabled.png",
    ];

    fn cli_agent_toolbar_enabled(&self, app: &AppContext) -> bool {
        self.onboarding_state
            .as_ref(app)
            .agent_settings()
            .cli_agent_toolbar_enabled
    }

    fn show_agent_notifications(&self, app: &AppContext) -> bool {
        self.onboarding_state
            .as_ref(app)
            .agent_settings()
            .show_agent_notifications
    }

    fn model_intention(&self, app: &AppContext) -> OnboardingIntention {
        *self.onboarding_state.as_ref(app).intention()
    }

    fn render_content(
        &self,
        appearance: &Appearance,
        cli_toolbar_enabled: bool,
        show_agent_notifications: bool,
        intention: OnboardingIntention,
    ) -> Box<dyn Element> {
        let bottom_nav = Align::new(self.render_bottom_nav(appearance, intention)).finish();

        let mut sections = vec![
            self.render_header(appearance),
            self.render_toolbar_section(appearance, cli_toolbar_enabled),
        ];

        // Only show the notifications toggle for terminal intention.
        // For agent intention, notifications are always enabled.
        if matches!(intention, OnboardingIntention::Terminal) {
            sections.push(self.render_notifications_section(appearance, show_agent_notifications));
        }

        slide_content::onboarding_slide_content(
            sections,
            bottom_nav,
            self.scroll_state.clone(),
            appearance,
        )
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let title = appearance
            .ui_builder()
            .paragraph("Customize third party agents")
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();

        let subtitle = FormattedTextElement::from_str(
            "Select defaults for using agents like Claude Code, Codex, Gemini, and Hermes.",
            appearance.ui_font_family(),
            16.,
        )
        .with_color(internal_colors::text_sub(
            appearance.theme(),
            appearance.theme().background().into_solid(),
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

    fn render_toolbar_section(
        &self,
        appearance: &Appearance,
        cli_toolbar_enabled: bool,
    ) -> Box<dyn Element> {
        let is_selected = self.selected_setting == Some(SettingCard::CliToolbar);

        let card = render_toggle_card(
            appearance,
            ToggleCardSpec {
                title: "CLI agent toolbar",
                is_expanded: is_selected,
                is_left_selected: cli_toolbar_enabled,
                left_label: "Enabled",
                right_label: "Disabled",
                card_mouse_state: self.cli_toolbar_card_mouse_state.clone(),
                on_expand: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(ThirdPartySlideAction::SelectSettingCard {
                        card: SettingCard::CliToolbar,
                    });
                }),
                left_mouse: self.cli_toolbar_seg_left_mouse.clone(),
                right_mouse: self.cli_toolbar_seg_right_mouse.clone(),
                on_left: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(ThirdPartySlideAction::SetCliAgentToolbarEnabled {
                        enabled: true,
                    });
                }),
                on_right: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(ThirdPartySlideAction::SetCliAgentToolbarEnabled {
                        enabled: false,
                    });
                }),
                chips: vec![],
            },
        );

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(card)
                .finish(),
        )
        .with_margin_top(40.)
        .finish()
    }

    fn render_notifications_section(
        &self,
        appearance: &Appearance,
        show_agent_notifications: bool,
    ) -> Box<dyn Element> {
        let is_selected = self.selected_setting == Some(SettingCard::Notifications);

        let card = render_toggle_card(
            appearance,
            ToggleCardSpec {
                title: "Notifications",
                is_expanded: is_selected,
                is_left_selected: show_agent_notifications,
                left_label: "Enabled",
                right_label: "Disabled",
                card_mouse_state: self.notifications_card_mouse_state.clone(),
                on_expand: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(ThirdPartySlideAction::SelectSettingCard {
                        card: SettingCard::Notifications,
                    });
                }),
                left_mouse: self.notifications_seg_left_mouse.clone(),
                right_mouse: self.notifications_seg_right_mouse.clone(),
                on_left: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(ThirdPartySlideAction::SetShowAgentNotifications {
                        enabled: true,
                    });
                }),
                on_right: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(ThirdPartySlideAction::SetShowAgentNotifications {
                        enabled: false,
                    });
                }),
                chips: vec![],
            },
        );

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(card)
                .finish(),
        )
        .with_margin_top(16.)
        .finish()
    }

    fn render_bottom_nav(
        &self,
        appearance: &Appearance,
        intention: OnboardingIntention,
    ) -> Box<dyn Element> {
        let back_button = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(ThirdPartySlideAction::BackClicked);
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
                        ctx.dispatch_typed_action(ThirdPartySlideAction::NextClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let is_terminal = matches!(intention, OnboardingIntention::Terminal);
        let (step_index, step_count) = if is_terminal { (2, 4) } else { (3, 5) };
        bottom_nav::onboarding_bottom_nav(
            appearance,
            step_index,
            step_count,
            Some(back_button),
            Some(next_button),
        )
    }

    fn render_visual(
        &self,
        cli_toolbar_enabled: bool,
        show_agent_notifications: bool,
        vertical: bool,
    ) -> Box<dyn Element> {
        if self.selected_setting == Some(SettingCard::Notifications) {
            let path = if show_agent_notifications {
                Self::VISUAL_IMAGE_PATHS[4]
            } else {
                Self::VISUAL_IMAGE_PATHS[5]
            };
            layout::onboarding_right_panel_with_bg(path, layout::FOREGROUND_LAYOUT_CODE_REVIEW)
        } else {
            let path = match (cli_toolbar_enabled, vertical) {
                (true, true) => Self::VISUAL_IMAGE_PATHS[0],
                (true, false) => Self::VISUAL_IMAGE_PATHS[1],
                (false, true) => Self::VISUAL_IMAGE_PATHS[2],
                (false, false) => Self::VISUAL_IMAGE_PATHS[3],
            };
            layout::onboarding_right_panel_with_bg(path, layout::FOREGROUND_LAYOUT_THIRD_PARTY)
        }
    }
}

impl Entity for ThirdPartySlide {
    type Event = ();
}

impl View for ThirdPartySlide {
    fn ui_name() -> &'static str {
        "ThirdPartySlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let cli_toolbar_enabled = self.cli_agent_toolbar_enabled(app);
        let show_agent_notifications = self.show_agent_notifications(app);
        let intention = self.model_intention(app);
        let vertical = self
            .onboarding_state
            .as_ref(app)
            .ui_customization()
            .use_vertical_tabs;

        layout::static_left(
            || {
                self.render_content(
                    appearance,
                    cli_toolbar_enabled,
                    show_agent_notifications,
                    intention,
                )
            },
            || self.render_visual(cli_toolbar_enabled, show_agent_notifications, vertical),
        )
    }
}

impl ThirdPartySlide {
    fn select_setting_card(&mut self, card: SettingCard, ctx: &mut ViewContext<Self>) {
        self.selected_setting = Some(card);
        ctx.notify();
    }

    fn next(&mut self, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            model.next(ctx);
        });
    }
}

impl OnboardingSlide for ThirdPartySlide {
    fn on_up(&mut self, ctx: &mut ViewContext<Self>) {
        let new_card = match self.selected_setting {
            None => SettingCard::CliToolbar,
            Some(SettingCard::CliToolbar) => SettingCard::CliToolbar,
            Some(SettingCard::Notifications) => SettingCard::CliToolbar,
        };
        self.selected_setting = Some(new_card);
        ctx.notify();
    }

    fn on_down(&mut self, ctx: &mut ViewContext<Self>) {
        let is_terminal = matches!(self.model_intention(ctx), OnboardingIntention::Terminal);
        let new_card = match self.selected_setting {
            None => SettingCard::CliToolbar,
            Some(SettingCard::CliToolbar) => {
                if is_terminal {
                    SettingCard::Notifications
                } else {
                    SettingCard::CliToolbar
                }
            }
            Some(SettingCard::Notifications) => SettingCard::Notifications,
        };
        self.selected_setting = Some(new_card);
        ctx.notify();
    }

    fn on_left(&mut self, ctx: &mut ViewContext<Self>) {
        match self.selected_setting {
            Some(SettingCard::CliToolbar) => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_cli_agent_toolbar_enabled(true, ctx);
                });
                ctx.notify();
            }
            Some(SettingCard::Notifications) => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_show_agent_notifications(true, ctx);
                });
                ctx.notify();
            }
            None => {}
        }
    }

    fn on_right(&mut self, ctx: &mut ViewContext<Self>) {
        match self.selected_setting {
            Some(SettingCard::CliToolbar) => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_cli_agent_toolbar_enabled(false, ctx);
                });
                ctx.notify();
            }
            Some(SettingCard::Notifications) => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_show_agent_notifications(false, ctx);
                });
                ctx.notify();
            }
            None => {}
        }
    }

    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.next(ctx);
    }
}

impl TypedActionView for ThirdPartySlide {
    type Action = ThirdPartySlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ThirdPartySlideAction::SelectSettingCard { card } => {
                self.select_setting_card(*card, ctx);
            }
            ThirdPartySlideAction::SetCliAgentToolbarEnabled { enabled } => {
                let value = *enabled;
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_cli_agent_toolbar_enabled(value, ctx);
                });
                ctx.notify();
            }
            ThirdPartySlideAction::SetShowAgentNotifications { enabled } => {
                let value = *enabled;
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_show_agent_notifications(value, ctx);
                });
                ctx.notify();
            }
            ThirdPartySlideAction::BackClicked => {
                let onboarding_state = self.onboarding_state.clone();
                onboarding_state.update(ctx, |model, ctx| {
                    model.back(ctx);
                });
            }
            ThirdPartySlideAction::NextClicked => {
                self.next(ctx);
            }
        }
    }
}
