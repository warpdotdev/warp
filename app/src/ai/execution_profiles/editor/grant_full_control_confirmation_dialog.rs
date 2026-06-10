use warpui::elements::{ChildView, Container, Dismiss, Empty};
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::ui_components::dialog::{dialog_styles, Dialog};
use crate::view_components::action_button::{ActionButton, DangerPrimaryTheme, NakedTheme};

const DIALOG_WIDTH: f32 = 460.;

const DIALOG_BODY: &str =
    "This grants the Agent full control: it will apply code diffs, read files, \
execute commands, interact with running commands, run orchestrated agents, and call MCP servers \
without ever asking for approval. Your command and MCP denylists for this profile will also be \
cleared. You can change individual permissions again afterward.";

pub enum GrantFullControlConfirmationDialogEvent {
    Cancel,
    Confirm,
}

#[derive(Debug)]
pub enum GrantFullControlConfirmationDialogAction {
    Cancel,
    Confirm,
}

pub struct GrantFullControlConfirmationDialog {
    visible: bool,
    cancel_button: ViewHandle<ActionButton>,
    confirm_button: ViewHandle<ActionButton>,
}

impl GrantFullControlConfirmationDialog {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(GrantFullControlConfirmationDialogAction::Cancel);
            })
        });

        let confirm_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Grant full control", DangerPrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(GrantFullControlConfirmationDialogAction::Confirm);
            })
        });

        Self {
            visible: false,
            cancel_button,
            confirm_button,
        }
    }

    pub fn show(&mut self, ctx: &mut ViewContext<Self>) {
        self.visible = true;
        ctx.notify();
    }

    pub fn hide(&mut self, ctx: &mut ViewContext<Self>) {
        self.visible = false;
        ctx.notify();
    }
}

impl Entity for GrantFullControlConfirmationDialog {
    type Event = GrantFullControlConfirmationDialogEvent;
}

impl View for GrantFullControlConfirmationDialog {
    fn ui_name() -> &'static str {
        "GrantFullControlConfirmationDialog"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if !self.visible {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);

        let dialog = Dialog::new(
            "Grant full control?".to_string(),
            Some(DIALOG_BODY.to_string()),
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
                ctx.dispatch_typed_action(GrantFullControlConfirmationDialogAction::Cancel)
            })
            .finish()
    }
}

impl TypedActionView for GrantFullControlConfirmationDialog {
    type Action = GrantFullControlConfirmationDialogAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            GrantFullControlConfirmationDialogAction::Cancel => {
                ctx.emit(GrantFullControlConfirmationDialogEvent::Cancel)
            }
            GrantFullControlConfirmationDialogAction::Confirm => {
                ctx.emit(GrantFullControlConfirmationDialogEvent::Confirm)
            }
        }
    }
}
