use warpui::elements::{Container, Flex, MainAxisSize, MouseStateHandle, ParentElement};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::UiComponent;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::style::{self, MODAL_PADDING};
use crate::appearance::Appearance;

const SESSION_BUILD_FREE_PLAN_SUBHEADER: &str = "Warp's free and pro plans come with a limited number of shared sessions.\n\nFor increased access to session sharing upgrade to the Build plan.";
const VIEW_PLANS_TEXT: &str = "View plans";

pub struct DeniedBody {
    button_mouse_state: MouseStateHandle,
}

#[derive(Debug, Clone, Copy)]
pub enum DeniedBodyAction {
    Upgrade,
}

pub enum DeniedBodyEvent {
    Upgrade,
}

impl DeniedBody {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            button_mouse_state: Default::default(),
        }
    }
}

impl Entity for DeniedBody {
    type Event = DeniedBodyEvent;
}

impl View for DeniedBody {
    fn ui_name() -> &'static str {
        "ShareModalDeniedBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut col = Flex::column();
        let subheader = SESSION_BUILD_FREE_PLAN_SUBHEADER;

        let text = appearance
            .ui_builder()
            .wrappable_text(subheader, true)
            .with_style(style::subheader_styles(appearance))
            .build()
            .finish();

        let button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.button_mouse_state.clone())
            .with_centered_text_label(VIEW_PLANS_TEXT.to_owned())
            .with_style(style::button_styles())
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(DeniedBodyAction::Upgrade))
            .finish();

        col.add_child(text);
        col.add_child(
            Container::new(button)
                .with_margin_top(MODAL_PADDING)
                .finish(),
        );
        col.with_main_axis_size(MainAxisSize::Min).finish()
    }
}

impl TypedActionView for DeniedBody {
    type Action = DeniedBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            DeniedBodyAction::Upgrade => ctx.emit(DeniedBodyEvent::Upgrade),
        }
    }
}
