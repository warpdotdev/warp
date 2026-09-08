use std::time::Duration;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use uuid::Uuid;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::elements::{
    ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult,
    DropShadow, EventHandler, Expanded, Flex, Hoverable, Icon, MouseStateHandle, OffsetPositioning,
    Padding, ParentElement, PositionedElementAnchor, PositionedElementOffsetBounds, Radius,
    SavePosition, Stack,
};
use warpui::keymap::Keystroke;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, EntityId, SingletonEntity, TypedActionView, View, ViewContext,
    WindowId,
};

use crate::appearance::Appearance;
use crate::settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier};
use crate::terminal::view::TerminalAction;
use crate::util::bindings::keybinding_name_to_keystroke;
use crate::workspace::{Workspace, WorkspaceAction};

const AGENT_TOAST_WIDTH: f32 = 260.;
const AGENT_TOAST_PADDING: f32 = 12.;
const AGENT_TOAST_CORNER_RADIUS: f32 = 4.;
const CLOSE_BUTTON_SIZE: f32 = 20.;

/// Data for an individual agent toast
struct AgentToastData {
    /// The toast itself
    toast: AgentToast,
    /// Abort handle for timeout-based dismissal
    abort_handle: Option<SpawnedFutureHandle>,
    /// Unique identifier for the toast
    uuid: Uuid,
}

/// A stack of agent-specific toasts for displaying task completion notifications
pub struct AgentToastStack {
    timeout: Duration,
    toasts: Vec<AgentToastData>,
    /// Cached keystroke for the jump to latest toast action
    jump_to_toast_shortcut: Option<Keystroke>,
    /// Navigation data for the most recent toast. Persists even after toast is dismissed
    latest_toast_navigation_data: Option<(WindowId, usize, EntityId)>,
}

impl AgentToastStack {
    pub fn new(timeout: Duration, ctx: &mut ViewContext<Self>) -> Self {
        // Set up caching for the keyboard shortcut
        let jump_to_toast_shortcut =
            keybinding_name_to_keystroke("workspace:jump_to_latest_toast", ctx);

        // Subscribe to keybinding changes to update the cached shortcut
        ctx.subscribe_to_model(
            &KeybindingChangedNotifier::handle(ctx),
            move |me, _, event, ctx| {
                let KeybindingChangedEvent::BindingChanged {
                    binding_name,
                    new_trigger,
                } = event;
                if binding_name == "workspace:jump_to_latest_toast" {
                    me.jump_to_toast_shortcut = new_trigger.clone();
                    ctx.notify();
                }
            },
        );

        Self {
            timeout,
            toasts: Vec::new(),
            jump_to_toast_shortcut,
            latest_toast_navigation_data: None,
        }
    }

    /// Add a new agent toast to the stack
    pub fn add_toast(&mut self, toast: AgentToast, ctx: &mut ViewContext<Self>) {
        let uuid = Uuid::new_v4();
        let abort_handle = ctx.spawn_abortable(
            Timer::after(self.timeout),
            move |view, _, ctx| view.dismiss_toast_by_uuid(&uuid, ctx),
            |_, _| {},
        );

        self.latest_toast_navigation_data =
            Some((toast.window_id, toast.tab_index, toast.terminal_view_id));

        self.toasts.push(AgentToastData {
            toast,
            abort_handle: Some(abort_handle),
            uuid,
        });

        ctx.notify();
    }

    /// Dismiss a toast by its UUID
    pub fn dismiss_toast_by_uuid(&mut self, uuid: &Uuid, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.toasts.iter().position(|toast| toast.uuid == *uuid) {
            let toast_data = self.toasts.remove(index);
            if let Some(abort_handle) = toast_data.abort_handle {
                abort_handle.abort();
            }
            ctx.notify();
        }
    }

    /// Cancel the dismissal timeout for a toast
    pub fn cancel_dismissal_timeout(&mut self, uuid: &Uuid) {
        if let Some(toast_data) = self.toasts.iter_mut().find(|toast| toast.uuid == *uuid)
            && let Some(abort_handle) = toast_data.abort_handle.take()
        {
            abort_handle.abort();
        }
    }

    /// Start a new dismissal timeout for a toast
    pub fn start_dismissal_timeout(&mut self, uuid: Uuid, ctx: &mut ViewContext<Self>) {
        if let Some(toast_data) = self.toasts.iter_mut().find(|toast| toast.uuid == uuid) {
            // Cancel any existing timeout
            if let Some(abort_handle) = toast_data.abort_handle.take() {
                abort_handle.abort();
            }

            // Start a new timeout
            let abort_handle = ctx.spawn_abortable(
                Timer::after(self.timeout),
                move |view, _, ctx| view.dismiss_toast_by_uuid(&uuid, ctx),
                |_, _| {},
            );

            toast_data.abort_handle = Some(abort_handle);
        }
    }

    /// Get the UUID of the most recent (latest) toast
    pub fn latest_toast_uuid(&self) -> Option<Uuid> {
        self.toasts.last().map(|toast_data| toast_data.uuid)
    }

    pub fn get_latest_toast_navigation_data(&self) -> Option<(WindowId, usize, EntityId)> {
        self.latest_toast_navigation_data
    }
}

impl View for AgentToastStack {
    fn ui_name() -> &'static str {
        "AgentToastStack"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut rendered_toasts =
            Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Center);

        let latest_toast_uuid = self.toasts.last().map(|toast| toast.uuid);

        // Render toasts in reverse order so most recent appears at top
        for toast_data in self.toasts.iter().rev() {
            let is_latest = latest_toast_uuid == Some(toast_data.uuid);
            rendered_toasts.add_child(
                Container::new(toast_data.toast.render(
                    app,
                    toast_data.uuid,
                    is_latest,
                    self.jump_to_toast_shortcut.clone(),
                ))
                .with_margin_bottom(5.)
                .finish(),
            );
        }

        Container::new(rendered_toasts.finish())
            // Tried handling this with OffsetPositioning as we do with top margin.
            // For whatever reason, it did not work for right margin when using TopRight alignment.
            .with_margin_right(AGENT_TOAST_PADDING)
            .finish()
    }
}

impl Entity for AgentToastStack {
    type Event = ();
}

/// Actions that can be dispatched on the agent toast stack
#[derive(Debug)]
pub enum AgentToastAction {
    ClickDismissButton(Uuid),
    CancelDismissalTimeout(Uuid),
    StartDismissalTimeout(Uuid),
    ClickToastBody(Uuid),
}

impl TypedActionView for AgentToastStack {
    type Action = AgentToastAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentToastAction::ClickDismissButton(uuid) => {
                self.dismiss_toast_by_uuid(uuid, ctx);
            }
            AgentToastAction::CancelDismissalTimeout(uuid) => {
                self.cancel_dismissal_timeout(uuid);
            }
            AgentToastAction::StartDismissalTimeout(uuid) => {
                self.start_dismissal_timeout(*uuid, ctx);
            }
            AgentToastAction::ClickToastBody(uuid) => {
                if let Some((window_id, tab_id, terminal_view_id)) = self
                    .toasts
                    .iter()
                    .find(|toast| toast.uuid == *uuid)
                    .map(|toast| {
                        (
                            toast.toast.window_id,
                            toast.toast.tab_index,
                            toast.toast.terminal_view_id,
                        )
                    })
                {
                    ctx.windows().show_window_and_focus_app(window_id);

                    if let Some(workspaces) = ctx.views_of_type::<Workspace>(window_id)
                        && let Some(handle) = workspaces.first()
                    {
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            handle.id(),
                            &WorkspaceAction::ActivateTab(tab_id),
                        );
                    }
                    ctx.dispatch_typed_action_for_view(
                        window_id,
                        terminal_view_id,
                        &TerminalAction::Focus,
                    );
                }
                self.dismiss_toast_by_uuid(uuid, ctx);
            }
        }
    }
}

/// A specialized toast for Agent Mode completion notifications
#[derive(Clone)]
pub struct AgentToast {
    task_name: String,
    icon: Icon,
    window_id: WindowId,
    tab_index: usize,
    terminal_view_id: EntityId,
    close_button_mouse_state: MouseStateHandle,
    container_hover_state: MouseStateHandle,
    close_button_hover_state: MouseStateHandle,
}

impl AgentToast {
    pub fn new(
        task_name: String,
        icon: Icon,
        window_id: WindowId,
        tab_index: usize,
        terminal_view_id: EntityId,
    ) -> Self {
        Self {
            task_name,
            icon,
            window_id,
            tab_index,
            terminal_view_id,
            close_button_mouse_state: Default::default(),
            container_hover_state: Default::default(),
            close_button_hover_state: Default::default(),
        }
    }

    fn text_color(&self, appearance: &Appearance) -> ColorU {
        appearance
            .theme()
            .main_text_color(appearance.theme().background())
            .into()
    }

    fn position_id(&self, uuid: Uuid) -> String {
        format!("agent_toast_{uuid}")
    }

    pub fn render(
        &self,
        app: &AppContext,
        uuid: Uuid,
        is_latest: bool,
        jump_to_toast_keystroke: Option<Keystroke>,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let ui_builder = appearance.ui_builder();

        let mut row = Flex::row();

        let icon_size = appearance.ui_font_size() * 1.2;
        row.add_child(
            Container::new(
                ConstrainedBox::new(self.icon.finish())
                    .with_height(icon_size)
                    .with_width(icon_size)
                    .finish(),
            )
            .with_margin_right(8.)
            // Accounts for line height
            .with_vertical_margin(1.)
            .finish(),
        );

        row.add_child(
            Expanded::new(
                1.,
                Flex::column()
                    .with_child({
                        let font_size = appearance.ui_font_size() * 1.2;
                        let line_height = font_size * appearance.line_height_ratio();
                        let max_height_for_3_lines = line_height * 3.0;

                        let text_content = ConstrainedBox::new(
                            ui_builder
                                .wrappable_text(self.task_name.clone(), true)
                                .with_style(UiComponentStyles {
                                    font_size: Some(font_size),
                                    font_color: Some(self.text_color(appearance)),
                                    ..Default::default()
                                })
                                .build()
                                .finish(),
                        )
                        .with_max_height(max_height_for_3_lines)
                        .finish();

                        if is_latest {
                            // Add keyboard shortcut to the latest toast
                            let mut row =
                                Flex::row().with_child(Expanded::new(1., text_content).finish());
                            if let Some(keystroke) = jump_to_toast_keystroke {
                                row = row.with_child(
                                    self.render_keyboard_shortcut(app, appearance, keystroke),
                                );
                            }
                            row.finish()
                        } else {
                            text_content
                        }
                    })
                    .finish(),
            )
            .finish(),
        );

        let row = ConstrainedBox::new(row.finish())
            .with_max_width(AGENT_TOAST_WIDTH)
            .finish();

        self.render_container(row, appearance, uuid)
    }

    fn render_container(
        &self,
        content: Box<dyn Element>,
        appearance: &Appearance,
        uuid: Uuid,
    ) -> Box<dyn Element> {
        let navigation_action = WorkspaceAction::FocusTerminalViewInWorkspace {
            terminal_view_id: self.terminal_view_id,
        };
        EventHandler::new(
            Hoverable::new(self.container_hover_state.clone(), |mouse_state| {
                let container = Container::new(content)
                    .with_padding(Padding::uniform(AGENT_TOAST_PADDING))
                    .with_background(appearance.theme().surface_3())
                    .with_drop_shadow(DropShadow::default())
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                        AGENT_TOAST_CORNER_RADIUS,
                    )));

                let mut stack = Stack::new().with_child(
                    SavePosition::new(container.finish(), &self.position_id(uuid)).finish(),
                );

                let is_close_button_hovered = self
                    .close_button_hover_state
                    .lock()
                    .is_ok_and(|state| state.is_hovered());

                if mouse_state.is_hovered() || is_close_button_hovered {
                    stack.add_positioned_overlay_child(
                        self.render_close_button(appearance, uuid),
                        OffsetPositioning::offset_from_save_position_element(
                            self.position_id(uuid),
                            vec2f(4., -4.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::TopRight,
                            ChildAnchor::TopRight,
                        ),
                    );
                }
                stack.finish()
            })
            .on_hover(move |is_hovered, ctx, _, _| {
                // Cancel dismissal timeout when hovering
                if is_hovered {
                    ctx.dispatch_typed_action(AgentToastAction::CancelDismissalTimeout(uuid));
                } else {
                    ctx.dispatch_typed_action(AgentToastAction::StartDismissalTimeout(uuid));
                }
            })
            .finish(),
        )
        .on_left_mouse_down(move |ctx, _, _| {
            // Dismiss immediately when clicked on the toast body
            ctx.dispatch_typed_action(AgentToastAction::ClickToastBody(uuid));

            ctx.dispatch_typed_action(navigation_action.clone());
            DispatchEventResult::PropagateToParent
        })
        .finish()
    }

    fn render_keyboard_shortcut(
        &self,
        _app: &AppContext,
        appearance: &Appearance,
        keystroke: Keystroke,
    ) -> Box<dyn Element> {
        use warpui::ui_components::keyboard_shortcut::KeyboardShortcut;

        use crate::ui_components::blended_colors;

        let theme = appearance.theme();

        let keybinding_style = UiComponentStyles {
            font_family_id: Some(appearance.monospace_font_family()),
            font_color: Some(blended_colors::text_main(theme, theme.surface_2())),
            font_size: Some(appearance.ui_font_size()),
            background: Some(theme.surface_2().into()),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(3.0))),
            padding: Some(Coords {
                top: 1.0,
                bottom: 1.0,
                left: 4.0,
                right: 4.0,
            }),
            ..Default::default()
        };

        Container::new(
            KeyboardShortcut::new(&keystroke, keybinding_style)
                .build()
                .finish(),
        )
        .with_margin_left(8.)
        .finish()
    }

    fn render_close_button(&self, appearance: &Appearance, uuid: Uuid) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();

        EventHandler::new(
            Hoverable::new(self.close_button_hover_state.clone(), |_| {
                Container::new(
                    ui_builder
                        .close_button(CLOSE_BUTTON_SIZE, self.close_button_mouse_state.clone())
                        .with_style(UiComponentStyles {
                            font_color: Some(appearance.theme().foreground().into()),
                            background: Some(appearance.theme().surface_2().into()),
                            border_color: Some(appearance.theme().surface_3().into()),
                            border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
                            border_width: Some(2.),
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
                            ctx.dispatch_typed_action(AgentToastAction::ClickDismissButton(uuid));
                        })
                        .finish(),
                )
                .finish()
            })
            .finish(),
        )
        .on_left_mouse_down(|_, _, _| {
            // Stop propagation so the parent toast click handler doesn't get called
            DispatchEventResult::StopPropagation
        })
        .finish()
    }
}
