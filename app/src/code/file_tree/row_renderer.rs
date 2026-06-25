//! Shared rendering primitives for file-tree style list rows.
//!
//! This module provides the constants and [`render_tree_row`] function used by
//! both the left-panel [`FileTreeView`] and the code review file-tree sidebar so
//! that their visual styling stays in sync without duplicating code.

use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Empty, Flex, MainAxisSize, ParentElement,
    Shrinkable, Text,
};
use warpui::fonts::{Properties, Style, Weight};
use warpui::Element;

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;
use crate::ui_components::item_highlight::{ImageOrIcon, ItemHighlightState};

/// Font size for all file-tree row labels.
pub(crate) const ITEM_FONT_SIZE: f32 = 14.;
/// Indentation added per directory level; also used as the chevron/icon size.
pub(crate) const FOLDER_INDENT: f32 = 16.;
/// Vertical padding (top and bottom) inside each row's container.
pub(crate) const ITEM_PADDING: f32 = 4.;

/// Visual configuration for a single file-tree row.
pub(crate) struct TreeRowConfig {
    /// Nesting depth (0 = root-level items, 1 = one level in, …).
    pub depth: usize,
    /// Display name shown as the row label (filename or directory name).
    pub name: String,
    /// File-type icon (or [`Icon::File`] / [`Icon::Folder`] fallback).
    pub icon: ImageOrIcon,
    /// `None` for file nodes (no chevron); `Some(expanded)` for directory nodes.
    pub is_expanded: Option<bool>,
    /// Whether the item is git-ignored (renders name in italic + light weight).
    pub is_ignored: bool,
    /// Hover / selected state used to colour the icon, text, and background.
    pub item_highlight_state: ItemHighlightState,
}

/// Renders the inner content of a file-tree row.
///
/// Returns a [`Flex::row`] containing:
/// 1. An indentation spacer (`depth × FOLDER_INDENT` px wide).
/// 2. A chevron icon (`ChevronDown` / `ChevronRight`) for directory nodes, or an
///    empty spacer of the same width for file nodes.
/// 3. The item icon (16 × 16 px).
/// 4. A [`Shrinkable`] text label.
///
/// **The returned element has no outer [`Container`]** — callers are responsible
/// for adding padding, background, and corner-radius via their own `Container`
/// (typically inside a `Hoverable` closure so the background reacts to hover/
/// selection state). Callers may also append trailing elements (e.g. +/- change
/// counts) to the returned flex row before wrapping.
pub(crate) fn render_tree_row(config: TreeRowConfig, appearance: &Appearance) -> Box<dyn Element> {
    let mut row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Indentation spacer.
    if config.depth > 0 {
        row.add_child(
            Container::new(
                ConstrainedBox::new(Empty::new().finish())
                    .with_width(config.depth as f32 * FOLDER_INDENT)
                    .finish(),
            )
            .finish(),
        );
    }

    // Chevron for directories; empty placeholder of the same size for files so
    // the icon column stays aligned regardless of item type.
    let chevron_color = config.item_highlight_state.text_and_icon_color(appearance);
    let chevron_element = match config.is_expanded {
        Some(expanded) => {
            let icon = if expanded {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            };
            icon.to_warpui_icon(chevron_color.into()).finish()
        }
        None => Empty::new().finish(),
    };
    row.add_child(
        Container::new(
            ConstrainedBox::new(chevron_element)
                .with_width(FOLDER_INDENT)
                .with_height(FOLDER_INDENT)
                .finish(),
        )
        .with_margin_right(4.)
        .finish(),
    );

    // File / folder icon.
    let icon_color = config.item_highlight_state.text_and_icon_color(appearance);
    let icon_element = match config.icon {
        ImageOrIcon::Icon(icon) => icon.to_warpui_icon(icon_color.into()).finish(),
        ImageOrIcon::Image(image) => image,
    };
    row.add_child(
        Container::new(
            ConstrainedBox::new(icon_element)
                .with_width(FOLDER_INDENT)
                .with_height(FOLDER_INDENT)
                .finish(),
        )
        .with_margin_right(8.)
        .finish(),
    );

    // Text label (shrinkable so long names are clipped, not truncated at a fixed width).
    let text_color = config.item_highlight_state.text_and_icon_color(appearance);
    let text_style = if config.is_ignored {
        Properties::default()
            .style(Style::Italic)
            .weight(Weight::Light)
    } else {
        Properties::default()
    };
    row.add_child(
        Shrinkable::new(
            1.,
            Text::new_inline(config.name, appearance.ui_font_family(), ITEM_FONT_SIZE)
                .with_color(text_color)
                .with_style(text_style)
                .finish(),
        )
        .finish(),
    );

    row.finish()
}
