use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields};
use crate::pane_group::{PaneHeaderAction, PaneHeaderCustomAction};
use crate::terminal::view::TerminalAction;
use crate::{
    appearance::Appearance,
    ui_components::{buttons::icon_button, icons::Icon},
};
use instant::Duration;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use session_sharing_protocol::common::{ParticipantId, ParticipantInfo, Role};
use session_sharing_protocol::sharer::RoleUpdateReason;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{
    accessibility::AccessibilityContent,
    elements::{
        Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius,
        CrossAxisAlignment, Fill, Flex, Hoverable, MainAxisAlignment, MouseStateHandle,
        OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack,
    },
    platform::Cursor,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};
use warpui::{FocusContext, ViewHandle};

use super::render_util::non_hoverable_participant_avatar;

#[derive(Debug, Clone)]
pub enum HoveredElement {
    Avatar,
    ContextMenu,
}

#[derive(Debug, Clone)]
pub enum ParticipantAvatarAction {
    ScrollToSharedSessionParticipant {
        participant_id: ParticipantId,
    },
    UpdateRole {
        participant_id: ParticipantId,
        role: Role,
    },
    OpenTooltip,
    CloseTooltip,
    /// Opens the context menu on hover
    HoveredIn(HoveredElement),
    /// Closes context menu only if both elements
    /// have been hovered out of
    HoveredOut(HoveredElement),
}

pub enum ParticipantAvatarEvent {
    ScrollToSharedSessionParticipant {
        participant_id: ParticipantId,
    },
    UpdateRole {
        participant_id: ParticipantId,
        role: Role,
    },
    MenuOpened {
        participant_id: ParticipantId,
    },
    MenuClosed,
}

pub struct ParticipantAvatarView {
    // Field from role of [`PresenceManager`]
    // Indicates whether we ourselves are the sharer
    is_manager_sharer: bool,

    // Fields from [`Participant`] needed for rendering
    participant_id: ParticipantId,
    display_name: String,
    image_url: Option<String>,
    participant_color: ColorU,
    is_muted: bool,
    role: Option<Role>,

    // Mouse state to handle hover and click on avatar
    mouse_state_handle: MouseStateHandle,

    // Context menu fields
    menu: ViewHandle<Menu<ParticipantAvatarAction>>,
    is_menu_open: bool,
    is_menu_hovered: bool,
    is_avatar_hovered: bool,
    menu_mouse_state_handle: MouseStateHandle,
    close_menu_abort_handle: Option<SpawnedFutureHandle>,
    // Avatar context menu shouldn't trigger
    // while the pane header overflow menu is open
    is_pane_header_overflow_menu_open: bool,
}

impl ParticipantAvatarView {
    pub fn new(
        is_manager_sharer: bool,
        info: ParticipantInfo,
        participant_color: ColorU,
        is_muted: bool,
        role: Option<Role>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let menu = ctx.add_typed_action_view(|_| Menu::new().with_width(170.));
        ctx.subscribe_to_view(&menu, move |me, _, event, ctx| {
            me.handle_menu_event(event, ctx);
        });

        Self {
            is_manager_sharer,
            participant_id: info.id,
            display_name: info.profile_data.display_name,
            image_url: info.profile_data.photo_url,
            participant_color,
            is_muted,
            role,
            mouse_state_handle: Default::default(),
            menu,
            is_menu_open: false,
            is_menu_hovered: false,
            is_avatar_hovered: false,
            menu_mouse_state_handle: Default::default(),
            close_menu_abort_handle: None,
            is_pane_header_overflow_menu_open: false,
        }
    }

    pub fn set_participant_id(&mut self, id: ParticipantId) {
        self.participant_id = id;
    }

    pub fn set_display_name(&mut self, name: String) {
        self.display_name = name;
    }

    pub fn set_image_url(&mut self, path: Option<String>) {
        self.image_url = path;
    }

    pub fn set_participant_color(&mut self, color: ColorU) {
        self.participant_color = color;
    }

    pub fn set_is_muted(&mut self, is_muted: bool) {
        self.is_muted = is_muted;
    }

    pub fn set_role(&mut self, role: Option<Role>) {
        self.role = role;
    }

    pub fn set_is_pane_header_overflow_menu_open(&mut self, is_open: bool) {
        self.is_pane_header_overflow_menu_open = is_open;
    }

    pub fn is_menu_open(&self) -> bool {
        self.is_menu_open
    }

    fn context_menu_items(&self, app: &AppContext) -> Vec<MenuItem<ParticipantAvatarAction>> {
        let participant_id = self.participant_id.clone();
        let mut items = vec![MenuItemFields::new(self.display_name.clone())
            .with_disabled(true)
            .into_item()];

        match self.role {
            Some(Role::Reader) => {
                items.extend([
                    MenuItemFields::new(crate::i18n::tr_static(app, "Make editor"))
                        .with_on_select_action(ParticipantAvatarAction::UpdateRole {
                            participant_id,
                            role: Role::Executor,
                        })
                        .into_item(),
                ])
            }
            Some(Role::Executor) => {
                items.extend([
                    MenuItemFields::new(crate::i18n::tr_static(app, "Make viewer"))
                        .with_on_select_action(ParticipantAvatarAction::UpdateRole {
                            participant_id,
                            role: Role::Reader,
                        })
                        .into_item(),
                ])
            }
            // Sharer does not have context menu
            _ => {}
        }

        items
    }

    fn handle_menu_event(&mut self, event: &MenuEvent, ctx: &mut ViewContext<Self>) {
        if let MenuEvent::Close { .. } = event {
            self.close_context_menu(ctx);
        }
    }

    pub fn open_context_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_menu_open = true;
        self.menu.update(ctx, |menu, ctx| {
            let items = self.context_menu_items(ctx);
            menu.set_items(items, ctx);
        });
        ctx.notify();
    }

    pub fn close_context_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_menu_open = false;
        ctx.notify();
    }

    fn render_edit_icon(&self, appearance: &Appearance) -> Box<dyn Element> {
        let background = appearance.theme().surface_3();
        Container::new(
            ConstrainedBox::new(
                Icon::Edit
                    .to_warpui_icon(appearance.theme().foreground())
                    .finish(),
            )
            .with_height(8.)
            .with_width(8.)
            .finish(),
        )
        .with_uniform_padding(2.)
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(
            warpui::elements::Radius::Percentage(50.),
        ))
        .finish()
    }

    /// Helper function to render avatar context menu.
    /// Handles events on hover.
    fn render_menu(&self) -> Box<dyn Element> {
        let is_menu_hovered = self.is_menu_hovered;
        Hoverable::new(self.menu_mouse_state_handle.clone(), |_| {
            ChildView::new(&self.menu).finish()
        })
        .on_hover(move |mouse_in, ctx, _, _| {
            if mouse_in & !is_menu_hovered {
                // Ensure menu isn't already being hovered over
                ctx.dispatch_typed_action(ParticipantAvatarAction::HoveredIn(
                    HoveredElement::ContextMenu,
                ));
            } else if !mouse_in {
                ctx.dispatch_typed_action(ParticipantAvatarAction::HoveredOut(
                    HoveredElement::ContextMenu,
                ));
            }
        })
        .finish()
    }

    /// Helper function to render non-hoverable participant avatar.
    /// Specifically handles adding edit icon to participants with `Role::Executor`.
    fn render_participant_avatar(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_executor = self.role.is_some_and(|r| r.can_execute());
        let avatar = non_hoverable_participant_avatar(
            self.display_name.clone(),
            self.image_url.clone(),
            self.participant_color,
            self.is_muted,
            is_executor,
            app,
        );

        let mut stack = Stack::new();
        stack.add_positioned_child(
            avatar,
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::ParentBySize,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );

        if is_executor {
            let icon = self.render_edit_icon(appearance);
            stack.add_positioned_child(
                icon,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::ParentBySize,
                    ParentAnchor::BottomRight,
                    ChildAnchor::BottomRight,
                ),
            );
        }

        Container::new(
            ConstrainedBox::new(stack.finish())
                .with_min_height(20.)
                .with_min_width(20.)
                .finish(),
        )
        .with_vertical_padding(2.)
        .finish()
    }

    /// Helper function to render hoverable participant avatar.
    /// Handles hover and click events.
    fn render_hoverable_participant_avatar(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let participant_id = self.participant_id.clone();
        let is_manager_sharer = self.is_manager_sharer;
        let is_avatar_hovered = self.is_avatar_hovered;

        Hoverable::new(self.mouse_state_handle.clone(), |_| {
            self.render_participant_avatar(appearance, app)
        })
        .with_cursor(Cursor::PointingHand)
        .on_hover(move |mouse_in, ctx, _, _| {
            match (mouse_in, is_manager_sharer) {
                (true, true) => {
                    if !is_avatar_hovered {
                        ctx.dispatch_typed_action(ParticipantAvatarAction::HoveredIn(
                            HoveredElement::Avatar,
                        ))
                    }
                }
                (true, false) => ctx.dispatch_typed_action(ParticipantAvatarAction::OpenTooltip),
                (false, true) => ctx.dispatch_typed_action(ParticipantAvatarAction::HoveredOut(
                    HoveredElement::Avatar,
                )),
                (false, false) => ctx.dispatch_typed_action(ParticipantAvatarAction::CloseTooltip),
            };
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ParticipantAvatarAction::ScrollToSharedSessionParticipant {
                participant_id: participant_id.clone(),
            });
        })
        .finish()
    }

    pub fn role(&self) -> Option<Role> {
        self.role
    }
}

impl Entity for ParticipantAvatarView {
    type Event = ParticipantAvatarEvent;
}

impl View for ParticipantAvatarView {
    fn ui_name() -> &'static str {
        "ParticipantAvatar"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.notify();
        }
    }

    fn accessibility_contents(&self, _ctx: &AppContext) -> Option<AccessibilityContent> {
        // TO DO
        None
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let avatar_hoverable = self.render_hoverable_participant_avatar(appearance, app);
        let mut stack = Stack::new().with_child(avatar_hoverable);

        // Add tooltip if hovering over avatars as viewer
        if !self.is_manager_sharer && self.is_avatar_hovered {
            stack.add_positioned_overlay_child(
                render_tooltip(self.display_name.clone(), appearance),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopMiddle,
                    ChildAnchor::BottomMiddle,
                ),
            );
        // Render context menu if hovering over viewer avatar as a sharer
        } else if self.is_manager_sharer
            && self.is_menu_open
            && !self.is_pane_header_overflow_menu_open
            && self.role.is_some()
        {
            stack.add_positioned_overlay_child(
                self.render_menu(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        stack.finish()
    }
}

impl TypedActionView for ParticipantAvatarView {
    type Action = ParticipantAvatarAction;

    fn handle_action(&mut self, action: &ParticipantAvatarAction, ctx: &mut ViewContext<Self>) {
        match action {
            ParticipantAvatarAction::ScrollToSharedSessionParticipant { participant_id } => {
                ctx.emit(ParticipantAvatarEvent::ScrollToSharedSessionParticipant {
                    participant_id: participant_id.clone(),
                });
            }
            ParticipantAvatarAction::UpdateRole {
                participant_id,
                role,
            } => {
                ctx.emit(ParticipantAvatarEvent::UpdateRole {
                    participant_id: participant_id.clone(),
                    role: *role,
                });
            }
            ParticipantAvatarAction::OpenTooltip => {
                self.is_avatar_hovered = true;
            }
            ParticipantAvatarAction::CloseTooltip => {
                self.is_avatar_hovered = false;
            }
            ParticipantAvatarAction::HoveredIn(menu_source) => {
                // Abort closing timer on open
                if let Some(old_abort_handle) = self.close_menu_abort_handle.take() {
                    old_abort_handle.abort();
                }
                // Update hover state
                match menu_source {
                    HoveredElement::Avatar => self.is_avatar_hovered = true,
                    HoveredElement::ContextMenu => self.is_menu_hovered = true,
                }
                self.open_context_menu(ctx);
                ctx.emit(ParticipantAvatarEvent::MenuOpened {
                    participant_id: self.participant_id.clone(),
                });
            }
            ParticipantAvatarAction::HoveredOut(menu_source) => {
                // Reset timer, if old one is still in progress
                if let Some(old_abort_handle) = self.close_menu_abort_handle.take() {
                    old_abort_handle.abort();
                }
                // Update hover state
                match menu_source {
                    HoveredElement::Avatar => self.is_avatar_hovered = false,
                    HoveredElement::ContextMenu => self.is_menu_hovered = false,
                }
                let should_close_menu = !self.is_avatar_hovered && !self.is_menu_hovered;

                // Add delay before closing
                if should_close_menu {
                    let close_menu_abort_handle = ctx.spawn_abortable(
                        Timer::after(Duration::from_millis(100)),
                        move |me, _, ctx| {
                            if should_close_menu {
                                me.close_context_menu(ctx);
                                ctx.emit(ParticipantAvatarEvent::MenuClosed);
                            }
                        },
                        |_, _| (),
                    );
                    self.close_menu_abort_handle = Some(close_menu_abort_handle);
                }
            }
        }
    }
}

pub fn render_tooltip(label: String, appearance: &Appearance) -> Box<dyn Element> {
    let tooltip_background = appearance.theme().tooltip_background();
    appearance
        .ui_builder()
        .tool_tip(label)
        .with_style(UiComponentStyles {
            font_size: Some(12.),
            background: Some(Fill::Solid(tooltip_background)),
            font_color: Some(appearance.theme().background().into_solid()),
            ..Default::default()
        })
        .build()
        .finish()
}

/// Helper function to render a button that revokes executor role from all viewers.
pub fn render_revoke_all_button(
    mouse_state_handle: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let edit = Icon::Edit
        .to_warpui_icon(appearance.theme().foreground())
        .finish();
    let slash = Icon::Slash
        .to_warpui_icon(appearance.theme().terminal_colors().normal.red.into())
        .finish();
    let mut stack = Stack::new().with_constrain_absolute_children();

    stack.add_positioned_child(
        edit,
        OffsetPositioning::offset_from_parent(
            vec2f(0., 0.),
            ParentOffsetBounds::WindowByPosition,
            ParentAnchor::TopLeft,
            ChildAnchor::TopLeft,
        ),
    );

    stack.add_positioned_child(
        slash,
        OffsetPositioning::offset_from_parent(
            vec2f(0., 0.),
            ParentOffsetBounds::WindowByPosition,
            ParentAnchor::TopLeft,
            ChildAnchor::TopLeft,
        ),
    );

    Hoverable::new(mouse_state_handle, |state| {
        let mut button = Container::new(
            ConstrainedBox::new(stack.finish())
                .with_width(16.)
                .with_height(16.)
                .finish(),
        )
        .with_border(Border::all(1.))
        .with_uniform_padding(4.)
        .with_margin_right(2.);

        let mut stack = Stack::new();
        if state.is_hovered() {
            let background_color = if state.is_clicked() {
                appearance.theme().background().into()
            } else {
                appearance.theme().surface_2().into()
            };
            button = button
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_background_color(background_color)
                .with_border(
                    Border::all(1.).with_border_color(appearance.theme().surface_3().into()),
                );

            stack.add_positioned_child(
                render_tooltip("Revoke all edit permissions".to_string(), appearance),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 3.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::BottomMiddle,
                    ChildAnchor::TopMiddle,
                ),
            );
        }

        stack.add_child(button.finish());
        stack.finish()
    })
    .on_click(|ctx, _, _| {
        // We have to dispatch a pane header action because the button is rendered in the pane header.
        ctx.dispatch_typed_action(PaneHeaderCustomAction::<TerminalAction, TerminalAction>(
            TerminalAction::MakeAllParticipantsReaders {
                reason: RoleUpdateReason::UpdatedBySharer,
            },
        ));
    })
    .finish()
}

/// Helper function to render a button that indicates a viewer's role.
pub fn render_viewer_role_button(
    role: Option<Role>,
    mouse_state_handle: MouseStateHandle,
    menu_handle: Option<ViewHandle<Menu<PaneHeaderAction<TerminalAction, TerminalAction>>>>,
    is_menu_open: bool,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let icon = match role {
        Some(role) if role.can_execute() => Icon::Edit,
        _ => Icon::Eye,
    };

    let ui_builder = appearance.ui_builder().clone();
    let tooltip = crate::i18n::tr_static(app, "Change role").to_string();
    let mut stack = Stack::new();
    let button = icon_button(appearance, icon, false, mouse_state_handle.clone())
        .with_tooltip(move || ui_builder.tool_tip(tooltip.clone()).build().finish())
        .build()
        .on_click(|ctx, _, _| {
            // We have to dispatch a pane header action because the button is rendered in the pane header.
            ctx.dispatch_typed_action(PaneHeaderCustomAction::<TerminalAction, TerminalAction>(
                TerminalAction::OpenSharedSessionViewerRoleMenu,
            ));
        })
        .finish();

    stack.add_child(button);

    if let Some(menu) = menu_handle {
        if is_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&menu).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }
    }

    Container::new(stack.finish()).with_margin_left(8.).finish()
}

/// Helper function to render participant avatar list and role buttons in the pane header.
pub fn render_participants_and_role_elements(
    participants: Vec<ViewHandle<ParticipantAvatarView>>,
    role: Option<Role>,
    mouse_state_handle: MouseStateHandle,
    menu_handle: Option<ViewHandle<Menu<PaneHeaderAction<TerminalAction, TerminalAction>>>>,
    is_menu_open: bool,
    hide_role_change_button: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let mut row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Only render button for sharer (client is sharer iff role is none)
    // when there exists viewers that are executors.
    let num_executors = participants
        .iter()
        .filter(|participant| {
            participant
                .as_ref(app)
                .role()
                .is_some_and(|r| r.can_execute())
        })
        .count();
    if role.is_none() && num_executors > 0 {
        row.add_child(render_revoke_all_button(
            mouse_state_handle.clone(),
            appearance,
        ));
    }

    for participant in participants.iter() {
        row.add_child(
            Container::new(ChildView::new(participant).finish())
                .with_horizontal_margin(1.)
                .finish(),
        );
    }

    // Only render button for viewer, unless hide_role_change_button is true
    // (e.g., in cloud mode conversations where role changes are not supported)
    if role.is_some() && !hide_role_change_button {
        row.add_child(render_viewer_role_button(
            role,
            mouse_state_handle.clone(),
            menu_handle.clone(),
            is_menu_open,
            appearance,
            app,
        ));
        Container::new(row.finish()).finish()
    } else {
        row.finish()
    }
}
