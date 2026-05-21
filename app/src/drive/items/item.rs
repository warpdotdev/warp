use pathfinder_geometry::{rect::RectF, vector::Vector2F};
use warpui::{
    elements::{
        AcceptedByDropTarget, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius,
        CrossAxisAlignment, Draggable, DraggableState, DropShadow, Empty, Flex, Hoverable,
        MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
        ParentOffsetBounds, Radius, SavePosition, Shrinkable, SizeConstraintCondition,
        SizeConstraintSwitch, Stack,
    },
    fonts::Weight,
    platform::Cursor,
    presenter::PositionCache,
    ui_components::{
        components::{UiComponent, UiComponentStyles},
        text::Span,
    },
    AppContext, Element, SingletonEntity, ViewHandle,
};

use crate::{
    cloud_object::{
        model::{persistence::CloudModel, view::CloudViewModel},
        CloudObject, CloudObjectMetadataExt, Owner,
    },
    drive::CloudObjectTypeAndId,
    workspaces::{user_profiles::UserProfiles, user_workspaces::UserWorkspaces},
};

use crate::workspace::header_toolbar_item::HeaderToolbarItemKind;
use crate::workspace::tab_settings::TabSettings;
use crate::{
    appearance::Appearance,
    cloud_object::Space,
    drive::{
        index::{
            DriveIndexAction, AUTOSCROLL_DETECTION_DISTANCE, AUTOSCROLL_SPEED_MULTIPLIER,
            DRIVE_INDEX_VIEW_POSITION_ID, FOLDER_DEPTH_INDENT, INDEX_CONTENT_MARGIN_LEFT,
            ITEM_FONT_SIZE, ITEM_MARGIN_BOTTOM, ITEM_PADDING_HORIZONTAL, ITEM_PADDING_VERTICAL,
        },
        panel::WARP_DRIVE_POSITION_ID,
    },
    menu::Menu,
    ui_components::{
        blended_colors,
        icons::{Icon, ICON_DIMENSIONS},
        menu_button::{
            highlight_icon_button_with_context_menu_drive, icon_button_with_context_menu_drive,
            MenuDirection,
        },
    },
};
use crate::{cloud_object::CloudObjectLocation, drive::items::WarpDriveItem};

use super::WarpDriveItemId;

pub(crate) fn tools_panel_menu_direction(app: &AppContext) -> MenuDirection {
    let config = TabSettings::as_ref(app)
        .header_toolbar_chip_selection
        .clone();
    if config
        .left_items()
        .contains(&HeaderToolbarItemKind::ToolsPanel)
    {
        MenuDirection::Right
    } else {
        MenuDirection::Left
    }
}

#[derive(Default, Clone)]
pub struct ItemStates {
    pub item_mouse_state: MouseStateHandle,
    pub item_hover_state: MouseStateHandle,
    pub menu_button_state: MouseStateHandle,
    pub draggable_state: DraggableState,
    pub item_sync_icon_hover_state: MouseStateHandle,
}

struct WarpDriveItemStyles {
    // Height of each item
    item_height: f32,
    /// Default styles of the WarpDriveItem
    default: UiComponentStyles,
    /// On top of the default styles, active contains extra styling for when the item is being dragged
    dragged: UiComponentStyles,
    /// Similarly to active styles, hovered contains extra styling for a hovered item
    hovered: UiComponentStyles,
}

impl WarpDriveItemStyles {
    fn merge(self, style: UiComponentStyles) -> Self {
        Self {
            default: self.default.merge(style),
            ..self
        }
    }

    fn default(appearance: &Appearance) -> WarpDriveItemStyles {
        let theme = appearance.theme();
        let item_height = ITEM_FONT_SIZE * 2.0 - ITEM_MARGIN_BOTTOM;
        let background = theme.background();
        WarpDriveItemStyles {
            item_height,
            default: UiComponentStyles::default()
                .set_font_color(blended_colors::text_sub(theme, background))
                .set_font_family_id(appearance.ui_builder().ui_font_family())
                .set_font_size(ITEM_FONT_SIZE),
            dragged: UiComponentStyles::default()
                .set_font_family_id(appearance.ui_builder().ui_font_family())
                .set_font_size(ITEM_FONT_SIZE)
                .set_font_color(theme.foreground().into())
                .set_background(
                    warp_core::ui::theme::color::internal_colors::fg_overlay_4(theme).into(),
                )
                .set_border_color(theme.accent().into()),
            hovered: UiComponentStyles::default()
                .set_font_family_id(appearance.ui_builder().ui_font_family())
                .set_font_size(ITEM_FONT_SIZE)
                .set_font_color(blended_colors::text_main(theme, background))
                .set_background(
                    warp_core::ui::theme::color::internal_colors::fg_overlay_2(theme).into(),
                ),
        }
    }
}

/// A UI wrapper around a row in warp drive that holds important UI state for the row and implements
/// a unified look for all rows in warp drive, like padding and hover states.
///
/// The item-specific information like icon, name, click_action, and preview modal are abstracted as much as
/// possible into the WarpDriveType enum.
pub struct WarpDriveRow<'a> {
    item: Box<dyn WarpDriveItem>,
    space: Space,
    item_states: ItemStates,
    overflow_button: Box<dyn Element>,
    /// how many levels into a folder hierarchy the row is.
    /// 0 means the object is in the root directory.
    folder_depth: usize,
    sync_icon: Option<Box<dyn Element>>,
    can_move: bool,
    styles: WarpDriveItemStyles,
    menu_open: bool,
    share_dialog_open: bool,
    is_selected: bool,
    is_focused: bool,
    overflow_on_left: bool,
    appearance: &'a Appearance,
}

impl<'a> WarpDriveRow<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        item: Box<dyn WarpDriveItem>,
        item_states: ItemStates,
        space: Space,
        folder_depth: usize,
        menu: ViewHandle<Menu<DriveIndexAction>>,
        can_move: bool,
        has_menu_items: bool,
        menu_open: bool,
        share_dialog_open: bool,
        is_selected: bool,
        is_focused: bool,
        sync_queue_is_dequeueing: bool,
        menu_direction: MenuDirection,
        appearance: &'a Appearance,
    ) -> Option<Self> {
        let warp_drive_item_id = item.warp_drive_id();
        let overflow_button = match has_menu_items {
            true => {
                if is_focused || item_states.draggable_state.is_dragging() {
                    ConstrainedBox::new(
                        highlight_icon_button_with_context_menu_drive(
                            Icon::DotsVertical,
                            move |ctx, _, _| {
                                ctx.dispatch_typed_action(
                                    DriveIndexAction::ToggleItemOverflowMenu {
                                        space,
                                        warp_drive_item_id,
                                    },
                                );
                            },
                            item_states.menu_button_state.clone(),
                            &menu,
                            menu_open,
                            menu_direction,
                            appearance,
                        )
                        .finish(),
                    )
                    .with_width(20.)
                    .with_height(ICON_DIMENSIONS)
                    .finish()
                } else {
                    ConstrainedBox::new(
                        icon_button_with_context_menu_drive(
                            Icon::DotsVertical,
                            move |ctx, _, _| {
                                ctx.dispatch_typed_action(
                                    DriveIndexAction::ToggleItemOverflowMenu {
                                        space,
                                        warp_drive_item_id,
                                    },
                                );
                            },
                            item_states.menu_button_state.clone(),
                            &menu,
                            menu_open,
                            menu_direction,
                            None, /* cursor */
                            appearance,
                        )
                        .finish(),
                    )
                    .with_width(20.)
                    .with_height(ICON_DIMENSIONS)
                    .finish()
                }
            }
            false => Empty::new().finish(),
        };

        let sync_icon = item.sync_status_icon(
            sync_queue_is_dequeueing,
            item_states.item_sync_icon_hover_state.clone(),
            appearance,
        );

        Some(Self {
            item,
            space,
            item_states,
            overflow_button,
            folder_depth,
            sync_icon,
            can_move,
            styles: WarpDriveItemStyles::default(appearance),
            menu_open,
            share_dialog_open,
            is_selected,
            is_focused,
            overflow_on_left: matches!(menu_direction, MenuDirection::Left),
            appearance,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_from_cloud_object(
        object: &dyn CloudObject,
        item_states: ItemStates,
        space: Space,
        folder_depth: usize,
        menu: ViewHandle<Menu<DriveIndexAction>>,
        can_move: bool,
        has_menu_items: bool,
        menu_open: bool,
        share_dialog_open: bool,
        is_selected: bool,
        is_focused: bool,
        sync_queue_is_dequeueing: bool,
        menu_direction: MenuDirection,
        appearance: &'a Appearance,
    ) -> Option<Self> {
        let item = object.to_warp_drive_item(appearance)?;
        Self::new(
            item,
            item_states,
            space,
            folder_depth,
            menu,
            can_move,
            has_menu_items,
            menu_open,
            share_dialog_open,
            is_selected,
            is_focused,
            sync_queue_is_dequeueing,
            menu_direction,
            appearance,
        )
    }

    pub fn should_show_preview(&self) -> bool {
        self.item_states
            .item_hover_state
            .lock()
            .expect("Should be able to lock")
            .is_hovered()
            && !self
                .item_states
                .item_sync_icon_hover_state
                .lock()
                .expect("Should be able to lock")
                .is_hovered()
    }

    /// Wraps an object preview in some uniformly-styled modal. Also conditionally returns an empty
    /// view if there's limited horizontal space.
    pub fn render_preview(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        self.item.preview(appearance).map(|content_preview| {
            let mut stacked_preview_panels: Vec<Box<dyn Element>> =
                vec![Container::new(content_preview)
                    .with_uniform_padding(16.)
                    .finish()];

            stacked_preview_panels.extend(self.render_shared_object_owner(appearance, app));

            // Tracks whether the object history rectangle is the bottommost preview panel, which determines if we
            // need to render with rounded corners or not.
            let countdown = self.render_object_deletion_countdown(appearance, app);

            // If there's an object history preview, add this panel to our vector.
            if let Some(object_history) =
                self.render_object_history(appearance, countdown.is_none(), app)
            {
                // Insert above the permadeletion stat if it exists.
                stacked_preview_panels.push(object_history);
            }

            // If there's a deletion stat, add this panel to our vector.
            if let Some(countdown) = countdown {
                stacked_preview_panels.push(
                    Container::new(Empty::new().finish())
                        .with_border(
                            Border::bottom(1.).with_border_fill(appearance.theme().outline()),
                        )
                        .finish(),
                );

                stacked_preview_panels.push(countdown);
            }

            // The full hover preview is the column containing all these sub-panels.
            let full_hover_preview = Flex::column().with_children(stacked_preview_panels);

            SizeConstraintSwitch::new(
                ConstrainedBox::new(
                    Container::new(full_hover_preview.finish())
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                        .with_background(appearance.theme().surface_2())
                        .with_border(
                            Border::all(1.).with_border_fill(appearance.theme().surface_3()),
                        )
                        .with_drop_shadow(DropShadow::default())
                        .finish(),
                )
                .with_max_width(400.)
                .finish(),
                vec![(
                    SizeConstraintCondition::WidthLessThan(180.),
                    Empty::new().finish(),
                )],
            )
            .finish()
        })
    }

    /// Returns a Box<dyn Element> representing a displayable view of this cloud object's history, including
    /// for example the last metadata on edits.
    fn render_object_history(
        &self,
        appearance: &Appearance,
        with_rounded_bottom: bool,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if let Some(metadata) = self.item.metadata() {
            let editing_history = metadata.semantic_editing_history(app);

            let action_history = self.item.action_summary(app);

            let full_object_history_text = match (editing_history, action_history) {
                (Some(edits), Some(actions)) => format!("{edits}  |  {actions}"),
                (Some(edits), None) => edits,
                _ => return None,
            };

            let history_text = appearance
                .ui_builder()
                .wrappable_text(full_object_history_text, false)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_color: Some(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_2())
                            .into(),
                    ),
                    font_size: Some(12.),
                    font_weight: Some(Weight::Normal),
                    ..Default::default()
                })
                .build()
                .finish();

            // Render the element to span its parent horizontally
            let text_spanning_parent = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(Shrinkable::new(1., history_text).finish())
                .finish();

            let container = Container::new(text_spanning_parent)
                .with_background(appearance.theme().surface_1())
                .with_padding_top(8.)
                .with_padding_bottom(8.)
                .with_padding_left(16.)
                .with_padding_right(16.);

            // Conditionally add the rounded bottom
            Some(if with_rounded_bottom {
                container
                    .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
                    .finish()
            } else {
                container.finish()
            })
        } else {
            None
        }
    }

    /// Render owner information, only for shared objects.
    fn render_shared_object_owner(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let WarpDriveItemId::Object(object_id) = self.item.warp_drive_id() else {
            return None;
        };

        if CloudViewModel::as_ref(app).object_space(&object_id.uid(), app) != Some(Space::Shared) {
            return None;
        }

        let owner = CloudModel::as_ref(app)
            .get_by_uid(&object_id.uid())?
            .permissions()
            .owner;

        let mut owner_label = "From ".to_string();
        match owner {
            Owner::User { user_uid } => {
                match UserProfiles::as_ref(app).displayable_identifier_for_uid(user_uid) {
                    Some(user) => owner_label.push_str(&user),
                    None => owner_label.push_str("unknown user"),
                }
            }
            Owner::Team { team_uid, .. } => owner_label.push_str(
                UserWorkspaces::as_ref(app)
                    .team_from_uid(team_uid)
                    .map_or("unknown team", |team| &team.name),
            ),
        }

        let background = appearance.theme().surface_1();
        let text_color = appearance.theme().sub_text_color(background);

        let icon = Container::new(
            ConstrainedBox::new(Icon::Users.to_warpui_icon(text_color).finish())
                .with_height(15.)
                .with_width(15.)
                .finish(),
        )
        .with_margin_right(6.)
        .finish();

        let owner_text = appearance
            .ui_builder()
            .wrappable_text(owner_label, false)
            .with_style(UiComponentStyles {
                font_color: Some(text_color.into()),
                ..Default::default()
            })
            .build()
            .finish();

        Some(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(icon)
                    .with_child(Shrinkable::new(1., owner_text).finish())
                    .finish(),
            )
            .with_background(background)
            .with_vertical_padding(8.)
            .with_horizontal_padding(16.)
            .finish(),
        )
    }

    fn render_object_deletion_countdown(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if let Some(metadata) = self.item.metadata() {
            if let Some(countdown) = metadata.semantic_permadeletion_countdown(app) {
                let icon_and_text = Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_child(
                            Container::new(
                                ConstrainedBox::new(
                                    Icon::Clock
                                        .to_warpui_icon(
                                            appearance
                                                .theme()
                                                .sub_text_color(appearance.theme().surface_2()),
                                        )
                                        .finish(),
                                )
                                .with_height(15.)
                                .with_width(15.)
                                .finish(),
                            )
                            .with_margin_right(6.)
                            .finish(),
                        )
                        .with_child(
                            Shrinkable::new(
                                1.,
                                appearance
                                    .ui_builder()
                                    .wrappable_text(countdown, false)
                                    .with_style(UiComponentStyles {
                                        font_family_id: Some(appearance.ui_font_family()),
                                        font_color: Some(
                                            appearance
                                                .theme()
                                                .sub_text_color(appearance.theme().surface_2())
                                                .into(),
                                        ),
                                        font_size: Some(12.),
                                        font_weight: Some(Weight::Normal),
                                        ..Default::default()
                                    })
                                    .build()
                                    .finish(),
                            )
                            .finish(),
                        )
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .finish(),
                )
                .finish();

                Some(
                    Container::new(icon_and_text)
                        .with_background(appearance.theme().surface_1())
                        .with_padding_top(8.)
                        .with_padding_bottom(8.)
                        .with_padding_left(16.)
                        .with_padding_right(16.)
                        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
                        .finish(),
                )
            } else {
                None
            }
        } else {
            None
        }
    }

    fn render_chevron(&self, style: UiComponentStyles) -> Box<dyn Element> {
        // Only render chevron for folders
        if let Some(is_open) = self.item.is_folder_open() {
            let chevron_icon = if is_open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            };

            let icon_color = style.font_color.unwrap_or_else(|| {
                blended_colors::text_sub(
                    self.appearance.theme(),
                    self.appearance.theme().background(),
                )
            });

            Container::new(
                ConstrainedBox::new(chevron_icon.to_warpui_icon(icon_color.into()).finish())
                    .with_width(16.)
                    .with_height(16.)
                    .finish(),
            )
            .with_margin_right(4.)
            .finish()
        } else {
            // Not a folder, render empty spacer to maintain alignment
            Container::new(
                ConstrainedBox::new(Empty::new().finish())
                    .with_width(16.)
                    .finish(),
            )
            .with_margin_right(4.)
            .finish()
        }
    }

    fn render_icon(&self, style: UiComponentStyles) -> Box<dyn Element> {
        let icon_to_render = match self.item.warp_drive_id() {
            // This sets the icon color of folders correctly in color contrast cases, e.g. being dragged or focused
            WarpDriveItemId::Object(CloudObjectTypeAndId::Folder(_))
                if style == self.styles.dragged =>
            {
                self.item
                    .icon(self.appearance, Some(style.font_color.unwrap().into()))
            }
            _ => self.item.icon(self.appearance, None),
        };

        if let Some(icon) = icon_to_render {
            Container::new(
                ConstrainedBox::new(icon)
                    .with_width(16.)
                    .with_height(16.)
                    .finish(),
            )
            .with_margin_right(8.)
            .finish()
        } else {
            Empty::new().finish()
        }
    }

    fn render_secondary_icon(&self, style: UiComponentStyles) -> Box<dyn Element> {
        let icon_to_render = match self.item.warp_drive_id() {
            WarpDriveItemId::Object(CloudObjectTypeAndId::Folder(_)) => self
                .item
                .secondary_icon(Some(style.font_color.unwrap().into())),
            _ => self.item.secondary_icon(None),
        };

        if let Some(icon) = icon_to_render {
            Container::new(
                ConstrainedBox::new(icon)
                    .with_width(self.styles.default.font_size.unwrap_or_default())
                    .with_height(self.styles.default.font_size.unwrap_or_default())
                    .finish(),
            )
            .with_padding_left(2.)
            .finish()
        } else {
            Empty::new().finish()
        }
    }

    fn render_item_name(&self, style: UiComponentStyles) -> Box<dyn Element> {
        Span::new(
            self.item
                .display_name()
                .unwrap_or_else(|| "Untitled".to_string()),
            style,
        )
        .build()
        .finish()
    }

    pub fn render_item(&self, style: UiComponentStyles) -> Box<dyn Element> {
        let chevron = self.render_chevron(style);
        let icon = self.render_icon(style);
        let name = self.render_item_name(style);
        let secondary_icon = self.render_secondary_icon(style);

        let item = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., name).finish())
            .with_child(secondary_icon)
            .finish();

        let action = self.item.click_action();
        let space = self.space;
        let warp_drive_item_id = self.item.warp_drive_id();
        match warp_drive_item_id {
            WarpDriveItemId::Object(_)
            | WarpDriveItemId::AIFactCollection
            | WarpDriveItemId::MCPServerCollection => {
                Hoverable::new(self.item_states.item_mouse_state.clone(), move |_| {
                    Container::new(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(chevron)
                            .with_child(icon)
                            .with_child(Shrinkable::new(1., item).finish())
                            .finish(),
                    )
                    .with_margin_left(FOLDER_DEPTH_INDENT * self.folder_depth as f32)
                    .finish()
                })
                .on_click(move |ctx, _, _| {
                    if let Some(action) = action.clone() {
                        ctx.dispatch_typed_action(action);
                    }
                })
                .on_right_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(DriveIndexAction::ToggleItemOverflowMenu {
                        space,
                        warp_drive_item_id,
                    });
                })
                .finish()
            }
            _ => unreachable!(),
        }
    }
}

/// Generate a callback for calculating the Drag bounds within Warp Drive
fn drag_bounds_callback() -> impl Fn(&PositionCache, Vector2F) -> Option<RectF> {
    move |position_cache, window: Vector2F| {
        let drive_index = position_cache.get_position(WARP_DRIVE_POSITION_ID)?;

        let top_left = drive_index.origin();

        Some(RectF::from_points(top_left, window))
    }
}

impl UiComponent for WarpDriveRow<'_> {
    type ElementType = SavePosition;

    fn build(self) -> Self::ElementType {
        let is_dragging = self.item_states.draggable_state.is_dragging();
        let overflow_on_left = self.overflow_on_left;

        // This is ONLY for rendering the font color correctly, which is set at the render_item level
        let style = if is_dragging || self.is_focused {
            self.styles.dragged
        } else {
            self.styles.default
        };
        let inner_item = self.render_item(style);

        // Hoverable here doesn't have any action, it's mostly used for setting background styling based on
        // the mouse state
        let hoverable_item = Hoverable::new(
            self.item_states.item_hover_state.clone(),
            move |mouse_state| {
                // If dragging or object has been focused, then theme accent background.
                // If hovering / menu is open / object has been selected, then thick overlay background.
                // If an object is both selected and focused, show focused background
                let container_background_fill = if is_dragging || self.is_focused {
                    self.styles.dragged.background
                } else if mouse_state.is_hovered()
                    || self.menu_open
                    || self.is_selected
                    || self.share_dialog_open
                {
                    self.styles.hovered.background
                } else {
                    None
                };

                let show_overflow = mouse_state.is_hovered() || self.menu_open;

                let mut items_row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center);

                items_row.add_child(Shrinkable::new(1., inner_item).finish());
                if let Some(sync_icon) = self.sync_icon {
                    items_row.add_child(Container::new(sync_icon).with_margin_right(4.).finish());
                }

                let row_element: Box<dyn Element> = if overflow_on_left {
                    let row_container = Container::new(items_row.finish())
                        .with_margin_left(INDEX_CONTENT_MARGIN_LEFT)
                        .with_padding_right(ITEM_PADDING_HORIZONTAL)
                        .with_padding_left(ITEM_PADDING_HORIZONTAL)
                        .with_padding_top(ITEM_PADDING_VERTICAL)
                        .with_padding_bottom(ITEM_PADDING_VERTICAL)
                        .finish();
                    if show_overflow {
                        let mut stack = Stack::new().with_child(row_container);
                        stack.add_positioned_child(
                            self.overflow_button,
                            OffsetPositioning::offset_from_parent(
                                pathfinder_geometry::vector::vec2f(0., 0.),
                                ParentOffsetBounds::ParentByPosition,
                                ParentAnchor::MiddleLeft,
                                ChildAnchor::MiddleLeft,
                            ),
                        );
                        stack.finish()
                    } else {
                        row_container
                    }
                } else {
                    if show_overflow {
                        items_row.add_child(self.overflow_button);
                    }
                    Container::new(items_row.finish())
                        .with_margin_left(INDEX_CONTENT_MARGIN_LEFT)
                        .with_padding_right(ITEM_PADDING_HORIZONTAL)
                        .with_padding_left(ITEM_PADDING_HORIZONTAL)
                        .with_padding_top(ITEM_PADDING_VERTICAL)
                        .with_padding_bottom(ITEM_PADDING_VERTICAL)
                        .finish()
                };

                let result_container = Container::new(
                    ConstrainedBox::new(row_element)
                        .with_height(self.styles.item_height)
                        .finish(),
                )
                .with_margin_bottom(ITEM_MARGIN_BOTTOM)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

                match container_background_fill {
                    Some(background_fill) => {
                        result_container.with_background(background_fill).finish()
                    }
                    None => result_container.finish(),
                }
            },
        )
        .with_cursor(Cursor::PointingHand)
        .finish();

        match self.item.warp_drive_id() {
            WarpDriveItemId::Object(item) => {
                let save_position_child = match self.can_move {
                    true => {
                        Draggable::new(self.item_states.draggable_state, hoverable_item)
                            .with_drag_bounds_callback(drag_bounds_callback())
                            .with_accepted_by_drop_target_fn(move |drop_data, app| {
                                let Some(location) =
                                    drop_data.as_any().downcast_ref::<CloudObjectLocation>()
                                else {
                                    return AcceptedByDropTarget::No;
                                };
                                let cloud_model = CloudModel::handle(app);
                                if cloud_model.as_ref(app).can_move_object_to_location(
                                    &item.uid(),
                                    *location,
                                    app,
                                ) {
                                    AcceptedByDropTarget::Yes
                                } else {
                                    AcceptedByDropTarget::No
                                }
                            })
                            .on_drop(move |ctx, _, _, data| {
                                if let Some(location) = data.and_then(|data| {
                                    data.as_any().downcast_ref::<CloudObjectLocation>()
                                }) {
                                    ctx.dispatch_typed_action(DriveIndexAction::DropIndexItem {
                                        cloud_object_type_and_id: item,
                                        drop_target_location: *location,
                                    });
                                }
                            })
                            .on_drag(move |ctx, _, dragged_item, data| {
                                // First, check if we are over a drop target for styling
                                if let Some(location) = data.and_then(|data| {
                                    data.as_any().downcast_ref::<CloudObjectLocation>()
                                }) {
                                    ctx.dispatch_typed_action(
                                        DriveIndexAction::UpdateCurrentDropTarget {
                                            drop_target_location: *location,
                                        },
                                    )
                                } else {
                                    ctx.dispatch_typed_action(DriveIndexAction::ClearDropTarget)
                                }

                                // On a drag event, check to see if the index needs to be scrolled up or down
                                // to reveal new content.
                                if let Some(drive_index_view_position) =
                                    ctx.element_position_by_id(DRIVE_INDEX_VIEW_POSITION_ID)
                                {
                                    // First, check to see if we should scroll upwards (revealing more content at the top).
                                    // This computes the distance between the top of the *currently-dragging* item and the top
                                    // of the drive index view. If distance < 10, we emit a scroll event back to the index view.
                                    let pixels_from_top =
                                        dragged_item.min_y() - drive_index_view_position.min_y();
                                    if pixels_from_top < AUTOSCROLL_DETECTION_DISTANCE {
                                        // The speed of the autoscroll is a function of (1) how far away from the relevant border the object is
                                        // and (2) what the speed multiplier is.
                                        // Note: Scrolling upwards is decreasing the scroll value, so we multiply by -1 in this case.
                                        let scroll_speed = ((AUTOSCROLL_DETECTION_DISTANCE
                                            - pixels_from_top)
                                            / AUTOSCROLL_DETECTION_DISTANCE)
                                            * AUTOSCROLL_SPEED_MULTIPLIER;
                                        ctx.dispatch_typed_action(DriveIndexAction::Autoscroll {
                                            delta: -scroll_speed,
                                        });
                                        return;
                                    }

                                    // Otherwise, check to see if we should scroll downwards (revealing more content at the
                                    // bottom).
                                    // This computes the distance between the bottom of the *currently-dragging* item
                                    // and the bottom of the drive index view. If distance < 10, emit a scroll event.
                                    let pixels_from_bottom =
                                        drive_index_view_position.max_y() - dragged_item.max_y();
                                    if pixels_from_bottom < AUTOSCROLL_DETECTION_DISTANCE {
                                        // See comment above about determining the speed of the scroll.
                                        let scroll_speed = ((AUTOSCROLL_DETECTION_DISTANCE
                                            - pixels_from_bottom)
                                            / AUTOSCROLL_DETECTION_DISTANCE)
                                            * AUTOSCROLL_SPEED_MULTIPLIER;
                                        ctx.dispatch_typed_action(DriveIndexAction::Autoscroll {
                                            delta: scroll_speed,
                                        })
                                    }
                                }
                            })
                            .finish()
                    }
                    false => hoverable_item,
                };
                SavePosition::new(
                    save_position_child,
                    &self.item.warp_drive_id().drive_row_position_id(),
                )
            }
            WarpDriveItemId::AIFactCollection | WarpDriveItemId::MCPServerCollection => {
                SavePosition::new(
                    hoverable_item,
                    &self.item.warp_drive_id().drive_row_position_id(),
                )
            }
            _ => unreachable!(),
        }
    }

    fn with_style(self, style: UiComponentStyles) -> Self {
        Self {
            styles: self.styles.merge(style),
            ..self
        }
    }
}
