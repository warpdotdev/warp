mod cell_glyph_cache;
mod cell_type;

use crate::terminal::grid_size_util::calculate_grid_baseline_position;
use crate::terminal::model::ansi::{Color, CursorShape, CursorStyle};
use crate::terminal::model::cell::{Cell, Flags};
use crate::terminal::{color, SizeInfo};

use crate::terminal::model::grid::Dimensions;
use crate::terminal::model::index::Point;
use crate::terminal::model::selection::SelectionPoint;
use crate::terminal::model::{ObfuscateSecrets, SecretHandle};

use crate::themes::theme::WarpTheme;
use crate::util::color::{ContrastingColor, MinimumAllowedContrast};

use core::mem;
use lazy_static::lazy_static;
use num_traits::Float as _;
use std::cmp::Ordering;
use std::ops::Range;
use std::{collections::HashMap, ops::RangeInclusive};
use unicode_width::UnicodeWidthChar;
use warp_core::features::FeatureFlag;
use warpui::assets::asset_cache::{AssetCache, AssetSource, AssetState};
use warpui::color::ColorU;
use warpui::elements::{Border, CornerRadius, Fill, Radius, DEFAULT_UI_LINE_HEIGHT_RATIO};
use warpui::fonts::{FamilyId, FontId, Properties, Style, Weight};
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{vec2f, Vector2F};
use warpui::image_cache::{AnimatedImageBehavior, CacheOption, FitType, Image, ImageCache};
use warpui::platform::LineStyle;
use warpui::text_layout::{Line, StyleAndFont, TextStyle, DEFAULT_TOP_BOTTOM_RATIO};
use warpui::units::{IntoLines as _, Lines, Pixels};
use warpui::{AppContext, Element, EntityId, PaintContext, Scene, SingletonEntity};

pub use self::cell_glyph_cache::CellGlyphCache;
use self::cell_type::{CellType, IsFocused, Secret};

use super::block_filter::{BLOCK_FILTER_DOTTED_LINE_DASH, BLOCK_FILTER_DOTTED_LINE_WIDTH};
use super::blockgrid_renderer::GridRenderParams;
use super::model::char_or_str::CharOrStr;
use super::model::grid::grid_handler::{ContainsPoint, GridHandler, Link};
use super::model::grid::RespectDisplayedOutput;
use super::model::image_map::{ImagePlacementData, StoredImageMetadata};
use super::model::terminal_model::RangeInModel;
use crate::settings::EnforceMinimumContrast;

// The scale factor of the cursor relative to the cursor width.
const CURSOR_THICKNESS_SCALE_FACTOR: f32 = 0.15;

// The scale factor of the underline relative to the glyph width.
const UNDERLINE_THICKNESS_SCALE_FACTOR: f32 = 0.15;

/// Diameter of the circle at the top of the selection cursor.
const SELECTION_CURSOR_TOP_DIAMETER: f32 = 5.;

/// Stores count of occurrences of distinct colors as a grid is rendered, which we can use to
/// compute the most common background color of a grid and color-match other UI elements against
/// it.
#[derive(Debug, Default)]
pub struct ColorSampler {
    counts: HashMap<ColorU, usize>,
    total_samples: usize,
}

impl ColorSampler {
    pub fn new() -> Self {
        Self {
            counts: HashMap::new(),
            total_samples: 0,
        }
    }

    pub fn sample(&mut self, color: ColorU) {
        self.total_samples += 1;
        // Sample every 8th color (cell).
        if !self.total_samples.is_multiple_of(8) {
            return;
        }

        let color = if color.is_fully_transparent() {
            // Sample all fully transparent colors as the same color even if they have differing
            // rgb values, cause that makes no difference in the rendered "color".
            ColorU::transparent_black()
        } else {
            color
        };

        *self.counts.entry(color).or_default() += 1;
    }

    pub fn most_common(&self) -> Option<ColorU> {
        self.counts
            .iter()
            .max_by_key(|(_, &count)| count)
            .map(|(&color, _)| color)
    }

    pub fn reset(&mut self) {
        self.counts.clear();
        self.total_samples = 0;
    }
}

lazy_static! {
    pub static ref MATCH_COLOR: ColorU = ColorU::new(255, 254, 61, 255);
    pub static ref URL_COLOR: ColorU = ColorU::new(103, 171, 250, 255);
    static ref BLOCK_FILTER_MATCH_COLOR: ColorU = ColorU::new(118, 167, 250, 255);
    pub static ref FOCUSED_MATCH_COLOR: ColorU = ColorU::new(238, 146, 59, 255);
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialOrd, PartialEq, variant_count::VariantCount)]
enum FontStyle {
    Default = 0,
    Bold,
    Italic,
    BoldItalic,
}

impl From<Flags> for FontStyle {
    fn from(flags: Flags) -> Self {
        match flags & Flags::BOLD_ITALIC {
            Flags::BOLD_ITALIC => Self::BoldItalic,
            Flags::ITALIC => Self::Italic,
            Flags::BOLD => Self::Bold,
            _ => Self::Default,
        }
    }
}

impl FontStyle {
    fn to_properties(self) -> Properties {
        match self {
            FontStyle::Default => Properties::default(),
            FontStyle::Bold => Properties::default().weight(Weight::Bold),
            FontStyle::Italic => Properties::default().style(Style::Italic),
            FontStyle::BoldItalic => Properties::default()
                .weight(Weight::Bold)
                .style(Style::Italic),
        }
    }
}

/// Holds the data for drawing a cell decoration, which can be an underline or strikethrough.
struct DecorationData {
    origin: Vector2F,
    size: Vector2F,
    color: ColorU,
}

/// Holds data necessary to draw a "native" glyph - a character that we render
/// via UI framework primitives instead of a glyph from a font.
struct NativeGlyph {
    cell_bounds: RectF,
    foreground_color: ColorU,
    glyph_type: NativeGlyphType,
}

/// Describes a specific type of glyph that we are able to render natively.
#[derive(Debug)]
enum NativeGlyphType {
    UpperHalfBlock,
    PowerlineLeftHardDivider,
    PowerlineRightHardDivider,
    BottomAlignedFractionalBlock {
        eighths: u8,
    },
    LeftAlignedFractionalBlock {
        eighths: u8,
    },
    RightHalfBlock,
    Shade {
        alpha: u8,
    },
    UpperOneEighthBlock,
    RightOneEighthBlock,
    /// The Quadrant unicode characters. ("Quadrant Lower Left", "Quadrant Lower Right", "Quadrant
    /// Upper Left", "Quadrant Upper Right").
    Quadrant {
        upper: bool,
        left: bool,
    },
    /// The "Quadrant Upper Left and Upper Right and Lower Left" unicode character. See
    /// https://www.compart.com/en/unicode/U+259B. (The ▛ character)
    QuadrantUpperLeftUpperRightLowerLeft,
    /// The "Quadrant Upper Left and Upper Right and Lower Right" unicode character. See
    /// https://www.compart.com/en/unicode/U+259C.
    QuadrantUpperLeftUpperRightLowerRight,
    /// The "Quadrant Upper Left and Lower Left and Lower Right" unicode character. See
    /// https://www.compart.com/en/unicode/U+2599.
    QuadrantUpperLeftLowerLeftLowerRight,
    /// The "Quadrant Upper Right and Lower Left and Lower Right" unicode character. See
    /// https://www.compart.com/en/unicode/U+259F.
    QuadrantUpperRightLowerLeftLowerRight,
    NFHalfCircleLeftThin,
    NFHalfCircleRightThin,
    /// Technically, the `NFHalfCircleLeft` should be a perfect
    /// semi-circle, but in our case we use the `NFHalfCircleLeftThick`,
    /// which is a semi-circle with a bit of a rectangle to extend it.
    NFHalfCircleLeft,
    NFHalfCircleLeftThick,
    NFHalfCircleRightThick,
    NFSlantTriangleTopLeft,
    NFSlantTriangleBottomLeft,
    NFSlantTriangleTopRight,
    NFSlantTriangleBottomRight,
}

#[derive(Clone, Copy)]
struct CachedBackgroundColor {
    start: SelectionPoint,
    end: SelectionPoint,
    background_color: ColorU,
}

impl CachedBackgroundColor {
    fn new(color: ColorU, start_col: usize, start_row: usize) -> Self {
        CachedBackgroundColor {
            background_color: color,
            start: SelectionPoint {
                row: start_row.into_lines(),
                col: start_col,
            },
            end: SelectionPoint {
                row: start_row.into_lines(),
                col: start_col,
            },
        }
    }

    fn from_selection_points(color: ColorU, start: &SelectionPoint, end: &SelectionPoint) -> Self {
        CachedBackgroundColor {
            background_color: color,
            start: *start,
            end: *end,
        }
    }

    fn with_end(&self, col: usize, row: usize) -> Self {
        Self {
            background_color: self.background_color,
            start: self.start,
            end: SelectionPoint {
                row: row.into_lines(),
                col,
            },
        }
    }

    // We are using 0.00001 as a threshold to get around floating point imprecision around selections
    // between command blocks and rich blocks. This threshold was experimentally determined.
    fn is_single_row_background(&self) -> bool {
        (self.end.row - self.start.row).abs().as_f64() < 0.00001
    }
}

/// Returns the match that applies to the given point and advances the iterator if necessary.
///
/// NOTE: `matches_iter` must be sorted in ascending order.
fn active_or_next_match<'a, I>(
    matches_iter: &mut I,
    active_match: Option<&'a RangeInclusive<Point>>,
    current_point: &Point,
) -> Option<&'a RangeInclusive<Point>>
where
    I: Iterator<Item = &'a RangeInclusive<Point>>,
{
    let mut current_match = active_match.or_else(|| matches_iter.next());
    while let Some(curr_match) = current_match {
        match curr_match.end().cmp(current_point) {
            Ordering::Greater | Ordering::Equal => {
                return current_match;
            }
            Ordering::Less => {
                current_match = matches_iter.next();
            }
        }
    }

    None
}

#[allow(clippy::too_many_arguments)]
pub fn render_grid<'a>(
    grid: &GridHandler,
    start_row: usize,
    end_row: usize,
    colors: &color::List,
    override_colors: &color::OverrideList,
    theme: &WarpTheme,
    default_font_properties: Properties,
    font_family: FamilyId,
    font_size: f32,
    line_height_ratio: f32,
    cell_size: Vector2F,
    padding_x: Pixels,
    grid_origin: Vector2F,
    glyphs: &mut CellGlyphCache,
    alpha: u8,
    highlighted_url: Option<&Link>, // Url highlighted on mouse hover.
    link_tool_tip: Option<&Link>,   // Link that has an opened tool-tip.
    ordered_matches: Option<impl Iterator<Item = &'a RangeInclusive<Point>>>,
    focused_match_range: Option<&RangeInclusive<Point>>,
    enforce_minimal_contrast: EnforceMinimumContrast,
    obfuscate_secrets: ObfuscateSecrets,
    hovered_secret: Option<SecretHandle>,
    use_ligature_rendering: bool,
    visible_cursor_shape: Option<CursorShape>,
    respect_displayed_output: RespectDisplayedOutput,
    image_metadata: &HashMap<u32, StoredImageMetadata>,
    bg_color_sampler: Option<&mut ColorSampler>,
    hide_cursor_cell: bool,
    ctx: &mut PaintContext,
    app: &AppContext,
) {
    // TODO: even if the ligatures setting is enabled, we should avoid using the ligature
    // codepath (which is known to be less performant) if the font does not
    // even support ligatures.
    let respect_displayed_output = matches!(respect_displayed_output, RespectDisplayedOutput::Yes);
    match (use_ligature_rendering, grid.displayed_output_rows()) {
        (true, Some(rows)) if respect_displayed_output => {
            let visible_rows = rows.skip(start_row).take(end_row.saturating_sub(start_row));
            render_grid_with_ligatures(
                grid,
                start_row,
                end_row,
                visible_rows,
                colors,
                override_colors,
                theme,
                default_font_properties,
                font_family,
                font_size,
                line_height_ratio,
                cell_size,
                padding_x,
                grid_origin,
                alpha,
                highlighted_url,
                link_tool_tip,
                ordered_matches,
                focused_match_range,
                enforce_minimal_contrast,
                obfuscate_secrets,
                hovered_secret,
                visible_cursor_shape,
                image_metadata,
                bg_color_sampler,
                hide_cursor_cell,
                ctx,
                app,
            );
        }
        (true, _) => {
            render_grid_with_ligatures(
                grid,
                start_row,
                end_row,
                start_row..end_row,
                colors,
                override_colors,
                theme,
                default_font_properties,
                font_family,
                font_size,
                line_height_ratio,
                cell_size,
                padding_x,
                grid_origin,
                alpha,
                highlighted_url,
                link_tool_tip,
                ordered_matches,
                focused_match_range,
                enforce_minimal_contrast,
                obfuscate_secrets,
                hovered_secret,
                visible_cursor_shape,
                image_metadata,
                bg_color_sampler,
                hide_cursor_cell,
                ctx,
                app,
            );
        }
        (false, Some(rows)) if respect_displayed_output => {
            let visible_rows = rows.skip(start_row).take(end_row.saturating_sub(start_row));
            render_grid_without_ligatures(
                grid,
                start_row,
                end_row,
                visible_rows,
                colors,
                override_colors,
                theme,
                default_font_properties,
                font_family,
                font_size,
                line_height_ratio,
                cell_size,
                padding_x,
                grid_origin,
                glyphs,
                alpha,
                highlighted_url,
                link_tool_tip,
                ordered_matches,
                focused_match_range,
                enforce_minimal_contrast,
                obfuscate_secrets,
                hovered_secret,
                visible_cursor_shape,
                image_metadata,
                bg_color_sampler,
                hide_cursor_cell,
                ctx,
                app,
            );
        }
        (false, _) => {
            render_grid_without_ligatures(
                grid,
                start_row,
                end_row,
                start_row..end_row,
                colors,
                override_colors,
                theme,
                default_font_properties,
                font_family,
                font_size,
                line_height_ratio,
                cell_size,
                padding_x,
                grid_origin,
                glyphs,
                alpha,
                highlighted_url,
                link_tool_tip,
                ordered_matches,
                focused_match_range,
                enforce_minimal_contrast,
                obfuscate_secrets,
                hovered_secret,
                visible_cursor_shape,
                image_metadata,
                bg_color_sampler,
                hide_cursor_cell,
                ctx,
                app,
            );
        }
    }
}

// TODO (kevin): Solidify highlighted_url and link_tool_tip into one single struct.
#[allow(clippy::too_many_arguments)]
fn render_grid_without_ligatures<'a>(
    grid: &GridHandler,
    start_row: usize,
    end_row: usize,
    visible_rows: impl Iterator<Item = usize>,
    colors: &color::List,
    override_colors: &color::OverrideList,
    theme: &WarpTheme,
    default_font_properties: Properties,
    font_family: FamilyId,
    font_size: f32,
    line_height_ratio: f32,
    cell_size: Vector2F,
    padding_x: Pixels,
    grid_origin: Vector2F,
    glyphs: &mut CellGlyphCache,
    alpha: u8,
    highlighted_url: Option<&Link>, // Url highlighted on mouse hover.
    link_tool_tip: Option<&Link>,   // Link that has an opened tool-tip.
    mut ordered_matches: Option<impl Iterator<Item = &'a RangeInclusive<Point>>>,
    focused_match_range: Option<&RangeInclusive<Point>>,
    enforce_minimal_contrast: EnforceMinimumContrast,
    obfuscate_secrets: ObfuscateSecrets,
    hovered_secret: Option<SecretHandle>,
    visible_cursor_shape: Option<CursorShape>,
    image_metadata: &HashMap<u32, StoredImageMetadata>,
    mut bg_color_sampler: Option<&mut ColorSampler>,
    hide_cursor_cell: bool,
    ctx: &mut PaintContext,
    app: &AppContext,
) {
    let font_cache = ctx.font_cache;
    let mut grid_origin = grid_origin + vec2f(padding_x.as_f32(), 0.);

    // Vertically align the grid to pixel boundaries to avoid rendering artifacts
    // that cause things to look misaligned.
    grid_origin.set_y(grid_origin.y().round());

    let default_font_id = font_cache.select_font(font_family, default_font_properties);

    // To avoid selecting the correct font on every cell render we use a cache that adds `FontId`s
    // to the cache only when needed.
    let mut font_id_cache = FontIdCache::new(default_font_id);

    let mut minimal_contrast_cache = MinimalContrastCache::new();

    let baseline_position = calculate_grid_baseline_position(
        ctx.font_cache,
        font_family,
        font_size,
        line_height_ratio,
        cell_size.y(),
    );

    let mut find_match = None;
    let mut focused_match_complete = false;

    let filter_matches = grid.filter_matches();
    let filter_matches_present =
        filter_matches.is_some_and(|filter_matches| !filter_matches.is_empty());
    let mut filter_matches_iter = filter_matches
        .iter()
        .flat_map(|match_vec| match_vec.iter().rev());
    let mut current_filter_match = None;

    let visible_url = highlighted_url
        .as_ref()
        .filter(|url| is_url_visible(url, start_row, end_row));
    let visible_tool_tip = link_tool_tip
        .as_ref()
        .filter(|url| is_url_visible(url, start_row, end_row));
    let mut has_seen_cell_in_link = false;

    // Collect cell decorations and native glyphs as we loop through the grid, but don't draw
    // them within the loop b/c they might get overlapped by background colors. Background colors
    // don't get drawn cell-by-cell as individual rects. They get "batched" together for efficiency.
    // The batch doesn't get drawn until the background color changes in a later iteration of the
    // loop. When it does, we go back and draw it, likely over cells in previous loop iterations.
    let mut cell_decorations = Vec::new();
    let mut native_glyphs_to_render = Vec::new();
    let mut cached_background_color: Option<CachedBackgroundColor> = None;

    let mut prev_row = None;
    let dotted_line_color = theme.surface_3().into_solid();

    let hovered_secret_range = hovered_secret
        .and_then(|handle| grid.secret_by_handle(handle).map(|secret| secret.range()));
    let marked_text = grid.marked_text();
    let mut marked_text = marked_text
        .iter()
        .flat_map(|marked_text| marked_text.chars());
    let mut next_marked_text_cell_is_wide_char_spacer = false;

    if FeatureFlag::ITermImages.is_enabled() {
        let image_ids = grid.get_image_ids_in_range(start_row, end_row);

        let (background_image_ids, foreground_image_ids): (Vec<_>, Vec<_>) = image_ids
            .into_iter()
            .partition(|image_placement| image_placement.z_index < 0);

        for image_placement in background_image_ids {
            if let Some((image_metadata, image_placement_data)) = image_metadata
                .get(&image_placement.image_id)
                .zip(grid.get_image_placement_data(
                    image_placement.image_id,
                    image_placement.placement_id,
                ))
            {
                let glyph_offset = cell_size
                    * vec2f(
                        image_placement.top_left.col as f32,
                        image_placement.top_left.row as f32,
                    );

                render_image(
                    image_placement.image_id,
                    image_metadata,
                    image_placement_data,
                    grid_origin,
                    glyph_offset,
                    ctx,
                    app,
                );
            }
        }

        if !foreground_image_ids.is_empty() {
            ctx.scene.start_layer(warpui::ClipBounds::ActiveLayer);
            for image_placement in foreground_image_ids {
                if let Some((image_metadata, image_placement_data)) = image_metadata
                    .get(&image_placement.image_id)
                    .zip(grid.get_image_placement_data(
                        image_placement.image_id,
                        image_placement.placement_id,
                    ))
                {
                    let glyph_offset = cell_size
                        * vec2f(
                            image_placement.top_left.col as f32,
                            image_placement.top_left.row as f32,
                        );

                    render_image(
                        image_placement.image_id,
                        image_metadata,
                        image_placement_data,
                        grid_origin,
                        glyph_offset,
                        ctx,
                        app,
                    );
                }
            }
            ctx.scene.stop_layer();
        }
    }

    for (offset, row_idx) in visible_rows.enumerate() {
        let offset_row = start_row + offset;

        let Some(row) = grid.row(row_idx) else {
            #[cfg(debug_assertions)]
            log::error!("grid_renderer should not try to render an out-of-bounds row");
            continue;
        };

        for col in 0..grid.columns() {
            let current_point = Point::new(row_idx, col);

            // Skip the cursor cell when CLI agent rich input is open
            // AND the agent draws its own cursor (SHOW_CURSOR is off).
            // When Warp draws the cursor (SHOW_CURSOR on), we keep the cell
            // and only suppress the draw_cursor call.
            if hide_cursor_cell
                && visible_cursor_shape.is_none()
                && current_point == grid.cursor_render_point()
            {
                continue;
            }

            // Determine if we need to override the cell to display the marked text.
            if current_point >= grid.cursor_point() {
                // Account for a wide char spacer cell if necessary.
                if next_marked_text_cell_is_wide_char_spacer {
                    next_marked_text_cell_is_wide_char_spacer = false;
                    continue;
                }
                // Render the next marked text character.
                if let Some(marked_text_character) = marked_text.next() {
                    let mut marked_text_cell = Cell::default();
                    marked_text_cell.c = marked_text_character;
                    marked_text_cell.flags = Flags::UNDERLINE;
                    let cell_type = CellType::marked_text_char();

                    if marked_text_character
                        .width()
                        .is_some_and(|width| width >= 2)
                    {
                        marked_text_cell.flags = marked_text_cell.flags.union(Flags::WIDE_CHAR);
                        // Mark the next cell as a wide char spacer.
                        next_marked_text_cell_is_wide_char_spacer = true;
                    }

                    let cursor_color = (grid.cursor_render_point() == Point::new(offset_row, col)
                        && visible_cursor_shape == Some(CursorShape::Block))
                    .then(|| theme.cursor().into_solid());
                    cached_background_color = render_cell(
                        grid,
                        offset_row,
                        col,
                        &marked_text_cell,
                        cell_type,
                        false,
                        FirstCellInSecret::default(),
                        &mut cell_decorations,
                        cursor_color,
                        colors,
                        override_colors,
                        cached_background_color,
                        font_family,
                        font_size,
                        &mut font_id_cache,
                        cell_size,
                        grid_origin,
                        glyphs,
                        alpha,
                        enforce_minimal_contrast,
                        &mut minimal_contrast_cache,
                        baseline_position,
                        &mut native_glyphs_to_render,
                        obfuscate_secrets,
                        bg_color_sampler.as_deref_mut(),
                        ctx,
                    );
                    continue;
                }
            }
            // Check if the current block match contains the point.
            let cell = &row[col];
            let mut cell_type = CellType::default();
            let mut first_cell_in_link = false;
            let mut first_cell_in_secret = FirstCellInSecret::No;

            find_match = ordered_matches
                .as_mut()
                .and_then(|iter| active_or_next_match(iter, find_match, &current_point));

            if !focused_match_complete
                && focused_match_range
                    .is_some_and(|focused_match_range| focused_match_range.contains(&current_point))
            {
                cell_type.is_find_match = Some(IsFocused::Yes);
                // Since there is only one focused match per terminal, mark the focused match as complete. This way,
                // we don't keep checking for the focused match as we continue iterating through the grid.
                focused_match_complete =
                    focused_match_range.is_some_and(|range| *range.end() == current_point);
            } else {
                let cell_is_match =
                    find_match.is_some_and(|find_match| find_match.contains(&current_point));
                cell_type.is_find_match = cell_is_match.then_some(IsFocused::No);
            }

            if filter_matches_present {
                let current_point = Point::new(row_idx, col);
                current_filter_match = active_or_next_match(
                    &mut filter_matches_iter,
                    current_filter_match,
                    &current_point,
                );
                if let Some(current_filter_match) = current_filter_match {
                    cell_type.is_filter_match = current_filter_match.contains(&current_point);
                }
            }

            // Skip rendering WIDE_CHAR_SPACER cells as they are placeholders for the CJK
            // characters. However, they should not be skipped if they would highlight a matched
            // term from the search bar. Note that we should only skip after the find logic to avoid
            // block_match not being marked as complete properly.
            if cell.flags().intersects(Flags::WIDE_CHAR_SPACER)
                && !cell_type.is_find_match()
                && !cell_type.is_filter_match()
            {
                continue;
            }

            // Cell is empty, we should flush the background if there's any and continue
            if cell.is_empty() {
                if let Some(sampler) = bg_color_sampler.as_deref_mut() {
                    sampler.sample(
                        cached_background_color
                            .as_ref()
                            .map(|cached_color| cached_color.background_color)
                            .unwrap_or_else(ColorU::transparent_black),
                    );
                }

                if let Some(cached) = cached_background_color.take() {
                    render_background(
                        grid_origin,
                        cached,
                        cell_size,
                        grid.columns().saturating_sub(1),
                        ctx,
                    );
                }
                continue;
            }

            if visible_url.is_some_and(|link| link.contains(Point::new(offset_row, col))) {
                cell_type.is_url = true;
            };

            if visible_tool_tip.is_some_and(|link| link.contains(Point::new(offset_row, col))) {
                cell_type.is_url = true;
                first_cell_in_link = !has_seen_cell_in_link;
                has_seen_cell_in_link = true;
            }

            if obfuscate_secrets.should_redact_secret() {
                if let Some((handle, secret)) =
                    grid.secret_at_displayed_point(Point::new(offset_row, col))
                {
                    let range = secret.range();
                    if row_idx == range.start().row && col == range.start().col {
                        first_cell_in_secret = FirstCellInSecret::Yes { handle };
                    }
                    cell_type.secret = Some(Secret {
                        hovered: hovered_secret_range
                            .as_ref()
                            .is_some_and(|r| r.contains(&Point::new(row_idx, col))),
                        is_obfuscated: secret.is_obfuscated(),
                    });
                }
            }

            // Don't apply cursor contrast colouring when hide_cursor_cell
            // is active — the cursor itself won't be drawn, so the cell
            // should render with its normal colours.
            let cursor_color = (!hide_cursor_cell
                && grid.cursor_point() == Point::new(offset_row, col)
                && visible_cursor_shape == Some(CursorShape::Block))
            .then(|| theme.cursor().into_solid());
            cached_background_color = render_cell(
                grid,
                offset_row,
                col,
                cell,
                cell_type,
                first_cell_in_link,
                first_cell_in_secret,
                &mut cell_decorations,
                cursor_color,
                colors,
                override_colors,
                cached_background_color,
                font_family,
                font_size,
                &mut font_id_cache,
                cell_size,
                grid_origin,
                glyphs,
                alpha,
                enforce_minimal_contrast,
                &mut minimal_contrast_cache,
                baseline_position,
                &mut native_glyphs_to_render,
                obfuscate_secrets,
                bg_color_sampler.as_deref_mut(),
                ctx,
            );
        }

        if grid.filter_has_context_lines() {
            maybe_render_dotted_lines(
                grid,
                row_idx,
                prev_row,
                offset_row,
                DottedLineInfo {
                    cell_size,
                    offset_row,
                    grid_origin,
                    dotted_line_color,
                },
                ctx,
            );
        }
        prev_row = Some(row_idx);
    }

    // Draw rects for a merged background region, if appropriate.
    if let Some(cached) = cached_background_color.take() {
        render_background(
            grid_origin,
            cached,
            cell_size,
            grid.columns().saturating_sub(1),
            ctx,
        );
    }

    // After the merged background is drawn, go back and draw the native glyphs
    // and cell decorations to ensure that they sit on top of the merged background
    // rects.  (This is not needed for glyphs, as the rendering stack currently always
    // renders glyphs above rects within a given layer.)
    for native_glyph in native_glyphs_to_render {
        render_native_glyph(native_glyph, ctx, app);
    }
    for data in cell_decorations {
        ctx.scene
            .draw_rect_without_hit_recording(RectF::new(data.origin, data.size))
            .with_background(Fill::Solid(data.color));
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn render_cell(
    grid: &GridHandler,
    offset_row: usize,
    col: usize,
    cell: &Cell,
    cell_type: CellType,
    first_cell_in_link: bool,
    first_cell_in_secret: FirstCellInSecret,
    cell_decorations: &mut Vec<DecorationData>,
    cursor_color: Option<ColorU>,
    colors: &color::List,
    override_colors: &color::OverrideList,
    mut cached_background_color: Option<CachedBackgroundColor>,
    font_family: FamilyId,
    font_size: f32,
    font_id_cache: &mut FontIdCache,
    cell_size: Vector2F,
    grid_origin: Vector2F,
    glyphs: &mut CellGlyphCache,
    alpha: u8,
    enforce_minimal_contrast: EnforceMinimumContrast,
    minimal_contrast_cache: &mut MinimalContrastCache,
    baseline_position: Vector2F,
    native_glyphs_to_render: &mut Vec<NativeGlyph>,
    obfuscate_mode: ObfuscateSecrets,
    bg_color_sampler: Option<&mut ColorSampler>,
    ctx: &mut PaintContext,
) -> Option<CachedBackgroundColor> {
    let cell_colors = cell_colors(
        cell,
        colors,
        override_colors,
        &cell_type,
        alpha,
        enforce_minimal_contrast,
        minimal_contrast_cache,
        cursor_color,
        obfuscate_mode,
    );
    if let Some(sampler) = bg_color_sampler {
        sampler.sample(cell_colors.background_color);
    }
    cached_background_color = maybe_draw_background(
        cached_background_color,
        grid_origin,
        cell_size,
        col,
        offset_row,
        grid.columns() - 1,
        cell_colors.background_color,
        ctx,
    );

    let glyph_offset = cell_size * vec2f(col as f32, offset_row as f32);

    render_cell_glyph(
        cell,
        &cell_type,
        first_cell_in_link,
        first_cell_in_secret,
        font_id_cache,
        font_family,
        font_size,
        cell_size,
        grid_origin,
        glyph_offset,
        glyphs,
        baseline_position,
        cell_colors.foreground_color,
        native_glyphs_to_render,
        obfuscate_mode,
        ctx,
    );
    cell_decorations.extend(calculate_cell_decorations(
        cell,
        &cell_type,
        cell_size,
        grid_origin,
        glyph_offset,
        cell_colors.foreground_color,
        obfuscate_mode,
    ));
    // We move this in and then return it to avoid an extra copy in `maybe_draw_background`.
    cached_background_color
}

#[allow(clippy::too_many_arguments)]
fn render_grid_with_ligatures<'a>(
    grid: &GridHandler,
    start_row: usize,
    end_row: usize,
    visible_rows: impl Iterator<Item = usize>,
    colors: &color::List,
    override_colors: &color::OverrideList,
    theme: &WarpTheme,
    default_font_properties: Properties,
    font_family: FamilyId,
    font_size: f32,
    line_height_ratio: f32,
    cell_size: Vector2F,
    padding_x: Pixels,
    grid_origin: Vector2F,
    alpha: u8,
    highlighted_url: Option<&Link>, // Url highlighted on mouse hover.
    link_tool_tip: Option<&Link>,   // Link that has an opened tool-tip.
    mut ordered_matches: Option<impl Iterator<Item = &'a RangeInclusive<Point>>>,
    focused_match_range: Option<&RangeInclusive<Point>>,
    enforce_minimal_contrast: EnforceMinimumContrast,
    obfuscate_secrets: ObfuscateSecrets,
    hovered_secret: Option<SecretHandle>,
    visible_cursor_shape: Option<CursorShape>,
    image_metadata: &HashMap<u32, StoredImageMetadata>,
    mut bg_color_sampler: Option<&mut ColorSampler>,
    hide_cursor_cell: bool,
    ctx: &mut PaintContext,
    app: &AppContext,
) {
    let mut grid_origin = grid_origin + vec2f(padding_x.as_f32(), 0.);
    grid_origin.set_y(grid_origin.y().round());

    let default_font_id = ctx
        .font_cache
        .select_font(font_family, default_font_properties);
    let mut font_id_cache = HashMap::new();
    font_id_cache.insert(FontStyle::Default, default_font_id);

    let mut minimal_contrast_cache = MinimalContrastCache::new();

    let baseline_position = calculate_grid_baseline_position(
        ctx.font_cache,
        font_family,
        font_size,
        line_height_ratio,
        cell_size.y(),
    );

    let mut find_match = None;
    let mut focused_match_complete = false;

    let filter_matches = grid.filter_matches();
    let filter_matches_present =
        filter_matches.is_some_and(|filter_matches| !filter_matches.is_empty());
    let mut filter_matches_iter = filter_matches
        .iter()
        .flat_map(|match_vec| match_vec.iter().rev());
    let mut current_filter_match = None;

    let visible_url = highlighted_url
        .as_ref()
        .filter(|url| is_url_visible(url, start_row, end_row));
    let visible_tool_tip = link_tool_tip
        .as_ref()
        .filter(|url| is_url_visible(url, start_row, end_row));
    let mut has_seen_cell_in_link = false;

    // Collect cell decorations and native glyphs as we loop through the grid, but don't draw
    // them within the loop b/c they might get overlapped by background colors. Background colors
    // don't get drawn cell-by-cell as individual rects. They get "batched" together for efficiency.
    // The batch doesn't get drawn until the background color changes in a later iteration of the
    // loop. When it does, we go back and draw it, likely over cells in previous loop iterations.
    let mut cell_decorations = Vec::new();
    let mut native_glyphs_to_render = Vec::new();
    let mut cached_background_color: Option<CachedBackgroundColor> = None;

    let mut prev_row = None;
    let dotted_line_color = theme.surface_3().into_solid();

    let hovered_secret_range = hovered_secret
        .and_then(|handle| grid.secret_by_handle(handle).map(|secret| secret.range()));
    let marked_text = grid.marked_text();
    let mut marked_text = marked_text
        .iter()
        .flat_map(|marked_text| marked_text.chars())
        .peekable();
    let mut next_marked_text_cell_is_wide_char_spacer = false;
    if FeatureFlag::ITermImages.is_enabled() {
        let image_ids = grid.get_image_ids_in_range(start_row, end_row);

        let (background_image_ids, foreground_image_ids): (Vec<_>, Vec<_>) = image_ids
            .into_iter()
            .partition(|image_placement| image_placement.z_index < 0);

        for image_placement in background_image_ids {
            if let Some((image_metadata, image_placement_data)) = image_metadata
                .get(&image_placement.image_id)
                .zip(grid.get_image_placement_data(
                    image_placement.image_id,
                    image_placement.placement_id,
                ))
            {
                let glyph_offset = cell_size
                    * vec2f(
                        image_placement.top_left.col as f32,
                        image_placement.top_left.row as f32,
                    );

                render_image(
                    image_placement.image_id,
                    image_metadata,
                    image_placement_data,
                    grid_origin,
                    glyph_offset,
                    ctx,
                    app,
                );
            }
        }

        if !foreground_image_ids.is_empty() {
            ctx.scene.start_layer(warpui::ClipBounds::ActiveLayer);
            for image_placement in foreground_image_ids {
                if let Some((image_metadata, image_placement_data)) = image_metadata
                    .get(&image_placement.image_id)
                    .zip(grid.get_image_placement_data(
                        image_placement.image_id,
                        image_placement.placement_id,
                    ))
                {
                    let glyph_offset = cell_size
                        * vec2f(
                            image_placement.top_left.col as f32,
                            image_placement.top_left.row as f32,
                        );

                    render_image(
                        image_placement.image_id,
                        image_metadata,
                        image_placement_data,
                        grid_origin,
                        glyph_offset,
                        ctx,
                        app,
                    );
                }
            }
            ctx.scene.stop_layer();
        }
    }

    for (offset, row_idx) in visible_rows.enumerate() {
        let offset_row = start_row + offset;
        let mut string_builder =
            AttributedStringBuilder::new(font_family, font_family, grid.columns());

        let Some(row) = grid.row(row_idx) else {
            log::error!("grid_renderer should not try to render an out-of-bounds row");
            continue;
        };

        // Elide any empty cells at the end of the row, they don't need to be included in the text
        // layout
        let last_cell = match row[..]
            .iter()
            .enumerate()
            .rev()
            .find(|(_, cell)| !cell.is_empty())
        {
            Some((cell_index, _)) => cell_index,
            None => {
                // If there are no non-empty cells in the entire row, we can skip it entirely
                if marked_text.peek().is_none() {
                    if let Some(sampler) = bg_color_sampler.as_deref_mut() {
                        // Empty ligature-rendered rows still represent the terminal's default
                        // background. Keep them in the sampler so transient colored rows, such as
                        // tmux's status line during startup, do not dominate the inferred
                        // background used by surrounding UI.
                        for _ in 0..grid.columns() {
                            sampler.sample(ColorU::transparent_black());
                        }
                    }
                    continue;
                }
                grid.columns() - 1
            }
        };

        for col in 0..grid.columns() {
            let cell = &row[col];
            let mut cell_type = CellType::default();
            let mut first_cell_in_link = false;
            let mut first_cell_in_secret = FirstCellInSecret::No;

            let current_point = Point::new(row_idx, col);

            // Skip the cursor cell when CLI agent rich input is open
            // AND the agent draws its own cursor (SHOW_CURSOR is off).
            // When Warp draws the cursor (SHOW_CURSOR on), we keep the cell
            // and only suppress the draw_cursor call.
            if hide_cursor_cell
                && visible_cursor_shape.is_none()
                && current_point == grid.cursor_render_point()
            {
                continue;
            }

            // Determine if we need to override the cell to display the marked text.
            if current_point >= grid.cursor_point() {
                // Account for a wide char spacer cell if necessary.
                if next_marked_text_cell_is_wide_char_spacer {
                    next_marked_text_cell_is_wide_char_spacer = false;
                    continue;
                }

                // Render the next marked text character.
                if let Some(marked_text_character) = marked_text.next() {
                    let mut marked_text_cell = Cell::default();
                    marked_text_cell.c = marked_text_character;
                    marked_text_cell.flags = Flags::UNDERLINE;
                    let cell_type = CellType::marked_text_char();

                    let cursor_color = (grid.cursor_render_point() == Point::new(offset_row, col)
                        && visible_cursor_shape == Some(CursorShape::Block))
                    .then(|| theme.cursor().into_solid());

                    let cell_colors = cell_colors(
                        &marked_text_cell,
                        colors,
                        override_colors,
                        &cell_type,
                        alpha,
                        enforce_minimal_contrast,
                        &mut minimal_contrast_cache,
                        cursor_color,
                        obfuscate_secrets,
                    );

                    cached_background_color = maybe_draw_background(
                        cached_background_color,
                        grid_origin,
                        cell_size,
                        col,
                        offset_row,
                        grid.columns() - 1,
                        cell_colors.background_color,
                        ctx,
                    );

                    if marked_text_character
                        .width()
                        .is_some_and(|width| width >= 2)
                    {
                        marked_text_cell.flags = marked_text_cell.flags.union(Flags::WIDE_CHAR);
                        // Mark the next cell as a wide char spacer.
                        next_marked_text_cell_is_wide_char_spacer = true;
                    }

                    let glyph_offset = cell_size * vec2f(col as f32, offset_row as f32);
                    cell_decorations.extend(calculate_cell_decorations(
                        &marked_text_cell,
                        &cell_type,
                        cell_size,
                        grid_origin,
                        glyph_offset,
                        cell_colors.foreground_color,
                        obfuscate_secrets,
                    ));

                    string_builder
                        .update_style(marked_text_cell.flags.into(), cell_colors.foreground_color);
                    string_builder.append_character(marked_text_character, col);

                    continue;
                }
            }

            find_match = ordered_matches
                .as_mut()
                .and_then(|iter| active_or_next_match(iter, find_match, &current_point));

            if !focused_match_complete
                && focused_match_range
                    .is_some_and(|focused_match_range| focused_match_range.contains(&current_point))
            {
                cell_type.is_find_match = Some(IsFocused::Yes);
                // Since there is only one focused match per terminal, mark the focused match as complete. This way,
                // we don't keep checking for the focused match as we continue iterating through the grid.
                focused_match_complete =
                    focused_match_range.is_some_and(|range| *range.end() == current_point);
            } else {
                let cell_is_match =
                    find_match.is_some_and(|find_match| find_match.contains(&current_point));
                cell_type.is_find_match = cell_is_match.then_some(IsFocused::No);
            }

            if filter_matches_present {
                let current_point = Point::new(row_idx, col);
                current_filter_match = active_or_next_match(
                    &mut filter_matches_iter,
                    current_filter_match,
                    &current_point,
                );
                if let Some(current_filter_match) = current_filter_match {
                    cell_type.is_filter_match = current_filter_match.contains(&current_point);
                }
            }

            if cell.flags().intersects(Flags::WIDE_CHAR_SPACER)
                && !cell_type.is_find_match()
                && !cell_type.is_filter_match()
            {
                continue;
            }

            if cell.is_empty() {
                if let Some(sampler) = bg_color_sampler.as_deref_mut() {
                    // If the cell is empty, that means the bg is is `NamedBackground.`
                    //
                    // `NamedBackground` is rendered with 0 alpha (transparent), as the terminal
                    // background itself is rendered by the terminal view, not as part of grid
                    // rendering.
                    sampler.sample(ColorU::transparent_black());
                }

                if let Some(cached) = cached_background_color.take() {
                    render_background(
                        grid_origin,
                        cached,
                        cell_size,
                        grid.columns().saturating_sub(1),
                        ctx,
                    );
                }

                if col > last_cell {
                    continue;
                }
            }

            if visible_url.is_some_and(|link| link.contains(Point::new(offset_row, col))) {
                cell_type.is_url = true;
            };

            if visible_tool_tip.is_some_and(|link| link.contains(Point::new(offset_row, col))) {
                cell_type.is_url = true;
                first_cell_in_link = !has_seen_cell_in_link;
                has_seen_cell_in_link = true;
            }

            if obfuscate_secrets.should_redact_secret() {
                if let Some((handle, secret)) =
                    grid.secret_at_displayed_point(Point::new(offset_row, col))
                {
                    let range = secret.range();
                    if row_idx == range.start().row && col == range.start().col {
                        first_cell_in_secret = FirstCellInSecret::Yes { handle };
                    }
                    cell_type.secret = Some(Secret {
                        hovered: hovered_secret_range
                            .as_ref()
                            .is_some_and(|r| r.contains(&Point::new(row_idx, col))),
                        is_obfuscated: secret.is_obfuscated(),
                    });
                }
            }

            // Don't apply cursor contrast colouring when hide_cursor_cell
            // is active — the cursor itself won't be drawn, so the cell
            // should render with its normal colours.
            let cursor_color = (!hide_cursor_cell
                && grid.cursor_point() == Point::new(offset_row, col)
                && visible_cursor_shape == Some(CursorShape::Block))
            .then(|| theme.cursor().into_solid());
            let cell_colors = cell_colors(
                cell,
                colors,
                override_colors,
                &cell_type,
                alpha,
                enforce_minimal_contrast,
                &mut minimal_contrast_cache,
                cursor_color,
                obfuscate_secrets,
            );
            if let Some(sampler) = bg_color_sampler.as_mut() {
                sampler.sample(cell_colors.background_color);
            }
            cached_background_color = maybe_draw_background(
                cached_background_color,
                grid_origin,
                cell_size,
                col,
                offset_row,
                grid.columns() - 1,
                cell_colors.background_color,
                ctx,
            );

            let glyph_offset = cell_size * vec2f(col as f32, offset_row as f32);
            if first_cell_in_link {
                // We want this to be a bounding box to be around the cell, so we don't include baseline_position in the origin.
                let cell_origin = grid_origin + glyph_offset;
                ctx.position_cache.cache_position_indefinitely(
                    "terminal_view:first_cell_in_link".to_owned(),
                    RectF::new(cell_origin, cell_size),
                );
            }
            cell_decorations.extend(calculate_cell_decorations(
                cell,
                &cell_type,
                cell_size,
                grid_origin,
                glyph_offset,
                cell_colors.foreground_color,
                obfuscate_secrets,
            ));

            string_builder.update_style(
                if cell_type.is_filter_match() {
                    let mut flags = cell.flags;
                    flags.insert(Flags::BOLD);
                    flags.into()
                } else {
                    cell.flags.into()
                },
                cell_colors.foreground_color,
            );

            if !cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                let actual_cell_size = if cell.flags.intersects(Flags::WIDE_CHAR) {
                    cell_size * vec2f(2., 1.)
                } else {
                    cell_size
                };
                if let Some(secret_content) = handle_secret_redaction(
                    cell,
                    &cell_type,
                    first_cell_in_secret,
                    actual_cell_size,
                    grid_origin,
                    glyph_offset,
                    obfuscate_secrets,
                    ctx,
                ) {
                    string_builder.append_content(secret_content, col);
                } else if let Some(glyph_type) = native_glyph_for_cell(cell) {
                    native_glyphs_to_render.push(NativeGlyph {
                        cell_bounds: RectF::new(grid_origin + glyph_offset, actual_cell_size),
                        foreground_color: cell_colors.foreground_color,
                        glyph_type,
                    });
                    string_builder.append_placeholder(col);
                } else if cell.c == '\t' {
                    // For a tab, the grid has already taken into account the extra spaces needed
                    // to properly align the contents. We don't want the text layout engine to
                    // attempt the same thing, so we replace it with a placeholder
                    string_builder.append_placeholder(col);
                } else {
                    string_builder.append_content(cell.content_for_display(), col);
                }
            }
        }

        let string_data = string_builder.build();

        let laid_out = ctx.text_layout_cache.layout_line(
            &string_data.line,
            LineStyle {
                font_size,
                line_height_ratio,
                baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                fixed_width_tab_size: None,
            },
            &string_data.style_runs,
            cell_size.x() * grid.columns() as f32,
            Default::default(),
            &ctx.font_cache.text_layout_system(),
        );
        let line_origin =
            grid_origin + vec2f(0., cell_size.y() * offset_row as f32) + baseline_position;
        paint_line(
            laid_out.as_ref(),
            line_origin,
            cell_size.x(),
            &string_data.character_index_to_cell_map,
            ctx.scene,
        );

        if grid.filter_has_context_lines() {
            maybe_render_dotted_lines(
                grid,
                row_idx,
                prev_row,
                offset_row,
                DottedLineInfo {
                    cell_size,
                    offset_row,
                    grid_origin,
                    dotted_line_color,
                },
                ctx,
            );
        }
        prev_row = Some(row_idx);
    }

    if let Some(cached) = cached_background_color.take() {
        render_background(
            grid_origin,
            cached,
            cell_size,
            grid.columns().saturating_sub(1),
            ctx,
        );
    }

    for native_glyph in native_glyphs_to_render {
        render_native_glyph(native_glyph, ctx, app);
    }
    for data in cell_decorations {
        ctx.scene
            .draw_rect_without_hit_recording(RectF::new(data.origin, data.size))
            .with_background(Fill::Solid(data.color));
    }
}

fn paint_line(
    line: &Line,
    baseline: Vector2F,
    cell_width: f32,
    character_index_to_cell_map: &[usize],
    scene: &mut Scene,
) {
    for run in &line.runs {
        let glyph_color = run.styles.foreground_color.unwrap_or_default();

        for glyph in &run.glyphs {
            let glyph_x = character_index_to_cell_map[glyph.index] as f32 * cell_width;
            let glyph_origin = baseline + vec2f(glyph_x, 0.);

            scene.draw_glyph(
                glyph_origin,
                glyph.id,
                run.font_id,
                line.font_size,
                glyph_color,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn maybe_draw_background(
    cached_background_color: Option<CachedBackgroundColor>,
    grid_origin: Vector2F,
    cell_size: Vector2F,
    col: usize,
    row: usize,
    max_columns: usize,
    background_color: ColorU,
    ctx: &mut PaintContext,
) -> Option<CachedBackgroundColor> {
    if let Some(cached) = cached_background_color {
        if background_color == cached.background_color {
            return Some(cached.with_end(col, row));
        }
        render_background(grid_origin, cached, cell_size, max_columns, ctx);
    }
    // Either there was no background before or the backgrounds are now different so we want to
    // return the appropriate value of the new background start to cache.
    if background_color.is_fully_transparent() {
        return None;
    }
    Some(CachedBackgroundColor::new(background_color, col, row))
}

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
struct CellColors {
    background_color: ColorU,
    foreground_color: ColorU,
}

struct MinimalContrastCache {
    /// A mapping from a pair of cell colors to a foreground color that has the
    /// required minimum contrast against the provided background.
    map: HashMap<CellColors, ColorU>,

    /// The last set of cell colors that were queried, and the value they map
    /// to.
    ///
    /// The majority of the time, a cell will have the same fg and bg colors as
    /// the cell before it.  By storing this data outside of the map, we can
    /// avoid a significant number of HashMap lookups (which, while constant
    /// time, have a non-zero cost).
    last: (CellColors, ColorU),
}

impl MinimalContrastCache {
    fn new() -> Self {
        // Default the last colors to a map from transparent/transparent to
        // transparent.  This is a reasonable default because it's not likely
        // to actually occur, and if it does, this is the correct mapping.
        let transparent = CellColors {
            foreground_color: ColorU::transparent_black(),
            background_color: ColorU::transparent_black(),
        };
        Self {
            map: Default::default(),
            last: (transparent, ColorU::transparent_black()),
        }
    }

    fn get_or_insert(&mut self, colors: CellColors) -> ColorU {
        if colors == self.last.0 {
            self.last.1
        } else {
            use std::collections::hash_map::Entry;
            let fg_color = match self.map.entry(colors) {
                Entry::Occupied(entry) => *entry.get(),
                Entry::Vacant(entry) => {
                    let fg_color = entry
                        .key()
                        .foreground_color
                        .on_background(entry.key().background_color, MinimumAllowedContrast::Text);
                    entry.insert(fg_color);
                    fg_color
                }
            };
            self.last = (colors, fg_color);
            fg_color
        }
    }
}

/// Compute the [`CellColors`] for this cell.
///
/// The `cursor_color` will be Some only if the cursor is on this cell.
/// Note: this method assumes that the cell is not empty. Check for the emptiness outside.
#[allow(clippy::too_many_arguments)]
fn cell_colors(
    cell: &Cell,
    colors: &color::List,
    override_colors: &color::OverrideList,
    cell_type: &CellType,
    alpha: u8,
    enforce_minimal_contrast: EnforceMinimumContrast,
    minimal_contrast_cache: &mut MinimalContrastCache,
    cursor_color: Option<ColorU>,
    obfuscate_mode: ObfuscateSecrets,
) -> CellColors {
    // If the cell part of a find match, then we render black text so that it is visible over the yellow highlight.
    let mut foreground_color =
        cell_type.foreground_color(cell, colors, override_colors, obfuscate_mode);
    let mut background_color = cell_type.background_color(cell, colors, override_colors);

    if cell.flags.contains(Flags::INVERSE) {
        mem::swap(&mut foreground_color, &mut background_color);
    }

    foreground_color.a = alpha;

    if enforce_minimal_contrast == EnforceMinimumContrast::Always
        || (enforce_minimal_contrast == EnforceMinimumContrast::OnlyNamedColors
            && matches!(cell.fg, Color::Named(_)))
        || (enforce_minimal_contrast != EnforceMinimumContrast::Never && cursor_color.is_some())
    {
        // The cursor, if it's on this cell, is the "real" background since it is on top of the
        // cell's bg color but behind the fg glyph.
        let effective_background_color = cursor_color.unwrap_or(background_color);
        foreground_color = minimal_contrast_cache.get_or_insert(CellColors {
            foreground_color,
            background_color: effective_background_color,
        });
    }

    CellColors {
        background_color,
        foreground_color,
    }
}

struct FontIdCache {
    /// An array mapping a [`FontStyle`] enum variant to the [`FontId`] we've
    /// selected for it, if we've already made a selection.
    ///
    /// This was originally a [`HashMap`], and was changed for performance.
    /// While a `HashMap` also has O(1) constant-time lookups, the constant
    /// factor is substantially larger than a simple array index (hashing has
    /// a real cost), and we're doing this lookup for every cell on every
    /// render.
    font_ids: [Option<FontId>; FontStyle::VARIANT_COUNT],
}

impl FontIdCache {
    fn new(default_font_id: FontId) -> Self {
        let mut cache = Self {
            font_ids: [None; FontStyle::VARIANT_COUNT],
        };
        cache.font_ids[FontStyle::Default as usize] = Some(default_font_id);
        cache
    }
}

/// Draw the glyph for the cell here, but don't draw the decorations (underlines and strikethroughs)
/// yet.
#[allow(clippy::too_many_arguments)]
fn render_cell_glyph(
    cell: &Cell,
    cell_type: &CellType,
    first_cell_in_link: bool,
    first_cell_in_secret: FirstCellInSecret,
    font_id_cache: &mut FontIdCache,
    font_family: FamilyId,
    font_size: f32,
    cell_size: Vector2F,
    grid_origin: Vector2F,
    glyph_offset: Vector2F,
    glyphs: &mut CellGlyphCache,
    baseline_position: Vector2F,
    foreground_color: ColorU,
    native_glyphs_to_render: &mut Vec<NativeGlyph>,
    obfuscate_mode: ObfuscateSecrets,
    ctx: &mut PaintContext,
) {
    let cell_size = if cell.flags().intersects(Flags::WIDE_CHAR) {
        // WIDE_CHAR takes up two cells.
        Vector2F::new(cell_size.x() * 2., cell_size.y())
    } else {
        cell_size
    };

    let mut cell_content = cell.content_for_display();

    if let Some(secret_content) = handle_secret_redaction(
        cell,
        cell_type,
        first_cell_in_secret,
        cell_size,
        grid_origin,
        glyph_offset,
        obfuscate_mode,
        ctx,
    ) {
        cell_content = secret_content;
    }

    let flags = if cell_type.is_filter_match() {
        let mut cell_flags_copy = cell.flags;
        cell_flags_copy.insert(Flags::BOLD);
        cell_flags_copy
    } else {
        cell.flags
    };

    let font_style: FontStyle = flags.into();
    let properties = font_style.to_properties();

    let font_id = font_id_cache.font_ids[font_style as usize]
        .get_or_insert_with(|| ctx.font_cache.select_font(font_family, properties));

    let glyph_and_font = match cell_content {
        // Special-case whitespace, which doesn't need rendering.  We
        // explicitly check these two chars instead of using
        // `char::is_whitespace` for performance reasons.
        CharOrStr::Char(' ' | '\t') => None,
        CharOrStr::Char(char) => glyphs.glyph_for_char(char, *font_id, ctx.font_cache),
        // Certain zerowidth characters, such as emoji presentation selectors, can affect the underlying glyph and
        // change the rendering. Hence, we need to layout/render the text as a combined string, instead of simply
        // the single character. For example, in \0x2601\0xFE0F, the FE0F selector causes ☁️ to be changed from a
        // 1-width char to a 2-width char.
        CharOrStr::Str(content_with_zerowidth) => glyphs.glyph_for_string(
            content_with_zerowidth,
            *font_id,
            ctx.font_cache,
            font_family,
            font_size,
            properties,
            ctx,
        ),
    };

    let origin = grid_origin + glyph_offset + baseline_position;

    // Handle special unicode characters that will look better with native
    // rendering instead of using glyphs from the font.
    match native_glyph_for_cell(cell) {
        Some(glyph_type) => {
            native_glyphs_to_render.push(NativeGlyph {
                cell_bounds: RectF::new(grid_origin + glyph_offset, cell_size),
                foreground_color,
                glyph_type,
            });
        }
        None => {
            // Add FontId as part of the hashkey since characters with different
            // fonts will have different glyph ids.
            if let Some((glyph_id, font_id)) = glyph_and_font {
                // If we don't have special handling for the character, draw the
                // glyph from the font.
                ctx.scene
                    .draw_glyph(origin, glyph_id, font_id, font_size, foreground_color);
            }
        }
    }

    if first_cell_in_link {
        // We want this to be a bounding box to be around the cell, so we don't include baseline_position in the origin.
        ctx.position_cache.cache_position_indefinitely(
            "terminal_view:first_cell_in_link".to_owned(),
            RectF::new(grid_origin + glyph_offset, cell_size),
        );
    }
}

fn render_glyph_svg(
    path: &'static str,
    cell_size: Vector2F,
    cell_origin: Vector2F,
    foreground_color: ColorU,
    ctx: &mut PaintContext,
    app: &AppContext,
) {
    // TODO: abstract / share logic with icon.rs
    let bounds = (cell_size * ctx.scene.scale_factor()).to_i32();

    let asset_cache = AssetCache::as_ref(app);
    let svg = ImageCache::as_ref(app).image(
        AssetSource::Bundled { path },
        bounds,
        FitType::Stretch,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::BySize,
        ctx.max_texture_dimension_2d,
        asset_cache,
    );

    let AssetState::Loaded { data: svg } = svg else {
        return;
    };
    match svg.as_ref() {
        Image::Static(image) => {
            let logical_image_size = image.size().to_f32() / ctx.scene.scale_factor();
            ctx.scene.draw_icon(
                RectF::new(cell_origin, logical_image_size),
                image.clone(),
                1.,
                foreground_color,
            );
        }
        Image::Animated(_) => {
            #[cfg(debug_assertions)]
            log::warn!("Icon image should be static");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_image(
    image_id: u32,
    image_metadata: &StoredImageMetadata,
    image_placement_data: &ImagePlacementData,
    grid_origin: Vector2F,
    glyph_offset: Vector2F,
    ctx: &mut PaintContext,
    app: &AppContext,
) {
    let bounds = (image_placement_data.image_size * ctx.scene.scale_factor()).to_i32();

    let asset_cache = AssetCache::as_ref(app);
    let image = ImageCache::as_ref(app).image(
        AssetSource::Raw {
            id: image_id.to_string(),
        },
        bounds,
        if image_metadata.preserve_aspect_ratio() {
            FitType::Contain
        } else {
            FitType::Stretch
        },
        AnimatedImageBehavior::FullAnimation,
        CacheOption::BySize,
        ctx.max_texture_dimension_2d,
        asset_cache,
    );

    // The raw image should be loaded and ready to render.
    let image = match image {
        AssetState::Loaded { data } => data,
        AssetState::Evicted => {
            return;
        }
        _ => {
            log::warn!("Could not load image to render (image id = {image_id})");
            return;
        }
    };

    match image.as_ref() {
        Image::Static(image) => {
            let logical_image_size = image.size().to_f32() / ctx.scene.scale_factor();

            let image_origin = grid_origin + glyph_offset;
            ctx.scene.draw_image(
                RectF::new(image_origin, logical_image_size),
                image.clone(),
                1.,
                CornerRadius::default(),
            );
        }
        Image::Animated(_) => {
            log::warn!("Image should be static");
        }
    }
}

/// Checks to see if the cell's character is one that we can render natively
/// using rects or other primitives, and if so, updates the paint context
/// accordingly.  Returns true if the character was rendered here, or false
/// if it should be rendered with a font glyph.
fn native_glyph_for_cell(cell: &Cell) -> Option<NativeGlyphType> {
    let glyph_type = match cell.c {
        // Unicode upper half block (U+2580).
        '▀' => NativeGlyphType::UpperHalfBlock,
        // Unicode bottom-aligned fractional block characters (U+2581 - U+2588).
        c @ '▁'..='█' => {
            let index = c as usize - '▁' as usize;
            let eighths = (index + 1) as u8;

            NativeGlyphType::BottomAlignedFractionalBlock { eighths }
        }
        // Unicode left-aligned fractional block characters (U+2589 - U+258F).
        c @ '▉'..='▏' => {
            let index = c as usize - '▉' as usize;
            let eighths = (7 - index) as u8;

            NativeGlyphType::LeftAlignedFractionalBlock { eighths }
        }
        // Unicode right half block (U+2590).
        '▐' => NativeGlyphType::RightHalfBlock,
        // Light shade (U+2591) - we render the foreground color at 25% opacity.
        '░' => NativeGlyphType::Shade { alpha: 64 },
        // Medium shade (U+2592) - we render the foreground color at 50% opacity.
        '▒' => NativeGlyphType::Shade { alpha: 128 },
        // Dark shade (U+2593) - we render the foreground color at 75% opacity.
        '▓' => NativeGlyphType::Shade { alpha: 191 },
        // Upper one eight block (U+2594)
        '▔' => NativeGlyphType::UpperOneEighthBlock,
        // Right one eighth block (U+2595)
        '▕' => NativeGlyphType::RightOneEighthBlock,
        // Quadrant Lower Left (https://www.compart.com/en/unicode/U+2596)
        '▖' => NativeGlyphType::Quadrant {
            upper: false,
            left: true,
        },
        // Quadrant Lower Right (https://www.compart.com/en/unicode/U+2597)
        '▗' => NativeGlyphType::Quadrant {
            upper: false,
            left: false,
        },
        // Quadrant Upper Left (https://www.compart.com/en/unicode/U+2598)
        '▘' => NativeGlyphType::Quadrant {
            upper: true,
            left: true,
        },
        // Quadrant Upper Right (https://www.compart.com/en/unicode/U+259D)
        '▝' => NativeGlyphType::Quadrant {
            upper: true,
            left: false,
        },
        // The "Quadrant Upper Left and Upper Right and Lower Left" unicode character. See
        // https://www.compart.com/en/unicode/U+259B.
        '▛' => NativeGlyphType::QuadrantUpperLeftUpperRightLowerLeft,
        // "Quadrant Lower Right" unicode character. See https://www.compart.com/en/unicode/U+2597.
        '▜' => NativeGlyphType::QuadrantUpperLeftUpperRightLowerRight,
        // The "Quadrant Upper Left and Lower Left and Lower Right" unicode character. See
        // https://www.compart.com/en/unicode/U+2599.
        '▙' => NativeGlyphType::QuadrantUpperLeftLowerLeftLowerRight,
        // The "Quadrant Upper Right and Lower Left and Lower Right" unicode character. See
        // https://www.compart.com/en/unicode/U+259F.
        '▟' => NativeGlyphType::QuadrantUpperRightLowerLeftLowerRight,
        // These are all Nerd Font (NF) glyphs.
        '\u{e0b5}' => NativeGlyphType::NFHalfCircleRightThin,
        '\u{e0b7}' => NativeGlyphType::NFHalfCircleLeftThin,
        '\u{e0b0}' => NativeGlyphType::PowerlineLeftHardDivider,
        '\u{e0b2}' => NativeGlyphType::PowerlineRightHardDivider,
        '\u{e0b6}' => NativeGlyphType::NFHalfCircleLeftThick,
        '\u{f1395}' => NativeGlyphType::NFHalfCircleLeft,
        '\u{e0b4}' => NativeGlyphType::NFHalfCircleRightThick,
        '\u{e0bc}' => NativeGlyphType::NFSlantTriangleTopLeft,
        '\u{e0be}' => NativeGlyphType::NFSlantTriangleTopRight,
        '\u{e0b8}' => NativeGlyphType::NFSlantTriangleBottomLeft,
        '\u{e0ba}' => NativeGlyphType::NFSlantTriangleBottomRight,
        _ => return None,
    };

    Some(glyph_type)
}

/// Handles secret redaction logic and position caching.
/// Returns the character content that should be rendered.
///
/// It ensures that:
/// 1. Secrets are always rendered as asterisks (*) in ObfuscateSecrets::Yes mode
/// 2. Strike-through treatment is applied correctly
/// 3. Position caching for tooltips is handled consistently
#[allow(clippy::too_many_arguments)]
fn handle_secret_redaction<'a>(
    cell: &'a Cell,
    cell_type: &CellType,
    first_cell_in_secret: FirstCellInSecret,
    cell_size: Vector2F,
    grid_origin: Vector2F,
    glyph_offset: Vector2F,
    obfuscate_mode: ObfuscateSecrets,
    ctx: &mut PaintContext,
) -> Option<CharOrStr<'a>> {
    if let Some(Secret {
        is_obfuscated: true,
        ..
    }) = cell_type.secret
    {
        // Cache position for first cell in secret for tooltip positioning
        if let FirstCellInSecret::Yes { handle } = first_cell_in_secret {
            let cell_origin = grid_origin + glyph_offset;
            ctx.position_cache.cache_position_indefinitely(
                format!("terminal_view:first_cell_in_secret_{}", handle.id()),
                RectF::new(cell_origin, cell_size),
            );
        }

        if matches!(obfuscate_mode, ObfuscateSecrets::Yes) {
            // Always use asterisks for secret redaction, regardless of ligature settings
            Some(CharOrStr::Char('*'))
        } else {
            // For strikethrough mode or when obfuscation is disabled, render the actual character
            Some(cell.content_for_display())
        }
    } else {
        None
    }
}

/// Renders a native glyph.
fn render_native_glyph(native_glyph: NativeGlyph, ctx: &mut PaintContext, app: &AppContext) {
    let NativeGlyph {
        cell_bounds,
        foreground_color,
        glyph_type,
    } = native_glyph;
    let svg_data = match glyph_type {
        NativeGlyphType::UpperHalfBlock => {
            let rect = RectF::new(
                cell_bounds.origin(),
                vec2f(cell_bounds.width(), cell_bounds.height() / 2.0),
            );
            ctx.scene
                .draw_rect_without_hit_recording(rect)
                .with_background(Fill::Solid(foreground_color));
            None
        }
        NativeGlyphType::BottomAlignedFractionalBlock { eighths } => {
            let height = cell_bounds.height() / 8.0 * eighths as f32;
            let rect = RectF::new(
                cell_bounds.origin() + vec2f(0.0, cell_bounds.height() - height),
                vec2f(cell_bounds.size().x(), height),
            );
            ctx.scene
                .draw_rect_without_hit_recording(rect)
                .with_background(Fill::Solid(foreground_color));
            None
        }
        NativeGlyphType::LeftAlignedFractionalBlock { eighths } => {
            let width = cell_bounds.width() / 8.0 * eighths as f32;
            let rect = RectF::new(cell_bounds.origin(), vec2f(width, cell_bounds.height()));
            ctx.scene
                .draw_rect_without_hit_recording(rect)
                .with_background(Fill::Solid(foreground_color));
            None
        }
        NativeGlyphType::RightHalfBlock => {
            let width = cell_bounds.width() / 2.0;
            let rect = RectF::new(
                cell_bounds.origin() + vec2f(width, 0.0),
                vec2f(width, cell_bounds.height()),
            );
            ctx.scene
                .draw_rect_without_hit_recording(rect)
                .with_background(Fill::Solid(foreground_color));
            None
        }
        NativeGlyphType::Quadrant { upper, left } => {
            let width = cell_bounds.width() / 2.;
            let height = cell_bounds.height() / 2.;

            let origin_x = if left { 0. } else { width };

            let origin_y = if upper { 0. } else { height };

            let rect = RectF::new(
                cell_bounds.origin() + vec2f(origin_x, origin_y),
                vec2f(width, height),
            );
            ctx.scene
                .draw_rect_without_hit_recording(rect)
                .with_background(Fill::Solid(foreground_color));
            None
        }
        NativeGlyphType::QuadrantUpperLeftUpperRightLowerLeft => {
            // This glyph is composed of the entire upper half of the cell and the bottom left half
            // of the cell. Render this by breaking it up to render:
            // 1) The upper half of the block
            // 2) The lower left quadrant.
            render_native_glyph(
                NativeGlyph {
                    cell_bounds,
                    foreground_color,
                    glyph_type: NativeGlyphType::UpperHalfBlock,
                },
                ctx,
                app,
            );

            render_native_glyph(
                NativeGlyph {
                    cell_bounds,
                    foreground_color,
                    glyph_type: NativeGlyphType::Quadrant {
                        upper: false,
                        left: true,
                    },
                },
                ctx,
                app,
            );
            None
        }
        NativeGlyphType::QuadrantUpperLeftUpperRightLowerRight => {
            // This glyph is composed of the entire upper half of the cell and the bottom right half
            // of the cell. Render this by breaking it up to render:
            // 1) The upper half of the block
            // 2) The lower right quadrant.
            render_native_glyph(
                NativeGlyph {
                    cell_bounds,
                    foreground_color,
                    glyph_type: NativeGlyphType::UpperHalfBlock,
                },
                ctx,
                app,
            );

            render_native_glyph(
                NativeGlyph {
                    cell_bounds,
                    foreground_color,
                    glyph_type: NativeGlyphType::Quadrant {
                        upper: false,
                        left: false,
                    },
                },
                ctx,
                app,
            );
            None
        }
        NativeGlyphType::QuadrantUpperLeftLowerLeftLowerRight => {
            // This glyph is composed of the entire lower half of the cell and the upper left half
            // of the cell. Render this by breaking it up to render:
            // 1) The bottom half of the block
            // 2) The upper left quadrant.
            render_native_glyph(
                NativeGlyph {
                    cell_bounds,
                    foreground_color,
                    glyph_type: NativeGlyphType::BottomAlignedFractionalBlock { eighths: 4 },
                },
                ctx,
                app,
            );

            render_native_glyph(
                NativeGlyph {
                    cell_bounds,
                    foreground_color,
                    glyph_type: NativeGlyphType::Quadrant {
                        upper: true,
                        left: true,
                    },
                },
                ctx,
                app,
            );
            None
        }
        NativeGlyphType::QuadrantUpperRightLowerLeftLowerRight => {
            // This glyph is composed of the entire lower half of the cell and the upper right half
            // of the cell. Render this by breaking it up to render:
            // 1) The bottom half of the block
            // 2) The upper right quadrant.
            render_native_glyph(
                NativeGlyph {
                    cell_bounds,
                    foreground_color,
                    glyph_type: NativeGlyphType::BottomAlignedFractionalBlock { eighths: 4 },
                },
                ctx,
                app,
            );

            render_native_glyph(
                NativeGlyph {
                    cell_bounds,
                    foreground_color,
                    glyph_type: NativeGlyphType::Quadrant {
                        upper: true,
                        left: false,
                    },
                },
                ctx,
                app,
            );
            None
        }
        NativeGlyphType::Shade { alpha } => {
            let mut color = foreground_color;
            color.a = alpha;
            ctx.scene
                .draw_rect_without_hit_recording(cell_bounds)
                .with_background(Fill::Solid(color));
            None
        }
        NativeGlyphType::UpperOneEighthBlock => {
            let rect = RectF::new(
                cell_bounds.origin(),
                vec2f(cell_bounds.width(), cell_bounds.height() / 8.0),
            );
            ctx.scene
                .draw_rect_without_hit_recording(rect)
                .with_background(Fill::Solid(foreground_color));
            None
        }
        NativeGlyphType::RightOneEighthBlock => {
            let width = cell_bounds.width() / 8.0;
            let rect = RectF::new(
                cell_bounds.origin() + vec2f(cell_bounds.width() - width, 0.0),
                vec2f(width, cell_bounds.height()),
            );
            ctx.scene
                .draw_rect_without_hit_recording(rect)
                .with_background(Fill::Solid(foreground_color));
            None
        }

        NativeGlyphType::NFHalfCircleLeftThick | NativeGlyphType::NFHalfCircleLeft => {
            Some("bundled/svg/stretchable_glyphs/left-half-circle.svg")
        }
        NativeGlyphType::PowerlineLeftHardDivider => {
            Some("bundled/svg/stretchable_glyphs/left-hard-divider.svg")
        }
        NativeGlyphType::PowerlineRightHardDivider => {
            Some("bundled/svg/stretchable_glyphs/right-hard-divider.svg")
        }
        NativeGlyphType::NFHalfCircleRightThick => {
            Some("bundled/svg/stretchable_glyphs/right-half-circle.svg")
        }
        NativeGlyphType::NFHalfCircleLeftThin => {
            Some("bundled/svg/stretchable_glyphs/left-half-circle-thin.svg")
        }
        NativeGlyphType::NFHalfCircleRightThin => {
            Some("bundled/svg/stretchable_glyphs/right-half-circle-thin.svg")
        }
        NativeGlyphType::NFSlantTriangleTopLeft => {
            Some("bundled/svg/stretchable_glyphs/upper-left-triangle.svg")
        }
        NativeGlyphType::NFSlantTriangleTopRight => {
            Some("bundled/svg/stretchable_glyphs/upper-right-triangle.svg")
        }
        NativeGlyphType::NFSlantTriangleBottomLeft => {
            Some("bundled/svg/stretchable_glyphs/lower-left-triangle.svg")
        }
        NativeGlyphType::NFSlantTriangleBottomRight => {
            Some("bundled/svg/stretchable_glyphs/lower-right-triangle.svg")
        }
    };
    if let Some(path) = svg_data {
        render_glyph_svg(
            path,
            cell_bounds.size(),
            cell_bounds.origin(),
            foreground_color,
            ctx,
            app,
        );
    }
}

/// Calculate the Rect arguments for the underlines and strikethroughs, but don't actually draw
/// them yet. Instead, return their data so that they can be drawn later to avoid overlap problems
/// with the background.
fn calculate_cell_decorations(
    cell: &Cell,
    cell_type: &CellType,
    cell_size: Vector2F,
    grid_origin: Vector2F,
    glyph_offset: Vector2F,
    foreground_color: ColorU,
    obfuscate_mode: ObfuscateSecrets,
) -> Option<DecorationData> {
    let is_secret_in_highlight_mode = matches!(obfuscate_mode, ObfuscateSecrets::Strikethrough)
        && cell_type.is_unhovered_secret();

    if !cell.flags.intersects(Flags::CELL_DECORATIONS)
        && !cell_type.is_url()
        && !is_secret_in_highlight_mode
    {
        return None;
    }

    let thickness = UNDERLINE_THICKNESS_SCALE_FACTOR * cell_size.x().round().max(1.);

    let is_unhovered_secret = cell_type.is_unhovered_secret();
    let is_in_strikethrough_mode = matches!(obfuscate_mode, ObfuscateSecrets::Strikethrough);

    // Don't apply strikethrough to secrets that are inside hovered links
    let should_strikethrough_secret =
        is_unhovered_secret && is_in_strikethrough_mode && !cell_type.is_url();

    let decoration_rect_data = if cell.flags.intersects(Flags::DOUBLE_UNDERLINE) {
        Some((thickness * 2., cell_size.y(), foreground_color))
    } else if cell.flags.intersects(Flags::UNDERLINE) {
        Some((thickness, cell_size.y(), foreground_color))
    } else if cell.flags.intersects(Flags::STRIKEOUT) || should_strikethrough_secret {
        Some((thickness, cell_size.y() / 2., foreground_color))
    } else if cell_type.is_url() {
        Some((thickness, cell_size.y(), *URL_COLOR))
    } else {
        None
    };

    let actual_cell_size = if cell.flags.intersects(Flags::WIDE_CHAR) {
        cell_size * vec2f(2., 1.)
    } else {
        cell_size
    };
    decoration_rect_data.map(|(thickness, y, color)| DecorationData {
        origin: grid_origin + glyph_offset + vec2f(0., y - thickness),
        size: vec2f(actual_cell_size.x(), thickness),
        color,
    })
}

fn calculate_cursor_origin(
    grid_render_params: &GridRenderParams,
    line_top_origin: Vector2F,
    ctx: &mut PaintContext,
) -> Vector2F {
    let baseline_position = calculate_grid_baseline_position(
        ctx.font_cache,
        grid_render_params.font_family,
        grid_render_params.font_size,
        grid_render_params.line_height_ratio,
        grid_render_params.cell_size.y(),
    );
    line_top_origin
        + vec2f(
            0.,
            // Similar to input editor cursor size calculations:
            // 1. Go down by baseline offset (which is 80% of line height due to top bottom ratio)
            (baseline_position.y()
            // 2. Go up by font size * 80% (top bottom ratio) * 1.2 (default line height ratio) which
            // gets us to a good point for the cursor origin (the same exact point in default case).
            - (grid_render_params.font_size
                * DEFAULT_UI_LINE_HEIGHT_RATIO
                * DEFAULT_TOP_BOTTOM_RATIO))
                .max(0.0),
        )
}

#[allow(clippy::too_many_arguments)]
pub fn render_cursor(
    grid_render_params: &GridRenderParams,
    cursor_point: Point,
    is_cursor_on_wide_char: bool,
    cursor_style: CursorStyle,
    padding_x: Pixels,
    grid_origin: Vector2F,
    color: ColorU,
    ctx: &mut PaintContext,
    terminal_view_id: EntityId,
    hint_text: Option<&mut Box<dyn Element>>,
    app: &AppContext,
) {
    let line_top_origin = grid_origin
        + vec2f(padding_x.as_f32(), 0.)
        + grid_render_params.cell_size * vec2f(cursor_point.col as f32, cursor_point.row as f32);

    let cursor_top_origin = calculate_cursor_origin(grid_render_params, line_top_origin, ctx);
    let cell_width = if is_cursor_on_wide_char {
        grid_render_params.cell_size.x() * 2.
    } else {
        grid_render_params.cell_size.x()
    };
    let cursor_block_size = vec2f(
        cell_width,
        grid_render_params.font_size * DEFAULT_UI_LINE_HEIGHT_RATIO,
    );

    ctx.position_cache.cache_position_indefinitely(
        format!("terminal_view:cursor_{terminal_view_id}"),
        RectF::new(cursor_top_origin, cursor_block_size),
    );

    let thickness =
        CURSOR_THICKNESS_SCALE_FACTOR * grid_render_params.cell_size.x().round().max(1.);
    match cursor_style.shape {
        CursorShape::Block => {
            ctx.scene
                .draw_rect_with_hit_recording(RectF::new(cursor_top_origin, cursor_block_size))
                .with_background(Fill::Solid(color));
        }
        CursorShape::Underline => {
            let height = grid_render_params.cell_size.y();
            ctx.scene
                .draw_rect_with_hit_recording(RectF::new(
                    line_top_origin + vec2f(0., height - thickness),
                    vec2f(cell_width, thickness),
                ))
                .with_background(Fill::Solid(color));
        }
        CursorShape::Beam => {
            ctx.scene
                .draw_rect_with_hit_recording(RectF::new(
                    cursor_top_origin,
                    vec2f(thickness, cursor_block_size.y()),
                ))
                .with_background(Fill::Solid(color));
        }
        CursorShape::HollowBlock => {
            ctx.scene
                .draw_rect_with_hit_recording(RectF::new(cursor_top_origin, cursor_block_size))
                .with_border(Border::all(thickness).with_border_color(color));
        }
        CursorShape::Hidden => {}
    }

    if let Some(hint_text) = hint_text {
        // Render hint text with 4px padding to the right of the cursor, bottom aligned.
        hint_text.paint(
            vec2f(
                cursor_top_origin.x() + cursor_block_size.x() + 4.,
                cursor_top_origin.y() + cursor_block_size.y()
                    - hint_text
                        .size()
                        .expect("Text is laid out prior to paint")
                        .y(),
            ),
            ctx,
            app,
        );
    }
}

/// A background that spans multiple rows can be broken up into three individual background rows:
/// 1) The first row, which doesn't necessarily start on the left;
/// 2) The intermediate rows, each of which horizontally spans the entire grid; and
/// 3) The final row, which doesn't necessarily end on the right.
fn calculate_background_bounds(
    origin: Vector2F,
    cached: CachedBackgroundColor,
    cell_size: Vector2F,
    max_columns: usize,
) -> Vec<RectF> {
    let start = cached.start;
    let end = cached.end;
    let origin = origin + vec2f(0., start.row.as_f64() as f32 * cell_size.y());
    let mut rows = Vec::with_capacity(3);

    // Calculating bounds for the first row
    let first_row_origin = origin + vec2f(start.col as f32 * cell_size.x(), 0.);
    let first_row_end_column = if cached.is_single_row_background() {
        end.col
    } else {
        max_columns
    };
    let first_row_size = cell_size
        * vec2f(
            (first_row_end_column.saturating_sub(start.col) + 1) as f32,
            1.,
        );
    rows.push(RectF::new(first_row_origin, first_row_size));
    if cached.is_single_row_background() {
        return rows;
    }

    // Calculating bounds for the intermediate rows
    let middle_rows_origin = origin + vec2f(0., cell_size.y());
    let middle_rows_size = cell_size
        * vec2f(
            (max_columns + 1) as f32,
            ((end.row - start.row).as_f64() - 1.).max(0.) as f32,
        );
    if middle_rows_size.y() != 0. {
        rows.push(RectF::new(middle_rows_origin, middle_rows_size));
    }

    // Calculating bounds for the final row
    let final_row_origin = middle_rows_origin + vec2f(0., middle_rows_size.y());
    let final_row_size = cell_size * vec2f((end.col + 1) as f32, 1.);
    rows.push(RectF::new(final_row_origin, final_row_size));
    rows
}

/// Renders the background for the given range of cells.
/// It starts rendering from the 'origin' and adjusts accordingly using start and end vectors,
/// which denote the position in the grid at which the background should start and end, respectively.
/// Note that both vectors are inclusive (and don't account for the cell size, which is passed separately).
fn render_background(
    origin: Vector2F,
    cached: CachedBackgroundColor,
    cell_size: Vector2F,
    max_columns: usize,
    ctx: &mut PaintContext,
) {
    let color = cached.background_color;
    if color.is_fully_transparent() {
        return;
    }
    for rect in calculate_background_bounds(origin, cached, cell_size, max_columns) {
        ctx.scene
            .draw_rect_without_hit_recording(rect)
            .with_background(Fill::Solid(color));
    }
}

/// Move the origin used to render the selection to account for padding and scroll (note that the
/// selection may start offscreen).
fn account_for_padding_and_scroll(
    origin: Vector2F,
    cell_size: Vector2F,
    padding_x_px: Pixels,
    scroll_top: Lines,
) -> Vector2F {
    let displacement = vec2f(
        padding_x_px.as_f32(),
        -(cell_size.y() * scroll_top.as_f64() as f32),
    );
    origin + displacement
}

fn calculate_cell_size(size: &SizeInfo) -> Vector2F {
    Vector2F::new(
        size.cell_width_px().as_f32(),
        size.cell_height_px().as_f32(),
    )
}

/// Assumes that start is before end.
pub fn calculate_selection_bounds(
    start: &SelectionPoint,
    end: &SelectionPoint,
    size: &SizeInfo,
    scroll_top: Lines,
    origin: Vector2F,
) -> Vec<RectF> {
    let cell_size = calculate_cell_size(size);
    let selection_start =
        account_for_padding_and_scroll(origin, cell_size, size.padding_x_px, scroll_top);
    // The color passed into `from_selection_points` doesn't matter because we're not rendering anything
    let cached = CachedBackgroundColor::from_selection_points(ColorU::default(), start, end);
    calculate_background_bounds(selection_start, cached, cell_size, size.columns - 1)
}

/// Assumes that start is before end.
pub fn render_selection(
    start: &SelectionPoint,
    end: &SelectionPoint,
    size: &SizeInfo,
    scroll_top: Lines,
    origin: Vector2F,
    color: ColorU,
    ctx: &mut PaintContext,
) {
    let cell_size = calculate_cell_size(size);
    let selection_start =
        account_for_padding_and_scroll(origin, cell_size, size.padding_x_px, scroll_top);
    let cached = CachedBackgroundColor::from_selection_points(color, start, end);
    render_background(selection_start, cached, cell_size, size.columns - 1, ctx)
}

pub fn render_selection_cursor(
    cursor_point: &SelectionPoint,
    size: &SizeInfo,
    scroll_top: Lines,
    origin: Vector2F,
    color: ColorU,
    is_cursor_at_end: bool,
    ctx: &mut PaintContext,
) {
    let cell_size = calculate_cell_size(size);
    let mut origin =
        account_for_padding_and_scroll(origin, cell_size, size.padding_x_px, scroll_top);

    // Move origin to where the selection point is.
    // If the cursor is at the end of the selection, we need to move the point over one column
    // for it to be after the end of the selection (cursor is actually at start of the next col)
    origin += vec2f(
        (cursor_point.col + is_cursor_at_end as usize) as f32 * cell_size.x(),
        cursor_point.row.as_f64() as f32 * cell_size.y(),
    );

    // Render the vertical line cursor
    let cursor_size = Vector2F::new(1., cell_size.y());
    ctx.scene
        .draw_rect_without_hit_recording(RectF::new(origin, cursor_size))
        .with_background(Fill::Solid(color));
    // Render the circle at the top of the vertical line
    // The floor is needed to center the circle with the vertical line, since the rendering seems to be off with decimal pixels.
    origin -= vec2f(
        (SELECTION_CURSOR_TOP_DIAMETER / 2.).floor(),
        (SELECTION_CURSOR_TOP_DIAMETER / 2.).floor(),
    );
    ctx.scene
        .draw_rect_without_hit_recording(RectF::new(
            origin,
            vec2f(SELECTION_CURSOR_TOP_DIAMETER, SELECTION_CURSOR_TOP_DIAMETER),
        ))
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .with_background(Fill::Solid(color));
}

/// String builder that tracks information about styles and character index to cell mapping
///
/// Used by ligature rendering to build the attributed string for laying out text and connecting
/// the rendered glyphs back with their appropriate offsets.
struct AttributedStringBuilder {
    font_family: FamilyId,
    current_style: StyleAndFont,
    current_style_start_char_index: usize,
    style_runs: Vec<(Range<usize>, StyleAndFont)>,
    // Conceptually, this is a map from character index to cell index. However, since it is both
    // dense and build in order, we can avoid the overhead of hashing by using a `Vec`
    character_index_to_cell_map: Vec<usize>,
    line: String,
}

impl AttributedStringBuilder {
    fn new(font_family: FamilyId, default_font_family: FamilyId, columns: usize) -> Self {
        Self {
            font_family,
            current_style: StyleAndFont::new(
                default_font_family,
                Properties::default(),
                TextStyle::default(),
            ),
            current_style_start_char_index: 0,
            style_runs: Vec::new(),
            character_index_to_cell_map: Vec::with_capacity(columns),
            line: String::with_capacity(columns),
        }
    }

    fn next_character_index(&self) -> usize {
        self.character_index_to_cell_map.len()
    }

    /// Flushes the currently cached style into the style runs list and update the current style
    /// start index.
    ///
    /// Will only insert a value if the style would apply to at least one character.
    fn flush_style_run(&mut self) {
        let next_character_index = self.next_character_index();
        if next_character_index > self.current_style_start_char_index {
            self.style_runs.push((
                self.current_style_start_char_index..next_character_index,
                self.current_style,
            ));
        }

        self.current_style_start_char_index = next_character_index;
    }

    /// Update the style information for the current position.
    ///
    /// This will flush the cached style into the style runs list whenever the style changes.
    fn update_style(&mut self, font_style: FontStyle, foreground_color: ColorU) {
        let cell_style = StyleAndFont {
            font_family: self.font_family,
            properties: font_style.to_properties(),
            style: TextStyle {
                foreground_color: Some(foreground_color),
                // We don't need to include background color or text decorations in the data
                // that we send to the text layout engine, as those are all handled by native
                // drawing of rects within the grid (see `calculate_cell_decorations` above)
                ..Default::default()
            },
        };

        if cell_style != self.current_style {
            self.flush_style_run();
            self.current_style = cell_style;
        }
    }

    /// Insert a placeholder character
    ///
    /// This should be used whenever we are rendering a cell using non-glyph methods (e.g. native
    /// box drawing or secret obfuscation icons)
    ///
    /// The attributed string will be updated with a space character so that the text layout
    /// doesn't result in erroneous ligatures connecting characters before and after the
    /// placeholder
    fn append_placeholder(&mut self, column: usize) {
        self.append_character(' ', column);
    }

    /// Append a character to the attributed string at a specific grid column.
    ///
    /// This will update the mapping of cell indexes so that the character can be connected with
    /// its expected grid position later.
    fn append_character(&mut self, chr: char, column: usize) {
        self.line.push(chr);
        self.character_index_to_cell_map.push(column);
    }

    /// Append a cell's content to the attributed string at a specific grid column.
    ///
    /// This will update the mapping of cell indexes so that the character can be connected with
    /// its expected grid position later.
    fn append_content(&mut self, content: CharOrStr, column: usize) {
        match content {
            CharOrStr::Char(c) => self.append_character(c, column),
            CharOrStr::Str(s) => {
                for c in s.chars() {
                    self.append_character(c, column);
                }
            }
        }
    }

    /// Build the full attributed string data and return it so it can be used to lay out the text
    fn build(mut self) -> AttributedStringData {
        self.flush_style_run();

        AttributedStringData {
            line: self.line,
            style_runs: self.style_runs,
            character_index_to_cell_map: self.character_index_to_cell_map,
        }
    }
}

struct AttributedStringData {
    line: String,
    style_runs: Vec<(Range<usize>, StyleAndFont)>,
    character_index_to_cell_map: Vec<usize>,
}

#[derive(Default)]
enum FirstCellInSecret {
    Yes {
        handle: SecretHandle,
    },
    #[default]
    No,
}

fn is_url_visible(url: &Link, start_row: usize, end_row: usize) -> bool {
    url.range.start().row >= start_row && url.range.end().row <= end_row
}

enum DottedLinePosition {
    TopOfRow,
    /// Used for the rendering a dotted line at the end of a grid.
    BottomOfRow,
}

struct DottedLineInfo {
    cell_size: Vector2F,
    offset_row: usize,
    grid_origin: Vector2F,
    dotted_line_color: ColorU,
}

/// Renders dotted line(s) around a row to represent skipped lines when a block
/// filter is active, if rows are skipped.
fn maybe_render_dotted_lines(
    grid: &GridHandler,
    row: usize,
    prev_row: Option<usize>,
    offset_row: usize,
    dotted_line_info: DottedLineInfo,
    ctx: &mut PaintContext,
) {
    let has_skipped_rows = prev_row
        .map(|prev_row| row - prev_row > 1)
        .unwrap_or_else(|| row > 0);
    if has_skipped_rows {
        render_dotted_line(
            grid.columns(),
            &dotted_line_info,
            DottedLinePosition::TopOfRow,
            ctx,
        );
    }

    // Render the last dotted line to represent non-displayed rows at the end of the grid.
    if grid
        .len_displayed()
        .is_some_and(|len_displayed| offset_row == len_displayed.saturating_sub(1))
        && row < grid.max_content_row()
    {
        render_dotted_line(
            grid.columns(),
            &dotted_line_info,
            DottedLinePosition::BottomOfRow,
            ctx,
        );
    }
}

fn render_dotted_line(
    columns: usize,
    dotted_line_info: &DottedLineInfo,
    dotted_line_position: DottedLinePosition,
    ctx: &mut PaintContext,
) {
    let DottedLineInfo {
        cell_size,
        offset_row,
        grid_origin,
        dotted_line_color,
    } = dotted_line_info;
    let row_offset = vec2f(0., *offset_row as f32 * cell_size.y());
    let context_line_separator_origin = match dotted_line_position {
        DottedLinePosition::TopOfRow => *grid_origin + row_offset,
        DottedLinePosition::BottomOfRow => *grid_origin + row_offset + vec2f(0., cell_size.y()),
    };
    let line_size = vec2f(cell_size.x() * columns as f32, cell_size.y());
    ctx.scene
        .draw_rect_without_hit_recording(RectF::new(context_line_separator_origin, line_size))
        .with_border(
            Border::top(BLOCK_FILTER_DOTTED_LINE_WIDTH)
                .with_dashed_border(BLOCK_FILTER_DOTTED_LINE_DASH)
                .with_border_color(*dotted_line_color),
        );
}

#[cfg(test)]
#[path = "grid_renderer_tests.rs"]
pub mod tests;
