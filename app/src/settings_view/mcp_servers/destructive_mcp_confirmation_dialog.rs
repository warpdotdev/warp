use warpui::{
    elements::{ChildView, Container, Dismiss, Empty},
    ui_components::components::UiComponent,
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    appearance::Appearance,
    localization::{self, LocalizationUpdater},
    ui_components::dialog::{dialog_styles, Dialog},
    view_components::action_button::{ActionButton, DangerPrimaryTheme, NakedTheme},
};

const DIALOG_WIDTH: f32 = 450.;
pub enum DestructiveMCPConfirmationDialogEvent {
    Cancel,
    Confirm(DestructiveMCPConfirmationDialogVariant),
}

#[derive(Debug)]
pub enum DestructiveMCPConfirmationDialogAction {
    Cancel,
    Confirm,
}

struct DestructiveMCPConfirmationDialogDisplayOptions {
    title_key: &'static str,
    description_key: &'static str,
    confirm_button_key: &'static str,
}

impl DestructiveMCPConfirmationDialogDisplayOptions {
    pub fn new(
        title_key: &'static str,
        description_key: &'static str,
        confirm_button_key: &'static str,
    ) -> Self {
        Self {
            title_key,
            description_key,
            confirm_button_key,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DestructiveMCPConfirmationDialogVariant {
    DeleteLocal,
    DeleteShared,
    Unshare,
}

impl From<&DestructiveMCPConfirmationDialogVariant>
    for DestructiveMCPConfirmationDialogDisplayOptions
{
    fn from(variant: &DestructiveMCPConfirmationDialogVariant) -> Self {
        match *variant {
            DestructiveMCPConfirmationDialogVariant::DeleteLocal => {
                DestructiveMCPConfirmationDialogDisplayOptions::new(
                    "settings.mcp.confirmation.delete_local.title",
                    "settings.mcp.confirmation.delete_local.description",
                    "settings.mcp.edit.delete_mcp",
                )
            }
            DestructiveMCPConfirmationDialogVariant::DeleteShared => {
                DestructiveMCPConfirmationDialogDisplayOptions::new(
                    "settings.mcp.confirmation.delete_shared.title",
                    "settings.mcp.confirmation.delete_shared.description",
                    "settings.mcp.edit.delete_mcp",
                )
            }
            DestructiveMCPConfirmationDialogVariant::Unshare => {
                DestructiveMCPConfirmationDialogDisplayOptions::new(
                    "settings.mcp.confirmation.unshare.title",
                    "settings.mcp.confirmation.unshare.description",
                    "settings.mcp.edit.remove_from_team",
                )
            }
        }
    }
}

pub struct DestructiveMCPConfirmationDialog {
    visible: bool,
    variant: DestructiveMCPConfirmationDialogVariant,
    cancel_button: ViewHandle<ActionButton>,
    confirm_button: ViewHandle<ActionButton>,
}

impl DestructiveMCPConfirmationDialog {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let cancel_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(
                localization::text_for_app(ctx, "settings.action.cancel"),
                NakedTheme,
            )
            .on_click(|ctx| {
                ctx.dispatch_typed_action(DestructiveMCPConfirmationDialogAction::Cancel);
            })
        });

        let confirm_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("", DangerPrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(DestructiveMCPConfirmationDialogAction::Confirm);
            })
        });

        let me = Self {
            visible: false,
            variant: DestructiveMCPConfirmationDialogVariant::DeleteLocal,
            cancel_button,
            confirm_button,
        };

        ctx.subscribe_to_model(&LocalizationUpdater::handle(ctx), |me, _, _, ctx| {
            me.update_button_labels(ctx);
            ctx.notify();
        });

        me
    }

    pub fn show(
        &mut self,
        variant: DestructiveMCPConfirmationDialogVariant,
        ctx: &mut ViewContext<Self>,
    ) {
        self.variant = variant;
        self.update_button_labels(ctx);
        self.visible = true;

        ctx.notify();
    }

    pub fn hide(&mut self, ctx: &mut ViewContext<Self>) {
        self.visible = false;
        ctx.notify();
    }

    fn update_button_labels(&self, ctx: &mut ViewContext<Self>) {
        let display_options: DestructiveMCPConfirmationDialogDisplayOptions =
            (&self.variant).into();
        self.cancel_button.update(ctx, |button, ctx| {
            button.set_label(
                localization::text_for_app(ctx, "settings.action.cancel"),
                ctx,
            );
        });
        self.confirm_button.update(ctx, |button, ctx| {
            button.set_label(
                localization::text_for_app(ctx, display_options.confirm_button_key),
                ctx,
            );
        });
    }
}

impl Entity for DestructiveMCPConfirmationDialog {
    type Event = DestructiveMCPConfirmationDialogEvent;
}

impl View for DestructiveMCPConfirmationDialog {
    fn ui_name() -> &'static str {
        "DestructiveMCPConfirmationDialog"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if !self.visible {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);
        let display_options: DestructiveMCPConfirmationDialogDisplayOptions =
            (&self.variant).into();

        let dialog = Dialog::new(
            localization::text_for_app(app, display_options.title_key),
            Some(localization::text_for_app(
                app,
                display_options.description_key,
            )),
            dialog_styles(appearance),
        )
        .with_bottom_row_child(ChildView::new(&self.cancel_button).finish())
        .with_bottom_row_child(
            Container::new(ChildView::new(&self.confirm_button).finish())
                .with_margin_left(12.)
                .finish(),
        )
        .with_width(DIALOG_WIDTH)
        .build()
        .finish();

        Dismiss::new(dialog)
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(DestructiveMCPConfirmationDialogAction::Cancel)
            })
            .finish()
    }
}

impl TypedActionView for DestructiveMCPConfirmationDialog {
    type Action = DestructiveMCPConfirmationDialogAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            DestructiveMCPConfirmationDialogAction::Cancel => {
                ctx.emit(DestructiveMCPConfirmationDialogEvent::Cancel)
            }
            DestructiveMCPConfirmationDialogAction::Confirm => ctx.emit(
                DestructiveMCPConfirmationDialogEvent::Confirm(self.variant.clone()),
            ),
        }
    }
}
