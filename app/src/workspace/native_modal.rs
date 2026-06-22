use settings::Setting as _;
use warp_core::ui::theme::Fill;
use warpui::elements::{Align, Container, Empty, Flex, MouseStateHandle, ParentElement};
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::modals::{AlertDialogWithCallbacks, AppModalCallback};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::text::Span;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::appearance::Appearance;
use crate::terminal::general_settings::{GeneralSettings, GeneralSettingsChangedEvent};
use crate::ui_components::dialog::{dialog_styles, Dialog};

pub(super) fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings(vec![
        FixedBinding::new("escape", NativeModalAction::Close, id!("NativeModal")),
        FixedBinding::new("enter", NativeModalAction::Confirm, id!("NativeModal")),
    ]);
}

/// Used to show a Warp-native modal dialog above a [`super::Workspace`]. The first button is [`ButtonVariant::Accent`].
pub struct NativeModal {
    alert_dialog: Option<AlertDialogWithCallbacks<AppModalCallback>>,
    dont_show_again: bool,
    modal_button_mouse_states: Vec<MouseStateHandle>,
    dont_show_again_mouse_state: MouseStateHandle,
}

#[derive(Debug)]
pub enum NativeModalAction {
    ToggleDontShowAgain,
    /// Trigger a callback registered in [`NativeModal::alert_dialog`] and reset the modal.
    TriggerButtonCallback(usize),
    /// Triggers the last button in the list, as we assume the last button is "cancel".
    Close,
    /// Triggers the first button in the list, as we assume the first button is "confirm".
    Confirm,
}

pub enum NativeModalEvent {
    Close,
}

impl NativeModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let general_settings = GeneralSettings::handle(ctx);
        let dont_show_again = !*general_settings
            .as_ref(ctx)
            .show_warning_before_quitting
            .value();
        ctx.subscribe_to_model(&general_settings, Self::handle_general_settings_event);
        NativeModal {
            alert_dialog: None,
            dont_show_again,
            dont_show_again_mouse_state: Default::default(),
            modal_button_mouse_states: Default::default(),
        }
    }

    fn handle_general_settings_event(
        &mut self,
        general_settings: ModelHandle<GeneralSettings>,
        event: &GeneralSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let GeneralSettingsChangedEvent::ShowWarningBeforeQuitting { .. } = event {
            self.dont_show_again = !general_settings.read(ctx, |settings, _| {
                *settings.show_warning_before_quitting.value()
            });
            ctx.notify();
        }
    }

    pub fn set_alert_dialog(&mut self, alert_dialog: AlertDialogWithCallbacks<AppModalCallback>) {
        self.modal_button_mouse_states.clear();
        for _ in 0..alert_dialog.button_data.len() {
            self.modal_button_mouse_states.push(Default::default());
        }
        self.alert_dialog = Some(alert_dialog);
    }

    fn reset(&mut self) {
        self.alert_dialog = None;
        self.modal_button_mouse_states = Default::default();
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub(super) fn has_alert_dialog(&self) -> bool {
        self.alert_dialog.is_some()
    }

    fn trigger_button_callback(&mut self, idx: usize, ctx: &mut ViewContext<Self>) {
        // Once we trigger a callback from a button, we are guaranteed that the modal will close so
        // it's ok to take the alert dialog from Self.
        if let Some(mut dialog) = self.alert_dialog.take() {
            let button = dialog.button_data.remove(idx);
            (button.on_click)(ctx);
            if self.dont_show_again {
                (dialog.on_disable)(ctx);
            }
        }
        self.reset();
        ctx.emit(NativeModalEvent::Close);
    }
}

impl Entity for NativeModal {
    type Event = NativeModalEvent;
}

impl View for NativeModal {
    fn ui_name() -> &'static str {
        "NativeModal"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let Some(alert_dialog) = self.alert_dialog.as_ref() else {
            log::warn!("No alert dialog was set for the native modal");
            return Empty::new().finish();
        };
        let appearance = Appearance::as_ref(app);
        let button_style = UiComponentStyles {
            font_size: Some(14.),
            font_weight: Some(Weight::Bold),
            width: Some(240.),
            height: Some(40.),
            ..Default::default()
        };

        let dont_show_again_checkbox = appearance
            .ui_builder()
            .checkbox(self.dont_show_again_mouse_state.clone(), Some(14.))
            .with_label(Span::new("Don't show again.", Default::default()))
            .check(self.dont_show_again)
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(NativeModalAction::ToggleDontShowAgain))
            .finish();

        let mut dialog_column_contents = vec![Container::new(dont_show_again_checkbox)
            .with_padding_bottom(20.)
            .finish()];

        for (i, modal_button) in alert_dialog.button_data.iter().enumerate() {
            let button = Align::new(
                appearance
                    .ui_builder()
                    .button(
                        if i == 0 {
                            ButtonVariant::Accent
                        } else {
                            ButtonVariant::Basic
                        },
                        self.modal_button_mouse_states
                            .get(i)
                            .expect("Modal button mouse state should be set")
                            .clone(),
                    )
                    .with_centered_text_label(modal_button.title.clone())
                    .with_style(button_style)
                    .build()
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(NativeModalAction::TriggerButtonCallback(i))
                    })
                    .finish(),
            )
            .finish();

            dialog_column_contents.push(Container::new(button).with_padding_bottom(8.).finish());
        }

        let dialog_column = Flex::column()
            .with_children(dialog_column_contents)
            .finish();
        let dialog = Dialog::new(
            alert_dialog.message_text.clone(),
            Some(alert_dialog.info_text.clone()),
            UiComponentStyles {
                width: Some(280.),
                padding: Some(Coords::uniform(24.)),
                ..dialog_styles(appearance)
            },
        )
        .with_child(dialog_column)
        .build()
        .finish();

        // This blurs the background and makes it uninteractable.
        Container::new(Align::new(dialog).finish())
            .with_background_color(Fill::blur().into())
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }
}

impl TypedActionView for NativeModal {
    type Action = NativeModalAction;

    fn handle_action(&mut self, action: &NativeModalAction, ctx: &mut ViewContext<Self>) {
        match action {
            NativeModalAction::TriggerButtonCallback(idx) => {
                self.trigger_button_callback(*idx, ctx);
            }
            NativeModalAction::ToggleDontShowAgain => {
                self.dont_show_again = !self.dont_show_again;
                ctx.notify();
            }
            NativeModalAction::Close => {
                let last_button_idx = self.modal_button_mouse_states.len() - 1;
                self.trigger_button_callback(last_button_idx, ctx);
            }
            NativeModalAction::Confirm => {
                self.trigger_button_callback(0, ctx);
            }
        }
    }
}
