//! ASCII art portrait animation for the TUI zero state.
//!
//! The right-hand panel of the zero state renders an ASCII art portrait of
//! Kevin Yang, mapped through a 10-level luminance ramp (`" .:-=+*#@█"`) and
//! animated with three layered effects at ~30 fps:
//!
//!   (a) **Scanline shimmer** — a bright horizontal band sweeping top → bottom,
//!       repeating every two seconds.
//!   (b) **Glyph flicker** — roughly 4 % of portrait cells swap to an adjacent
//!       ramp character per frame.
//!   (c) **Accent highlight** — roughly 1.5 % of bright cells briefly take the
//!       terminal accent colour.
//!
//! The portrait is derived analytically from the panel dimensions on every
//! render, so it reflows automatically on resize without any per-frame state.
//!
//! [`ZeroStateAnimationElement`] is owned by [`super::TuiZeroStateView`]
//! and re-created on each render pass from the shared [`AnimationClock`],
//! keeping layout stateless while the clock ensures a continuous animation.

use std::time::Duration;

use warpui_core::AppContext;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{
    Color, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiScreenPoint, TuiScreenPosition, TuiSize, TuiStyle,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Target repaint cadence (~30 fps).
const REPAINT_INTERVAL: Duration = Duration::from_millis(33);

/// Minimum panel dimensions for the animation to render.
pub(crate) const MIN_ANIMATION_COLS: u16 = 20;
pub(crate) const MIN_ANIMATION_ROWS: u16 = 10;

/// 10-level luminance ramp: index 0 = darkest (space), index 9 = brightest (█).
const RAMP: &[&str] = &[" ", ".", ":", "-", "=", "+", "*", "#", "@", "█"];

// ---------------------------------------------------------------------------
// Portrait geometry
// ---------------------------------------------------------------------------

/// Returns the base luminance index (0–9) for the portrait at normalised panel
/// coordinates.
///
/// - `nr` is the row normalised to `[-1.0, 1.0]` (−1 = top, +1 = bottom).
/// - `nc` is the column normalised to `[-1.0, 1.0]` (−1 = left, +1 = right).
/// - `row` and `col` are the raw integer grid coordinates, used for per-cell
///   texture noise.
///
/// # Portrait anatomy (luminance values)
///
/// | Region            | Chars  | Luminance |
/// |-------------------|--------|-----------|
/// | Ivy background    | `.:- ` | 0–3       |
/// | Suit jacket       | ` .`   | 0–1       |
/// | Tie               | `-=`   | 3–4       |
/// | Neck / collar     | `=+.:` | 1–5       |
/// | Face skin         | `+*`   | 5–6       |
/// | Glasses frame     | `#`    | 7         |
/// | Glasses lens      | `-=`   | 3–4       |
/// | Hair              | `#@`   | 7–8       |
pub(crate) fn portrait_luminance(nr: f64, nc: f64, row: i32, col: i32) -> u8 {
    // ---- Suit region (lower portion of portrait) ----
    // This early return clips the bottom of the face oval so the suit starts
    // at roughly the collar line.
    if nr > 0.55 {
        if nc.abs() < 0.10 {
            // Tie: a narrow vertical stripe in the centre of the suit.
            let n = noise2(row, col);
            return 3 + n; // '-' or '='
        }
        // Suit jacket (very dark).
        return noise2(row, col); // ' ' or '.'
    }

    // ---- Face oval ----
    // Semi-axes: nc 0.45 (horizontal), nr 0.80 (vertical).
    // The large vertical extent compensates for terminal cells being ~2× taller
    // than wide, so the oval reads as a natural portrait shape on screen.
    let fc = nc / 0.45;
    let fr = nr / 0.80;
    let face_dist_sq = fc * fc + fr * fr;

    if face_dist_sq < 1.0 {
        let n = noise2(row, col);

        // Collar / upper lapel (bottom of face oval, above suit boundary).
        if nr > 0.42 && nc.abs() < 0.22 {
            return (1 + n).min(2); // '.' or ':'
        }
        // Neck (narrow vertical band just above the collar).
        if nr > 0.30 && nc.abs() < 0.14 {
            return 4 + n; // '=' or '+'
        }
        // Glasses band: a horizontal swath forming distinct character bands.
        // nr ∈ [−0.32, −0.06] covers both frames and the nose bridge.
        if nr >= -0.32 && nr <= -0.06 {
            // Two lens ovals — left eye nc ≈ −0.18, right eye nc ≈ +0.18.
            let left_lc = (nc + 0.18) / 0.14;
            let right_lc = (nc - 0.18) / 0.14;
            let lens_rc = (nr + 0.185) / 0.10;
            let left_dist = left_lc * left_lc + lens_rc * lens_rc;
            let right_dist = right_lc * right_lc + lens_rc * lens_rc;
            if left_dist < 1.0 || right_dist < 1.0 {
                // Lens interior: distinctly darker than surrounding skin.
                return 3 + n; // '-' or '='
            }
            // Frame and nose bridge: bright '#' forming the distinctive bands.
            return 7;
        }
        // Subtle mouth shadow.
        if nr > 0.18 && nr < 0.32 && nc.abs() < 0.24 {
            return 4 + n; // '=' or '+'
        }
        // General face skin.
        return 5 + n; // '+' or '*'
    }

    // ---- Hair ----
    // The region just outside the face oval in the upper half of the portrait.
    if face_dist_sq < 1.45 && nr < 0.20 {
        return 7 + noise2(row, col); // '#' or '@'
    }

    // ---- Background ivy texture ----
    ivy_luminance(row, col)
}

/// Deterministic 1-bit per-cell texture noise: returns 0 or 1.
#[inline]
pub(crate) fn noise2(row: i32, col: i32) -> u8 {
    ((row.wrapping_mul(7).wrapping_add(col.wrapping_mul(13))) as u64 & 1) as u8
}

/// Repeating `:.-` ivy-texture luminance (returns 1–3).
pub(crate) fn ivy_luminance(row: i32, col: i32) -> u8 {
    const PATTERN: [u8; 4] = [2, 1, 3, 1]; // ':', '.', '-', '.'
    let idx = (row.wrapping_add(col.wrapping_mul(3))).unsigned_abs() % 4;
    PATTERN[idx as usize]
}

// ---------------------------------------------------------------------------
// Per-frame cell hash (drives animation effects)
// ---------------------------------------------------------------------------

/// Fast deterministic hash of row, column, and frame index.
///
/// Used by all three animation effects to select which cells to modify each
/// frame without storing per-cell state.
pub(crate) fn cell_hash(row: u64, col: u64, frame: u64) -> u64 {
    let mut h = row ^ col.wrapping_shl(16) ^ frame.wrapping_shl(32);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h
}

// ---------------------------------------------------------------------------
// ZeroStateAnimationElement
// ---------------------------------------------------------------------------

/// Renders the Kevin Yang ASCII art portrait with scanline shimmer, glyph
/// flicker, and accent highlight animation effects.
pub struct ZeroStateAnimationElement {
    clock: AnimationClock,
    accent_color: Color,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl ZeroStateAnimationElement {
    pub(crate) fn new(clock: AnimationClock, accent_color: Color) -> Self {
        Self {
            clock,
            accent_color,
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
        if size.width < MIN_ANIMATION_COLS || size.height < MIN_ANIMATION_ROWS {
            return;
        }

        let cols = usize::from(size.width);
        let rows = usize::from(size.height);
        let elapsed = self.clock.elapsed().as_secs_f64();

        // (a) Scanline position: sweeps top → bottom every 2 seconds.
        let scanline_row = ((elapsed * 0.5).fract() * rows as f64) as usize;

        // Frame index for per-frame RNG (~30 fps).
        let frame = (elapsed * 30.0) as u64;

        let accent_color = self.accent_color;

        for row in 0..rows {
            for col in 0..cols {
                // Normalise grid coordinates to [−1, 1] with (0, 0) at centre.
                let half_rows = rows as f64 * 0.5;
                let half_cols = cols as f64 * 0.5;
                let nr = (row as f64 + 0.5 - half_rows) / half_rows;
                let nc = (col as f64 + 0.5 - half_cols) / half_cols;

                let base_lum = portrait_luminance(nr, nc, row as i32, col as i32);

                // (a) Scanline shimmer: boost luminance within ±1 row of the
                //     current scanline position.
                let shimmer = match row.abs_diff(scanline_row) {
                    0 => 2u8,
                    1 => 1,
                    _ => 0,
                };
                let mut lum = base_lum.saturating_add(shimmer).min(9);

                // (b) Glyph flicker: ~4 % of cells shift ±1 ramp level.
                let h = cell_hash(row as u64, col as u64, frame);
                if h % 25 == 0 {
                    if h & (1 << 16) != 0 {
                        lum = lum.saturating_sub(1);
                    } else {
                        lum = (lum + 1).min(9);
                    }
                }

                // (c) Accent highlight: ~1.5 % of bright cells get accent colour.
                let use_accent = lum >= 6 && h % 67 == 1;

                let glyph = RAMP[usize::from(lum)];
                let style = if use_accent {
                    TuiStyle::default().fg(accent_color)
                } else {
                    TuiStyle::default()
                };

                if let Some(cell) =
                    surface.cell_mut(origin.offset(col as i32, row as i32))
                {
                    cell.set_symbol(glyph).set_style(style);
                }
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
#[path = "zero_state_animation_tests.rs"]
mod tests;
