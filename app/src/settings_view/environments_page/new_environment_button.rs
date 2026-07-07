use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Border, Container, CornerRadius, DispatchEventResult, EventHandler, Flex, MainAxisAlignment,
    MouseStateHandle, ParentElement as _, Radius, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::{
    AppContext, BlurContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WeakViewHandle,
};

use super::EnvironmentsPageAction;
use crate::editor::EditorView;

pub struct NewEnvironmentButtonView {
    trigger_mouse_state: MouseStateHandle,
    search_editor: ViewHandle<EditorView>,
    self_handle: WeakViewHandle<Self>,
}
#[derive(Debug, Clone)]
pub enum NewEnvironmentButtonAction {
    OpenSelector,
    FocusSearch,
}

impl NewEnvironmentButtonView {
    pub fn new(search_editor: ViewHandle<EditorView>, ctx: &mut ViewContext<Self>) -> Self {
        Self {
            trigger_mouse_state: Default::default(),
            search_editor,
            self_handle: ctx.handle(),
        }
    }

    fn is_focused(&self, app: &AppContext) -> bool {
        self.self_handle
            .upgrade(app)
            .is_some_and(|v| v.is_focused(app))
    }
}

impl Entity for NewEnvironmentButtonView {
    type Event = ();
}
impl TypedActionView for NewEnvironmentButtonView {
    type Action = NewEnvironmentButtonAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            NewEnvironmentButtonAction::OpenSelector => {
                ctx.dispatch_typed_action(
                    &EnvironmentsPageAction::OpenEnvironmentSetupModeSelector,
                );
            }
            NewEnvironmentButtonAction::FocusSearch => {
                ctx.focus(&self.search_editor);
            }
        }
    }
}

impl View for NewEnvironmentButtonView {
    fn ui_name() -> &'static str {
        "NewEnvironmentButton"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.notify();
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let is_focused = self.is_focused(app);

        let trigger = {
            warpui::elements::Hoverable::new(self.trigger_mouse_state.clone(), move |s| {
                let is_hovered = s.is_hovered();
                let background = if is_hovered || is_focused {
                    Some(theme.surface_3())
                } else {
                    None
                };

                let row = Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(
                        Text::new(
                            "New environment",
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_style(Properties::default().weight(Weight::Medium))
                        .with_color(theme.active_ui_text_color().into())
                        .finish(),
                    );

                let mut container = Container::new(row.finish())
                    .with_horizontal_padding(12.)
                    .with_vertical_padding(6.)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .with_border(Border::all(1.).with_border_fill(theme.surface_3()));

                if let Some(bg) = background {
                    container = container.with_background(bg);
                }

                container.finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(NewEnvironmentButtonAction::OpenSelector);
            })
            .finish()
        };

        EventHandler::new(trigger)
            .on_keydown(move |ctx, _app, keystroke| {
                if keystroke.is_shift_tab() {
                    ctx.dispatch_typed_action(NewEnvironmentButtonAction::FocusSearch);
                    DispatchEventResult::StopPropagation
                } else if keystroke.is_unmodified_enter() {
                    ctx.dispatch_typed_action(NewEnvironmentButtonAction::OpenSelector);
                    DispatchEventResult::StopPropagation
                } else {
                    DispatchEventResult::PropagateToParent
                }
            })
            .finish()
    }
}
