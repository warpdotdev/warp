use warpui::elements::{ChildView, Element, Empty, ParentElement, Wrap};
use warpui::{AppContext, Entity, TypedActionView, View, ViewContext, ViewHandle};

use crate::ai::agent_management::notifications::{
    NotificationAction, NotificationActionKind, NotificationActionStyle, NotificationId,
};
use crate::view_components::action_button::{
    ActionButton, ButtonSize, PrimaryTheme, SecondaryTheme,
};

const BUTTON_SPACING: f32 = 8.;

/// A view that renders a notification's inline action buttons (e.g. the
/// auto-handoff sleep prompt's Enable/Dismiss). Mirrors [`ArtifactButtonsRow`]:
/// it owns the child [`ActionButton`] views and re-emits their clicks as a
/// [`NotificationActionButtonsRowEvent`] that the owning notification surface
/// (toast stack or mailbox) subscribes to.
pub struct NotificationActionButtonsRow {
    buttons: Vec<ViewHandle<ActionButton>>,
}

impl NotificationActionButtonsRow {
    pub fn new(
        notification_id: NotificationId,
        actions: &[NotificationAction],
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self {
            buttons: collect_buttons(notification_id, actions, ctx),
        }
    }
}

pub enum NotificationActionButtonsRowEvent {
    Clicked {
        notification_id: NotificationId,
        kind: NotificationActionKind,
    },
}

/// Typed action dispatched by an individual button up to its parent
/// [`NotificationActionButtonsRow`]. Public only to satisfy the
/// [`TypedActionView`] associated-type bound; constructed internally.
#[derive(Debug, Clone, Copy)]
pub struct NotificationActionButtonClick {
    notification_id: NotificationId,
    kind: NotificationActionKind,
}

impl Entity for NotificationActionButtonsRow {
    type Event = NotificationActionButtonsRowEvent;
}

impl View for NotificationActionButtonsRow {
    fn ui_name() -> &'static str {
        "NotificationActionButtonsRow"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        if self.buttons.is_empty() {
            return Empty::new().finish();
        }

        Wrap::row()
            .with_spacing(BUTTON_SPACING)
            .with_run_spacing(BUTTON_SPACING)
            .with_children(
                self.buttons
                    .iter()
                    .map(|button| ChildView::new(button).finish()),
            )
            .finish()
    }
}

impl TypedActionView for NotificationActionButtonsRow {
    type Action = NotificationActionButtonClick;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        ctx.emit(NotificationActionButtonsRowEvent::Clicked {
            notification_id: action.notification_id,
            kind: action.kind,
        });
    }
}

fn collect_buttons(
    notification_id: NotificationId,
    actions: &[NotificationAction],
    ctx: &mut ViewContext<NotificationActionButtonsRow>,
) -> Vec<ViewHandle<ActionButton>> {
    actions
        .iter()
        .map(|action| {
            let label = action.label.clone();
            let style = action.style;
            let kind = action.kind;
            ctx.add_typed_action_view(move |_| make_button(notification_id, label, style, kind))
        })
        .collect()
}

fn make_button(
    notification_id: NotificationId,
    label: String,
    style: NotificationActionStyle,
    kind: NotificationActionKind,
) -> ActionButton {
    let click = NotificationActionButtonClick {
        notification_id,
        kind,
    };
    let on_click = move |ctx: &mut warpui::EventContext| {
        ctx.dispatch_typed_action(click);
    };
    match style {
        NotificationActionStyle::Primary => ActionButton::new(label, PrimaryTheme)
            .with_size(ButtonSize::Small)
            .on_click(on_click),
        NotificationActionStyle::Secondary => ActionButton::new(label, SecondaryTheme)
            .with_size(ButtonSize::Small)
            .on_click(on_click),
    }
}
