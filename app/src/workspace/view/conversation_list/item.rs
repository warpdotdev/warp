use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent_conversations_model::{
    AgentConversationEntry, AgentConversationEntryId, AgentConversationProvenance,
};
use crate::ai::conversation_status_ui::STATUS_ELEMENT_PADDING;
use crate::appearance::Appearance;
use crate::drive::sharing::dialog::SharingDialog;
use crate::menu::Menu;
use crate::ui_components::agent_icon::agent_conversation_entry_icon_variant;
use crate::ui_components::icon_with_status::render_icon_with_status;
use crate::ui_components::icons::Icon;
use crate::ui_components::menu_button::{icon_button_with_context_menu, MenuDirection};
use crate::util::time_format::format_approx_duration_from_now_utc;
use crate::util::truncation::truncate_from_end;
use crate::workspace::view::conversation_list::view::ConversationListViewAction;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::theme::color::internal_colors;
use warp_util::path::user_friendly_path;
use warpui::elements::{
    AnchorPair, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, DispatchEventResult, Element, EventHandler, Flex, Highlight, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseInBehavior, MouseStateHandle, OffsetPositioning,
    OffsetType, ParentAnchor, ParentElement, ParentOffsetBounds, PositionedElementOffsetBounds,
    PositioningAxis, Radius, SavePosition, Shrinkable, Stack, Text, XAxisAnchor, YAxisAnchor,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::text_layout::TextStyle;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, SingletonEntity, ViewHandle};

/// Maximum length for tooltip text before truncation
const MAX_TOOLTIP_LENGTH: usize = 80;

/// Spacing between icon and title
const ICON_SPACING: f32 = 4.;

/// Offset for the sharing dialog from the item row
const DIALOG_OFFSET_PIXELS: f32 = -16.;

/// Total size of the agent icon-with-status component rendered in each conversation list
/// row.
const LIST_ITEM_AGENT_SIZE: f32 = 22.;
/// Extra overhang past the default overlay position, as a fraction of
/// `LIST_ITEM_AGENT_SIZE`. Pushes the badge all the way to the bounding box's BR corner;
/// the conversation list reads better with the status sitting slightly further out than
/// on the other surfaces.
const LIST_ITEM_OVERLAY_EXTRA_OVERHANG: f32 = 0.05;

/// Generate a position ID for a conversation list item
fn conversation_item_position_id(id: &AgentConversationEntryId) -> String {
    match id {
        AgentConversationEntryId::Conversation(conv_id) => {
            format!("conversation_list_item_{conv_id}")
        }
        AgentConversationEntryId::AmbientRun(task_id) => {
            format!("conversation_list_task_{task_id}")
        }
    }
}

/// Minimum height for static list items (section headers, StartNewConversation).
/// Ensures UniformList uses consistent item heights (and doesn't clip any items).
pub const STATIC_ITEM_MIN_HEIGHT: f32 = 42.;

#[derive(Clone, Default)]
pub struct ItemState {
    pub mouse_state: MouseStateHandle,
    pub overflow_button_state: MouseStateHandle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OverflowMenuDisplay {
    Closed,
    /// Menu was opened from the kebab button.
    OpenAtKebab,
    /// Menu was opened from a right click (at the click position).
    OpenAtRightClickPosition,
}

pub struct ItemProps<'a> {
    pub conversation: &'a AgentConversationEntry,
    pub highlight_indices: Option<&'a Vec<usize>>,
    pub is_selected: bool,
    pub is_focused_conversation: bool,
    pub index: usize,
    pub state: &'a ItemState,
    pub overflow_menu: &'a ViewHandle<Menu<ConversationListViewAction>>,
    pub overflow_menu_display: OverflowMenuDisplay,
    pub conversation_id: AgentConversationEntryId,
    pub sharing_dialog: &'a ViewHandle<SharingDialog>,
    pub is_share_dialog_open: bool,
    pub list_position_id: &'a str,
    pub tooltip_opens_right: bool,
}

pub struct StaticItemProps<'a> {
    pub is_selected: bool,
    pub index: usize,
    pub state: &'a ItemState,
}

pub fn render_static_item(props: StaticItemProps<'_>, app: &AppContext) -> Box<dyn Element> {
    let StaticItemProps {
        is_selected,
        index,
        state,
    } = props;
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let icon_color = theme.main_text_color(theme.background());
    let icon = Container::new(
        ConstrainedBox::new(Icon::Plus.to_warpui_icon(icon_color).finish())
            .with_width(appearance.ui_font_size())
            .with_height(appearance.ui_font_size())
            .finish(),
    )
    .with_uniform_padding(STATUS_ELEMENT_PADDING)
    .with_background(coloru_with_opacity(icon_color.into(), 10))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
    .finish();

    let title_text = Text::new_inline(
        "New conversation",
        appearance.ui_font_family(),
        appearance.ui_font_size() + 2.,
    )
    .with_color(theme.main_text_color(theme.background()).into())
    .finish();

    let row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(ICON_SPACING)
        .with_child(icon)
        .with_child(title_text)
        .finish();

    let hoverable = Hoverable::new(state.mouse_state.clone(), move |_| {
        let mut container = Container::new(row).with_horizontal_padding(12.);
        if is_selected {
            container = container.with_background(theme.surface_overlay_1());
        }
        container.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(ConversationListViewAction::NewConversationInNewTab);
    });

    EventHandler::new(
        ConstrainedBox::new(hoverable.finish())
            .with_min_height(STATIC_ITEM_MIN_HEIGHT)
            .finish(),
    )
    .on_mouse_in(
        move |ctx, _, _| {
            ctx.dispatch_typed_action(ConversationListViewAction::SetSelectedIndex(index));
            DispatchEventResult::PropagateToParent
        },
        Some(MouseInBehavior {
            fire_on_synthetic_events: false,
            fire_when_covered: true,
        }),
    )
    .finish()
}

pub fn render_item(props: ItemProps<'_>, app: &AppContext) -> Box<dyn Element> {
    let ItemProps {
        conversation,
        highlight_indices,
        is_selected,
        is_focused_conversation,
        index,
        state,
        overflow_menu,
        overflow_menu_display,
        conversation_id,
        sharing_dialog,
        is_share_dialog_open,
        list_position_id,
        tooltip_opens_right,
    } = props;
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let ui_builder = appearance.ui_builder().clone();
    let font_family = appearance.ui_font_family();
    let font_size = appearance.ui_font_size();

    let title_font_size = font_size + 2.;
    let mut title_text = Text::new_inline(
        conversation.display.title.clone(),
        font_family,
        title_font_size,
    )
    .with_color(theme.main_text_color(theme.background()).into());

    if let Some(indices) = highlight_indices {
        if !indices.is_empty() {
            let highlight = Highlight::new()
                .with_properties(Properties::default().weight(Weight::Bold))
                .with_text_style(
                    TextStyle::new()
                        .with_foreground_color(theme.main_text_color(theme.background()).into())
                        .with_background_color(
                            internal_colors::accent_overlay_3(theme).into_solid(),
                        ),
                );
            title_text = title_text.with_single_highlight(highlight, indices.clone());
        }
    }

    let status_element_size = font_size + STATUS_ELEMENT_PADDING * 2.;
    let icon_element = render_icon_with_status(
        agent_conversation_entry_icon_variant(conversation),
        LIST_ITEM_AGENT_SIZE,
        LIST_ITEM_OVERLAY_EXTRA_OVERHANG,
        theme,
        theme.background(),
    );

    let icon_and_title_row = Shrinkable::new(
        1.0,
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(ICON_SPACING)
            .with_child(icon_element)
            .with_child(Shrinkable::new(1.0, title_text.finish()).finish())
            .finish(),
    )
    .finish();

    let timestamp = Text::new_inline(
        format_approx_duration_from_now_utc(conversation.display.last_updated),
        font_family,
        font_size - 2.,
    )
    .with_color(theme.sub_text_color(theme.background()).into())
    .finish();

    let bottom_row = if let Some(subtext) = format_item_subtext(conversation, app) {
        let subtext_element = Shrinkable::new(
            1.0,
            Text::new_inline(subtext, font_family, title_font_size - 2.)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
        )
        .finish();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::End)
                .with_child(subtext_element)
                .with_child(timestamp)
                .finish(),
        )
        .with_padding_left(status_element_size + ICON_SPACING)
        .finish()
    } else {
        // If no subtext, still show timestamp in the bottom row
        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_child(timestamp)
                .finish(),
        )
        .with_padding_left(status_element_size + ICON_SPACING)
        .finish()
    };

    let row = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(icon_and_title_row)
        .with_child(bottom_row)
        .finish();

    let can_open = conversation.capabilities.can_open;
    let tooltip_text = truncate_from_end(&conversation.display.title, MAX_TOOLTIP_LENGTH);
    let overflow_button_state = state.overflow_button_state.clone();
    let hoverable = Hoverable::new(state.mouse_state.clone(), move |_| {
        let container = Container::new(row)
            .with_horizontal_padding(12.)
            .with_padding_top(8.);

        let container = if is_focused_conversation {
            container.with_background(theme.surface_overlay_2())
        } else if is_selected || !matches!(overflow_menu_display, OverflowMenuDisplay::Closed) {
            container.with_background(theme.surface_overlay_1())
        } else {
            container
        };

        let mut stack = Stack::new().with_child(container.finish());

        // We show the overflow menu button when the item is selected, or the overflow menu is already open.
        if is_selected || !matches!(overflow_menu_display, OverflowMenuDisplay::Closed) {
            let button_style = UiComponentStyles::default()
                .set_background(theme.surface_2().into())
                .set_border_color(theme.surface_3().into());
            let menu_direction = if tooltip_opens_right {
                MenuDirection::Right
            } else {
                MenuDirection::Left
            };
            let overflow_button = icon_button_with_context_menu(
                Icon::DotsVertical,
                move |ctx, _, _| {
                    ctx.dispatch_typed_action(ConversationListViewAction::ToggleOverflowMenu {
                        conversation_id,
                        position: None,
                    });
                },
                overflow_button_state.clone(),
                overflow_menu,
                matches!(overflow_menu_display, OverflowMenuDisplay::OpenAtKebab),
                menu_direction,
                Some(Cursor::PointingHand),
                Some(button_style),
                appearance,
            );
            // The kebab button is pinned to the right edge of the item regardless of which
            // side of the screen the conversation list panel is on; only the menu's open
            // direction (handled above) flips with `tooltip_opens_right`.
            let overflow_offset = OffsetPositioning::offset_from_parent(
                vec2f(-8., 6.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            );
            // Use add_positioned_child (not overlay) so button stays within item bounds
            stack.add_positioned_child(overflow_button.finish(), overflow_offset);
        }

        // Hide the tooltip when the overflow menu is being shown so that they don't overlap.
        if is_selected && matches!(overflow_menu_display, OverflowMenuDisplay::Closed) {
            let tooltip = ui_builder.tool_tip(tooltip_text).build().finish();
            let (parent_anchor, child_anchor, offset_x) = if tooltip_opens_right {
                (ParentAnchor::MiddleRight, ChildAnchor::MiddleLeft, 4.)
            } else {
                (ParentAnchor::MiddleLeft, ChildAnchor::MiddleRight, -4.)
            };
            let tooltip_offset = OffsetPositioning::offset_from_parent(
                vec2f(offset_x, 0.),
                ParentOffsetBounds::WindowByPosition,
                parent_anchor,
                child_anchor,
            );
            stack.add_positioned_overlay_child(tooltip, tooltip_offset);
        }
        stack.finish()
    })
    .on_right_click({
        let list_position_id = list_position_id.to_string();
        move |ctx, _, position| {
            let Some(parent_bounds) = ctx.element_position_by_id(&list_position_id) else {
                log::warn!("Could not retrieve the position of the conversation list for overflow menu display.");
                return;
            };

            let offset = position - parent_bounds.origin();
            ctx.dispatch_typed_action(ConversationListViewAction::ToggleOverflowMenu {
                conversation_id,
                position: Some(offset),
            });
        }
    })
    .with_defer_events_to_children();

    let hoverable_element = if can_open {
        hoverable
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ConversationListViewAction::OpenItem {
                    id: conversation_id,
                });
            })
            .finish()
    } else {
        hoverable.finish()
    };

    let event_handler = EventHandler::new(hoverable_element)
        .on_mouse_in(
            move |ctx, _, _| {
                ctx.dispatch_typed_action(ConversationListViewAction::SetSelectedIndex(index));
                DispatchEventResult::PropagateToParent
            },
            Some(MouseInBehavior {
                fire_on_synthetic_events: false,
                fire_when_covered: true,
            }),
        )
        .finish();

    // Wrap in a stack to support the sharing dialog overlay
    let position_id = conversation_item_position_id(&conversation_id);
    let mut item_stack = Stack::new().with_child(event_handler);

    // Add the sharing dialog as a positioned overlay when open for this item
    if is_share_dialog_open {
        // Position the dialog to the right of the item row
        item_stack.add_positioned_overlay_child(
            ChildView::new(sharing_dialog).finish(),
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    &position_id,
                    PositionedElementOffsetBounds::WindowBySize,
                    OffsetType::Pixel(DIALOG_OFFSET_PIXELS),
                    AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
                ),
                PositioningAxis::relative_to_stack_child(
                    &position_id,
                    PositionedElementOffsetBounds::WindowByPosition,
                    OffsetType::Pixel(DIALOG_OFFSET_PIXELS),
                    AnchorPair::new(YAxisAnchor::Middle, YAxisAnchor::Middle),
                ),
            ),
        );
    }

    SavePosition::new(item_stack.finish(), &position_id).finish()
}

/// Returns the secondary label for a conversation list item:
/// - For local conversations: the working directory.
/// - For tasks: the source (Linear, Slack, CLI, etc.)
fn format_item_subtext(conversation: &AgentConversationEntry, app: &AppContext) -> Option<String> {
    if matches!(
        conversation.provenance,
        AgentConversationProvenance::AmbientRun
    ) {
        return conversation
            .display
            .source
            .as_ref()
            .map(|source| source.display_name().to_string());
    }

    let live_pwd = conversation
        .identity
        .local_conversation_id
        .and_then(|conversation_id| {
            ActiveAgentViewsModel::as_ref(app)
                .get_active_session_for_conversation(conversation_id, app)
                .and_then(|session| session.as_ref(app).current_working_directory().cloned())
        });

    let pwd = live_pwd.or_else(|| conversation.display.working_directory.clone());
    pwd.map(|pwd| {
        let home_dir = dirs::home_dir().and_then(|p| p.to_str().map(String::from));
        user_friendly_path(&pwd, home_dir.as_deref()).into_owned()
    })
}
