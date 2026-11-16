use warpui::elements::{
    Container, CrossAxisAlignment, Flex, MainAxisAlignment, MouseStateHandle, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::text::Span;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::{MODAL_PADDING, TEXT_FONT_SIZE};
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
const BUTTON_HEIGHT: f32 = 40.;
const BUTTON_WIDTH: f32 = 172.;

#[derive(Debug)]
pub enum SharerGrantBodyAction {
    Cancel,
    GrantRole { dont_show_again: bool },
    ToggleDontShowAgain,
}

#[derive(Debug)]
pub enum SharerGrantBodyEvent {
    Cancel,
    GrantRole { dont_show_again: bool },
}

pub struct SharerGrantBody {
    cancel_button_mouse_state: MouseStateHandle,
    approve_button_mouse_state: MouseStateHandle,
    dont_show_again_mouse_state: MouseStateHandle,
    dont_show_again: bool,
}

impl SharerGrantBody {
    pub fn new() -> Self {
        Self {
            cancel_button_mouse_state: Default::default(),
            approve_button_mouse_state: Default::default(),
            dont_show_again_mouse_state: Default::default(),
            dont_show_again: false,
        }
    }

    fn render_button_row(&self, appearance: &Appearance) -> Box<dyn Element> {
        let cancel_button = Container::new(
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Outlined,
                    self.cancel_button_mouse_state.clone(),
                )
                .with_style(UiComponentStyles {
                    font_size: Some(TEXT_FONT_SIZE),
                    font_weight: Some(Weight::Bold),
                    height: Some(BUTTON_HEIGHT),
                    width: Some(BUTTON_WIDTH),
                    ..Default::default()
                })
                .with_centered_text_label(String::from("Cancel"))
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(SharerGrantBodyAction::Cancel))
                .finish(),
        )
        .with_padding_right(8.)
        .finish();

        let dont_show_again = self.dont_show_again;
        let approve_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.approve_button_mouse_state.clone(),
            )
            .with_style(UiComponentStyles {
                font_size: Some(TEXT_FONT_SIZE),
                font_weight: Some(Weight::Bold),
                height: Some(BUTTON_HEIGHT),
                width: Some(BUTTON_WIDTH),
                ..Default::default()
            })
            .with_centered_text_label(String::from("Make Editor"))
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SharerGrantBodyAction::GrantRole { dont_show_again })
            })
            .finish();

        Flex::row()
            .with_child(cancel_button)
            .with_child(approve_button)
            .finish()
    }
}

impl Entity for SharerGrantBody {
    type Event = SharerGrantBodyEvent;
}

impl View for SharerGrantBody {
    fn ui_name() -> &'static str {
        "SharerGrantBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let button_row = self.render_button_row(appearance);

        let text1 = "This grants the ability to execute commands on your";
        let text2 = "behalf. Use with caution.";
        let text_body = Container::new(
            Flex::column()
                .with_child(
                    Text::new_inline(text1, appearance.ui_font_family(), TEXT_FONT_SIZE)
                        .with_color(blended_colors::text_main(
                            appearance.theme(),
                            appearance.theme().background(),
                        ))
                        .with_style(Properties::default().weight(Weight::Normal))
                        .finish(),
                )
                .with_child(
                    Text::new_inline(text2, appearance.ui_font_family(), TEXT_FONT_SIZE)
                        .with_color(blended_colors::text_main(
                            appearance.theme(),
                            appearance.theme().background(),
                        ))
                        .with_style(Properties::default().weight(Weight::Normal))
                        .finish(),
                )
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_padding_bottom(16.)
        .finish();

        let dont_show_again_checkbox = Container::new(
            appearance
                .ui_builder()
                .checkbox(
                    self.dont_show_again_mouse_state.clone(),
                    Some(TEXT_FONT_SIZE),
                )
                .with_label(Span::new("Don't show again.", Default::default()))
                .check(self.dont_show_again)
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(SharerGrantBodyAction::ToggleDontShowAgain)
                })
                .finish(),
        )
        .with_padding_bottom(MODAL_PADDING)
        .finish();

        Flex::column()
            .with_child(text_body)
            .with_child(dont_show_again_checkbox)
            .with_child(button_row)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish()
    }
}

impl TypedActionView for SharerGrantBody {
    type Action = SharerGrantBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SharerGrantBodyAction::Cancel => {
                ctx.emit(SharerGrantBodyEvent::Cancel);
                self.dont_show_again = false;
            }
            SharerGrantBodyAction::GrantRole { dont_show_again } => {
                ctx.emit(SharerGrantBodyEvent::GrantRole {
                    dont_show_again: *dont_show_again,
                });
            }
            SharerGrantBodyAction::ToggleDontShowAgain => {
                self.dont_show_again = !self.dont_show_again;
                ctx.notify();
            }
        }
    }
}
