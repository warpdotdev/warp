use warp_core::ui::theme::color::internal_colors;
use warpui::elements::new_scrollable::{ScrollableAppearance, SingleAxisConfig};
use warpui::elements::{
    Border, ChildView, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Dismiss, DispatchEventResult, Element, Empty, EventHandler,
    Fill as ElementFill, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    NewScrollable, Padding, ParentElement, Radius, SavePosition, ScrollTarget,
    ScrollToPositionMode, ScrollbarWidth, Shrinkable,
};
use warpui::fonts::Weight;
use warpui::keymap::macros::id;
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use crate::ai::agent_management::notifications::item::NotificationFilter;
use crate::ai::agent_management::notifications::item_rendering::{
    create_notification_artifact_buttons_view, handle_notification_artifact_buttons_event,
    render_notification_item_content, NotificationRenderContext,
};
use crate::ai::agent_management::notifications::{
    NotificationId, NotificationItem, NotificationItems,
};
use crate::ai::agent_management::{AgentManagementEvent, AgentNotificationsModel};
use crate::ai::artifacts::{Artifact, ArtifactButtonsRow, ArtifactButtonsRowEvent};
use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize, NakedTheme};

const ITEM_PADDING: f32 = 12.;

/// Position ID prefix used with `SavePosition` so the clipped scrollable can
/// scroll keyboard-selected items into view.
const ITEM_POSITION_PREFIX: &str = "notification_mailbox_item_";

pub struct NotificationMailboxView {
    active_filter: NotificationFilter,
    scroll_state: ClippedScrollStateHandle,
    filter_button_mouse_states: Vec<MouseStateHandle>,
    close_button: ViewHandle<ActionButton>,
    mark_all_read_button: ViewHandle<ActionButton>,
    notification_mouse_states: Vec<MouseStateHandle>,
    // Cached IDs of notifications matching the active filter, in display order.
    // (Avoids re-filtering the full list on every individual item render.)
    filtered_ids: Vec<NotificationId>,
    /// Artifact button views for each filtered notification (parallel to `filtered_ids`).
    artifact_buttons_views: Vec<Option<ViewHandle<ArtifactButtonsRow>>>,
    /// Index of the currently keyboard-selected notification item, if any.
    selected_index: Option<usize>,
}

impl Entity for NotificationMailboxView {
    type Event = NotificationMailboxViewEvent;
}

#[derive(Debug, Clone)]
pub enum NotificationMailboxViewEvent {
    NavigateToTerminal { terminal_view_id: warpui::EntityId },
    Dismissed,
}

#[derive(Debug)]
pub enum NotificationMailboxViewAction {
    SetFilter(NotificationFilter),
    MarkAllRead,
    ClickItem(NotificationId),
    Dismiss,
    SelectPrevious,
    SelectNext,
    CycleFilter,
    ActivateSelected,
}

impl NotificationMailboxView {
    pub fn init(app: &mut AppContext) {
        app.register_fixed_bindings([
            FixedBinding::new(
                "up",
                NotificationMailboxViewAction::SelectPrevious,
                id!(NotificationMailboxView::ui_name()),
            ),
            FixedBinding::new(
                "down",
                NotificationMailboxViewAction::SelectNext,
                id!(NotificationMailboxView::ui_name()),
            ),
            FixedBinding::new(
                "shift-tab",
                NotificationMailboxViewAction::CycleFilter,
                id!(NotificationMailboxView::ui_name()),
            ),
            FixedBinding::new(
                "enter",
                NotificationMailboxViewAction::ActivateSelected,
                id!(NotificationMailboxView::ui_name()),
            ),
            FixedBinding::new(
                "escape",
                NotificationMailboxViewAction::Dismiss,
                id!(NotificationMailboxView::ui_name()),
            ),
        ]);
    }

    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let model_handle = AgentNotificationsModel::handle(ctx);
        ctx.subscribe_to_model(&model_handle, |me, _handle, event, ctx| match event {
            AgentManagementEvent::NotificationAdded { .. }
            | AgentManagementEvent::NotificationUpdated
            | AgentManagementEvent::AllNotificationsMarkedRead => {
                me.rebuild_filtered_ids(ctx);
                ctx.notify();
            }
            // Legacy toast path.
            AgentManagementEvent::ConversationNeedsAttention { .. } => {}
        });

        let close_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::XSmall)
                .with_tooltip(crate::i18n::tr_static(ctx, "Close"))
                .with_tooltip_sublabel("Esc")
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(NotificationMailboxViewAction::Dismiss);
                })
        });

        let mark_all_read_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Mark all as read"), NakedTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(NotificationMailboxViewAction::MarkAllRead);
                })
        });

        Self {
            active_filter: NotificationFilter::All,
            scroll_state: Default::default(),
            filter_button_mouse_states: (0..enum_iterator::cardinality::<NotificationFilter>())
                .map(|_| MouseStateHandle::default())
                .collect(),
            close_button,
            mark_all_read_button,
            notification_mouse_states: Vec::new(),
            filtered_ids: Vec::new(),
            artifact_buttons_views: Vec::new(),
            selected_index: None,
        }
    }

    /// Resets the mailbox state when opening. Called from the workspace toggle handler.
    pub fn reset_for_open(&mut self, select_first: bool, ctx: &mut ViewContext<Self>) {
        self.rebuild_filtered_ids(ctx);
        self.selected_index = if select_first && !self.filtered_ids.is_empty() {
            Some(0)
        } else {
            None
        };
    }

    fn set_active_filter(&mut self, filter: NotificationFilter, ctx: &mut ViewContext<Self>) {
        self.active_filter = filter;
        self.selected_index = None;
        self.rebuild_filtered_ids(ctx);
        ctx.notify();
    }

    fn activate_notification(&mut self, id: NotificationId, ctx: &mut ViewContext<Self>) {
        let terminal_view_id = AgentNotificationsModel::as_ref(ctx)
            .notifications()
            .get_by_id(id)
            .map(|item| item.terminal_view_id);

        AgentNotificationsModel::handle(ctx).update(ctx, |model, ctx| {
            model.mark_item_read(id, ctx);
        });

        if let Some(terminal_view_id) = terminal_view_id {
            ctx.emit(NotificationMailboxViewEvent::NavigateToTerminal { terminal_view_id });
        }
    }

    /// Refreshes the cached filtered notification IDs and mouse states.
    fn rebuild_filtered_ids(&mut self, ctx: &mut ViewContext<Self>) {
        let notifications = AgentNotificationsModel::as_ref(ctx).notifications();

        // If the active filter's tab would be hidden (0 items), fall back to "All".
        if self.active_filter != NotificationFilter::All
            && notifications.filtered_count(self.active_filter) == 0
        {
            self.active_filter = NotificationFilter::All;
        }
        self.filtered_ids = notifications
            .items_filtered(self.active_filter)
            .map(|item| item.id)
            .collect();
        self.notification_mouse_states
            .resize_with(self.filtered_ids.len(), MouseStateHandle::default);

        let artifact_data: Vec<_> = notifications
            .items_filtered(self.active_filter)
            .map(|item| item.artifacts.clone())
            .collect();
        let _ = notifications;

        self.artifact_buttons_views = artifact_data
            .iter()
            .map(|artifacts| Self::create_artifact_buttons_view_from_artifacts(artifacts, ctx))
            .collect();

        // Clamp selection to valid range after list contents change.
        if self.filtered_ids.is_empty() {
            self.selected_index = None;
        } else if let Some(idx) = self.selected_index {
            if idx >= self.filtered_ids.len() {
                self.selected_index = Some(self.filtered_ids.len() - 1);
            }
        }
    }

    fn render_item_at_index(&self, index: usize, app: &AppContext) -> Box<dyn Element> {
        let notifications = AgentNotificationsModel::as_ref(app).notifications();
        let Some(item) = self
            .filtered_ids
            .get(index)
            .and_then(|id| notifications.get_by_id(*id))
        else {
            return Empty::new().finish();
        };

        let Some(mouse_state) = self.notification_mouse_states.get(index).cloned() else {
            log::error!("missing mouse state for notification item at index {index}");
            return Empty::new().finish();
        };

        let artifact_buttons = self
            .artifact_buttons_views
            .get(index)
            .and_then(|v| v.as_ref());
        let is_selected = self.selected_index == Some(index);
        self.render_notification_item(
            item,
            mouse_state,
            artifact_buttons,
            is_selected,
            Appearance::as_ref(app),
        )
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
}

impl TypedActionView for NotificationMailboxView {
    type Action = NotificationMailboxViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            NotificationMailboxViewAction::SetFilter(filter) => {
                self.set_active_filter(*filter, ctx);
            }
            NotificationMailboxViewAction::MarkAllRead => {
                AgentNotificationsModel::handle(ctx).update(ctx, |model, ctx| {
                    model.mark_all_items_read(ctx);
                });
            }
            NotificationMailboxViewAction::ClickItem(id) => {
                self.activate_notification(*id, ctx);
            }
            NotificationMailboxViewAction::Dismiss => {
                ctx.emit(NotificationMailboxViewEvent::Dismissed);
            }
            NotificationMailboxViewAction::SelectPrevious => {
                match self.selected_index {
                    Some(idx) if idx > 0 => self.selected_index = Some(idx - 1),
                    None if !self.filtered_ids.is_empty() => self.selected_index = Some(0),
                    _ => {}
                }
                self.scroll_selected_into_view();
                ctx.notify();
            }
            NotificationMailboxViewAction::SelectNext => {
                let max = self.filtered_ids.len().saturating_sub(1);
                match self.selected_index {
                    Some(idx) if idx < max => self.selected_index = Some(idx + 1),
                    None if !self.filtered_ids.is_empty() => self.selected_index = Some(0),
                    _ => {}
                }
                self.scroll_selected_into_view();
                ctx.notify();
            }
            NotificationMailboxViewAction::CycleFilter => {
                let notifications = AgentNotificationsModel::as_ref(ctx).notifications();
                let visible = notifications.visible_filters();
                let current_pos = visible
                    .iter()
                    .position(|f| *f == self.active_filter)
                    .unwrap_or(0);
                let next_filter = visible[(current_pos + 1) % visible.len()];
                self.set_active_filter(next_filter, ctx);
            }
            NotificationMailboxViewAction::ActivateSelected => {
                if let Some(idx) = self.selected_index {
                    if let Some(id) = self.filtered_ids.get(idx).copied() {
                        self.activate_notification(id, ctx);
                    }
                }
            }
        }
    }
}

impl View for NotificationMailboxView {
    fn ui_name() -> &'static str {
        "NotificationMailboxView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let notifications = AgentNotificationsModel::as_ref(app).notifications();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(self.render_header(appearance, app))
            .with_child(self.render_filter_bar(notifications, app));

        if notifications.filtered_count(self.active_filter) == 0 {
            column.add_child(self.render_empty_state(appearance, app));
        } else {
            let theme = appearance.theme();

            // Render all items directly in a column so the scrollable can
            // measure their natural height instead of always filling its
            // max constraint (which was the behaviour with the viewport-based
            // List element).
            let mut items_column =
                Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
            for index in 0..self.filtered_ids.len() {
                items_column.add_child(
                    SavePosition::new(
                        self.render_item_at_index(index, app),
                        &format!("{ITEM_POSITION_PREFIX}{index}"),
                    )
                    .finish(),
                );
            }

            let item_list = NewScrollable::vertical(
                SingleAxisConfig::Clipped {
                    handle: self.scroll_state.clone(),
                    child: items_column.finish(),
                },
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                ElementFill::None,
            )
            .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, true))
            .finish();
            column.add_child(Shrinkable::new(1.0, item_list).finish());
        }

        let popup = Container::new(column.finish())
            .with_padding(Padding::default().with_top(4.))
            .with_background(appearance.theme().surface_2())
            .with_border(Border::all(1.).with_border_color(appearance.theme().outline().into()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));

        // Wrap the popup in an EventHandler that consumes clicks on empty space
        // so they don't fall through to the Dismiss layer.
        let popup = EventHandler::new(popup.finish())
            .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
            .finish();

        Dismiss::new(
            ConstrainedBox::new(popup)
                .with_width(420.)
                .with_max_height(500.)
                .finish(),
        )
        .on_dismiss(|ctx, _app| {
            ctx.dispatch_typed_action(NotificationMailboxViewAction::Dismiss);
        })
        .prevent_interaction_with_other_elements()
        .finish()
    }
}

impl NotificationMailboxView {
    fn render_header(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();

        let label = appearance
            .ui_builder()
            .wrappable_text(
                crate::i18n::tr_static(app, "Notifications").to_string(),
                false,
            )
            .with_style(UiComponentStyles {
                font_size: Some(14.),
                font_color: Some(theme.main_text_color(theme.surface_2()).into()),
                font_family_id: Some(appearance.ui_font_family()),
                ..Default::default()
            })
            .build()
            .finish();

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(label)
                .with_child(ChildView::new(&self.close_button).finish())
                .finish(),
        )
        .with_padding(
            Padding::default()
                .with_top(8.)
                .with_bottom(4.)
                .with_horizontal(12.),
        )
        .finish()
    }

    fn render_filter_bar(
        &self,
        notifications: &NotificationItems,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let has_unread = notifications.filtered_count(NotificationFilter::Unread) > 0;

        let mut filter_buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(2.);

        for (i, filter) in notifications.visible_filters().into_iter().enumerate() {
            let Some(mouse_state) = self.filter_button_mouse_states.get(i).cloned() else {
                log::warn!("missing mouse state for filter button at index {i}");
                continue;
            };

            let is_active = self.active_filter == filter;
            let count = notifications.filtered_count(filter);
            let label = if count == 0 {
                filter.label().to_string()
            } else {
                format!("{} ({count})", filter.label())
            };
            let text_color = if is_active {
                theme.main_text_color(theme.surface_2())
            } else {
                theme.sub_text_color(theme.surface_2())
            };
            let background = if is_active {
                Some(internal_colors::fg_overlay_3(theme).into())
            } else {
                None
            };

            let button = EventHandler::new(
                Hoverable::new(mouse_state, move |state| {
                    let bg = if state.is_hovered() && !is_active {
                        Some(internal_colors::fg_overlay_2(theme).into())
                    } else {
                        background
                    };

                    let mut container = Container::new(
                        appearance
                            .ui_builder()
                            .wrappable_text(label.clone(), false)
                            .with_style(UiComponentStyles {
                                font_size: Some(12.),
                                font_weight: Some(Weight::Semibold),
                                font_color: Some(text_color.into()),
                                font_family_id: Some(appearance.ui_font_family()),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .with_padding(Padding::default().with_vertical(4.).with_horizontal(8.))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

                    if let Some(bg) = bg {
                        container = container.with_background_color(bg);
                    }

                    container.finish()
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
            )
            .on_left_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(NotificationMailboxViewAction::SetFilter(filter));
                DispatchEventResult::StopPropagation
            })
            .finish();

            filter_buttons.add_child(button);
        }

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(filter_buttons.finish());

        if has_unread {
            row.add_child(ChildView::new(&self.mark_all_read_button).finish());
        }

        Container::new(row.finish())
            .with_padding(
                Padding::default()
                    .with_vertical(12.)
                    .with_left(12.)
                    .with_right(6.),
            )
            .with_border(Border::top(1.).with_border_color(theme.outline().into()))
            .with_border(Border::bottom(1.).with_border_color(theme.outline().into()))
            .finish()
    }

    fn render_empty_state(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();

        Container::new(
            appearance
                .ui_builder()
                .wrappable_text(
                    crate::i18n::tr_static(app, "No notifications").to_string(),
                    false,
                )
                .with_style(UiComponentStyles {
                    font_size: Some(14.),
                    font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
                    font_family_id: Some(appearance.ui_font_family()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_uniform_padding(ITEM_PADDING)
        .finish()
    }

    /// Asks the clipped scrollable to bring the currently selected item into
    /// view on the next paint pass.
    fn scroll_selected_into_view(&self) {
        if let Some(idx) = self.selected_index {
            self.scroll_state.scroll_to_position(ScrollTarget {
                position_id: format!("{ITEM_POSITION_PREFIX}{idx}"),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        }
    }

    fn render_notification_item(
        &self,
        item: &NotificationItem,
        mouse_state: MouseStateHandle,
        artifact_buttons: Option<&ViewHandle<ArtifactButtonsRow>>,
        is_selected: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let id = item.id;
        let has_branch = item.branch.is_some();
        let row = render_notification_item_content(
            item,
            artifact_buttons,
            NotificationRenderContext::Mailbox,
            false,
            Box::new(|_| {}),
            None,
            appearance,
        );

        EventHandler::new(
            Hoverable::new(mouse_state, move |state| {
                let item_padding = if has_branch {
                    Padding::uniform(ITEM_PADDING)
                } else {
                    Padding::default()
                        .with_vertical(ITEM_PADDING)
                        .with_horizontal(16.)
                };
                let mut container = Container::new(row).with_padding(item_padding);

                if is_selected {
                    container = container
                        .with_background_color(internal_colors::fg_overlay_3(theme).into());
                } else if state.is_hovered() {
                    container = container
                        .with_background_color(internal_colors::fg_overlay_2(theme).into());
                }

                container.finish()
            })
            .with_cursor(Cursor::PointingHand)
            .finish(),
        )
        .on_left_mouse_down(move |ctx, _, _| {
            ctx.dispatch_typed_action(NotificationMailboxViewAction::ClickItem(id));
            DispatchEventResult::StopPropagation
        })
        .finish()
    }
}
