use std::time::Duration;

use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    ChildView, Container, CrossAxisAlignment, Flex, MouseStateHandle, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::r#async::Timer;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::modal::Modal;
use crate::ui_components::blended_colors;

pub const MODAL_WIDTH: f32 = 400.;
pub const MODAL_PADDING: f32 = 24.;
pub const HEADER_FONT_SIZE: f32 = 16.;
pub const TEXT_FONT_SIZE: f32 = 14.;
pub const BUTTON_WIDTH: f32 = 172.;
pub const BUTTON_HEIGHT: f32 = 40.;

#[derive(Debug, Clone)]
pub enum InactivityModalEvent {
    TimedOut,
    StopSharing,
    ContinueSharing,
}

pub struct InactivityModal {
    modal: ViewHandle<Modal<InactivityModalBody>>,
}

impl InactivityModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let body = ctx.add_typed_action_view(|_| InactivityModalBody::new());
        ctx.subscribe_to_view(&body, |me, _, event, ctx| me.handle_body_event(event, ctx));

        let modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(None, body, ctx)
                .with_modal_style(UiComponentStyles {
                    width: Some(MODAL_WIDTH),
                    ..Default::default()
                })
                .with_body_style(UiComponentStyles {
                    padding: Some(Coords {
                        top: MODAL_PADDING,
                        bottom: MODAL_PADDING,
                        left: MODAL_PADDING,
                        right: MODAL_PADDING,
                    }),
                    ..Default::default()
                })
                .with_background_opacity(100)
                .close_modal_button_disabled()
        });

        Self { modal }
    }

    fn handle_body_event(&mut self, event: &InactivityModalBodyEvent, ctx: &mut ViewContext<Self>) {
        match event {
            InactivityModalBodyEvent::TimedOut => ctx.emit(InactivityModalEvent::TimedOut),
            InactivityModalBodyEvent::StopSharing => ctx.emit(InactivityModalEvent::StopSharing),
            InactivityModalBodyEvent::ContinueSharing => {
                ctx.emit(InactivityModalEvent::ContinueSharing)
            }
        }
    }

    pub fn reset_timer(&mut self, duration: Duration, ctx: &mut ViewContext<Self>) {
        self.modal.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.reset_timer(duration, ctx);
            });
        });
    }
}

impl Entity for InactivityModal {
    type Event = InactivityModalEvent;
}

impl View for InactivityModal {
    fn ui_name() -> &'static str {
        "SharerInactivityModal"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.modal).finish()
    }
}

#[derive(Debug, Clone)]
enum InactivityModalBodyAction {
    StopSharing,
    ContinueSharing,
}

#[derive(Debug, Clone)]
enum InactivityModalBodyEvent {
    TimedOut,
    StopSharing,
    ContinueSharing,
}

struct InactivityModalBody {
    stop_sharing_button: MouseStateHandle,
    continue_sharing_button: MouseStateHandle,
    is_countdown_running: bool,
    duration: Duration,
}

impl InactivityModalBody {
    fn new() -> Self {
        Self {
            stop_sharing_button: Default::default(),
            continue_sharing_button: Default::default(),
            is_countdown_running: false,
            duration: Duration::from_secs(0),
        }
    }

    fn reset_timer(&mut self, duration: Duration, ctx: &mut ViewContext<Self>) {
        self.is_countdown_running = true;
        self.duration = duration;
        self.update_countdown(ctx);
    }

    fn update_countdown(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.duration.is_zero() && self.is_countdown_running {
            self.duration -= Duration::from_secs(1);
            ctx.notify();

            let _ = ctx.spawn(
                async move {
                    Timer::after(std::time::Duration::from_secs(1)).await;
                },
                |me, _, ctx| me.update_countdown(ctx),
            );
        } else if self.is_countdown_running {
            self.is_countdown_running = false;
            ctx.emit(InactivityModalBodyEvent::TimedOut);
        }
    }

    fn render_countdown(&self, appearance: &Appearance) -> Box<dyn Element> {
        let text = format!(
            "Sharing will end in {}:{:02} due to inactivity.",
            self.duration.as_secs() / 60,
            self.duration.as_secs() % 60,
        );

        Container::new(
            Text::new_inline(text, appearance.ui_font_family(), TEXT_FONT_SIZE)
                .with_color(blended_colors::text_main(
                    appearance.theme(),
                    appearance.theme().background(),
                ))
                .with_style(Properties::default().weight(Weight::Normal))
                .finish(),
        )
        .with_padding_bottom(MODAL_PADDING)
        .finish()
    }

    fn render_stop_sharing_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .button(ButtonVariant::Outlined, self.stop_sharing_button.clone())
                .with_style(UiComponentStyles {
                    width: Some(BUTTON_WIDTH),
                    height: Some(BUTTON_HEIGHT),
                    font_size: Some(TEXT_FONT_SIZE),
                    font_weight: Some(Weight::Bold),
                    ..Default::default()
                })
                .with_centered_text_label(String::from("Stop sharing"))
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(InactivityModalBodyAction::StopSharing)
                })
                .finish(),
        )
        .with_padding_right(8.)
        .finish()
    }

    fn render_continue_sharing_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(
                ButtonVariant::Outlined,
                self.continue_sharing_button.clone(),
            )
            .with_style(UiComponentStyles {
                width: Some(BUTTON_WIDTH),
                height: Some(BUTTON_HEIGHT),
                font_size: Some(TEXT_FONT_SIZE),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .with_centered_text_label(String::from("Continue sharing"))
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(InactivityModalBodyAction::ContinueSharing)
            })
            .finish()
    }
}

impl Entity for InactivityModalBody {
    type Event = InactivityModalBodyEvent;
}

impl View for InactivityModalBody {
    fn ui_name() -> &'static str {
        "SharerInactivityModalBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let stop_sharing_button = self.render_stop_sharing_button(appearance);
        let continue_sharing_button = self.render_continue_sharing_button(appearance);
        let countdown = self.render_countdown(appearance);

        let header = Container::new(
            Text::new_inline(
                "Are you still there?",
                appearance.ui_font_family(),
                HEADER_FONT_SIZE,
            )
            .with_color(blended_colors::text_main(
                appearance.theme(),
                appearance.theme().background(),
            ))
            .with_style(Properties::default().weight(Weight::Bold))
            .finish(),
        )
        .with_padding_bottom(8.)
        .finish();

        let button_row = Flex::row()
            .with_child(stop_sharing_button)
            .with_child(continue_sharing_button)
            .finish();

        Flex::column()
            .with_child(header)
            .with_child(countdown)
            .with_child(button_row)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish()
    }
}

impl TypedActionView for InactivityModalBody {
    type Action = InactivityModalBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            InactivityModalBodyAction::StopSharing => {
                self.is_countdown_running = false;
                ctx.emit(InactivityModalBodyEvent::StopSharing)
            }
            InactivityModalBodyAction::ContinueSharing => {
                self.is_countdown_running = false;
                ctx.emit(InactivityModalBodyEvent::ContinueSharing)
            }
        }
    }
}
