//! Starfield animation for the TUI zero state.
//!
//! 220 stars radiate outward from the terminal centre, streaking as they
//! accelerate toward the screen edge — a "warp speed" effect driven by an
//! analytically-computed spring-based speed ramp.
//!
//! Animation state ([`StarfieldState`]) is owned by [`super::TuiZeroStateView`]
//! and shared with [`ZeroStateAnimationElement`] via an `Rc<RefCell<…>>`, so
//! the animation continues uninterrupted across view re-renders (e.g. when MCP
//! connects or a changelog loads).

use std::cell::RefCell;
use std::rc::Rc;
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

/// Target repaint cadence (~30 fps — smooth and low-CPU for a background FX).
const REPAINT_INTERVAL: Duration = Duration::from_millis(33);

/// Simulation step rate: advance stars this many times per elapsed second.
const SIM_FPS: f64 = 60.0;

/// Number of stars in the field.
pub(crate) const NUM_STARS: usize = 220;

/// ASCII glyph ramp: dim near centre, bright near the edge.
const STAR_GLYPHS: &[u8] = b".:-=+*#@";

/// Warp factor the spring starts at.
pub(crate) const WARP_INITIAL: f64 = 0.15;

/// Warp factor the spring converges to.
pub(crate) const WARP_TARGET: f64 = 0.85;

/// Lavender accent colour for glowing stars (`#D0D1FE`).
const GLOW_R: u8 = 208;
const GLOW_G: u8 = 209;
const GLOW_B: u8 = 254;

/// Minimum width / height for the animation to render.
const MIN_ANIMATION_COLS: u16 = 4;
const MIN_ANIMATION_ROWS: u16 = 2;

// ---------------------------------------------------------------------------
// Warp spring (analytical solution)
// ---------------------------------------------------------------------------
// Underdamped spring with ζ = 0.8, ω = 1.0:
//   ωd = ω · √(1 − ζ²) = √0.36 = 0.6
//
//   warp(t) = target + e^(−ζωt) · [ Δ·cos(ωd·t) + (ζω·Δ/ωd)·sin(ωd·t) ]
// where Δ = initial − target = 0.15 − 0.85 = −0.7

const SPRING_ZETA: f64 = 0.8;
const SPRING_OMEGA: f64 = 1.0;
const SPRING_OMEGA_D: f64 = 0.6;
const SPRING_DELTA: f64 = WARP_INITIAL - WARP_TARGET; // −0.7

pub(crate) fn warp_at(t: f64) -> f64 {
    let envelope = (-SPRING_ZETA * SPRING_OMEGA * t).exp();
    let cos_term = SPRING_DELTA * (SPRING_OMEGA_D * t).cos();
    let sin_term =
        (SPRING_ZETA * SPRING_OMEGA * SPRING_DELTA / SPRING_OMEGA_D) * (SPRING_OMEGA_D * t).sin();
    (WARP_TARGET + envelope * (cos_term + sin_term)).clamp(WARP_INITIAL - 0.01, WARP_TARGET + 0.1)
}

// ---------------------------------------------------------------------------
// Minimal deterministic PRNG (xorshift64)
// ---------------------------------------------------------------------------

pub(crate) fn xorshift64(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

pub(crate) fn xorshift_f64(s: &mut u64) -> f64 {
    (xorshift64(s) >> 11) as f64 / ((1u64 << 53) as f64)
}

// ---------------------------------------------------------------------------
// Star
// ---------------------------------------------------------------------------

pub(crate) struct Star {
    pub(crate) angle: f64,   // radians, direction from centre
    pub(crate) r: f64,       // current radius from centre
    pub(crate) r_prev: f64,
    pub(crate) speed: f64,   // per-star speed multiplier in [0.5, 1.5]
    glow: bool,              // lavender highlight
}

fn new_star(rng: &mut u64, max_r: f64) -> Star {
    let angle = xorshift_f64(rng) * std::f64::consts::TAU;
    let r = xorshift_f64(rng) * max_r * 0.15;
    Star {
        angle,
        r,
        r_prev: r,
        speed: 0.5 + xorshift_f64(rng) * 1.0,
        glow: xorshift_f64(rng) < 0.35,
    }
}

// ---------------------------------------------------------------------------
// StarfieldState — owned by TuiZeroStateView, shared with the element
// ---------------------------------------------------------------------------

pub(crate) struct StarfieldState {
    pub(crate) stars: Vec<Star>,
    rng: u64,
    sim_secs: f64,
    max_r: f64,
}

impl StarfieldState {
    pub(crate) fn new() -> Self {
        let mut rng: u64 = 0xDEAD_BEEF_CAFE_1234;
        // Typical 80×24 maxR for initial placement; updated on first render.
        let init_max_r = 62.5_f64;
        let stars = (0..NUM_STARS)
            .map(|_| new_star(&mut rng, init_max_r))
            .collect();
        Self { stars, rng, sim_secs: 0.0, max_r: init_max_r }
    }

    /// Advances simulation by one 1/60s step.
    fn advance_one_step(&mut self, warp: f64) {
        for star in &mut self.stars {
            star.r_prev = star.r;
            star.r += (0.12 + star.r * 0.012) * star.speed * warp;
            // ~2% chance per step to toggle glow.
            if xorshift_f64(&mut self.rng) < 0.02 {
                star.glow = !star.glow;
            }
            if star.r >= self.max_r {
                *star = new_star(&mut self.rng, self.max_r);
            }
        }
    }

    /// Simulates up to `target_secs` in 1/60s increments.
    pub(crate) fn simulate_to(&mut self, target_secs: f64) {
        let step = 1.0 / SIM_FPS;
        // Cap at 2 seconds worth of steps to avoid stalls after long pauses.
        let max_steps = (SIM_FPS * 2.0) as usize;
        let mut steps = 0;
        while self.sim_secs + step <= target_secs && steps < max_steps {
            let warp = warp_at(self.sim_secs);
            self.advance_one_step(warp);
            self.sim_secs += step;
            steps += 1;
        }
        self.sim_secs = target_secs;
    }

    pub(crate) fn set_max_r(&mut self, max_r: f64) {
        self.max_r = max_r;
    }
}

// ---------------------------------------------------------------------------
// TuiElement
// ---------------------------------------------------------------------------

/// Per-render geometry cache — avoids threading 5 scalars through every call.
struct GridGeometry {
    cx: f64,
    cy: f64,
    max_r: f64,
    cols: usize,
    rows: usize,
}

pub struct ZeroStateAnimationElement {
    state: Rc<RefCell<StarfieldState>>,
    clock: AnimationClock,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl ZeroStateAnimationElement {
    pub(crate) fn new(state: Rc<RefCell<StarfieldState>>, clock: AnimationClock) -> Self {
        Self { state, clock, size: None, origin: None }
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

        let cx = cols as f64 / 2.0;
        let cy = rows as f64 / 2.0;
        // Aspect correction: terminal cells ~2× taller than wide.
        let max_r = cx.hypot(cy * 2.0);

        let mut sf = self.state.borrow_mut();
        sf.set_max_r(max_r);
        sf.simulate_to(elapsed);

        let geo = GridGeometry { cx, cy, max_r, cols, rows };
        for star in &sf.stars {
            draw_star_streak(surface, origin, star, &geo);
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

// ---------------------------------------------------------------------------
// Star streak renderer
// ---------------------------------------------------------------------------

fn draw_star_streak(
    surface: &mut TuiPaintSurface<'_>,
    origin: TuiScreenPosition,
    star: &Star,
    geo: &GridGeometry,
) {
    if star.r <= 0.0 || geo.max_r <= 0.0 {
        return;
    }
    let (dir_x, dir_y) = (star.angle.cos(), star.angle.sin());
    let length = star.r - star.r_prev;
    let steps = (length as usize).max(1);

    let color = if star.glow {
        Color::Rgb(GLOW_R, GLOW_G, GLOW_B)
    } else {
        Color::Rgb(255, 255, 255)
    };
    let style = TuiStyle::default().fg(color);

    for st in 0..=steps {
        let rr = star.r_prev + length * st as f64 / steps as f64;
        // Y halved for aspect correction.
        let px = (geo.cx + dir_x * rr).round() as i32;
        let py = (geo.cy + dir_y * rr / 2.0).round() as i32;
        if px < 0 || py < 0 || px as usize >= geo.cols || py as usize >= geo.rows {
            continue;
        }
        let gi = ((rr / geo.max_r) * (STAR_GLYPHS.len() - 1) as f64) as usize;
        let gi = gi.min(STAR_GLYPHS.len() - 1);
        let glyph = STAR_GLYPHS[gi];
        let buf = [glyph];
        let s = std::str::from_utf8(&buf).unwrap_or(".");
        if let Some(c) = surface.cell_mut(origin.offset(px, py)) {
            c.set_symbol(s).set_style(style);
        }
    }
}

#[cfg(test)]
#[path = "zero_state_animation_tests.rs"]
mod tests;
