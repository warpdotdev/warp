//! Renders the AI block "header", which includes a version of the AI "prompt" as it was rendered
//! when the query was submitted.
use warp_core::features::FeatureFlag;
use warp_util::path::user_friendly_path;
use warpui::elements::{
    ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Hoverable,
    Icon as ElementIcon, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius,
    SavePosition, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, EntityId, SingletonEntity, ViewHandle};

use crate::ai::agent::AIAgentExchangeId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::block::DirectoryContext;
use crate::ai::blocklist::{
    get_ai_block_overflow_menu_element_position_id, get_attached_blocks_chip_element_position_id,
};
use crate::appearance::Appearance;
use crate::terminal::block_list_element::render_hoverable_block_button;
use crate::terminal::view::{TerminalAction, WARP_PROMPT_HEIGHT_LINES};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::ActionButton;

/// Data required to render the AI block header.
pub(super) struct Props<'a> {
    pub(super) view_id: &'a EntityId,
    pub(super) exchange_id: &'a AIAgentExchangeId,
    pub(super) conversation_id: &'a AIConversationId,
    pub(super) attached_blocks_chip_mouse_state: &'a MouseStateHandle,
    pub(super) overflow_menu_mouse_state: &'a MouseStateHandle,
    pub(super) rewind_button: &'a ViewHandle<ActionButton>,
    pub(super) num_attached_context_blocks: usize,
    pub(super) has_attached_context_selected_text: bool,
    pub(super) directory_context: &'a DirectoryContext,
    pub(super) is_selected_text_attached_as_context: bool,
    pub(super) is_restored: bool,
}

/// Render the AI Block's header which is the "AI prompt" that displays context about the AI query.
pub(super) fn render(props: Props, app: &AppContext) -> Option<Box<dyn Element>> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let mut did_render_child = false;
    let mut left_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    let font_size = prompt_font_size(appearance);
    if !FeatureFlag::AgentView.is_enabled()
        && let Some(pwd) = &props.directory_context.pwd
    {
        let current_directory =
            user_friendly_path(pwd.as_str(), props.directory_context.home_dir.as_deref())
                .to_string();
        left_row.add_child(
            Container::new(
                Text::new_inline(
                    current_directory,
                    appearance.monospace_font_family(),
                    font_size,
                )
                .with_color(blended_colors::text_sub(theme, theme.surface_1()))
                .with_selection_color(if props.is_selected_text_attached_as_context {
                    theme.text_selection_as_context_color().into_solid()
                } else {
                    theme.text_selection_color().into_solid()
                })
                .finish(),
            )
            .with_margin_right(8.)
            .finish(),
        );
        did_render_child |= true;
    }

    // When AgentViewBlockContext is enabled, blocks are auto-attached so we don't
    // show the attached context chip for blocks.
    let show_attached_blocks_chip =
        props.num_attached_context_blocks > 0 && !FeatureFlag::AgentViewBlockContext.is_enabled();

    if show_attached_blocks_chip || props.has_attached_context_selected_text {
        let chip_display_text = match (
            props.has_attached_context_selected_text,
            props.num_attached_context_blocks,
        ) {
            (true, _) => "selected text".to_owned(),
            (false, 1) => "1 block".to_owned(),
            (false, n) => format!("{n} blocks"),
        };

        left_row.add_child(render_attached_context_chip(
            props.attached_blocks_chip_mouse_state.clone(),
            chip_display_text,
            *props.view_id,
            *props.exchange_id,
            *props.conversation_id,
            app,
        ));
        did_render_child |= true;
    }

    if FeatureFlag::AgentView.is_enabled() && !did_render_child {
        return None;
    }

    let mut right_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    if FeatureFlag::RevertToCheckpoints.is_enabled() && !props.is_restored {
        right_row.add_child(
            Container::new(ChildView::new(props.rewind_button).finish())
                .with_margin_right(4.)
                .finish(),
        );
    }

    right_row.add_child(render_overflow_menu_button(
        props.overflow_menu_mouse_state.clone(),
        *props.view_id,
        *props.exchange_id,
        *props.conversation_id,
        props.is_restored,
        app,
    ));

    Some(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(left_row.finish())
            .with_child(right_row.finish())
            .finish(),
    )
}

/// Render the chip that shows what context (i.e. block(s), text, or none) was attached to this
/// AI query and can be clicked to show the list of attached blocks and/or selected text.
fn render_attached_context_chip(
    attached_context_chip_mouse_state: MouseStateHandle,
    display_text: String,
    ai_block_view_id: EntityId,
    exchange_id: AIAgentExchangeId,
    conversation_id: AIConversationId,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let font_size = prompt_font_size(appearance);
    let block_count_color = blended_colors::text_sub(theme, theme.background());

    SavePosition::new(
        Hoverable::new(attached_context_chip_mouse_state, |_state| {
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        Container::new(
                            ConstrainedBox::new(
                                Icon::Paperclip
                                    .to_warpui_icon(block_count_color.into())
                                    .finish(),
                            )
                            .with_height(font_size)
                            .with_width(font_size)
                            .finish(),
                        )
                        .with_margin_right(4.)
                        .finish(),
                    )
                    .with_child(
                        Text::new_inline(
                            display_text,
                            appearance.monospace_font_family(),
                            font_size,
                        )
                        .with_color(block_count_color)
                        .finish(),
                    )
                    .with_main_axis_size(MainAxisSize::Min)
                    .finish(),
            )
            .with_background(theme.surface_3())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_vertical_overdraw(2.)
            .with_horizontal_padding(8.)
            .with_vertical_padding(4.)
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(TerminalAction::OpenAIBlockAttachedBlocksMenu {
                exchange_id,
                conversation_id,
                ai_block_view_id,
            })
        })
        .finish(),
        &get_attached_blocks_chip_element_position_id(ai_block_view_id),
    )
    .finish()
}

pub(super) const OVERFLOW_BUTTON_SIZE: f32 = 26.;

/// Render the overflow menu button (three dots icon) for the AI block
pub(super) fn render_overflow_menu_button(
    overflow_menu_mouse_state: MouseStateHandle,
    ai_block_view_id: EntityId,
    exchange_id: AIAgentExchangeId,
    conversation_id: AIConversationId,
    is_restored: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let icon_color = theme.sub_text_color(theme.surface_2()).into_solid();

    let icon = Container::new(
        ConstrainedBox::new(ElementIcon::new("bundled/svg/overflow.svg", icon_color).finish())
            .with_height(OVERFLOW_BUTTON_SIZE)
            .with_width(OVERFLOW_BUTTON_SIZE)
            .finish(),
    );

    SavePosition::new(
        ConstrainedBox::new(render_hoverable_block_button(
            icon,
            None,  // no tooltip
            false, // don't ignore mouse events
            true,  // allow action
            overflow_menu_mouse_state,
            theme,
            appearance.ui_builder(),
            move |ctx, _, _| {
                ctx.dispatch_typed_action(TerminalAction::OpenAIBlockOverflowMenu {
                    ai_block_view_id,
                    exchange_id,
                    conversation_id,
                    is_restored,
                });
            },
        ))
        .with_width(OVERFLOW_BUTTON_SIZE)
        .with_height(OVERFLOW_BUTTON_SIZE)
        .finish(),
        &get_ai_block_overflow_menu_element_position_id(ai_block_view_id),
    )
    .finish()
}

/// Returns the font size to be used to render text in the AI block "prompt" line.
///
/// This matches the font size used for the warp prompt in completed command blocks.
fn prompt_font_size(appearance: &Appearance) -> f32 {
    appearance.monospace_font_size() * WARP_PROMPT_HEIGHT_LINES
}
