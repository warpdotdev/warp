use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::{
    elements::{Align, Container, CrossAxisAlignment, Dismiss, Flex, ParentElement, Stack},
    keymap::FixedBinding,
    platform::Cursor,
    ui_components::{
        button::ButtonVariant,
        components::{BorderStyle, Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::appearance::Appearance;
use crate::ui_components::{
    buttons,
    dialog::{dialog_styles, Dialog},
};

use warpui::fonts::Weight;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "escape",
            SummarizationCancelDialogAction::Continue,
            id!(SummarizationCancelDialog::ui_name()),
        ),
        FixedBinding::new(
            "ctrl-c",
            SummarizationCancelDialogAction::ConfirmCancel,
            id!(SummarizationCancelDialog::ui_name()),
        ),
    ]);
}

const DIALOG_WIDTH: f32 = 460.;
const CONTINUE_BUTTON_WIDTH: f32 = 185.;
const CANCEL_BUTTON_WIDTH: f32 = 171.;
const BUTTON_HEIGHT: f32 = 32.;

pub enum SummarizationCancelDialogEvent {
    ConfirmCancel,
    Continue,
}

#[derive(Debug)]
pub enum SummarizationCancelDialogAction {
    ConfirmCancel,
    Continue,
}

use warpui::elements::MouseStateHandle;

#[derive(Default)]
pub struct SummarizationCancelDialog {
    cancel_mouse: MouseStateHandle,
    continue_mouse: MouseStateHandle,
    close_header_mouse: MouseStateHandle,
}

impl Entity for SummarizationCancelDialog {
    type Event = SummarizationCancelDialogEvent;
}

impl View for SummarizationCancelDialog {
    fn ui_name() -> &'static str {
        "SummarizationCancelDialog"
    }

    fn on_focus(&mut self, _focus_ctx: &warpui::FocusContext, ctx: &mut ViewContext<Self>) {
        // Ensure this dialog takes focus away from the terminal editor
        ctx.focus_self();
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let dialog_styles = dialog_styles(appearance);
        // Standard confirmation dialog button styles (consistent with other dialogs)
        let button_style = UiComponentStyles {
            border_width: Some(0.),
            border_style: Some(BorderStyle::None),
            font_family_id: Some(appearance.ui_font_family()),
            font_size: Some(appearance.ui_font_size() + 2.),
            font_weight: Some(Weight::Bold),
            font_color: dialog_styles.font_color,
            height: Some(BUTTON_HEIGHT),
            ..Default::default()
        };

        let cancel_button = Container::new(
            appearance
                .ui_builder()
                .button(ButtonVariant::Secondary, self.cancel_mouse.clone())
                .with_centered_text_label(
                    crate::i18n::tr_static(app, "Cancel summarization").into(),
                )
                .with_style(UiComponentStyles {
                    width: Some(CANCEL_BUTTON_WIDTH),
                    ..button_style
                })
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(SummarizationCancelDialogAction::ConfirmCancel)
                })
                .finish(),
        )
        .with_margin_right(8.)
        .finish();

        let continue_button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.continue_mouse.clone())
            .with_centered_text_label(crate::i18n::tr_static(app, "Continue summarization").into())
            .with_style(UiComponentStyles {
                width: Some(CONTINUE_BUTTON_WIDTH),
                ..button_style
            })
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(SummarizationCancelDialogAction::Continue)
            })
            .finish();

        // Close header with Icon::X and ESC pill
        let esc_keystroke = warpui::keymap::Keystroke::parse("escape").expect("Valid keystroke");
        let close_header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children([
                Container::new(
                    buttons::close_button(appearance, self.close_header_mouse.clone())
                        .with_style(UiComponentStyles {
                            height: Some(22.),
                            width: Some(22.),
                            ..Default::default()
                        })
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(SummarizationCancelDialogAction::Continue)
                        })
                        .finish(),
                )
                //.with_margin_right(6.)
                .finish(),
                appearance
                    .ui_builder()
                    .keyboard_shortcut(&esc_keystroke)
                    .with_style(UiComponentStyles {
                        font_size: Some(appearance.ui_font_size() - 2.),
                        font_color: dialog_styles.font_color,
                        padding: Some(Coords {
                            top: 0.,
                            bottom: 0.,
                            left: 3.,
                            right: 3.,
                        }),
                        margin: Some(Coords {
                            right: 8.,
                            ..Default::default()
                        }),
                        height: Some(16.),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            ])
            .finish();

        // Build dialog content
        let dialog_core = Dialog::new(
            "Cancel summarization?".to_string(),
            Some("Summarization is already running. If you cancel now, the request may still incur cost, any progress so far will be lost, and restarting will take longer.\n\nAre you sure you want to cancel?".to_string()),
            UiComponentStyles {
                padding: Some(Coords::uniform(24.)),
                ..dialog_styles
            },
        )
        .with_close_button(close_header)
        .with_bottom_row_child(cancel_button)
        .with_bottom_row_child(continue_button)
        .with_width(DIALOG_WIDTH)
        .with_separator()
        .build()
        .finish();

        // Use an outer Dismiss to handle outside clicks and to block interaction with other elements.
        let dialog_dismiss = Dismiss::new(dialog_core)
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(SummarizationCancelDialogAction::Continue)
            })
            .prevent_interaction_with_other_elements()
            .finish();

        // Center the dialog and add a non-interactive translucent backdrop
        let mut stack = Stack::new();
        stack.add_positioned_child(
            dialog_dismiss,
            warpui::elements::OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                warpui::elements::ParentOffsetBounds::WindowByPosition,
                warpui::elements::ParentAnchor::Center,
                warpui::elements::ChildAnchor::Center,
            ),
        );

        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(Fill::blur().into())
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }
}

impl TypedActionView for SummarizationCancelDialog {
    type Action = SummarizationCancelDialogAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SummarizationCancelDialogAction::ConfirmCancel => {
                ctx.emit(SummarizationCancelDialogEvent::ConfirmCancel);
            }
            SummarizationCancelDialogAction::Continue => {
                ctx.emit(SummarizationCancelDialogEvent::Continue);
            }
        }
    }
}
