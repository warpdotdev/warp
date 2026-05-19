use warpui::{
    elements::MouseStateHandle,
    fonts::Weight,
    platform::Cursor,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::{
    appearance::Appearance,
    localization,
    ui_components::dialog::{dialog_styles, Dialog},
};

// This follows our new design standard for confirmation dialogs (e.g. used in the session sharing dialog)
// Design team has discouraged us from continuing to use CloudActionConfirmationDialog's current design
// TODO: update CloudActionConfirmationDialog to use this design
pub enum EmptyTrashConfirmationEvent {
    Confirm,
    Cancel,
}

#[derive(Debug)]
pub enum EmptyTrashConfirmationAction {
    Confirm,
    Cancel,
}

pub struct EmptyTrashConfirmationDialog {
    cancel_mouse_state: MouseStateHandle,
    confirm_mouse_state: MouseStateHandle,
}

impl EmptyTrashConfirmationDialog {
    pub fn new() -> Self {
        Self {
            cancel_mouse_state: Default::default(),
            confirm_mouse_state: Default::default(),
        }
    }
}

impl Entity for EmptyTrashConfirmationDialog {
    type Event = EmptyTrashConfirmationEvent;
}

impl View for EmptyTrashConfirmationDialog {
    fn ui_name() -> &'static str {
        "EmptyTrashConfirmationDialog"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let button_style = UiComponentStyles {
            font_size: Some(14.),
            font_weight: Some(Weight::Bold),
            width: Some(202.),
            height: Some(40.),
            ..Default::default()
        };

        let confirm_button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.confirm_mouse_state.clone())
            .with_centered_text_label(text(app, "drive.confirmation.empty_trash.confirm"))
            .with_style(button_style)
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(EmptyTrashConfirmationAction::Confirm)
            })
            .finish();

        let cancel_button = appearance
            .ui_builder()
            .button(ButtonVariant::Basic, self.cancel_mouse_state.clone())
            .with_centered_text_label(text(app, "drive.confirmation.cancel"))
            .with_style(button_style)
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(EmptyTrashConfirmationAction::Cancel)
            })
            .finish();

        Dialog::new(
            text(app, "drive.confirmation.empty_trash.title"),
            Some(text(app, "drive.confirmation.empty_trash.body")),
            UiComponentStyles {
                width: Some(460.),
                padding: Some(Coords::uniform(24.)),
                ..dialog_styles(appearance)
            },
        )
        .with_bottom_row_child(cancel_button)
        .with_bottom_row_child(confirm_button)
        .build()
        .finish()
    }
}

impl TypedActionView for EmptyTrashConfirmationDialog {
    type Action = EmptyTrashConfirmationAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            EmptyTrashConfirmationAction::Confirm => ctx.emit(EmptyTrashConfirmationEvent::Confirm),
            EmptyTrashConfirmationAction::Cancel => ctx.emit(EmptyTrashConfirmationEvent::Cancel),
        }
    }
}

fn text(app: &AppContext, key: &str) -> String {
    localization::text_for_app(app, key)
}
