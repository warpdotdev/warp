use warpui::{
    elements::{ChildView, Container, Dismiss, Empty},
    ui_components::components::UiComponent,
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    appearance::Appearance,
    localization,
    localization::LocalizationUpdater,
    server::ids::SyncId,
    ui_components::dialog::{dialog_styles, Dialog},
    view_components::action_button::{ActionButton, DangerPrimaryTheme, NakedTheme},
};

const DIALOG_WIDTH: f32 = 450.;

pub enum DeleteEnvironmentConfirmationDialogEvent {
    Cancel,
    Confirm(SyncId),
}

#[derive(Debug)]
pub enum DeleteEnvironmentConfirmationDialogAction {
    Cancel,
    Confirm,
}

pub struct DeleteEnvironmentConfirmationDialog {
    pub(crate) visible: bool,
    pub(crate) env_id: Option<SyncId>,
    pub(crate) env_name: String,
    cancel_button: ViewHandle<ActionButton>,
    confirm_button: ViewHandle<ActionButton>,
}

impl DeleteEnvironmentConfirmationDialog {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let cancel_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(
                localization::text_for_app(ctx, "settings.action.cancel"),
                NakedTheme,
            )
            .on_click(|ctx| {
                ctx.dispatch_typed_action(DeleteEnvironmentConfirmationDialogAction::Cancel);
            })
        });

        let confirm_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(
                localization::text_for_app(ctx, "settings.environment.form.delete"),
                DangerPrimaryTheme,
            )
            .on_click(|ctx| {
                ctx.dispatch_typed_action(DeleteEnvironmentConfirmationDialogAction::Confirm);
            })
        });

        let me = Self {
            visible: false,
            env_id: None,
            env_name: String::new(),
            cancel_button,
            confirm_button,
        };

        ctx.subscribe_to_model(&LocalizationUpdater::handle(ctx), |me, _, _, ctx| {
            me.cancel_button.update(ctx, |button, ctx| {
                button.set_label(
                    localization::text_for_app(ctx, "settings.action.cancel"),
                    ctx,
                );
            });
            me.confirm_button.update(ctx, |button, ctx| {
                button.set_label(
                    localization::text_for_app(ctx, "settings.environment.form.delete"),
                    ctx,
                );
            });
            ctx.notify();
        });

        me
    }

    pub fn show(&mut self, env_id: SyncId, env_name: String, ctx: &mut ViewContext<Self>) {
        self.env_id = Some(env_id);
        self.env_name = env_name;
        self.visible = true;
        ctx.notify();
    }

    pub fn hide(&mut self, ctx: &mut ViewContext<Self>) {
        self.visible = false;
        ctx.notify();
    }
}

impl Entity for DeleteEnvironmentConfirmationDialog {
    type Event = DeleteEnvironmentConfirmationDialogEvent;
}

impl View for DeleteEnvironmentConfirmationDialog {
    fn ui_name() -> &'static str {
        "DeleteEnvironmentConfirmationDialog"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if !self.visible {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);

        let description =
            localization::text_for_app(app, "settings.environment.delete_confirmation.description")
                .replace("{name}", &self.env_name);

        let dialog = Dialog::new(
            localization::text_for_app(app, "settings.environment.delete_confirmation.title"),
            Some(description),
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
                ctx.dispatch_typed_action(DeleteEnvironmentConfirmationDialogAction::Cancel)
            })
            .finish()
    }
}

impl TypedActionView for DeleteEnvironmentConfirmationDialog {
    type Action = DeleteEnvironmentConfirmationDialogAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            DeleteEnvironmentConfirmationDialogAction::Cancel => {
                ctx.emit(DeleteEnvironmentConfirmationDialogEvent::Cancel)
            }
            DeleteEnvironmentConfirmationDialogAction::Confirm => {
                if let Some(env_id) = self.env_id {
                    ctx.emit(DeleteEnvironmentConfirmationDialogEvent::Confirm(env_id));
                }
            }
        }
    }
}
