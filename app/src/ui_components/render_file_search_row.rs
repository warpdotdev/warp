//! File search row rendering components.
//!
//! This module provides UI components for rendering file and directory search
//! results in search interfaces. It handles the display of file names with
//! their parent paths and supports fuzzy match highlighting.
//!
//! Two truncation mechanisms are available:
//! - An optional combined character-count cap (`max_combined_length`) that
//!   pre-truncates the path's trailing characters with `...`. Useful for very
//!   compact UIs.
//! - Pixel-aware clipping by the text layout engine, which fades or renders a
//!   leading `…` when the row is too narrow to fit the full path. This is the
//!   default for callers that pass `max_combined_length: None`.

use fuzzy_match::FuzzyMatchResult;
use std::path::Path;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Container, CrossAxisAlignment, Flex, Highlight, MainAxisSize, ParentElement, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::text_layout::{ClipConfig, ClipDirection, ClipStyle};
use warpui::{AppContext, Element};

use crate::appearance::Appearance;
use crate::search::ai_context_menu::safe_truncate;
use crate::search::ItemHighlightState;
use warpui::SingletonEntity;

pub const MAX_COMBINED_LENGTH: usize = 55;

pub struct FileSearchRowOptions<'a> {
    pub match_result: Option<&'a FuzzyMatchResult>,
    pub highlight_state: ItemHighlightState,
    pub item_font_size: Option<f32>,
    pub path_font_size: Option<f32>,
    pub item_text_fill_override: Option<Fill>,
    pub text_color_override: Option<Fill>,
    pub max_combined_length: Option<usize>,
}

impl<'a> Default for FileSearchRowOptions<'a> {
    fn default() -> Self {
        Self {
            match_result: None,
            highlight_state: ItemHighlightState::Default,
            item_font_size: None,
            path_font_size: None,
            item_text_fill_override: None,
            text_color_override: None,
            max_combined_length: Some(MAX_COMBINED_LENGTH),
        }
    }
}

/// Renders a file search result row containing a file/directory name with optional path context.
///
/// This function creates a UI element that displays a file or directory name along with its parent
/// path, with support for fuzzy match highlighting and intelligent truncation. The layout adapts
/// based on the highlight state and prioritizes showing the filename over the full path when space
/// is limited.
///
/// # Arguments
///
/// * `path` - The full path to the file or directory
/// * `options` - Rendering options (match result, highlight state, font sizes, etc.)
/// * `app` - Application context for accessing themes and fonts
///
/// # Returns
///
/// A boxed UI element representing the file search row
pub fn render_file_search_row(
    path: &Path,
    options: FileSearchRowOptions<'_>,
    app: &AppContext,
) -> Box<dyn Element> {
    let FileSearchRowOptions {
        match_result,
        highlight_state,
        item_font_size,
        path_font_size,
        item_text_fill_override,
        text_color_override,
        max_combined_length,
    } = options;
    let appearance = Appearance::as_ref(app);

    // Extract item name from path (file or directory name)
    let max_combined_length = max_combined_length.unwrap_or(MAX_COMBINED_LENGTH);

    let original_item_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string();
    let mut item_name = original_item_name.clone();

    // Create path display (show it grayed out)
    let original_path_display = path
        .parent()
        .and_then(|parent| parent.to_str())
        .unwrap_or("")
        .to_string();
    let mut path_display = original_path_display.clone();

    // Track if we truncated anything for highlight adjustment
    let mut filename_truncated = false;
    let mut path_truncated = false;
    let mut path_truncation_offset = 0;

    // Ensure combined length is less than MAX_COMBINED_LENGTH characters
    if options.max_combined_length.is_some() {
        let combined_length = item_name.len() + path_display.len();

        if combined_length > max_combined_length {
            if item_name.len() >= max_combined_length {
                safe_truncate(&mut item_name, max_combined_length - 3);
                item_name.push_str("...");
                filename_truncated = true;
                path_display.clear();
            } else {
                let available_for_path = max_combined_length - item_name.len();
                if path_display.len() > available_for_path {
                    let new_path_len = available_for_path.saturating_sub(3);
                    path_truncation_offset = path_display.len() - new_path_len;
                    safe_truncate(&mut path_display, new_path_len);
                    path_display.push_str("...");
                    path_truncated = true;
                }
            }
        }
    }

    // Calculate highlight indices for item name and path
    let (item_name_highlights, path_highlights) = if let Some(match_result) = match_result {
        let full_path_str = path.to_string_lossy();
        let item_name_start_in_full_path = full_path_str.len() - original_item_name.len();
        calculate_highlight_indices(
            match_result,
            &original_path_display,
            item_name_start_in_full_path,
            filename_truncated,
            path_truncated,
            path_truncation_offset,
            max_combined_length,
        )
    } else {
        (Vec::new(), Vec::new())
    };

    let item_font_size: f32 =
        item_font_size.unwrap_or_else(|| appearance.monospace_font_size() - 1.0);
    let path_font_size: f32 =
        path_font_size.unwrap_or_else(|| appearance.monospace_font_size() - 2.0);

    let base_item_fill: Fill =
        item_text_fill_override.unwrap_or_else(|| highlight_state.main_text_fill(appearance));
    let base_path_fill: Fill = highlight_state.sub_text_fill(appearance);

    let base_item_color = base_item_fill.into_solid();
    let base_path_color = base_path_fill.into_solid();

    let (item_color, path_color) = if let Some(override_fill) = text_color_override {
        let override_color = override_fill.into_solid();
        (override_color, override_color)
    } else {
        (base_item_color, base_path_color)
    };

    // Create item name with match highlighting
    let mut item_text = Text::new_inline(item_name, appearance.ui_font_family(), item_font_size)
        .with_color(item_color)
        .soft_wrap(false);

    if !item_name_highlights.is_empty() {
        item_text = item_text.with_single_highlight(
            Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
            item_name_highlights,
        );
    }

    // Create path text with lighter color and highlights. Clipping happens at the
    // leading edge with a literal `…` so the trailing (more informative) directories
    // remain visible when the row is too narrow to show the full path.
    let path_text = if !path_display.is_empty() {
        let mut path_text =
            Text::new_inline(path_display, appearance.ui_font_family(), path_font_size)
                .with_color(path_color)
                .with_clip(ClipConfig {
                    direction: ClipDirection::Start,
                    style: ClipStyle::Ellipsis,
                })
                .soft_wrap(false);

        if !path_highlights.is_empty() {
            path_text = path_text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                path_highlights,
            );
        }

        Some(path_text)
    } else {
        None
    };

    // Create row with item name and path
    let mut row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    row.add_child(
        Shrinkable::new(
            // setting this to a high value so that we don't shrink the item text until absolutely necessary
            20.0,
            item_text.finish(),
        )
        .finish(),
    );

    if let Some(path_text) = path_text {
        row.add_child(
            Shrinkable::new(
                1.0,
                Container::new(path_text.finish())
                    .with_padding_left(3.)
                    .finish(),
            )
            .finish(),
        );
    }

    row.finish()
}

/// Calculates highlight indices for both the item name and path portions of a file search result.
///
/// This function takes fuzzy match indices from the full path and splits them appropriately
/// between the filename and directory path components. It handles truncation adjustments
/// to ensure highlights remain accurate when text is shortened with ellipsis.
///
/// # Arguments
///
/// * `match_result` - The fuzzy match result containing highlight indices for the full path
/// * `original_path` - The original (untruncated) directory path string
/// * `item_name_start_in_full_path` - Byte offset where the filename begins in the full path
/// * `item_name_truncated` - Whether the filename was truncated with ellipsis
/// * `path_truncated` - Whether the directory path was truncated with ellipsis
/// * `path_truncation_offset` - Number of characters removed from the start of the path
///
/// # Returns
///
/// A tuple containing:
/// - `Vec<usize>` - Highlight indices for the item name portion
/// - `Vec<usize>` - Highlight indices for the directory path portion
fn calculate_highlight_indices(
    match_result: &FuzzyMatchResult,
    original_path: &str,
    item_name_start_in_full_path: usize,
    item_name_truncated: bool,
    path_truncated: bool,
    path_truncation_offset: usize,
    max_combined_length: usize,
) -> (Vec<usize>, Vec<usize>) {
    let mut item_name_highlights = Vec::new();
    let mut path_highlights = Vec::new();

    for &index in &match_result.matched_indices {
        if index >= item_name_start_in_full_path {
            // This highlight is in the item name
            let item_name_index = index - item_name_start_in_full_path;

            // Only include if within the displayed item name range
            if !item_name_truncated || item_name_index < (max_combined_length - 3) {
                item_name_highlights.push(item_name_index);
            }
        } else {
            // This highlight is in the path
            let path_index = index;

            // Adjust for path truncation
            if path_truncated {
                if path_index >= path_truncation_offset {
                    let adjusted_index = path_index - path_truncation_offset;
                    if adjusted_index < original_path.len().saturating_sub(3) {
                        path_highlights.push(adjusted_index);
                    }
                }
            } else {
                // No truncation, use index as-is
                if path_index < original_path.len() {
                    path_highlights.push(path_index);
                }
            }
        }
    }

    (item_name_highlights, path_highlights)
}
