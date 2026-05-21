use std::path::PathBuf;

use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::{
    elements::{
        Align, ChildAnchor, ChildView, Container, OffsetPositioning, ParentAnchor,
        ParentOffsetBounds, Stack,
    },
    keymap::{FixedBinding, Keystroke},
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    appearance::Appearance,
    ui_components::dialog::{dialog_styles, Dialog},
    view_components::action_button::{
        ActionButton, DangerPrimaryTheme, KeystrokeSource, NakedTheme,
    },
};

pub(crate) fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "escape",
            RemoveTabConfigConfirmationAction::Cancel,
            id!(RemoveTabConfigConfirmationDialog::ui_name()),
        ),
        FixedBinding::new(
            "enter",
            RemoveTabConfigConfirmationAction::Confirm,
            id!(RemoveTabConfigConfirmationDialog::ui_name()),
        ),
    ]);
}

const DIALOG_WIDTH: f32 = 460.;

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) enum RemoveTabConfigConfirmationEvent {
    Confirm { path: PathBuf },
    Cancel,
}

#[derive(Debug)]
pub(crate) enum RemoveTabConfigConfirmationAction {
    Confirm,
    Cancel,
}

pub(crate) struct RemoveTabConfigConfirmationDialog {
    cancel_button: ViewHandle<ActionButton>,
    confirm_button: ViewHandle<ActionButton>,
    config_name: String,
    config_path: Option<PathBuf>,
}

impl RemoveTabConfigConfirmationDialog {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let cancel_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Cancel"), NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(RemoveTabConfigConfirmationAction::Cancel);
            })
        });

        let enter_keystroke = Keystroke::parse("enter").expect("Valid keystroke");
        let confirm_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Remove"), DangerPrimaryTheme)
                .with_keybinding(KeystrokeSource::Fixed(enter_keystroke), ctx)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(RemoveTabConfigConfirmationAction::Confirm);
                })
        });

        Self {
            cancel_button,
            confirm_button,
            config_name: String::new(),
            config_path: None,
        }
    }

    pub fn set_config(&mut self, name: String, path: PathBuf) {
        self.config_name = name;
        self.config_path = Some(path);
    }
}

impl Entity for RemoveTabConfigConfirmationDialog {
    type Event = RemoveTabConfigConfirmationEvent;
}

impl View for RemoveTabConfigConfirmationDialog {
    fn ui_name() -> &'static str {
        "RemoveTabConfigConfirmationDialog"
    }

    fn on_focus(&mut self, _focus_ctx: &warpui::FocusContext, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let cancel_button = Container::new(ChildView::new(&self.cancel_button).finish())
            .with_margin_right(12.)
            .finish();

        let title = format!("Remove '{}'?", self.config_name);

        let dialog = Dialog::new(
            title,
            Some(
                "This tab config will be permanently deleted. This action cannot be undone.".into(),
            ),
            UiComponentStyles {
                width: Some(DIALOG_WIDTH),
                ..dialog_styles(appearance)
            },
        )
        .with_bottom_row_child(cancel_button)
        .with_bottom_row_child(ChildView::new(&self.confirm_button).finish())
        .build()
        .finish();

        let mut stack = Stack::new();
        stack.add_positioned_child(
            dialog,
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );

        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(Fill::blur().into())
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }
}

impl TypedActionView for RemoveTabConfigConfirmationDialog {
    type Action = RemoveTabConfigConfirmationAction;

    fn handle_action(
        &mut self,
        action: &RemoveTabConfigConfirmationAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            RemoveTabConfigConfirmationAction::Confirm => {
                let Some(path) = self.config_path.clone() else {
                    log::error!("Remove confirm button pressed with no config path");
                    return;
                };
                ctx.emit(RemoveTabConfigConfirmationEvent::Confirm { path });
            }
            RemoveTabConfigConfirmationAction::Cancel => {
                ctx.emit(RemoveTabConfigConfirmationEvent::Cancel);
            }
        }
    }
}
