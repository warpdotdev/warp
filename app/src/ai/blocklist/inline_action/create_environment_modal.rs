use crate::{
    settings_view::handoff_environment_creation_modal::{
        HandoffEnvironmentCreationModal, HandoffEnvironmentCreationModalEvent,
    },
    view_components::DismissibleToast,
    workspace::ToastStack,
};
use warpui::{
    elements::{ChildView, Element, Empty},
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

#[derive(Debug, Clone)]
pub enum CreateEnvironmentModalEvent {
    Cancelled,
    Created { environment_id: String },
}

pub struct CreateEnvironmentModal {
    visible: bool,
    handoff_modal: ViewHandle<HandoffEnvironmentCreationModal>,
}

impl CreateEnvironmentModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let handoff_modal =
            ctx.add_typed_action_view(HandoffEnvironmentCreationModal::new_for_orchestration);
        ctx.subscribe_to_view(&handoff_modal, |me, _, event, ctx| match event {
            HandoffEnvironmentCreationModalEvent::Created { env_id } => {
                me.visible = false;
                ctx.emit(CreateEnvironmentModalEvent::Created {
                    environment_id: env_id.uid(),
                });
                ctx.notify();
            }
            HandoffEnvironmentCreationModalEvent::Cancelled => {
                me.cancel(ctx);
            }
            HandoffEnvironmentCreationModalEvent::CreationFailed { error_message } => {
                me.visible = false;
                me.show_error_toast(
                    format!("Failed to create environment: {error_message}"),
                    ctx,
                );
                ctx.notify();
            }
        });

        Self {
            visible: false,
            handoff_modal,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn show(&mut self, ctx: &mut ViewContext<Self>) {
        self.visible = true;
        self.handoff_modal.update(ctx, |modal, ctx| {
            modal.show(ctx);
        });
        ctx.focus(&self.handoff_modal);
        ctx.notify();
    }

    pub fn hide(&mut self, ctx: &mut ViewContext<Self>) {
        self.visible = false;
        ctx.notify();
    }

    fn cancel(&mut self, ctx: &mut ViewContext<Self>) {
        self.hide(ctx);
        ctx.emit(CreateEnvironmentModalEvent::Cancelled);
    }

    fn show_error_toast(&self, message: String, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.add_ephemeral_toast(DismissibleToast::error(message), window_id, ctx);
        });
    }
}

impl Entity for CreateEnvironmentModal {
    type Event = CreateEnvironmentModalEvent;
}

impl TypedActionView for CreateEnvironmentModal {
    type Action = ();

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}

impl View for CreateEnvironmentModal {
    fn ui_name() -> &'static str {
        "CreateEnvironmentModal"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        if !self.visible {
            return Empty::new().finish();
        }

        ChildView::new(&self.handoff_modal).finish()
    }
}

#[cfg(test)]
#[path = "create_environment_modal_tests.rs"]
mod tests;
