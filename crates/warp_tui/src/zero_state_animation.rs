//! Rotating object animation for the TUI zero state.
//!
//! The built-in Warp mark or a user-provided ASCII silhouette is sampled into a
//! shallow, ghosted wireframe, rotated around its vertical axis, and projected
//! back onto terminal cells. A tiny z-buffer keeps the front-most sample for
//! each cell, while directional ASCII edges and sparse stippling retain depth
//! without a solid fill.

use std::sync::Arc;
use std::time::Duration;

use warpui_core::AppContext;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiScreenPoint,
    TuiScreenPosition, TuiSize, TuiStyle,
};

#[path = "zero_state_animation_config.rs"]
mod config;

use config::ZeroStateShape;
pub(crate) use config::{
    ZeroStateAnimationConfig, ZeroStateAnimationConfigEvent, ZeroStateAnimationLoadFailure,
};

/// A terminal does not need a 30 fps repaint for this deliberately slow motion.
const REPAINT_INTERVAL: Duration = Duration::from_millis(66);

const MIN_ANIMATION_COLS: u16 = 18;
const MIN_ANIMATION_ROWS: u16 = 7;
const MAX_LOGO_ROWS: u16 = 17;
const MIN_OBJECT_COLS: u16 = 5;
const MIN_OBJECT_ROWS: u16 = 5;
const BUILT_IN_LOGO_CELL_ASPECT_RATIO: f64 = 2.5;
const SURFACE_SAMPLES: usize = 3;
const DEPTH_SAMPLES: usize = 6;
const GHOST_STIPPLE_MODULUS: usize = 97;
const SIDE_STITCH_MODULUS: usize = 29;
const STARFIELD_REFERENCE_AREA: usize = 52 * 20;
const STARFIELD_REFERENCE_COUNT: usize = 36;
const STARFIELD_MIN_COUNT: usize = 18;
/// Bounds per-frame work for synthetic or otherwise impractical terminal sizes.
/// Ordinary terminal dimensions remain below this budget and retain full
/// area-proportional star density.
const STARFIELD_CANDIDATE_BUDGET: usize = 8_192;
const STAR_TRAVEL_SECS: f64 = 7.0;
const MAX_ASCII_ART_BYTES: u64 = 64 * 1024;
const MAX_ASCII_ART_COLS: usize = 128;
const MAX_ASCII_ART_ROWS: usize = 64;

/// Approximate visible bounds of the bundled Warp logo SVG.
const SVG_MIN_X: f64 = 35.0;
const SVG_MAX_X: f64 = 216.155;
const SVG_MIN_Y: f64 = 25.5701;
const SVG_MAX_Y: f64 = 170.489;

/// The upper-right and lower-left faces from `warp-logo-light.svg`.
///
/// Rounded corners are intentionally squared off: at terminal-cell resolution,
/// preserving the diagonal cut and offset silhouette contributes much more to
/// recognition than sub-cell corner curvature.
const UPPER_FACE: &[(f64, f64)] = &[
    (127.725, 25.5701),
    (196.111, 25.5701),
    (216.155, 46.2824),
    (216.155, 126.695),
    (196.111, 147.407),
    (98.2486, 147.407),
];
const LOWER_FACE: &[(f64, f64)] = &[
    (109.963, 48.652),
    (54.8733, 48.652),
    (35.0, 69.3643),
    (35.0, 149.777),
    (54.8733, 170.489),
    (122.676, 170.489),
    (125.395, 159.154),
    (83.4561, 159.154),
];

#[derive(Clone, Copy)]
pub(crate) struct WarpLogoStyles {
    pub(crate) front: TuiStyle,
    pub(crate) back: TuiStyle,
    pub(crate) side: TuiStyle,
    pub(crate) background: TuiStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogoSurface {
    Front,
    Back,
    Side,
    Ghost,
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogoGlyph {
    Horizontal,
    Vertical,
    ForwardSlash,
    Backslash,
    Dot,
    Plus,
    Star,
}

impl LogoGlyph {
    fn as_str(self) -> &'static str {
        match self {
            Self::Horizontal => "-",
            Self::Vertical => "|",
            Self::ForwardSlash => "/",
            Self::Backslash => "\\",
            Self::Dot => ".",
            Self::Plus => "+",
            Self::Star => "*",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LogoCell {
    surface: LogoSurface,
    glyph: LogoGlyph,
}

#[derive(Clone, Copy)]
struct ProjectedSample {
    depth: f64,
    cell: LogoCell,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LogoFrame {
    size: TuiSize,
    cells: Vec<Option<LogoCell>>,
}

impl LogoFrame {
    fn new(size: TuiSize) -> Self {
        Self {
            size,
            cells: vec![None; usize::from(size.width) * usize::from(size.height)],
        }
    }

    fn set(&mut self, x: usize, y: usize, cell: LogoCell) {
        self.cells[y * usize::from(self.size.width) + x] = Some(cell);
    }

    fn iter_cells(&self) -> impl Iterator<Item = (usize, usize, LogoCell)> + '_ {
        let width = usize::from(self.size.width);
        self.cells
            .iter()
            .enumerate()
            .filter_map(move |(index, cell)| cell.map(|cell| (index % width, index / width, cell)))
    }

    #[cfg(test)]
    fn to_lines(&self) -> Vec<String> {
        let width = usize::from(self.size.width);
        self.cells
            .chunks(width)
            .map(|row| {
                row.iter()
                    .map(|cell| cell.map_or(" ", |cell| cell.glyph.as_str()))
                    .collect()
            })
            .collect()
    }
}

pub struct ZeroStateAnimationElement {
    clock: AnimationClock,
    config: Arc<ZeroStateAnimationConfig>,
    styles: WarpLogoStyles,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl ZeroStateAnimationElement {
    pub(crate) fn new(
        clock: AnimationClock,
        config: Arc<ZeroStateAnimationConfig>,
        styles: WarpLogoStyles,
    ) -> Self {
        Self {
            clock,
            config,
            styles,
            size: None,
            origin: None,
        }
    }
}

impl TuiElement for ZeroStateAnimationElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let size = if constraint.max.width >= MIN_ANIMATION_COLS
            && constraint.max.height >= MIN_ANIMATION_ROWS
        {
            constraint.max
        } else {
            TuiSize::ZERO
        };
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        let Some(size) = self.size else { return };
        let Some(frame) = object_frame_at(self.clock.elapsed(), size, &self.config) else {
            return;
        };

        for (x, y, logo_cell) in frame.iter_cells() {
            let style = match logo_cell.surface {
                LogoSurface::Front => self.styles.front,
                LogoSurface::Back => self.styles.back,
                LogoSurface::Side | LogoSurface::Ghost => self.styles.side,
                LogoSurface::Background => self.styles.background,
            };
            if let Some(cell) = surface.cell_mut(origin.offset(x as i32, y as i32)) {
                cell.set_symbol(logo_cell.glyph.as_str()).set_style(style);
            }
        }

        ctx.repaint_after(REPAINT_INTERVAL);
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

#[cfg(test)]
pub(crate) fn logo_frame_at(elapsed: Duration, size: TuiSize) -> Option<LogoFrame> {
    object_frame_at(elapsed, size, &ZeroStateAnimationConfig::default())
}

fn object_frame_at(
    elapsed: Duration,
    size: TuiSize,
    config: &ZeroStateAnimationConfig,
) -> Option<LogoFrame> {
    let cell_aspect_ratio = config.shape.cell_aspect_ratio();
    let (logo_cols, logo_rows) = fitted_logo_size(size, cell_aspect_ratio)?;
    let revolution_secs = config.rotation_period.as_secs_f64();
    let angle = (elapsed.as_secs_f64() % revolution_secs) / revolution_secs * std::f64::consts::TAU;
    let (sin, cos) = angle.sin_cos();
    let mut frame = LogoFrame::new(size);
    draw_background_stars(&mut frame, elapsed);
    let mut z_buffer = vec![None; usize::from(size.width) * usize::from(size.height)];

    let source_cols = usize::from(logo_cols) * SURFACE_SAMPLES;
    let source_rows = usize::from(logo_rows) * SURFACE_SAMPLES;
    let center_x = (f64::from(size.width) - 1.0) / 2.0;
    let center_y = (f64::from(size.height) - 1.0) / 2.0;
    let scale_x = (f64::from(logo_cols) - 1.0) / 2.0;
    let scale_y = (f64::from(logo_rows) - 1.0) / 2.0;

    for source_y in 0..source_rows {
        let model_y = sample_coordinate(source_y, source_rows);
        for source_x in 0..source_cols {
            let model_x = sample_coordinate(source_x, source_cols);
            if !config.shape.contains(model_x, model_y) {
                continue;
            }
            let outline_glyph = logo_outline_glyph(
                config.shape.as_ref(),
                model_x,
                model_y,
                source_cols,
                source_rows,
                cos,
                cell_aspect_ratio,
            );
            let is_ghost_sample = (source_x * 17 + source_y * 31) % GHOST_STIPPLE_MODULUS == 0;
            let is_side_stitch = (source_x * 13 + source_y * 7) % SIDE_STITCH_MODULUS == 0;
            if outline_glyph.is_none() && !is_ghost_sample {
                continue;
            }

            for depth_index in 0..=DEPTH_SAMPLES {
                let is_face = depth_index == 0 || depth_index == DEPTH_SAMPLES;
                if !is_face && (outline_glyph.is_none() || !is_side_stitch) {
                    continue;
                }
                let model_z = -config.extrusion_depth
                    + 2.0 * config.extrusion_depth * depth_index as f64 / DEPTH_SAMPLES as f64;
                let rotated_x = model_x * cos + model_z * sin;
                let rotated_depth = -model_x * sin + model_z * cos;
                let projected_x = (center_x + rotated_x * scale_x).round() as i32;
                let projected_y = (center_y + model_y * scale_y).round() as i32;
                if projected_x < 0
                    || projected_y < 0
                    || projected_x >= i32::from(size.width)
                    || projected_y >= i32::from(size.height)
                {
                    continue;
                }

                let cell = if !is_face {
                    LogoCell {
                        surface: LogoSurface::Side,
                        glyph: LogoGlyph::Dot,
                    }
                } else if let Some(glyph) = outline_glyph {
                    let surface = if depth_index == DEPTH_SAMPLES {
                        LogoSurface::Front
                    } else {
                        LogoSurface::Back
                    };
                    LogoCell { surface, glyph }
                } else {
                    LogoCell {
                        surface: LogoSurface::Ghost,
                        glyph: LogoGlyph::Dot,
                    }
                };
                let index = projected_y as usize * usize::from(size.width) + projected_x as usize;
                let sample = ProjectedSample {
                    depth: rotated_depth,
                    cell,
                };
                if z_buffer[index]
                    .is_none_or(|current: ProjectedSample| sample.depth > current.depth)
                {
                    z_buffer[index] = Some(sample);
                }
            }
        }
    }

    for (index, sample) in z_buffer.into_iter().enumerate() {
        if let Some(sample) = sample {
            frame.set(
                index % usize::from(size.width),
                index / usize::from(size.width),
                sample.cell,
            );
        }
    }
    Some(frame)
}

fn draw_background_stars(frame: &mut LogoFrame, elapsed: Duration) {
    let width = usize::from(frame.size.width);
    let height = usize::from(frame.size.height);
    let star_count = star_count_for_size(frame.size);
    let center_x = (f64::from(frame.size.width) - 1.0) / 2.0;
    let center_y = (f64::from(frame.size.height) - 1.0) / 2.0;
    let elapsed = elapsed.as_secs_f64();

    for index in 0..star_count {
        let seed = index as u64 ^ 0xA5A5_6C8E_9CF5_703B;
        let angle = unit_random(seed) * std::f64::consts::TAU;
        let phase = unit_random(seed ^ 0x6E62_4EB7_F3A1_92D1);
        let speed = 0.75 + unit_random(seed ^ 0xD1B5_4A32_D192_ED03) * 0.5;
        let progress = (phase + elapsed / STAR_TRAVEL_SECS * speed).fract();
        let direction_x = angle.cos();
        let direction_y = angle.sin();
        let radius_x = center_x / direction_x.abs().max(0.001);
        let radius_y = center_y * 2.0 / direction_y.abs().max(0.001);
        let max_radius = radius_x.min(radius_y);
        let radius = 1.0 + progress.powf(1.4) * (max_radius - 1.0).max(0.0);
        let x = (center_x + direction_x * radius).round() as usize;
        let y = (center_y + direction_y * radius / 2.0).round() as usize;
        if x >= width || y >= height {
            continue;
        }

        let glyph = if progress < 0.7 {
            LogoGlyph::Dot
        } else if progress < 0.9 {
            LogoGlyph::Plus
        } else {
            LogoGlyph::Star
        };
        frame.set(
            x,
            y,
            LogoCell {
                surface: LogoSurface::Background,
                glyph,
            },
        );
    }
}

fn star_count_for_size(size: TuiSize) -> usize {
    let area = u64::from(size.width) * u64::from(size.height);
    let scaled_count = area * STARFIELD_REFERENCE_COUNT as u64 / STARFIELD_REFERENCE_AREA as u64;
    usize::try_from(scaled_count)
        .expect("u16 terminal dimensions produce a star count that fits usize")
        .clamp(STARFIELD_MIN_COUNT, STARFIELD_CANDIDATE_BUDGET)
}
fn unit_random(seed: u64) -> f64 {
    let mut value = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((value ^ (value >> 31)) >> 11) as f64 / (1_u64 << 53) as f64
}

fn fitted_logo_size(size: TuiSize, cell_aspect_ratio: f64) -> Option<(u16, u16)> {
    if size.width < MIN_ANIMATION_COLS || size.height < MIN_ANIMATION_ROWS {
        return None;
    }

    let available_cols = size.width.saturating_sub(2);
    let available_rows = size.height.saturating_sub(2);
    let max_rows = available_rows.min(MAX_LOGO_ROWS);
    let natural_cols = (f64::from(max_rows) * cell_aspect_ratio).round() as u16;
    if natural_cols <= available_cols {
        return Some((natural_cols.max(MIN_OBJECT_COLS), max_rows));
    }

    let cols = available_cols;
    let fitted_rows = (f64::from(cols) / cell_aspect_ratio).round() as u16;
    if fitted_rows < MIN_OBJECT_ROWS {
        // Preserve visibility when an extreme horizontal aspect ratio would
        // otherwise round the height to zero.
        return Some((cols, MIN_OBJECT_ROWS));
    }

    let rows = fitted_rows.min(max_rows);
    let cols = ((f64::from(rows) * cell_aspect_ratio).round() as u16)
        .min(cols)
        .max(MIN_OBJECT_COLS);
    Some((cols, rows))
}

fn sample_coordinate(index: usize, sample_count: usize) -> f64 {
    ((index as f64 + 0.5) / sample_count as f64) * 2.0 - 1.0
}
fn logo_outline_glyph(
    shape: &ZeroStateShape,
    x: f64,
    y: f64,
    source_cols: usize,
    source_rows: usize,
    rotation_cos: f64,
    cell_aspect_ratio: f64,
) -> Option<LogoGlyph> {
    let dx = 2.0 / source_cols as f64;
    let dy = 2.0 / source_rows as f64;
    let left = shape.contains(x - dx, y);
    let right = shape.contains(x + dx, y);
    let above = shape.contains(x, y - dy);
    let below = shape.contains(x, y + dy);
    if left && right && above && below {
        return None;
    }

    let normal_x = bool_as_scalar(left) - bool_as_scalar(right);
    let normal_y = bool_as_scalar(above) - bool_as_scalar(below);
    let tangent_x = -normal_y * rotation_cos * cell_aspect_ratio;
    let tangent_y = normal_x;
    if tangent_x.abs() > tangent_y.abs() * 1.8 {
        Some(LogoGlyph::Horizontal)
    } else if tangent_y.abs() > tangent_x.abs() * 1.8 {
        Some(LogoGlyph::Vertical)
    } else if tangent_x.signum() == tangent_y.signum() {
        Some(LogoGlyph::Backslash)
    } else {
        Some(LogoGlyph::ForwardSlash)
    }
}

fn bool_as_scalar(value: bool) -> f64 {
    if value { 1.0 } else { 0.0 }
}

fn warp_logo_contains(x: f64, y: f64) -> bool {
    let svg_x = SVG_MIN_X + (x + 1.0) * 0.5 * (SVG_MAX_X - SVG_MIN_X);
    let svg_y = SVG_MIN_Y + (y + 1.0) * 0.5 * (SVG_MAX_Y - SVG_MIN_Y);
    point_in_polygon(svg_x, svg_y, UPPER_FACE) || point_in_polygon(svg_x, svg_y, LOWER_FACE)
}

fn point_in_polygon(x: f64, y: f64, polygon: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let (current_x, current_y) = polygon[current];
        let (previous_x, previous_y) = polygon[previous];
        if ((current_y > y) != (previous_y > y))
            && x < (previous_x - current_x) * (y - current_y) / (previous_y - current_y) + current_x
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

#[cfg(test)]
#[path = "zero_state_animation_tests.rs"]
mod tests;
