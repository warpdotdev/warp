//! This module contains the [`View`] trait implementation for [`AIBlock`]
//!
//! Helper functions for rendering different AIBlock components are exported by the header, query,
//! and output submodules, where the intended layout is:
//! ```text
//! —————————————————————————————————
//! | 1 block attached              | <—————— header
//! |                               |
//! | <Avatar> What went wrong?     | <—————— query
//! |                               |
//! | * The error in your Rust      | <——
//! | project indicates a syntax    |    |
//! | issue. I will now try to run  |    |
//! | the following fix:            |    |——— output
//! | ——————————————————————        |    |
//! | | cargo fix          |        |    |
//! | ——————————————————————        | <——
//! —————————————————————————————————
//! ```

pub(super) mod common;
pub use common::FindContext;
mod comments;
mod header;
mod imported_comments;
mod input;
mod orchestration;
pub mod output;
pub mod query;
mod todos;

use common::get_highlight_ranges_for_find_matches;
use pathfinder_color::ColorU;
use settings::Setting as _;
use std::collections::{HashMap, HashSet};
use warp_core::features::FeatureFlag;
use warp_core::semantic_selection::SemanticSelection;
use warpui::elements::{
    Align, ConstrainedBox, CornerRadius, CrossAxisAlignment, Empty, Expanded, FormattedTextElement,
    Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, Radius, SavePosition,
    SelectableArea,
};
use warpui::{
    elements::{Border, Container, Flex, ParentElement},
    AppContext, Element, SingletonEntity,
};
use warpui::{View, ViewContext};

use crate::ai::agent::AIAgentCitation;
use crate::ai::agent::AIAgentInput;
use crate::ai::blocklist::block::view_impl::header::{
    render_overflow_menu_button, OVERFLOW_BUTTON_SIZE,
};
use crate::ai::blocklist::inline_action::inline_action_icons::icon_size;
use crate::ai::blocklist::model::AIBlockModelHelper;
use crate::appearance::Appearance;
use crate::settings::{AISettings, InputModeSettings, InputSettings};
use crate::terminal::model::blocks::{BlockHeightItem, RemovableBlocklistItem, RichContentItem};
use crate::terminal::model::rich_content::RichContentType;
use crate::terminal::view::ambient_agent::is_cloud_agent_pre_first_exchange;
use crate::terminal::TerminalView;
use crate::util::truncation::truncate_from_end;

use super::secret_redaction::SecretRedactionState;
use super::{
    attachment_names, AIBlock, AIBlockAction, DISPATCHED_REQUESTED_EDIT_KEYMAP_CONTEXT,
    HAS_PENDING_ACTION, RICH_CONTENT_SECRET_FIRST_CHAR_POSITION_ID,
};

use super::TextLocation;
use crate::ai::blocklist::block::view_impl::comments::address_comment_chips;
use crate::ai::blocklist::block::{DetectedLinksState, RICH_CONTENT_LINK_FIRST_CHAR_POSITION_ID};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::cloud_object::model::persistence::CloudModel;

use crate::settings_view::SettingsSection;
use crate::terminal::block_list_element::BlockListMenuSource;
use crate::terminal::grid_renderer::URL_COLOR;
use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::safe_mode_settings::get_secret_obfuscation_mode;
use crate::terminal::view::TerminalAction;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::util::link_detection::DetectedLinkType;
use crate::workspace::WorkspaceAction;
use itertools::Itertools;
use warp_core::ui::color::contrast::{
    foreground_color_with_minimum_contrast, MinimumAllowedContrast,
};
use warp_core::ui::color::Rgb;
use warp_core::ui::theme::{Fill, WarpTheme};
use warpui::elements::{Highlight, HighlightedRange, Text};
use warpui::fonts::Properties;
use warpui::platform::Cursor;
use warpui::text_layout::TextStyle;
use warpui::ui_components::components::UiComponent;

/// Helper function to create gray strikethrough highlight for secrets
fn create_secret_gray_highlight() -> Highlight {
    Highlight::new().with_text_style(
        TextStyle::new()
            .with_foreground_color(warpui::color::ColorU::new(128, 128, 128, 255))
            .with_show_strikethrough(true),
    )
}

/// Apply slash command highlighting (bold and magenta color) to an existing highlight or create a new one.
fn add_slash_command_highlight(
    appearance: &Appearance,
    existing_highlight: Option<Highlight>,
) -> Highlight {
    let theme = appearance.theme();
    let base_magenta = theme.ansi_fg_magenta();

    // Determine the background color to use for contrast checking
    // If there's an existing highlight with a background color in its TextStyle (e.g., find matches),
    // use that. Otherwise, use the theme's background.
    let background_color = if let Some(existing) = &existing_highlight {
        let current_style = existing.text_style();
        current_style
            .background_color
            .unwrap_or_else(|| theme.background().into_solid())
    } else {
        theme.background().into_solid()
    };

    // Enforce minimum contrast against the background
    let slash_command_foreground_color = foreground_color_with_minimum_contrast(
        base_magenta,
        Rgb::from(background_color),
        MinimumAllowedContrast::Text,
    );

    if let Some(existing) = existing_highlight {
        // Preserve existing text style and properties, but update foreground color and make bold
        let current_style = existing.text_style();
        let updated_style = (*current_style).with_foreground_color(slash_command_foreground_color);

        let current_properties = existing.properties();
        let mut bold_properties = current_properties;
        bold_properties.weight = warpui::fonts::Weight::Bold;

        Highlight::new()
            .with_text_style(updated_style)
            .with_properties(bold_properties)
    } else {
        // Create new highlight with default properties and bold weight
        let default_properties = Properties {
            weight: warpui::fonts::Weight::Bold,
            ..Default::default()
        };
        Highlight::new()
            .with_foreground_color(slash_command_foreground_color)
            .with_properties(default_properties)
    }
}

/// Adds the appropriate highlighting for secrets and links to the given text element.
#[allow(clippy::too_many_arguments)]
fn add_highlights_to_text(
    mut text_element: Text,
    detected_links_state: &DetectedLinksState,
    secret_redaction_state: &SecretRedactionState,
    find_context: Option<FindContext>,
    location: TextLocation,
    is_selecting: bool,
    hover_properties: Option<Properties>,
    slash_command_prefix_len: Option<usize>,
    app: &AppContext,
) -> Text {
    let mut link_highlight = Highlight::new().with_text_style(
        TextStyle::new()
            .with_foreground_color(*URL_COLOR)
            .with_underline_color(*URL_COLOR),
    );
    let secret_hover_click_highlight =
        Highlight::new().with_text_style(TextStyle::new().with_foreground_color(*URL_COLOR));
    let mut highlighted_ranges = vec![];

    if let Some(open_secret_tooltip) = &secret_redaction_state.open_tooltip_location() {
        if open_secret_tooltip.location == location {
            text_element = text_element.with_saved_char_position(
                open_secret_tooltip.secret_range.char_range.start,
                RICH_CONTENT_SECRET_FIRST_CHAR_POSITION_ID.to_owned(),
            );
        }
    }

    // Add gray + strikethrough styling for all detected secrets when in strikethrough mode
    if let Some(detected_secrets) = secret_redaction_state.secrets_for_location(&location) {
        if matches!(
            get_secret_obfuscation_mode(app),
            ObfuscateSecrets::Strikethrough
        ) {
            for secret_range in detected_secrets.detected_secrets.keys() {
                // Skip gray styling if this secret is currently hovered or has tooltip open
                if !secret_redaction_state.is_hovered(&location, secret_range)
                    && !secret_redaction_state.has_open_tooltip(&location, secret_range)
                {
                    let highlight_indices = secret_range.char_range.clone().collect_vec();
                    if highlight_indices.is_empty() {
                        continue;
                    }
                    highlighted_ranges.push(HighlightedRange {
                        highlight: create_secret_gray_highlight(),
                        highlight_indices,
                    });
                }
            }
        }
    }

    // If we have an open tooltip, that secret should be highlighted.
    if let Some(open_secret_tooltip) = &secret_redaction_state.open_tooltip_location() {
        if open_secret_tooltip.location == location {
            let highlight_indices = open_secret_tooltip
                .secret_range
                .char_range
                .clone()
                .collect_vec();
            if !highlight_indices.is_empty() {
                highlighted_ranges.push(HighlightedRange {
                    highlight: secret_hover_click_highlight,
                    highlight_indices,
                });
            }
        }
    }
    // Also highlight any currently hovered secret if it's different.
    if let Some(currently_hovered_secret) = &secret_redaction_state.hovered_location() {
        if currently_hovered_secret.location == location
            && secret_redaction_state.hovered_location()
                != secret_redaction_state.open_tooltip_location()
        {
            let highlight_indices = currently_hovered_secret
                .secret_range
                .char_range
                .clone()
                .collect_vec();
            if !highlight_indices.is_empty() {
                highlighted_ranges.push(HighlightedRange {
                    highlight: secret_hover_click_highlight,
                    highlight_indices,
                });
            }
        }
    }

    if !is_selecting {
        if let Some(properties) = hover_properties {
            link_highlight = link_highlight.with_properties(properties);
        }

        // Link highlighting.
        // If we have an open tooltip, that link should be highlighted.
        if let Some(open_link_tooltip) = &detected_links_state.link_location_open_tooltip {
            if open_link_tooltip.location == location {
                let highlight_indices = open_link_tooltip.link_range.clone().collect_vec();
                if !highlight_indices.is_empty() {
                    highlighted_ranges.push(HighlightedRange {
                        highlight: link_highlight,
                        highlight_indices,
                    });
                }
                text_element = text_element.with_saved_char_position(
                    open_link_tooltip.link_range.start,
                    RICH_CONTENT_LINK_FIRST_CHAR_POSITION_ID.to_owned(),
                );
            }
        }
        // Also highlight any currently hovered link if it's different.
        if let Some(currently_hovered_link) = &detected_links_state.currently_hovered_link_location
        {
            if currently_hovered_link.location == location
                && detected_links_state.currently_hovered_link_location
                    != detected_links_state.link_location_open_tooltip
            {
                let highlight_indices = currently_hovered_link.link_range.clone().collect_vec();
                if !highlight_indices.is_empty() {
                    highlighted_ranges.push(HighlightedRange {
                        highlight: link_highlight,
                        highlight_indices,
                    });
                }
            }
        }

        if let Some(find_context) = find_context {
            highlighted_ranges.extend(get_highlight_ranges_for_find_matches(
                location,
                find_context.state,
                find_context.model,
            ));
        }
    }

    // Merge overlapping or contiguous ranges
    let mut merged_highlighted_ranges =
        HighlightedRange::merge_overlapping_ranges(highlighted_ranges);

    if let Some(slash_command_prefix_len) = slash_command_prefix_len {
        let appearance = Appearance::as_ref(app);

        // Build a set of all indices in the slash_command prefix that need highlighting
        let mut unhandled_plan_indices: HashSet<usize> = (0..slash_command_prefix_len).collect();

        let mut result = vec![];

        // Process existing highlights, updating any that overlap with the slash command prefix.
        for range in merged_highlighted_ranges {
            let range_start = range.highlight_indices.first().cloned().unwrap_or(0);
            let range_end = range
                .highlight_indices
                .last()
                .cloned()
                .unwrap_or(range_start);

            if range_end < slash_command_prefix_len {
                // Range is entirely within /plan prefix
                // Update to use plan color + bold, and mark these indices as handled
                for &idx in &range.highlight_indices {
                    unhandled_plan_indices.remove(&idx);
                }

                result.push(HighlightedRange {
                    highlight: add_slash_command_highlight(appearance, Some(range.highlight)),
                    highlight_indices: range.highlight_indices,
                });
            } else if range_start >= slash_command_prefix_len {
                // Range is entirely after /plan prefix - keep as-is
                result.push(range);
            } else {
                // Range spans the /plan boundary - split it
                let mut prefix_part_indices = vec![];
                let mut after_part_indices = vec![];

                for idx in range.highlight_indices {
                    if idx < slash_command_prefix_len {
                        prefix_part_indices.push(idx);
                        unhandled_plan_indices.remove(&idx);
                    } else {
                        after_part_indices.push(idx);
                    }
                }

                // Add the styling to the part containing the slash command.
                if !prefix_part_indices.is_empty() {
                    result.push(HighlightedRange {
                        highlight: add_slash_command_highlight(appearance, Some(range.highlight)),
                        highlight_indices: prefix_part_indices,
                    });
                }

                // Add the after part with original styling
                if !after_part_indices.is_empty() {
                    result.push(HighlightedRange {
                        highlight: range.highlight,
                        highlight_indices: after_part_indices,
                    });
                }
            }
        }

        // Add highlights for any remaining /plan indices that didn't overlap with existing highlights
        if !unhandled_plan_indices.is_empty() {
            let mut sorted_indices: Vec<usize> = unhandled_plan_indices.into_iter().collect();
            sorted_indices.sort_unstable();

            // Group contiguous indices into ranges
            let mut current_group = vec![sorted_indices[0]];
            for &idx in &sorted_indices[1..] {
                if idx == current_group.last().unwrap() + 1 {
                    current_group.push(idx);
                } else {
                    // Finish current group
                    result.push(HighlightedRange {
                        highlight: add_slash_command_highlight(appearance, None),
                        highlight_indices: current_group,
                    });
                    current_group = vec![idx];
                }
            }
            // Add the final group
            result.push(HighlightedRange {
                highlight: add_slash_command_highlight(appearance, None),
                highlight_indices: current_group,
            });
        }

        merged_highlighted_ranges = result;
    }

    // If there are no highlights, don't add any
    if merged_highlighted_ranges.is_empty() {
        return text_element;
    }

    // Sort highlight ranges to avoid any panics (sorted ranges are expected).
    text_element = text_element.with_highlights(
        merged_highlighted_ranges
            .into_iter()
            .sorted_by_key(|highlighted_range| highlighted_range.highlight_indices[0]),
    );

    text_element
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn add_highlights_to_rich_text(
    mut formatted_text_element: FormattedTextElement,
    detected_links_state: Option<&DetectedLinksState>,
    secret_redaction_state: &SecretRedactionState,
    find_context: Option<FindContext<'_>>,
    location_index: usize,
    line_count: usize,
    theme: &WarpTheme,
    is_selecting: bool,
    is_action: bool,
    app: &AppContext,
) -> FormattedTextElement {
    let ansi_blue = theme.terminal_colors().normal.blue;
    let blue_fg = theme.ansi_fg(ansi_blue);
    let url_highlight = Highlight::new().with_foreground_color(ansi_blue.into());
    let url_hover_click_highlight = Highlight::new().with_text_style(
        TextStyle::new()
            .with_foreground_color(blue_fg)
            .with_underline_color(blue_fg),
    );
    let file_hover_click_highlight = Highlight::new().with_text_style(
        TextStyle::new()
            .with_foreground_color(ansi_blue.into())
            .with_underline_color(ansi_blue.into()),
    );
    let secret_hover_click_highlight =
        Highlight::new().with_text_style(TextStyle::new().with_foreground_color(ansi_blue.into()));

    for i in 0..line_count {
        let location = if is_action {
            TextLocation::Action {
                action_index: location_index,
                line_index: i,
            }
        } else {
            TextLocation::Output {
                section_index: location_index,
                line_index: i,
            }
        };

        let mut style_ranges = vec![];
        if let Some(detected_links_state) = detected_links_state {
            // Add highlighting to url links.
            if let Some(links) = detected_links_state
                .detected_links_by_location
                .get(&location)
            {
                style_ranges = links
                    .detected_links
                    .iter()
                    .filter_map(|(range, link)| {
                        // Stylings like [](https://example.com) would cause warp to panic. We need to filter that out.
                        if range.is_empty() {
                            return None;
                        }
                        let highlight_indices = range.clone().collect_vec();
                        if highlight_indices.is_empty() {
                            return None;
                        }
                        // If the current link is hovered or clicked, highlight it. We add the styles here to
                        // prevent duplicate styles for the same link.
                        let mut link_highlight_location = detected_links_state
                            .currently_hovered_link_location
                            .as_ref();

                        if link_highlight_location.is_none_or(|link| {
                            link.location != location || link.link_range != *range
                        }) {
                            link_highlight_location =
                                detected_links_state.link_location_open_tooltip.as_ref()
                        }

                        if is_selecting {
                            link_highlight_location = None;
                        }

                        if let Some(link_location) = link_highlight_location {
                            if link_location.location == location
                                && link_location.link_range == *range
                            {
                                let hover_highlight =
                                    if matches!(link.link, DetectedLinkType::Url(_)) {
                                        url_hover_click_highlight
                                    } else {
                                        file_hover_click_highlight
                                    };
                                return Some(HighlightedRange {
                                    highlight_indices,
                                    highlight: hover_highlight,
                                });
                            }
                        }

                        if matches!(link.link, DetectedLinkType::Url(_)) {
                            Some(HighlightedRange {
                                highlight_indices,
                                highlight: url_highlight,
                            })
                        } else {
                            None
                        }
                    })
                    .collect_vec();
            }

            if let Some(open_link_tooltip) = &detected_links_state.link_location_open_tooltip {
                if open_link_tooltip.location == location {
                    formatted_text_element = formatted_text_element.with_saved_glyph_position(
                        open_link_tooltip.link_range.start,
                        i,
                        RICH_CONTENT_LINK_FIRST_CHAR_POSITION_ID.to_owned(),
                    );
                }
            }
        }

        if let Some(open_secret_tooltip) = &secret_redaction_state.open_tooltip_location() {
            if open_secret_tooltip.location == location {
                formatted_text_element = formatted_text_element.with_saved_glyph_position(
                    open_secret_tooltip.secret_range.char_range.start,
                    i,
                    RICH_CONTENT_SECRET_FIRST_CHAR_POSITION_ID.to_owned(),
                );
                if !is_selecting {
                    let highlight_indices = open_secret_tooltip
                        .secret_range
                        .char_range
                        .clone()
                        .collect_vec();
                    if !highlight_indices.is_empty() {
                        style_ranges.push(HighlightedRange {
                            highlight_indices,
                            highlight: secret_hover_click_highlight,
                        });
                    }
                }
            }
        }

        // Also highlight any currently hovered secret if it's different.
        if let Some(currently_hovered_secret) = &secret_redaction_state.hovered_location() {
            if currently_hovered_secret.location == location
                && secret_redaction_state.hovered_location()
                    != secret_redaction_state.open_tooltip_location()
            {
                let highlight_indices = currently_hovered_secret
                    .secret_range
                    .char_range
                    .clone()
                    .collect_vec();
                if !highlight_indices.is_empty() {
                    style_ranges.push(HighlightedRange {
                        highlight_indices,
                        highlight: secret_hover_click_highlight,
                    });
                }
            }
        }

        // Add gray + strikethrough styling for all detected secrets in rich text
        if matches!(
            get_secret_obfuscation_mode(app),
            ObfuscateSecrets::Strikethrough
        ) {
            if let Some(detected_secrets) = secret_redaction_state.secrets_for_location(&location) {
                for secret_range in detected_secrets.detected_secrets.keys() {
                    // Skip gray styling if this secret is currently hovered or has tooltip open
                    if !secret_redaction_state.is_hovered(&location, secret_range)
                        && !secret_redaction_state.has_open_tooltip(&location, secret_range)
                    {
                        let highlight_indices = secret_range.char_range.clone().collect_vec();
                        if highlight_indices.is_empty() {
                            continue;
                        }
                        style_ranges.push(HighlightedRange {
                            highlight: create_secret_gray_highlight(),
                            highlight_indices,
                        });
                    }
                }
            }
        }

        if let Some(find_params) = find_context.as_ref() {
            style_ranges.extend(get_highlight_ranges_for_find_matches(
                location,
                find_params.state,
                find_params.model,
            ));
        }

        let merged_range = HighlightedRange::merge_overlapping_ranges(style_ranges);
        let sorted_range = merged_range
            .into_iter()
            .sorted_by_key(|range| range.highlight_indices[0]);

        formatted_text_element.add_styles(i, sorted_range);
    }
    formatted_text_element
}

/// Renders a row of citations.
///
/// TODO: All AIBlock footer-related rendering logic should probably be put into its own View.
/// This function is needed both above (i.e. `block.rs`) and below (i.e. `output.rs`), and as such
/// cannot reside in `output.rs` because we don't want to make `mod output` public.
pub fn render_citation_chips(
    citations: &[AIAgentCitation],
    citation_state_handles: &HashMap<AIAgentCitation, MouseStateHandle>,
    font_size: f32,
    padding: f32,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let renderable_citations = citations
        .iter()
        .filter_map(|citation| {
            citation_state_handles
                .get(citation)
                .and_then(|mouse_handle| {
                    render_citation(citation, mouse_handle.clone(), font_size, padding, app)
                })
        })
        .collect_vec();

    if renderable_citations.is_empty() {
        return None;
    }

    Some(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children(
                renderable_citations
                    .into_iter()
                    .map(|citation| Container::new(citation).with_margin_right(8.).finish()),
            )
            .finish(),
    )
}

/// Renders a single citations chip.
///
/// TODO: All AIBlock footer-related rendering logic should probably be put into its own View.
/// This function is needed both above (i.e. `block.rs`) and below (i.e. `output.rs`), and as such
/// cannot reside in `output.rs` because we don't want to make `mod output` public.
pub fn render_citation(
    citation: &AIAgentCitation,
    mouse_state_handle: MouseStateHandle,
    font_size: f32,
    padding: f32,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let (icon, name) = match citation {
        AIAgentCitation::WarpDriveObject { uid } => {
            let item = CloudModel::as_ref(app)
                .get_by_uid(uid)?
                .to_warp_drive_item(appearance)?;
            (
                item.icon(appearance, Some(theme.active_ui_text_color())),
                item.display_name().unwrap_or(String::from("Untitled")),
            )
        }
        AIAgentCitation::WarpDocumentation { .. } => {
            let icon = Icon::Warp.to_warpui_icon(theme.foreground()).finish();
            let name = String::from("Warp Docs");
            (Some(icon), name)
        }
        AIAgentCitation::WebPage { url } => {
            let icon = Icon::LinkExternal
                .to_warpui_icon(theme.foreground())
                .finish();
            let name = url.clone();
            (Some(icon), name)
        }
    };

    // Shorten the name to 30 chars.
    let shortened_name = truncate_from_end(&name, 30);

    let mut row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(icon) = icon {
        row.add_child(
            Container::new(
                ConstrainedBox::new(icon)
                    .with_width(font_size)
                    .with_height(font_size)
                    .finish(),
            )
            .with_margin_right(4.)
            .finish(),
        );
    }
    row.add_child(
        Text::new_inline(shortened_name, appearance.ui_font_family(), font_size)
            .with_color(theme.active_ui_text_color().into())
            .with_selectable(false)
            .finish(),
    );

    let chip = Container::new(row.finish())
        .with_uniform_padding(padding)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_background(blended_colors::neutral_3(theme))
        .finish();
    let citation_clone = citation.clone();
    Some(
        Hoverable::new(mouse_state_handle, |_| chip)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(AIBlockAction::OpenCitation(citation_clone.clone()));
            })
            .with_cursor(Cursor::PointingHand)
            .finish(),
    )
}

/// Renders the Ask-User-Question speedbump footer: a short description label, a
/// dropdown for the `ask_user_question` permission, and a right-aligned
/// "Manage AI Autonomy permissions" link. Matches the visual rhythm of
/// [`render_autonomy_checkbox_setting_speedbump_footer`].
pub fn render_autonomy_dropdown_setting_speedbump_footer<A>(
    description: &'static str,
    dropdown: &warpui::ViewHandle<crate::view_components::dropdown::Dropdown<A>>,
    settings_link_handle: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element>
where
    A: warpui::Action + Clone,
{
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(
            Container::new(
                Text::new(
                    description,
                    appearance.ui_font_family(),
                    appearance.monospace_font_size() - 1.,
                )
                .with_color(blended_colors::text_sub(theme, theme.surface_1()))
                .with_selectable(false)
                .finish(),
            )
            .with_margin_right(8.)
            .finish(),
        )
        .with_child(warpui::elements::ChildView::new(dropdown).finish())
        .with_child(
            Expanded::new(
                1.,
                Align::new(
                    appearance
                        .ui_builder()
                        .link(
                            "Manage AI Autonomy permissions".into(),
                            None,
                            Some(Box::new(move |ctx| {
                                ctx.dispatch_typed_action(
                                    WorkspaceAction::ShowSettingsPageWithSearch {
                                        search_query: "Autonomy".to_string(),
                                        section: Some(SettingsSection::AI),
                                    },
                                );
                            })),
                            settings_link_handle,
                        )
                        .build()
                        .finish(),
                )
                .right()
                .finish(),
            )
            .finish(),
        )
        .finish()
}

/// TODO: All AIBlock footer-related rendering logic should probably be put into its own View.
/// This function is needed both above (i.e. `block.rs`) and below (i.e. `output.rs`), and as such
/// cannot reside in `output.rs` because we don't want to make `mod output` public.
pub fn render_autonomy_checkbox_setting_speedbump_footer(
    description: &'static str,
    checked: bool,
    on_toggled_action: AIBlockAction,
    checkbox_handle: MouseStateHandle,
    settings_link_handle: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(
            Container::new(
                appearance
                    .ui_builder()
                    .checkbox(checkbox_handle, None)
                    .check(checked)
                    .build()
                    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(on_toggled_action.clone()))
                    .finish(),
            )
            .with_margin_left(-4.)
            .finish(),
        )
        .with_child(
            Container::new(
                Text::new(
                    description,
                    appearance.ui_font_family(),
                    appearance.monospace_font_size() - 1.,
                )
                .with_color(blended_colors::text_sub(theme, theme.surface_1()))
                .with_selectable(false)
                .finish(),
            )
            .with_margin_left(4.)
            .finish(),
        )
        .with_child(
            Expanded::new(
                1.,
                Align::new(
                    appearance
                        .ui_builder()
                        .link(
                            "Manage AI Autonomy permissions".into(),
                            None,
                            Some(Box::new(move |ctx| {
                                ctx.dispatch_typed_action(
                                    WorkspaceAction::ShowSettingsPageWithSearch {
                                        search_query: "Autonomy".to_string(),
                                        section: Some(SettingsSection::WarpAgent),
                                    },
                                );
                            })),
                            settings_link_handle,
                        )
                        .build()
                        .finish(),
                )
                .right()
                .finish(),
            )
            .finish(),
        )
        .finish()
}

impl View for AIBlock {
    fn ui_name() -> &'static str {
        "AIBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // When the AI block is hidden, we don't need to render anything.
        if self.is_hidden(app) {
            return ConstrainedBox::new(Empty::new().finish())
                .with_height(0.)
                .finish();
        }

        // When the backing conversation has been cleared (e.g., after logout/reset), skip rendering.
        // This can happen right after the user logs out, when the window is still potentially rendering for a few frames
        // but the history model has already been cleared. It's safe to just skip rendering in this case.
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(app).conversation(&self.client_ids.conversation_id)
        else {
            return ConstrainedBox::new(Empty::new().finish())
                .with_height(0.)
                .finish();
        };
        let addressed_comment_ids = conversation.addressed_comment_ids();
        let mut contents = Flex::column();

        let input_props = input::Props {
            comments: &self.comment_states,
            addressed_comment_ids: &addressed_comment_ids,
        };
        let initial_conversation_query = self
            .model
            .conversation(app)
            .and_then(|c| c.initial_user_query());
        let query_and_index = self
            .model
            .inputs_to_render(app)
            .iter()
            .enumerate()
            .find_map(|(input_index, input)| {
                let element_below_user_query = input.element_below_user_query(input_props, app);
                let user_query = input.display_user_query(initial_conversation_query.as_ref())?;
                let query_prefix_highlight_len =
                    common::query_prefix_highlight_len(input, &user_query);

                Some((
                    user_query,
                    input_index,
                    query_prefix_highlight_len,
                    element_below_user_query,
                ))
            });
        let query_and_index_is_some = query_and_index.is_some();
        let attachment_name_list = if FeatureFlag::ImageAsContext.is_enabled() {
            attachment_names(self.model.inputs_to_render(app))
        } else {
            vec![]
        };
        if let Some((
            query_for_display,
            input_index,
            query_prefix_highlight_len,
            elements_below_query,
        )) = query_and_index
        {
            let mut did_render_header = false;
            if let Some(header) = header::render(
                header::Props {
                    attached_blocks_chip_mouse_state: &self
                        .state_handles
                        .attached_blocks_chip_state_handle,
                    overflow_menu_mouse_state: &self.state_handles.overflow_menu_handle,
                    rewind_button: &self.rewind_button,
                    num_attached_context_blocks: self.num_attached_context_blocks,
                    has_attached_context_selected_text: self.has_attached_context_selected_text,
                    directory_context: &self.directory_context,
                    view_id: &self.view_id,
                    exchange_id: &self.client_ids.client_exchange_id,
                    conversation_id: &self.client_ids.conversation_id,
                    is_selected_text_attached_as_context: self
                        .context_model
                        .as_ref(app)
                        .pending_context_selected_text()
                        .is_some(),
                    is_restored: self.is_restored(),
                },
                app,
            ) {
                // Only render the prompt "header" for blocks containing a user query (as opposed to a
                // requested command result).
                contents.add_child(header.with_content_item_spacing().finish());
                did_render_header = true;
            }
            // Derive the display info for the participant who initiated this exchange.
            // For non-shared sessions, this is just the current user.
            // For shared sessions, this is the user who initiated the request.
            let (avatar_display_name, profile_image_path, avatar_color) = self
                .model
                .response_initiator(app)
                .and_then(|participant_id| {
                    app.view_with_id::<TerminalView>(self.window_id, self.terminal_view_id)
                        .and_then(|terminal_view| {
                            terminal_view.read(app, |view, app| {
                                view.shared_session_presence_manager().and_then(move |pm| {
                                    pm.as_ref(app).get_participant(&participant_id).map(
                                        |participant| {
                                            // Get the display info from the participant
                                            // who sent this query.
                                            (
                                                participant.info.profile_data.display_name.clone(),
                                                participant.info.profile_data.photo_url.clone(),
                                                Some(participant.color),
                                            )
                                        },
                                    )
                                })
                            })
                        })
                })
                // Fallback to the current user's info if this is not a shared session
                // or the participant is not found.
                .unwrap_or((
                    self.user_display_name.clone(),
                    self.profile_image_path.clone(),
                    None,
                ));
            if let Some(rendered_query) = query::maybe_render(
                query::Props {
                    user_display_name: &avatar_display_name,
                    profile_image_path: profile_image_path.as_ref(),
                    avatar_color,
                    query_and_index: Some((&query_for_display, input_index)),
                    query_prefix_highlight_len,
                    detected_links_state: &self.detected_links_state,
                    secret_redaction_state: &self.secret_redaction_state,
                    is_selecting_text: self.state_handles.selection_handle.is_selecting(),
                    is_ai_input_enabled: self
                        .context_model
                        .as_ref(app)
                        .pending_context_selected_text()
                        .is_some(),
                    attachments: &attachment_name_list,
                    find_context: self.find_model.as_ref(app).is_find_bar_open().then_some(
                        FindContext {
                            model: self.find_model.as_ref(app),
                            state: &self.find_state,
                        },
                    ),
                },
                app,
            ) {
                if did_render_header {
                    contents.add_child(rendered_query.with_content_item_spacing().finish());
                } else {
                    // The query element is designed to be exactly icon_size() height.
                    let rendered_query_height = icon_size(app);
                    let margin_bottom = (CONTENT_ITEM_VERTICAL_MARGIN
                        - (OVERFLOW_BUTTON_SIZE - rendered_query_height).max(0.))
                    .max(0.);
                    contents.add_child(
                        Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Start)
                            .with_child(Expanded::new(1., rendered_query).finish())
                            .with_child(render_overflow_menu_button(
                                self.state_handles.overflow_menu_handle.clone(),
                                self.view_id,
                                self.client_ids.client_exchange_id,
                                self.client_ids.conversation_id,
                                self.is_restored(),
                                app,
                            ))
                            .finish()
                            .with_content_item_spacing()
                            .with_margin_bottom(margin_bottom)
                            .finish(),
                    );
                }
            }

            if let Some(element) = elements_below_query {
                contents.add_child(element.with_agent_output_item_spacing(app).finish());
            }
        }

        let has_accepted_edits = self.has_accepted_file_edits_since_last_query(app);
        let terminal_model = self.terminal_model.lock();
        let shared_session_status = terminal_model.shared_session_status().clone();
        let is_conversation_transcript_viewer = terminal_model.is_conversation_transcript_viewer();

        let is_cloud_agent_pre_first_exchange = is_cloud_agent_pre_first_exchange(
            self.ambient_agent_view_model.as_ref(),
            &self.agent_view_controller,
            &terminal_model,
            app,
        );
        drop(terminal_model);

        contents.add_child(output::render(
            output::Props {
                model: self.model.as_ref(),
                state_handles: &self.state_handles,
                action_buttons: &self.action_buttons,
                view_screenshot_buttons: &self.view_screenshot_buttons,
                action_model: &self.action_model,
                editor_views: &self.code_editor_views,
                current_working_directory: self.current_working_directory.as_ref(),
                shell_launch_data: self.shell_launch_data.as_ref(),
                detected_links_state: &self.detected_links_state,
                secret_redaction_state: &self.secret_redaction_state,
                requested_commands: &self.requested_commands,
                requested_mcp_tools: &self.requested_mcp_tools,
                requested_edits: &self.requested_edits,
                unit_test_suggestions: &self.unit_tests_suggestions,
                todo_list_states: &self.todo_list_states,
                collapsible_block_states: &self.collapsible_block_states,
                is_selecting_text: self.state_handles.selection_handle.is_selecting(),
                is_ai_input_enabled: self
                    .context_model
                    .as_ref(app)
                    .pending_context_selected_text()
                    .is_some(),
                find_context: self.find_model.as_ref(app).is_find_bar_open().then_some(
                    FindContext {
                        model: self.find_model.as_ref(app),
                        state: &self.find_state,
                    },
                ),
                is_references_section_open: self.is_references_section_open,
                autonomy_setting_speedbump: &self.autonomy_setting_speedbump,
                suggested_rules: &self.suggested_rules,
                suggested_agent_mode_workflow: &self.suggested_agent_mode_workflow,
                manage_rules_button: &self.manage_rules_button,
                keyboard_navigable_buttons: self.keyboard_navigable_buttons.as_ref(),
                response_rating: &self.response_rating,
                request_refunded_count: self.request_refunded_count,
                search_codebase_view: &self.search_codebase_view,
                web_search_views: &self.web_search_views,
                web_fetch_views: &self.web_fetch_views,
                review_changes_button: &self.review_changes_button,
                open_all_comments_button: &self.open_all_comments_button,
                dismiss_suggestion_button: &self.dismiss_suggestion_button,
                disable_rule_suggestions_button: &self.disable_rule_suggestions_button,
                has_accepted_edits,
                current_todo_list: self.current_todo_list(app),
                finish_reason: self.finish_reason.as_ref(),
                is_usage_footer_expanded: self.is_usage_footer_expanded,
                shared_session_status: &shared_session_status,
                terminal_view_id: self.terminal_view_id,
                is_conversation_transcript_viewer,
                aws_bedrock_credentials_error_view: self
                    .aws_bedrock_credentials_error_view
                    .as_ref(),
                imported_comments: &self.imported_comments,
                run_agents_card_views: &self.run_agents_card_views,
                #[cfg(feature = "local_fs")]
                resolved_code_block_paths: &self.resolved_code_block_paths,
                #[cfg(feature = "local_fs")]
                resolved_blocklist_image_sources: &self.resolved_blocklist_image_sources,
                thinking_display_mode: AISettings::as_ref(app).thinking_display_mode,
                conversation_has_imported_comments: self
                    .model
                    .is_latest_visible_exchange_in_root_task(app)
                    && self.has_imported_comments_in_current_thread(app),
                ask_user_question_view: self.ask_user_question_view.as_ref(),
                is_cloud_agent_pre_first_exchange,
            },
            app,
        ));

        let should_use_transparent_overlay = InputSettings::as_ref(app)
            .is_universal_developer_input_enabled(app)
            || FeatureFlag::AgentView.is_enabled();

        let theme = Appearance::as_ref(app).theme();
        // Even though forked blocks are technically "restored", this is an implementation detail
        // and should not be exposed to the user. Only truly restored blocks (i.e. blocks from a closed pane or session)
        // should have the restored theme applied.
        let background_color = if self.model.is_restored()
            && !self.model.is_forked()
            && !FeatureFlag::AgentView.is_enabled()
        {
            theme.restored_ai_blocks_overlay()
        } else if should_use_transparent_overlay {
            // Use a fully transparent background for universal developer input
            Fill::Solid(ColorU::transparent_black())
        } else {
            theme.ai_blocks_overlay()
        };

        let mut content = Container::new(contents.finish()).with_background(background_color);

        // Only render visual separation between this block and the previous block if this block
        // is for a user query. Otherwise, this block is for an action result, which is tied to
        // the output of the previous block, so we omit visual separation to make this block appear
        // as if its part of the previous block.
        //
        // For example, consider a query that results in a requested command. We want the second
        // block (where requested command output is the AI input) to appear part of the original
        // query block.
        let contains_user_query_and_is_not_pin_to_top = query_and_index_is_some
            || InputModeSettings::as_ref(app)
                .input_mode
                .value()
                .is_inverted_blocklist();
        let should_render_separator =
            !FeatureFlag::AgentView.is_enabled() && contains_user_query_and_is_not_pin_to_top;
        if should_render_separator {
            content = content.with_border(Border::top(1.).with_border_fill(theme.outline()));
        }

        // Although `inputs_to_render` returns a vector, each AIBlock should only have one input.
        // We're assuming that the first element of the vector corresponds to the correct input.
        let renders_below_requested_command_view =
            self.model.inputs_to_render(app).iter().any(|input| {
                input.action_result().is_some_and(|result| {
                    result.result.is_requested_command() || result.result.is_call_mcp_tool()
                })
            });

        // We don't always apply top padding to every block because we don't want a block's top
        // padding to double up against its previous block's bottom padding. There are three exceptions:
        // 1) If a separator is present, we'll naturally want padding on both sides.
        // 2) If this block comes directly after a requested command, then the previous block will be
        //    a collapsible regular command block. Regardless of whether this command block is shown
        //    or hidden, this block will need to use top padding to create space.
        // 3) If the rendered element directly above this block is NOT an AI block,
        //    and this block is not a passive conversation, then it needs top padding to create visual separation.
        let terminal_model = self.terminal_model.lock();
        let is_previous_blocklist_item_ai_block = terminal_model
            .block_list()
            .get_previous_block_height_item(RemovableBlocklistItem::RichContent(self.view_id))
            .is_some_and(|item| {
                if let BlockHeightItem::RichContent(RichContentItem {
                    content_type: Some(content_type),
                    ..
                }) = item
                {
                    content_type == &RichContentType::AIBlock
                } else {
                    false
                }
            });
        let should_add_top_padding = contains_user_query_and_is_not_pin_to_top
            || renders_below_requested_command_view
            || (!is_previous_blocklist_item_ai_block && !self.is_passive_conversation(app));

        if should_add_top_padding {
            content = content.with_padding_top(CONTENT_VERTICAL_PADDING);
        }

        let semantic_selection = SemanticSelection::as_ref(app);
        let selected_text = self.selected_text.clone();
        let view_id = self.view_id;

        let mut selectable = SelectableArea::new(
            self.state_handles.selection_handle.clone(),
            move |selection_args, _, _| {
                *selected_text.write() = selection_args.selection;
            },
            SavePosition::new(content.finish(), self.saved_position_id().as_str()).finish(),
        )
        .with_word_boundaries_policy(semantic_selection.word_boundary_policy())
        .with_smart_select_fn(semantic_selection.smart_select_fn())
        .on_selection_right_click(move |ctx, position| {
            ctx.dispatch_typed_action(TerminalAction::BlockListContextMenu(
                BlockListMenuSource::RichContentTextRightClick {
                    rich_content_view_id: view_id,
                    position_in_rich_content: position,
                },
            ))
        })
        .on_selection_updated(|ctx, _| {
            ctx.dispatch_typed_action(AIBlockAction::SelectText);
        });

        if FeatureFlag::RectSelection.is_enabled() {
            selectable = selectable.should_support_rect_select();
        }

        // TODO(Simon): Bottom padding should be 24px on the final block when the input isn't visible.
        // It isn't sufficient to do an `is_streaming()` check because inline actions waiting for user
        // review (i.e. "OK if I run this command?") are technically completed blocks.
        selectable.finish()
    }

    fn on_focus(&mut self, focus_ctx: &warpui::FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focus_subview_if_necessary(ctx);
        }
    }

    fn keymap_context(&self, app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();

        if self
            .action_model
            .as_ref(app)
            .get_pending_action(app)
            .is_some_and(|action| self.requested_action_ids.contains(&action.id))
        {
            context.set.insert(HAS_PENDING_ACTION);
        }

        if self.has_pending_requested_edit(app) {
            context.set.insert(DISPATCHED_REQUESTED_EDIT_KEYMAP_CONTEXT);
        }

        context
    }
}

/// The horizontal padding applied to the AIBlock's content.
///
/// This should be applied to all content in the AIBlock, with the exception of the requested
/// command UX when it is in "expanded" state, when the requested command item expands to the full
/// terminal width as part of a UI that attempts to make the requested command UX in the
/// AIBlock appear visually connected to the actual shell block despite them existing in different
/// branches of the view hierarchy.
///
/// Each sub-component of the AI block (header, query, output) is responsible for implementing its
/// own padding and margin using these values.
pub(crate) const CONTENT_HORIZONTAL_PADDING: f32 = 20.;

/// The vertical padding applied to the AIBlock's content.
///
/// When there is an expanded requested command block, the padding from the bottom of the block is
/// removed; the UI attempts to make the requested command UX in the AIBlock appear visually
/// connected to the actual shell block despite them existing in different branches of the view
/// hierarchy.
///
/// Each sub-component of the AI block (header, query, output) is responsible for implementing its
/// own padding and margin using these values.
pub(crate) const CONTENT_VERTICAL_PADDING: f32 = 16.;

/// The space in between each "item" in the AI block, e.g. between header, query, and each output
/// "step".
///
/// Each sub-component of the AI block (header, query, output) is responsible for implementing its
/// own padding and margin using these values.
pub(crate) const CONTENT_ITEM_VERTICAL_MARGIN: f32 = 16.;

pub(crate) trait WithContentItemSpacing {
    /// Returns a [`Container`] with standard margin and padding values applied to be rendered as a
    /// "content item" in an AI block.
    ///
    /// The returned element is fit to be directly rendered within the AI block as a direct child
    /// of the top-level container.
    fn with_content_item_spacing(self) -> Container;

    /// Returns a [`Container`] "content item" spacing with additional left margin to be specifically
    /// applied to agent output items, intended to vertically align the left margin of agent output
    /// items (text, reasoning, actions) with the user query.
    fn with_agent_output_item_spacing(self, app: &AppContext) -> Container;
}

impl WithContentItemSpacing for Box<dyn Element> {
    fn with_content_item_spacing(self) -> Container {
        Container::new(self)
            .with_horizontal_margin(CONTENT_HORIZONTAL_PADDING)
            .with_margin_bottom(CONTENT_ITEM_VERTICAL_MARGIN)
    }

    fn with_agent_output_item_spacing(self, app: &AppContext) -> Container {
        let left_margin = CONTENT_HORIZONTAL_PADDING + icon_size(app) + 16.;
        Container::new(self)
            .with_margin_left(left_margin)
            .with_margin_right(CONTENT_HORIZONTAL_PADDING)
            .with_margin_bottom(CONTENT_ITEM_VERTICAL_MARGIN)
    }
}

impl AIAgentInput {
    /// Returns whether this [`AIAgentInput`] type should render a custom element below the user
    /// query.
    fn element_below_user_query(
        &self,
        props: input::Props,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        match self {
            AIAgentInput::CodeReview {
                review_comments, ..
            } => Some(address_comment_chips(
                &review_comments.review_comments(),
                props,
                app,
            )),
            AIAgentInput::UserQuery { .. }
            | AIAgentInput::AutoCodeDiffQuery { .. }
            | AIAgentInput::ResumeConversation { .. }
            | AIAgentInput::InitProjectRules { .. }
            | AIAgentInput::CreateEnvironment { .. }
            | AIAgentInput::TriggerPassiveSuggestion { .. }
            | AIAgentInput::CreateNewProject { .. }
            | AIAgentInput::CloneRepository { .. }
            | AIAgentInput::FetchReviewComments { .. }
            | AIAgentInput::SummarizeConversation { .. }
            | AIAgentInput::InvokeSkill { .. }
            | AIAgentInput::StartFromAmbientRunPrompt { .. }
            | AIAgentInput::ActionResult { .. }
            | AIAgentInput::MessagesReceivedFromAgents { .. }
            | AIAgentInput::EventsFromAgents { .. }
            | AIAgentInput::PassiveSuggestionResult { .. }
            | AIAgentInput::OrchestrationConfigUpdate { .. } => None,
        }
    }
}
