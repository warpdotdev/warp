use warp_core::ui::builder::UiBuilder;
use warpui::accessibility::{AccessibilityContent, WarpA11yRole};
use warpui::elements::{Align, Container, Element, Flex, MouseStateHandle, ParentElement};
use warpui::keymap::FixedBinding;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::appearance::Appearance;

fn text(app: &AppContext, key: &str) -> String {
    crate::localization::text_for_app(app, key)
}

// Constants for the *tada* emoji rendering
// Note: Long-term, we should convert this to a SVG, since there's no guarantee the emoji will be
// always renderable or that the size will be the same
const TADA: &str = "🎉";
const TADA_FONT_SIZE: f32 = 60.;
const TADA_MARGIN_TOP: f32 = 0.;
const TADA_MARGIN_BOTTOM: f32 = 50.;
const TITLE_FONT_SIZE: f32 = 20.;
const TITLE_MARGIN_BOTTOM: f32 = 25.;
const SUBTITLE_FONT_SIZE: f32 = 14.;
const SUBTITLE_MARGIN_BOTTOM: f32 = 40.;
const BUTTON_FONT_SIZE: f32 = 14.;
const BUTTON_HEIGHT: f32 = 45.;
const BUTTON_WIDTH: f32 = 240.;
const BUTTON_MARGIN_BOTTOM: f32 = 14.;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "enter",
        RewardAction::OpenThemePicker,
        id!("RewardView"),
    )]);
}

#[derive(Debug)]
pub enum RewardAction {
    OpenThemePicker,
}

pub enum RewardEvent {
    OpenThemePicker,
}

#[derive(Clone, Copy)]
pub enum RewardKind {
    SentReferralTheme,
    ReceivedReferralTheme,
}

pub struct RewardView {
    cta_mouse_state: MouseStateHandle,
    kind: RewardKind,
}

impl Default for RewardView {
    fn default() -> Self {
        Self::new()
    }
}

impl RewardView {
    pub fn new() -> Self {
        Self {
            cta_mouse_state: Default::default(),
            // Default to the Sent Referral Reward, which was previously the only thing this view
            // was used for. However, this will be updated when the view is shown, so the default
            // isn't super relevant
            kind: RewardKind::SentReferralTheme,
        }
    }

    pub fn update_reward_kind(&mut self, kind: RewardKind, ctx: &mut ViewContext<Self>) {
        self.kind = kind;
        ctx.notify();
    }

    fn subtitle(&self, app: &AppContext) -> String {
        match self.kind {
            RewardKind::SentReferralTheme => text(app, "reward.subtitle.sent_referral"),
            RewardKind::ReceivedReferralTheme => text(app, "reward.subtitle.received_referral"),
        }
    }

    fn render_icon(&self, ui_builder: &UiBuilder) -> Box<dyn Element> {
        Align::new(
            ui_builder
                .span(TADA)
                .with_style(UiComponentStyles {
                    font_size: Some(TADA_FONT_SIZE),
                    margin: Some(Coords {
                        top: TADA_MARGIN_TOP,
                        bottom: TADA_MARGIN_BOTTOM,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .finish()
    }

    fn render_title(&self, ui_builder: &UiBuilder, app: &AppContext) -> Box<dyn Element> {
        Align::new(
            ui_builder
                .span(text(app, "reward.title"))
                .with_style(UiComponentStyles {
                    font_size: Some(TITLE_FONT_SIZE),
                    margin: Some(Coords {
                        bottom: TITLE_MARGIN_BOTTOM,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .finish()
    }

    fn render_subtitle(&self, ui_builder: &UiBuilder, app: &AppContext) -> Box<dyn Element> {
        Align::new(
            ui_builder
                .paragraph(self.subtitle(app))
                .with_style(UiComponentStyles {
                    font_size: Some(SUBTITLE_FONT_SIZE),
                    margin: Some(Coords {
                        bottom: SUBTITLE_MARGIN_BOTTOM,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .finish()
    }

    fn render_button(&self, ui_builder: &UiBuilder, app: &AppContext) -> Box<dyn Element> {
        Align::new(
            Container::new(
                ui_builder
                    .button(ButtonVariant::Accent, self.cta_mouse_state.clone())
                    .with_centered_text_label(text(app, "reward.button.try_it_out"))
                    .with_style(UiComponentStyles {
                        height: Some(BUTTON_HEIGHT),
                        width: Some(BUTTON_WIDTH),
                        font_size: Some(BUTTON_FONT_SIZE),
                        ..Default::default()
                    })
                    .build()
                    .on_click(|ctx, _, _| ctx.dispatch_typed_action(RewardAction::OpenThemePicker))
                    .finish(),
            )
            .with_margin_bottom(BUTTON_MARGIN_BOTTOM)
            .finish(),
        )
        .finish()
    }
}

impl Entity for RewardView {
    type Event = RewardEvent;
}

impl View for RewardView {
    fn ui_name() -> &'static str {
        "RewardView"
    }

    fn accessibility_contents(&self, app: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new(
            format!("{} {}", text(app, "reward.title"), self.subtitle(app)),
            text(app, "reward.a11y.help"),
            WarpA11yRole::WindowRole,
        ))
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let ui_builder = Appearance::as_ref(app).ui_builder();
        Flex::column()
            .with_child(self.render_icon(ui_builder))
            .with_child(self.render_title(ui_builder, app))
            .with_child(self.render_subtitle(ui_builder, app))
            .with_child(self.render_button(ui_builder, app))
            .finish()
    }
}

impl TypedActionView for RewardView {
    type Action = RewardAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            RewardAction::OpenThemePicker => {
                ctx.emit(RewardEvent::OpenThemePicker);
            }
        }
    }
}
