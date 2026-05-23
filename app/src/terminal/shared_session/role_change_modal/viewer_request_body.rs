use session_sharing_protocol::common::Role;
use warpui::elements::{
    Container, CrossAxisAlignment, Flex, MainAxisAlignment, MouseStateHandle, ParentElement, Text,
};
use warpui::fonts::Weight;
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::{BODY_PADDING, HEADER_FONT_SIZE, MODAL_PADDING, TEXT_FONT_SIZE};
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;

pub const BUTTON_HEIGHT: f32 = 40.;
pub const BUTTON_WIDTH: f32 = 352.;

#[derive(Debug)]
pub enum ViewerRequestBodyAction {
    Cancel,
}

pub enum ViewerRequestBodyEvent {
    Cancel,
}

pub struct ViewerRequestBody {
    role: Role,
    display_name: String,
    mouse_state_handle: MouseStateHandle,
}

impl ViewerRequestBody {
    pub fn new() -> Self {
        Self {
            role: Default::default(),
            display_name: Default::default(),
            mouse_state_handle: Default::default(),
        }
    }

    fn role_label(&self) -> &str {
        match self.role {
            Role::Executor => "edit",
            _ => "view",
        }
    }

    pub fn open(&mut self, display_name: String, role: Role, ctx: &mut ViewContext<Self>) {
        self.role = role;
        self.display_name = display_name;
        ctx.notify();
    }
}

impl Entity for ViewerRequestBody {
    type Event = ViewerRequestBodyEvent;
}

impl View for ViewerRequestBody {
    fn ui_name() -> &'static str {
        "ViewerRequestBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let header = format!("You have requested {} mode", self.role_label());
        let text = format!("Waiting for {}...", self.display_name);

        let cancel_button = appearance
            .ui_builder()
            .button(ButtonVariant::Outlined, self.mouse_state_handle.clone())
            .with_centered_text_label(String::from("Cancel request"))
            .with_style(UiComponentStyles {
                font_size: Some(TEXT_FONT_SIZE),
                font_weight: Some(Weight::Bold),
                height: Some(BUTTON_HEIGHT),
                width: Some(BUTTON_WIDTH),
                ..Default::default()
            })
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(ViewerRequestBodyAction::Cancel))
            .finish();

        let text_body = Container::new(
            Flex::column()
                .with_child(
                    Container::new(
                        Text::new_inline(header, appearance.ui_font_family(), HEADER_FONT_SIZE)
                            .with_color(blended_colors::text_main(
                                appearance.theme(),
                                appearance.theme().background(),
                            ))
                            .finish(),
                    )
                    .with_padding_bottom(BODY_PADDING)
                    .finish(),
                )
                .with_child(
                    Text::new_inline(text, appearance.ui_font_family(), TEXT_FONT_SIZE)
                        .with_color(blended_colors::text_sub(
                            appearance.theme(),
                            appearance.theme().background(),
                        ))
                        .finish(),
                )
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_padding_bottom(MODAL_PADDING)
        .finish();

        Flex::column()
            .with_child(text_body)
            .with_child(cancel_button)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish()
    }
}

impl TypedActionView for ViewerRequestBody {
    type Action = ViewerRequestBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ViewerRequestBodyAction::Cancel => ctx.emit(ViewerRequestBodyEvent::Cancel),
        }
    }
}
