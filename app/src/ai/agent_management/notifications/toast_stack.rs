use std::time::Duration;

use pathfinder_geometry::vector::vec2f;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DispatchEventResult, Element, EventHandler, Flex, Hoverable, MouseStateHandle,
    OffsetPositioning, Padding, ParentElement, PositionedElementAnchor,
    PositionedElementOffsetBounds, Radius, SavePosition, Shrinkable, Stack,
};
use warpui::keymap::Keystroke;
use warpui::platform::Cursor;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::keyboard_shortcut::KeyboardShortcut;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use crate::ai::agent_management::notifications::item_rendering::{
    create_notification_artifact_buttons_view, handle_notification_artifact_buttons_event,
    render_notification_item_content, NotificationRenderContext, OnExpandClick,
};
use crate::ai::agent_management::notifications::{NotificationId, NotificationItem};
use crate::ai::agent_management::{AgentManagementEvent, AgentNotificationsModel};
use crate::ai::artifacts::{Artifact, ArtifactButtonsRow, ArtifactButtonsRowEvent};
use crate::appearance::Appearance;
use crate::terminal::session_settings::SessionSettings;
use crate::util::bindings::keybinding_name_to_keystroke;
use crate::workspace::view::JUMP_TO_LATEST_TOAST_BINDING_NAME;
use crate::workspace::WorkspaceAction;

const CLOSE_BUTTON_SIZE: f32 = 20.;

/// Tracks the state of a single visible toast in the stack
/// (its auto-dismiss timer, hover state, and close button).
struct NotificationToastItem {
    notification_id: NotificationId,
    abort_handle: Option<SpawnedFutureHandle>,
    mouse_state: MouseStateHandle,
    close_button_mouse_state: MouseStateHandle,
    close_button_hover_state: MouseStateHandle,
    artifact_buttons_view: Option<ViewHandle<ArtifactButtonsRow>>,
    message_expanded: bool,
}

pub struct AgentNotificationToastStack {
    toasts: Vec<NotificationToastItem>,
    mailbox_is_open: bool,
}

impl Entity for AgentNotificationToastStack {
    type Event = ();
}

#[derive(Debug)]
pub enum AgentNotificationToastAction {
    CancelDismissalTimeout(NotificationId),
    StartDismissalTimeout(NotificationId),
    Click(NotificationId),
    Dismiss(NotificationId),
    ToggleMessageExpanded(NotificationId),
}

impl AgentNotificationToastStack {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let model_handle = AgentNotificationsModel::handle(ctx);
        ctx.subscribe_to_model(&model_handle, |me, _handle, event, ctx| match event {
            AgentManagementEvent::NotificationAdded { id } => {
                me.on_notification_added(*id, ctx);
            }
            AgentManagementEvent::NotificationUpdated
            | AgentManagementEvent::AllNotificationsMarkedRead => {
                me.remove_dismissed_toasts(ctx);
            }
            AgentManagementEvent::ConversationNeedsAttention { .. } => {}
        });

        Self {
            toasts: Vec::new(),
            mailbox_is_open: false,
        }
    }

    /// Updates the mailbox-open state. When opening, all visible toasts are dismissed
    /// (the mailbox already shows the same notifications).
    pub fn set_mailbox_open(&mut self, open: bool, ctx: &mut ViewContext<Self>) {
        self.mailbox_is_open = open;
        if open {
            self.dismiss_all(ctx);
        }
    }

    /// Dismiss all visible toasts (called when the mailbox opens).
    fn dismiss_all(&mut self, ctx: &mut ViewContext<Self>) {
        for entry in self.toasts.drain(..) {
            if let Some(handle) = entry.abort_handle {
                handle.abort();
            }
        }
        ctx.notify();
    }

    fn on_notification_added(&mut self, id: NotificationId, ctx: &mut ViewContext<Self>) {
        // Don't show in-app toasts when the window is not active.
        // Native desktop notifications handle the unfocused case.
        if ctx.windows().active_window() != Some(ctx.window_id()) {
            return;
        }

        // Don't show toasts when the notification mailbox is already open.
        // (dismiss_all is called on open, so any new arrival would be immediately visible in the mailbox.)
        if self.mailbox_is_open {
            return;
        }

        let notifications = AgentNotificationsModel::as_ref(ctx).notifications();
        let Some(item) = notifications.get_by_id(id) else {
            return;
        };

        // Don't show a toast for notifications that are already read
        // (e.g. the terminal was visible when the notification was created).
        if item.is_read {
            return;
        }

        // Clone artifacts before releasing the immutable borrow on ctx.
        let artifacts = item.artifacts.clone();
        let _ = notifications;

        // The notification model de-dupes by origin, so a new notification for the same
        // conversation replaces the old one with a new ID.
        self.remove_dismissed_toasts(ctx);

        let artifact_buttons_view =
            Self::create_artifact_buttons_view_from_artifacts(&artifacts, ctx);

        self.toasts.push(NotificationToastItem {
            notification_id: id,
            abort_handle: None,
            mouse_state: MouseStateHandle::default(),
            close_button_mouse_state: MouseStateHandle::default(),
            close_button_hover_state: MouseStateHandle::default(),
            artifact_buttons_view,
            message_expanded: false,
        });
        self.start_dismissal_timeout(id, ctx);

        // Evict the oldest toasts if we exceed the visible limit.
        while self.toasts.len() > 2 {
            let evicted = self.toasts.remove(0);
            if let Some(handle) = evicted.abort_handle {
                handle.abort();
            }
        }

        ctx.notify();
    }

    /// Removes toasts for notifications that no longer exist in the model or have been read.
    fn remove_dismissed_toasts(&mut self, ctx: &mut ViewContext<Self>) {
        let notifications = AgentNotificationsModel::as_ref(ctx).notifications();
        let before = self.toasts.len();
        self.toasts.retain(|entry| {
            let should_remove = notifications
                .get_by_id(entry.notification_id)
                .is_none_or(|item| item.is_read);
            if should_remove {
                if let Some(handle) = &entry.abort_handle {
                    handle.abort();
                }
                false
            } else {
                true
            }
        });
        if self.toasts.len() != before {
            ctx.notify();
        }
    }

    fn dismiss_toast_by_id(&mut self, id: &NotificationId, ctx: &mut ViewContext<Self>) {
        if let Some(idx) = self.toasts.iter().position(|e| e.notification_id == *id) {
            let entry = self.toasts.remove(idx);
            if let Some(handle) = entry.abort_handle {
                handle.abort();
            }
            ctx.notify();
        }
    }

    /// Pauses the auto-dismiss timer for a toast (e.g. while the user is hovering over it).
    fn cancel_dismissal_timeout(&mut self, id: &NotificationId) {
        if let Some(entry) = self.toasts.iter_mut().find(|e| e.notification_id == *id) {
            if let Some(handle) = entry.abort_handle.take() {
                handle.abort();
            }
        }
    }

    fn create_artifact_buttons_view_from_artifacts(
        artifacts: &[Artifact],
        ctx: &mut ViewContext<Self>,
    ) -> Option<ViewHandle<ArtifactButtonsRow>> {
        let view = create_notification_artifact_buttons_view(artifacts, ctx)?;
        ctx.subscribe_to_view(&view, Self::handle_artifact_buttons_event);
        Some(view)
    }

    fn handle_artifact_buttons_event(
        &mut self,
        _view: ViewHandle<ArtifactButtonsRow>,
        event: &ArtifactButtonsRowEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        handle_notification_artifact_buttons_event(event, ctx);
    }

    fn start_dismissal_timeout(&mut self, id: NotificationId, ctx: &mut ViewContext<Self>) {
        if let Some(entry) = self.toasts.iter_mut().find(|e| e.notification_id == id) {
            if let Some(handle) = entry.abort_handle.take() {
                handle.abort();
            }
            let duration_secs = *SessionSettings::as_ref(ctx).notification_toast_duration_secs;
            let abort_handle = ctx.spawn_abortable(
                Timer::after(Duration::from_secs(duration_secs)),
                move |me, _, ctx| me.dismiss_toast_by_id(&id, ctx),
                |_, _| {},
            );
            entry.abort_handle = Some(abort_handle);
        }
    }
}

impl TypedActionView for AgentNotificationToastStack {
    type Action = AgentNotificationToastAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentNotificationToastAction::CancelDismissalTimeout(id) => {
                self.cancel_dismissal_timeout(id);
            }
            AgentNotificationToastAction::StartDismissalTimeout(id) => {
                self.start_dismissal_timeout(*id, ctx);
            }
            AgentNotificationToastAction::Click(id) => {
                let terminal_view_id = AgentNotificationsModel::as_ref(ctx)
                    .notifications()
                    .get_by_id(*id)
                    .map(|item| item.terminal_view_id);

                AgentNotificationsModel::handle(ctx).update(ctx, |model, ctx| {
                    model.mark_item_read(*id, ctx);
                });

                if let Some(terminal_view_id) = terminal_view_id {
                    ctx.dispatch_typed_action(&WorkspaceAction::FocusTerminalViewInWorkspace {
                        terminal_view_id,
                    });
                }

                self.dismiss_toast_by_id(id, ctx);
            }
            AgentNotificationToastAction::Dismiss(id) => {
                self.dismiss_toast_by_id(id, ctx);
            }
            AgentNotificationToastAction::ToggleMessageExpanded(id) => {
                if let Some(entry) = self.toasts.iter_mut().find(|e| e.notification_id == *id) {
                    entry.message_expanded = !entry.message_expanded;
                    ctx.notify();
                }
            }
        }
    }
}

impl View for AgentNotificationToastStack {
    fn ui_name() -> &'static str {
        "AgentNotificationToastStack"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let notifications = AgentNotificationsModel::as_ref(app).notifications();
        let keystroke = keybinding_name_to_keystroke(JUMP_TO_LATEST_TOAST_BINDING_NAME, app);

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::End);

        for (i, entry) in self.toasts.iter().rev().enumerate() {
            let Some(item) = notifications.get_by_id(entry.notification_id) else {
                continue;
            };
            let is_newest = i == 0;
            let toast = render_toast(
                item,
                entry.notification_id,
                entry.mouse_state.clone(),
                entry.close_button_mouse_state.clone(),
                entry.close_button_hover_state.clone(),
                entry.artifact_buttons_view.as_ref(),
                entry.message_expanded,
                is_newest.then(|| keystroke.clone()).flatten(),
                appearance,
                app,
            );
            column.add_child(Container::new(toast).with_margin_bottom(4.).finish());
        }

        column.finish()
    }
}

fn toast_position_id(id: NotificationId) -> String {
    format!("notification_toast_{id:?}")
}

fn render_close_button(
    id: NotificationId,
    close_button_mouse_state: MouseStateHandle,
    close_button_hover_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    EventHandler::new(
        Hoverable::new(close_button_hover_state, |_| {
            Container::new(
                appearance
                    .ui_builder()
                    .close_button(CLOSE_BUTTON_SIZE, close_button_mouse_state.clone())
                    .with_style(UiComponentStyles {
                        font_color: Some(theme.foreground().into()),
                        background: Some(theme.surface_3().into()),
                        border_color: Some(theme.outline().into()),
                        border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
                        border_width: Some(1.),
                        padding: Some(Coords {
                            top: 2.,
                            bottom: 2.,
                            left: 2.,
                            right: 2.,
                        }),
                        ..Default::default()
                    })
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(AgentNotificationToastAction::Dismiss(id));
                    })
                    .finish(),
            )
            .finish()
        })
        .finish(),
    )
    .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
    .finish()
}

#[allow(clippy::too_many_arguments)]
fn render_toast(
    item: &NotificationItem,
    id: NotificationId,
    mouse_state: MouseStateHandle,
    close_button_mouse_state: MouseStateHandle,
    close_button_hover_state: MouseStateHandle,
    artifact_buttons: Option<&ViewHandle<ArtifactButtonsRow>>,
    message_expanded: bool,
    keystroke: Option<Keystroke>,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let on_expand: OnExpandClick = Box::new(move |ctx: &mut warpui::EventContext| {
        ctx.dispatch_typed_action(AgentNotificationToastAction::ToggleMessageExpanded(id));
    });
    let keybinding_hint = keystroke.map(|ks| render_keybinding_hint(ks, appearance, app));

    let content = render_notification_item_content(
        item,
        artifact_buttons,
        NotificationRenderContext::Toast,
        message_expanded,
        on_expand,
        keybinding_hint,
        appearance,
    );

    let inner_column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(content);

    let position_id = toast_position_id(id);

    EventHandler::new(
        Hoverable::new(mouse_state, move |state| {
            let bg = if state.is_hovered() {
                theme
                    .surface_2()
                    .blend(&internal_colors::fg_overlay_3(theme))
            } else {
                theme
                    .surface_2()
                    .blend(&internal_colors::fg_overlay_2(theme))
            };

            let container = Container::new(inner_column.finish())
                .with_padding(Padding::uniform(12.))
                .with_background(bg)
                .with_border(Border::all(1.).with_border_color(theme.outline().into()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)));

            let sized = ConstrainedBox::new(container.finish())
                .with_width(420.)
                .finish();

            let is_close_hovered = close_button_hover_state
                .lock()
                .is_ok_and(|s| s.is_hovered());

            let mut stack =
                Stack::new().with_child(SavePosition::new(sized, &position_id).finish());

            if state.is_hovered() || is_close_hovered {
                stack.add_positioned_overlay_child(
                    render_close_button(
                        id,
                        close_button_mouse_state.clone(),
                        close_button_hover_state.clone(),
                        appearance,
                    ),
                    OffsetPositioning::offset_from_save_position_element(
                        &position_id,
                        vec2f(-4., -4.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            }

            stack.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_hover(move |is_hovered, ctx, _, _| {
            if is_hovered {
                ctx.dispatch_typed_action(AgentNotificationToastAction::CancelDismissalTimeout(id));
            } else {
                ctx.dispatch_typed_action(AgentNotificationToastAction::StartDismissalTimeout(id));
            }
        })
        .finish(),
    )
    .on_left_mouse_down(move |ctx, _, _| {
        ctx.dispatch_typed_action(AgentNotificationToastAction::Click(id));
        DispatchEventResult::StopPropagation
    })
    .finish()
}

fn render_keybinding_hint(
    keystroke: Keystroke,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    let hint_text = appearance
        .ui_builder()
        .wrappable_text(
            crate::i18n::tr_static(app, "Open conversation").to_string(),
            false,
        )
        .with_style(UiComponentStyles {
            font_size: Some(12.),
            font_color: Some(theme.disabled_text_color(theme.surface_2()).into()),
            font_family_id: Some(appearance.ui_font_family()),
            ..Default::default()
        })
        .build()
        .finish();

    let keybinding_style = UiComponentStyles {
        font_family_id: Some(appearance.monospace_font_family()),
        font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
        font_size: Some(12.),
        background: Some(internal_colors::fg_overlay_3(theme).into()),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(3.0))),
        padding: Some(Coords {
            top: 1.0,
            bottom: 1.0,
            left: 4.0,
            right: 4.0,
        }),
        ..Default::default()
    };

    let shortcut = KeyboardShortcut::new(&keystroke, keybinding_style)
        .build()
        .finish();

    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1.0, hint_text).finish())
            .with_child(Container::new(shortcut).with_margin_left(8.).finish())
            .finish(),
    )
    .with_margin_top(8.)
    .finish()
}
