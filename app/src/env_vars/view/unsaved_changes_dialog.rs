use warpui::{
    elements::{Container, MouseStateHandle},
    fonts::Weight,
    platform::Cursor,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element,
};

use crate::ui_components::dialog::{dialog_styles, Dialog};
use crate::{localization, Appearance};

use super::env_var_collection::{EnvVarCollectionAction, EnvVarCollectionView};

const BUTTON_FONT_SIZE: f32 = 14.;
const BUTTON_PADDING: f32 = 12.;
const MODAL_HORIZONTAL_MARGIN: f32 = 28.;
const DIALOG_WIDTH: f32 = 460.;

fn text(app: &AppContext, key: &str) -> String {
    localization::text_for_app(app, key)
}

impl EnvVarCollectionView {
    pub fn render_unsaved_changes_dialog_button(
        &self,
        appearance: &Appearance,
        button_mouse_state: MouseStateHandle,
        action: EnvVarCollectionAction,
        text: &str,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, button_mouse_state)
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                font_weight: Some(Weight::Bold),
                padding: Some(Coords::uniform(BUTTON_PADDING)),
                ..Default::default()
            })
            .with_text_label(text.into())
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish()
    }

    pub fn render_unsaved_changes_dialog(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let keep_editing_button = self.render_unsaved_changes_dialog_button(
            appearance,
            self.button_mouse_states.keep_editing_state.clone(),
            EnvVarCollectionAction::CloseUnsavedChangesDialog,
            &text(app, "workflow.unsaved_changes.keep_editing"),
        );

        let discard_changes_button = self.render_unsaved_changes_dialog_button(
            appearance,
            self.button_mouse_states.discard_changes_state.clone(),
            EnvVarCollectionAction::ForceClose,
            &text(app, "workflow.unsaved_changes.discard"),
        );

        Container::new(
            Dialog::new(
                text(app, "workflow.unsaved_changes.message"),
                None,
                dialog_styles(appearance),
            )
            .with_bottom_row_child(keep_editing_button)
            .with_bottom_row_child(discard_changes_button)
            .with_width(DIALOG_WIDTH)
            .build()
            .finish(),
        )
        .with_margin_left(MODAL_HORIZONTAL_MARGIN)
        .with_margin_right(MODAL_HORIZONTAL_MARGIN)
        .finish()
    }
}
