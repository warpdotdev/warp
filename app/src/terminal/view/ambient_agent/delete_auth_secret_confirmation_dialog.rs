use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_cli::agent::Harness;
use warp_managed_secrets::client::SecretOwner;
use warpui::elements::{Align, ChildView, Container, Dismiss, DropShadow, Empty};
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::ui_components::dialog::{Dialog, dialog_styles};
use crate::view_components::action_button::{ActionButton, DangerPrimaryTheme, NakedTheme};

const DIALOG_WIDTH: f32 = 450.;

#[derive(Clone, Debug)]
pub(super) struct PendingAuthSecretDeletion {
    pub(super) harness: Harness,
    pub(super) name: String,
    pub(super) owner: SecretOwner,
}

pub(super) enum DeleteAuthSecretConfirmationDialogEvent {
    Cancel,
    Confirm(PendingAuthSecretDeletion),
}

#[derive(Debug)]
pub(super) enum DeleteAuthSecretConfirmationDialogAction {
    Cancel,
    Confirm,
}

pub(super) struct DeleteAuthSecretConfirmationDialog {
    pending_deletion: Option<PendingAuthSecretDeletion>,
    cancel_button: ViewHandle<ActionButton>,
    delete_button: ViewHandle<ActionButton>,
}

impl DeleteAuthSecretConfirmationDialog {
    pub(super) fn new(ctx: &mut ViewContext<Self>) -> Self {
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(DeleteAuthSecretConfirmationDialogAction::Cancel);
            })
        });

        let delete_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Delete", DangerPrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(DeleteAuthSecretConfirmationDialogAction::Confirm);
            })
        });

        Self {
            pending_deletion: None,
            cancel_button,
            delete_button,
        }
    }

    pub(super) fn show(
        &mut self,
        pending_deletion: PendingAuthSecretDeletion,
        ctx: &mut ViewContext<Self>,
    ) {
        self.pending_deletion = Some(pending_deletion);
        ctx.notify();
    }

    pub(super) fn hide(&mut self, ctx: &mut ViewContext<Self>) {
        self.pending_deletion = None;
        ctx.notify();
    }
}

impl Entity for DeleteAuthSecretConfirmationDialog {
    type Event = DeleteAuthSecretConfirmationDialogEvent;
}

impl View for DeleteAuthSecretConfirmationDialog {
    fn ui_name() -> &'static str {
        "DeleteAuthSecretConfirmationDialog"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if self.pending_deletion.is_none() {
            return Empty::new().finish();
        }

        let Some(pending_deletion) = self.pending_deletion.as_ref() else {
            return Empty::new().finish();
        };

        let appearance = Appearance::as_ref(app);
        let description = format!(
            "Are you sure you want to delete {}? This action cannot be undone. Any agents or environments referencing this secret will no longer have access to it.",
            pending_deletion.name
        );

        let dialog = Dialog::new(
            "Delete secret".to_string(),
            Some(description),
            dialog_styles(appearance),
        )
        .with_bottom_row_child(ChildView::new(&self.cancel_button).finish())
        .with_bottom_row_child(
            Container::new(ChildView::new(&self.delete_button).finish())
                .with_margin_left(12.)
                .finish(),
        )
        .with_width(DIALOG_WIDTH)
        .build()
        .finish();
        let dialog = Container::new(dialog)
            .with_drop_shadow(DropShadow {
                color: ColorU::new(0, 0, 0, 77),
                offset: vec2f(0., 7.),
                blur_radius: 7.,
                spread_radius: 0.,
            })
            .finish();

        let dialog = Dismiss::new(dialog)
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(DeleteAuthSecretConfirmationDialogAction::Cancel)
            })
            .finish();

        Container::new(Align::new(dialog).finish())
            .with_background(appearance.theme().dark_overlay())
            .finish()
    }
}

impl TypedActionView for DeleteAuthSecretConfirmationDialog {
    type Action = DeleteAuthSecretConfirmationDialogAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            DeleteAuthSecretConfirmationDialogAction::Cancel => {
                ctx.emit(DeleteAuthSecretConfirmationDialogEvent::Cancel)
            }
            DeleteAuthSecretConfirmationDialogAction::Confirm => {
                if let Some(pending_deletion) = self.pending_deletion.clone() {
                    ctx.emit(DeleteAuthSecretConfirmationDialogEvent::Confirm(
                        pending_deletion,
                    ));
                }
            }
        }
    }
}
