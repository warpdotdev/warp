use warpui::elements::{ChildView, Container, Dismiss, Empty};
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::ui_components::dialog::{Dialog, dialog_styles};
use crate::view_components::action_button::{ActionButton, DangerPrimaryTheme, NakedTheme};

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

#[derive(Default)]
struct DestructiveMCPConfirmationDialogDisplayOptions {
    title_text: String,
    description_text: String,
    confirm_button_label: String,
    cancel_button_label: String,
}

impl DestructiveMCPConfirmationDialogDisplayOptions {
    pub fn new(
        title_text: String,
        description_text: String,
        confirm_button_label: String,
        cancel_button_label: String,
    ) -> Self {
        Self {
            title_text,
            description_text,
            confirm_button_label,
            cancel_button_label,
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
            DestructiveMCPConfirmationDialogVariant::DeleteLocal => DestructiveMCPConfirmationDialogDisplayOptions::new(
                "Delete MCP server?".to_string(),
                "This will uninstall and remove this MCP server from all your devices.".to_string(),
                "Delete MCP".to_string(),
                "Cancel".to_string(),
            ),
            DestructiveMCPConfirmationDialogVariant::DeleteShared => DestructiveMCPConfirmationDialogDisplayOptions::new(
                "Delete shared MCP server?".to_string(),
                "This will not only delete this MCP server for yourself, but also uninstall and remove this MCP server from Warp and across all of your teammates' devices.".to_string(),
                "Delete MCP".to_string(),
                "Cancel".to_string(),
            ),
            DestructiveMCPConfirmationDialogVariant::Unshare => DestructiveMCPConfirmationDialogDisplayOptions::new(
                "Remove shared MCP server from team?".to_string(),
                "This will uninstall and remove this MCP server from Warp and across all of your teammates' devices.".to_string(),
                "Remove from team".to_string(),
                "Cancel".to_string(),
            ),
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
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("", NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(DestructiveMCPConfirmationDialogAction::Cancel);
            })
        });

        let confirm_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("", DangerPrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(DestructiveMCPConfirmationDialogAction::Confirm);
            })
        });

        Self {
            visible: false,
            variant: DestructiveMCPConfirmationDialogVariant::DeleteLocal,
            cancel_button,
            confirm_button,
        }
    }

    pub fn show(
        &mut self,
        variant: DestructiveMCPConfirmationDialogVariant,
        ctx: &mut ViewContext<Self>,
    ) {
        let display_options: DestructiveMCPConfirmationDialogDisplayOptions = (&variant).into();

        self.cancel_button.update(ctx, |button, ctx| {
            button.set_label(display_options.cancel_button_label.clone(), ctx);
        });
        self.confirm_button.update(ctx, |button, ctx| {
            button.set_label(display_options.confirm_button_label.clone(), ctx);
        });

        self.variant = variant;
        self.visible = true;

        ctx.notify();
    }

    pub fn hide(&mut self, ctx: &mut ViewContext<Self>) {
        self.visible = false;
        ctx.notify();
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
            display_options.title_text.clone(),
            Some(display_options.description_text.clone()),
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
