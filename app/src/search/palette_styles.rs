use warp_core::ui::appearance::Appearance;
use warpui::elements::{Border, CornerRadius, DropShadow, Radius, ScrollbarWidth};

use crate::search::result_renderer::QueryResultRendererStyles;

pub const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;

pub const SEARCH_BAR_PADDING_VERTICAL: f32 = 16.;

/// Horizontal padding between this element and the results.
pub const RESULT_PADDING_HORIZONTAL: f32 = 24.;
/// Vertical padding between this element and the results.
pub const RESULT_PADDING_VERTICAL: f32 = 4.;
pub const MULTILINE_RESULT_EXTRA_VERTICAL_PADDING: f32 = 2.;

/// Baseline total row height for command palette results.
///
/// Figma reference: "Palette Menu Item" (node-id=6241:68275) is 28px tall with 4px vertical
/// padding, leaving 20px for inner content.
/// https://www.figma.com/design/YjhPAtwuMsy6QnldxfL1DH/Open-files-in-Warp?node-id=6241-68275&m=dev
const COMMAND_PALETTE_BASE_ROW_HEIGHT: f32 = 28.;

pub const PALETTE_HEIGHT: f32 = 464.;
pub const PALETTE_WIDTH: f32 = 640.;

lazy_static::lazy_static! {
    pub static ref DROP_SHADOW: DropShadow = DropShadow::default();

    pub static ref QUERY_RESULT_RENDERER_STYLES: QueryResultRendererStyles =
        QueryResultRendererStyles {
            result_item_height_fn: |appearance| {
                let scaled_base_height =
                    COMMAND_PALETTE_BASE_ROW_HEIGHT * appearance.monospace_ui_scalar();

                // Make sure a single line of text at the user's line height doesn't get clipped.
                let min_for_text = (appearance.line_height_ratio() * appearance.monospace_font_size())
                    + (2.0 * RESULT_PADDING_VERTICAL);

                scaled_base_height.max(min_for_text)
            },
            panel_border_fn: panel_border,
            result_horizontal_padding: RESULT_PADDING_HORIZONTAL,
            result_vertical_padding: RESULT_PADDING_VERTICAL,
            result_multiline_vertical_padding:
                RESULT_PADDING_VERTICAL + MULTILINE_RESULT_EXTRA_VERTICAL_PADDING,
            // Figma "Palette Menu Item" applies an outer gutter so the highlight doesn't look full-bleed.
            result_outer_horizontal_padding_fn: |appearance| 4.0 * appearance.monospace_ui_scalar(),
            item_highlight_corner_radius: CornerRadius::with_all(Radius::Pixels(4.)),
            ..Default::default()
        };

}

/// Returns the `Border` for both the search results panel and details panel.
pub fn panel_border(appearance: &Appearance) -> Border {
    Border::all(1.).with_border_fill(appearance.theme().outline())
}
