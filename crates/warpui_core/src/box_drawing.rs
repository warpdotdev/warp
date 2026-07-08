//! Procedural geometry for Unicode box-drawing (`U+2500..=U+257F`) and block
//! (`U+2580..=U+259F`) glyphs.
//!
//! # Why this exists
//!
//! When these characters are rendered from a font (like any other glyph), the
//! font's ink for a vertical bar (`│`) is not guaranteed to fill the full cell
//! height, and the glyph pipeline snaps each glyph to the pixel grid
//! independently. Stacked box-drawing characters therefore show thin horizontal
//! seams between vertically-adjacent cells (visible especially when zoomed in).
//!
//! Instead of rasterizing these codepoints from the font, we draw them
//! procedurally as filled rectangles that span the **entire** cell along the
//! stroke axis. Because every cell's bar spans the full cell and cells tile
//! exactly (the same way cell backgrounds / cursors / underlines already do via
//! the radius-0 rect pipeline), vertically- and horizontally-adjacent glyphs
//! abut with zero gap at any zoom/resolution. This mirrors how terminals such as
//! ghostty, kitty, and alacritty render these characters.
//!
//! # Non-overlapping invariant
//!
//! The rectangles returned for a single glyph are **mutually non-overlapping**:
//! no device pixel is covered by more than one rectangle. This is required for
//! correctness with a **non-opaque foreground color** — overlapping
//! semi-transparent rectangles would composite to a different (darker) color at
//! the overlap (e.g. the crossing of `┼`). The junction decomposition below
//! partitions the cell so the vertical band owns the center and the horizontal
//! arms are clipped to lie strictly outside it.

use std::sync::atomic::{AtomicBool, Ordering};

use smallvec::SmallVec;

use crate::geometry::rect::RectF;
use crate::geometry::vector::vec2f;

/// Process-global toggle for the procedural box-drawing renderer.
///
/// `warpui_core` does not depend on the feature-flag crate, so the app sets this
/// once at startup from `FeatureFlag::BoxDrawingGlyphs` (see
/// `app/src/features.rs`). The paint path reads it via [`is_enabled`].
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Enables or disables procedural box-drawing rendering process-wide.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether procedural box-drawing rendering is currently enabled.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// A cell-local rectangle for a box-drawing/block glyph.
///
/// `bounds` is expressed in **device** pixels with its origin at the top-left of
/// the character cell. The caller passes the cell size in device pixels (snapped
/// to the integer pixel grid) and offsets/scales the result back to logical
/// coordinates. `alpha_scale` (in `0.0..=1.0`) is multiplied into the foreground
/// color's alpha and is used to render the shade characters (`░ ▒ ▓`); it is
/// `1.0` for solid strokes and blocks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellRect {
    pub bounds: RectF,
    pub alpha_scale: f32,
}

/// Weight of a box-drawing stroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Weight {
    Light,
    Heavy,
}

/// The four arms of a box-drawing line glyph, each optionally present with a
/// given [`Weight`].
#[derive(Debug, Clone, Copy, Default)]
struct Edges {
    up: Option<Weight>,
    down: Option<Weight>,
    left: Option<Weight>,
    right: Option<Weight>,
}

impl Edges {
    const fn new(
        up: Option<Weight>,
        down: Option<Weight>,
        left: Option<Weight>,
        right: Option<Weight>,
    ) -> Self {
        Self {
            up,
            down,
            left,
            right,
        }
    }

    fn has_vertical(&self) -> bool {
        self.up.is_some() || self.down.is_some()
    }

    fn has_horizontal(&self) -> bool {
        self.left.is_some() || self.right.is_some()
    }

    /// The thickness weight to use for the vertical band (the heavier of the two
    /// vertical arms, defaulting to light).
    fn vertical_weight(&self) -> Weight {
        heavier(self.up, self.down)
    }

    /// The thickness weight to use for the horizontal band.
    fn horizontal_weight(&self) -> Weight {
        heavier(self.left, self.right)
    }
}

fn heavier(a: Option<Weight>, b: Option<Weight>) -> Weight {
    if a == Some(Weight::Heavy) || b == Some(Weight::Heavy) {
        Weight::Heavy
    } else {
        Weight::Light
    }
}

const L: Option<Weight> = Some(Weight::Light);
const H: Option<Weight> = Some(Weight::Heavy);
const N: Option<Weight> = None;

/// Returns `true` if this crate renders `c` procedurally (so the caller should
/// bypass font rasterization for it).
///
/// Note that this is intentionally a subset of the full box-drawing/block
/// ranges: dashed lines, double lines, rounded corners and diagonals are **not**
/// handled here and fall back to the font (they either can't be expressed as
/// axis-aligned rectangles or are a planned follow-up); they do not exhibit the
/// vertical-run seam this module fixes.
pub fn is_supported(c: char) -> bool {
    line_edges(c).is_some() || is_supported_block(c)
}

/// Computes the non-overlapping, cell-local rectangles for a supported glyph.
///
/// `cell_width` and `cell_height` are in **device** pixels and are expected to be
/// whole numbers snapped to the pixel grid, so that vertically- and
/// horizontally-adjacent cells (which share the same snapped edges) tile with no
/// gap. Returns an empty [`SmallVec`] if `c` is not supported (see
/// [`is_supported`]) or if the cell has no area.
pub fn cell_rects(c: char, cell_width: f32, cell_height: f32) -> SmallVec<[CellRect; 8]> {
    let mut out = SmallVec::new();
    if cell_width <= 0.0 || cell_height <= 0.0 {
        return out;
    }

    if let Some(edges) = line_edges(c) {
        push_line_rects(&mut out, edges, cell_width, cell_height);
    } else {
        push_block_rects(&mut out, c, cell_width, cell_height);
    }

    debug_assert!(
        !rects_overlap(&out),
        "box_drawing produced overlapping rects for {c:?}"
    );
    out
}

/// Pushes a rectangle spanning `[x0, x1] x [y0, y1]` (logical, cell-local),
/// skipping degenerate rectangles.
fn push_rect(out: &mut SmallVec<[CellRect; 8]>, x0: f32, y0: f32, x1: f32, y1: f32, alpha: f32) {
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    out.push(CellRect {
        bounds: RectF::new(vec2f(x0, y0), vec2f(x1 - x0, y1 - y0)),
        alpha_scale: alpha,
    });
}

/// The device-pixel thickness for a stroke weight, given the cell size (in
/// device pixels). Always at least one device pixel.
fn thickness_device(weight: Weight, cell_width: f32, cell_height: f32) -> f32 {
    let light = (cell_width.min(cell_height) / 8.0).round().max(1.0);
    match weight {
        Weight::Light => light,
        // Heavy strokes are ~2x a light stroke, but always at least one device
        // pixel thicker so the two weights are visually distinct.
        Weight::Heavy => (light * 2.0).max(light + 1.0),
    }
}

/// Returns the `[lo, hi]` extent (device pixels) of a band of the given
/// thickness, centered on `center` and snapped to the integer pixel grid so the
/// stroke stays crisp.
fn band(center: f32, thickness: f32) -> (f32, f32) {
    let lo = (center - thickness / 2.0).round();
    (lo, lo + thickness)
}

fn push_line_rects(out: &mut SmallVec<[CellRect; 8]>, edges: Edges, w: f32, h: f32) {
    let cx = w / 2.0;
    let cy = h / 2.0;

    if edges.has_vertical() {
        let tv = thickness_device(edges.vertical_weight(), w, h);
        let (vlo, vhi) = band(cx, tv);
        // The horizontal band is needed both to cap a vertical-only stub at the
        // center and to place the horizontal arms.
        let th = thickness_device(edges.horizontal_weight(), w, h);
        let (hlo, hhi) = band(cy, th);

        // The vertical band spans the full cell height when both arms are
        // present; otherwise it stops at the center (capped by the horizontal
        // band so a lone stub still reaches the middle).
        let top = if edges.up.is_some() { 0.0 } else { hlo };
        let bot = if edges.down.is_some() { h } else { hhi };
        push_rect(out, vlo.max(0.0), top, vhi.min(w), bot, 1.0);

        // Horizontal arms are clipped to lie strictly OUTSIDE the vertical band,
        // so they never overlap it (required for non-opaque colors).
        if edges.left.is_some() {
            push_rect(out, 0.0, hlo, vlo, hhi, 1.0);
        }
        if edges.right.is_some() {
            push_rect(out, vhi, hlo, w, hhi, 1.0);
        }
    } else if edges.has_horizontal() {
        // Pure horizontal glyph: a single rect owns the center, spanning from
        // the relevant edge(s) to the center for a stub, or edge-to-edge.
        let th = thickness_device(edges.horizontal_weight(), w, h);
        let (hlo, hhi) = band(cy, th);
        let (clo, chi) = band(cx, th);
        let left = if edges.left.is_some() { 0.0 } else { clo };
        let right = if edges.right.is_some() { w } else { chi };
        push_rect(out, left, hlo, right.min(w), hhi, 1.0);
    }
}

/// The `(up, down, left, right)` arms for a supported box-drawing line glyph, or
/// `None` if `c` is not a supported line glyph.
///
/// Supported: solid light/heavy horizontals & verticals, all corners, the pure
/// light/heavy tees & crosses, and the light/heavy stubs & transitions. Dashed,
/// double, rounded and diagonal glyphs are intentionally excluded.
fn line_edges(c: char) -> Option<Edges> {
    let e = |up, down, left, right| Some(Edges::new(up, down, left, right));
    match c as u32 {
        // Horizontals / verticals.
        0x2500 => e(N, N, L, L), // ─
        0x2501 => e(N, N, H, H), // ━
        0x2502 => e(L, L, N, N), // │
        0x2503 => e(H, H, N, N), // ┃
        // Corners (including mixed weights).
        0x250C => e(N, L, N, L), // ┌
        0x250D => e(N, L, N, H), // ┍
        0x250E => e(N, H, N, L), // ┎
        0x250F => e(N, H, N, H), // ┏
        0x2510 => e(N, L, L, N), // ┐
        0x2511 => e(N, L, H, N), // ┑
        0x2512 => e(N, H, L, N), // ┒
        0x2513 => e(N, H, H, N), // ┓
        0x2514 => e(L, N, N, L), // └
        0x2515 => e(L, N, N, H), // ┕
        0x2516 => e(H, N, N, L), // ┖
        0x2517 => e(H, N, N, H), // ┗
        0x2518 => e(L, N, L, N), // ┘
        0x2519 => e(L, N, H, N), // ┙
        0x251A => e(H, N, L, N), // ┚
        0x251B => e(H, N, H, N), // ┛
        // Pure-weight tees and crosses.
        0x251C => e(L, L, N, L), // ├
        0x2523 => e(H, H, N, H), // ┣
        0x2524 => e(L, L, L, N), // ┤
        0x252B => e(H, H, H, N), // ┫
        0x252C => e(N, L, L, L), // ┬
        0x2533 => e(N, H, H, H), // ┳
        0x2534 => e(L, N, L, L), // ┴
        0x253B => e(H, N, H, H), // ┻
        0x253C => e(L, L, L, L), // ┼
        0x254B => e(H, H, H, H), // ╋
        // Stubs and light/heavy transitions.
        0x2574 => e(N, N, L, N), // ╴
        0x2575 => e(L, N, N, N), // ╵
        0x2576 => e(N, N, N, L), // ╶
        0x2577 => e(N, L, N, N), // ╷
        0x2578 => e(N, N, H, N), // ╸
        0x2579 => e(H, N, N, N), // ╹
        0x257A => e(N, N, N, H), // ╺
        0x257B => e(N, H, N, N), // ╻
        0x257C => e(N, N, L, H), // ╼
        0x257D => e(L, H, N, N), // ╽
        0x257E => e(N, N, H, L), // ╾
        0x257F => e(H, L, N, N), // ╿
        _ => None,
    }
}

fn is_supported_block(c: char) -> bool {
    // The entire Block Elements range is handled by `push_block_rects` (half
    // blocks, eighths, full block, right half, shades, one-eighths, quadrants).
    matches!(c as u32, 0x2580..=0x259F)
}

fn push_block_rects(out: &mut SmallVec<[CellRect; 8]>, c: char, w: f32, h: f32) {
    let cx = w / 2.0;
    let cy = h / 2.0;
    match c as u32 {
        0x2580 => push_rect(out, 0.0, 0.0, w, cy, 1.0), // ▀ upper half
        // ▁..▇ lower eighths (1/8 .. 7/8 tall, anchored to the bottom).
        0x2581..=0x2587 => {
            let eighths = (c as u32 - 0x2580) as f32; // 1..=7
            let top = h * (1.0 - eighths / 8.0);
            push_rect(out, 0.0, top, w, h, 1.0);
        }
        0x2588 => push_rect(out, 0.0, 0.0, w, h, 1.0), // █ full block
        // ▉..▏ left eighths (7/8 .. 1/8 wide, anchored to the left).
        0x2589..=0x258F => {
            let eighths = (8 - (c as u32 - 0x2588)) as f32; // 7..=1
            let right = w * (eighths / 8.0);
            push_rect(out, 0.0, 0.0, right, h, 1.0);
        }
        0x2590 => push_rect(out, cx, 0.0, w, h, 1.0), // ▐ right half
        // ░ ▒ ▓ shades: a full-cell fill at reduced alpha.
        0x2591 => push_rect(out, 0.0, 0.0, w, h, 0.25), // ░
        0x2592 => push_rect(out, 0.0, 0.0, w, h, 0.5),  // ▒
        0x2593 => push_rect(out, 0.0, 0.0, w, h, 0.75), // ▓
        0x2594 => push_rect(out, 0.0, 0.0, w, h / 8.0, 1.0), // ▔ upper one-eighth
        0x2595 => push_rect(out, w * 7.0 / 8.0, 0.0, w, h, 1.0), // ▕ right one-eighth
        // Quadrants (each a union of non-overlapping quarter cells).
        0x2596..=0x259F => push_quadrants(out, c, w, h, cx, cy),
        _ => {}
    }
}

fn push_quadrants(out: &mut SmallVec<[CellRect; 8]>, c: char, w: f32, h: f32, cx: f32, cy: f32) {
    // Bit flags: 1 = top-left, 2 = top-right, 4 = bottom-left, 8 = bottom-right.
    const TL: u8 = 1;
    const TR: u8 = 2;
    const BL: u8 = 4;
    const BR: u8 = 8;
    let mask = match c as u32 {
        0x2596 => BL,           // ▖
        0x2597 => BR,           // ▗
        0x2598 => TL,           // ▘
        0x2599 => TL | BL | BR, // ▙
        0x259A => TL | BR,      // ▚
        0x259B => TL | TR | BL, // ▛
        0x259C => TL | TR | BR, // ▜
        0x259D => TR,           // ▝
        0x259E => TR | BL,      // ▞
        0x259F => TR | BL | BR, // ▟
        _ => return,
    };
    if mask & TL != 0 {
        push_rect(out, 0.0, 0.0, cx, cy, 1.0);
    }
    if mask & TR != 0 {
        push_rect(out, cx, 0.0, w, cy, 1.0);
    }
    if mask & BL != 0 {
        push_rect(out, 0.0, cy, cx, h, 1.0);
    }
    if mask & BR != 0 {
        push_rect(out, cx, cy, w, h, 1.0);
    }
}

/// Whether any two rectangles in `rects` overlap on a shared area. Used only by
/// tests and the debug assertion that guards the non-overlapping invariant.
fn rects_overlap(rects: &[CellRect]) -> bool {
    for (i, a) in rects.iter().enumerate() {
        for b in &rects[i + 1..] {
            let ax0 = a.bounds.origin().x();
            let ay0 = a.bounds.origin().y();
            let ax1 = ax0 + a.bounds.width();
            let ay1 = ay0 + a.bounds.height();
            let bx0 = b.bounds.origin().x();
            let by0 = b.bounds.origin().y();
            let bx1 = bx0 + b.bounds.width();
            let by1 = by0 + b.bounds.height();
            // Positive-area intersection.
            let ix = ax0.max(bx0) < ax1.min(bx1);
            let iy = ay0.max(by0) < ay1.min(by1);
            if ix && iy {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cell dimensions are in device pixels (whole numbers, as the caller snaps
    // them to the pixel grid before calling in).
    const W: f32 = 8.0;
    const H: f32 = 18.0;

    fn rects(c: char) -> SmallVec<[CellRect; 8]> {
        cell_rects(c, W, H)
    }

    fn covers_row(rects: &[CellRect], y: f32) -> bool {
        rects.iter().any(|r| {
            let y0 = r.bounds.origin().y();
            let y1 = y0 + r.bounds.height();
            y >= y0 && y < y1
        })
    }

    fn covers_col(rects: &[CellRect], x: f32) -> bool {
        rects.iter().any(|r| {
            let x0 = r.bounds.origin().x();
            let x1 = x0 + r.bounds.width();
            x >= x0 && x < x1
        })
    }

    #[test]
    fn sprite_vline_fills_cell_height() {
        // The vertical bar of `│` must span the FULL cell height (0..=cell_h) at
        // several cell sizes, so that stacked cells — which share an integer
        // pixel boundary — abut with no gap.
        for (w, h) in [(8.0, 18.0), (10.0, 22.0), (13.0, 30.0), (7.0, 15.0)] {
            let r = cell_rects('│', w, h);
            assert!(!r.is_empty(), "no rects for cell {w}x{h}");
            let top = r
                .iter()
                .map(|r| r.bounds.origin().y())
                .fold(f32::INFINITY, f32::min);
            let bot = r
                .iter()
                .map(|r| r.bounds.origin().y() + r.bounds.height())
                .fold(f32::NEG_INFINITY, f32::max);
            assert_eq!(top, 0.0, "top should be exactly the cell top for {w}x{h}");
            assert_eq!(
                bot, h,
                "bottom should be exactly the cell bottom for {w}x{h}"
            );
        }
    }

    #[test]
    fn sprite_vline_no_seam_when_stacked() {
        // Two `│` cells stacked vertically share the boundary at y == H. Because
        // each bar spans its full cell height exactly, the top cell covers its
        // bottom edge and the bottom cell covers its top edge at the same x, so
        // there is no uncovered row at the seam.
        for (w, h) in [(8.0, 18.0), (10.0, 22.0), (7.0, 15.0)] {
            let cell = cell_rects('│', w, h);
            let stroke_x = w / 2.0;
            assert!(
                covers_row_at(&cell, h - 0.01, stroke_x),
                "cell must cover its bottom edge for {w}x{h}"
            );
            assert!(
                covers_row_at(&cell, 0.0, stroke_x),
                "cell must cover its top edge for {w}x{h}"
            );
        }
    }

    fn covers_row_at(rects: &[CellRect], y: f32, x: f32) -> bool {
        rects.iter().any(|r| {
            let x0 = r.bounds.origin().x();
            let x1 = x0 + r.bounds.width();
            let y0 = r.bounds.origin().y();
            let y1 = y0 + r.bounds.height();
            x >= x0 && x < x1 && y >= y0 && y < y1
        })
    }

    #[test]
    fn sprite_hline_fills_cell_width() {
        // The horizontal bar of `─` must span the full cell width so horizontally
        // adjacent cells connect.
        let r = rects('─');
        let left = r
            .iter()
            .map(|r| r.bounds.origin().x())
            .fold(f32::INFINITY, f32::min);
        let right = r
            .iter()
            .map(|r| r.bounds.origin().x() + r.bounds.width())
            .fold(f32::NEG_INFINITY, f32::max);
        assert_eq!(left, 0.0);
        assert_eq!(right, W);
    }

    #[test]
    fn sprite_cross_covers_all_edges() {
        // `┼` must have ink touching all four cell edges through the centered
        // strokes.
        let r = rects('┼');
        assert!(covers_row(&r, 0.0), "top edge");
        assert!(covers_row(&r, H - 0.01), "bottom edge");
        assert!(covers_col(&r, 0.0), "left edge");
        assert!(covers_col(&r, W - 0.01), "right edge");
    }

    #[test]
    fn sprite_cross_is_non_overlapping() {
        // The crossing of `┼`/`╋` is the canonical overlap risk; verify the
        // partition is disjoint (required for non-opaque foreground colors)
        // across a range of cell sizes.
        for c in ['┼', '╋', '├', '┤', '┬', '┴', '┌', '┐', '└', '┘'] {
            for (w, h) in [(8.0, 18.0), (11.0, 24.0), (7.0, 15.0)] {
                let r = cell_rects(c, w, h);
                assert!(
                    !rects_overlap(&r),
                    "{c:?} produced overlapping rects for cell {w}x{h}"
                );
            }
        }
    }

    #[test]
    fn sprite_heavy_is_thicker_than_light() {
        let light = rects('│');
        let heavy = rects('┃');
        let light_w: f32 = light.iter().map(|r| r.bounds.width()).sum();
        let heavy_w: f32 = heavy.iter().map(|r| r.bounds.width()).sum();
        assert!(
            heavy_w > light_w,
            "heavy bar ({heavy_w}) should be wider than light ({light_w})"
        );
    }

    #[test]
    fn sprite_full_block_fills_cell() {
        let r = rects('█');
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].bounds.origin(), vec2f(0.0, 0.0));
        assert_eq!(r[0].bounds.width(), W);
        assert_eq!(r[0].bounds.height(), H);
        assert_eq!(r[0].alpha_scale, 1.0);
    }

    #[test]
    fn sprite_half_blocks() {
        let upper = rects('▀');
        assert_eq!(upper[0].bounds.height(), H / 2.0);
        assert_eq!(upper[0].bounds.origin().y(), 0.0);

        let lower = rects('▄');
        assert_eq!(lower[0].bounds.origin().y(), H / 2.0);

        let left = rects('▌');
        assert_eq!(left[0].bounds.width(), W / 2.0);

        let right = rects('▐');
        assert_eq!(right[0].bounds.origin().x(), W / 2.0);
    }

    #[test]
    fn sprite_shades_use_alpha() {
        assert_eq!(rects('░')[0].alpha_scale, 0.25);
        assert_eq!(rects('▒')[0].alpha_scale, 0.5);
        assert_eq!(rects('▓')[0].alpha_scale, 0.75);
    }

    #[test]
    fn sprite_quadrants_are_disjoint_and_cover() {
        // `▟` = top-right + bottom-left + bottom-right; three disjoint quarters.
        let r = rects('▟');
        assert_eq!(r.len(), 3);
        assert!(!rects_overlap(&r));
    }

    #[test]
    fn unsupported_glyphs_return_empty() {
        // Double lines, rounded corners, diagonals and plain text are not
        // handled here (they fall back to the font).
        for c in ['═', '║', '╬', '╭', '╱', 'a', ' '] {
            assert!(!super::is_supported(c), "{c:?} should be unsupported");
            assert!(cell_rects(c, W, H).is_empty(), "{c:?} should be empty");
        }
    }

    #[test]
    fn is_supported_matches_cell_rects() {
        for cp in 0x2500u32..=0x259F {
            let c = char::from_u32(cp).unwrap();
            assert_eq!(
                is_supported(c),
                !cell_rects(c, W, H).is_empty(),
                "mismatch for U+{cp:04X} {c:?}"
            );
        }
    }
}
