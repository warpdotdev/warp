//! Procedural geometry for supported solid Unicode box-drawing glyphs.

use smallvec::SmallVec;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::vec2f;

/// Weight of a box-drawing stroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Weight {
    Light,
    Heavy,
}

/// The optional weighted arms of a box-drawing glyph.
#[derive(Debug, Clone, Copy, Default)]
struct Edges {
    up: Option<Weight>,
    down: Option<Weight>,
    left: Option<Weight>,
    right: Option<Weight>,
}

impl Edges {
    /// Creates box-drawing edge geometry.
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

    /// Returns whether the glyph has a vertical arm.
    fn has_vertical(&self) -> bool {
        self.up.is_some() || self.down.is_some()
    }

    /// Returns whether the glyph has a horizontal arm.
    fn has_horizontal(&self) -> bool {
        self.left.is_some() || self.right.is_some()
    }

    /// Returns the heavier vertical-arm weight.
    fn vertical_weight(&self) -> Weight {
        heavier(self.up, self.down)
    }

    /// Returns the heavier horizontal-arm weight.
    fn horizontal_weight(&self) -> Weight {
        heavier(self.left, self.right)
    }
}

const L: Option<Weight> = Some(Weight::Light);
const H: Option<Weight> = Some(Weight::Heavy);
const N: Option<Weight> = None;

/// Stroke thicknesses derived from a box-drawing cell's nominal device-pixel size.
#[derive(Debug, Clone, Copy)]
pub(super) struct StrokeMetrics {
    light: f32,
    heavy: f32,
}

impl StrokeMetrics {
    /// Computes stable light and heavy stroke thicknesses.
    pub(super) fn new(cell_width: f32, cell_height: f32) -> Self {
        let light = (cell_width.min(cell_height) / 8.0).round().max(1.0);
        Self {
            light,
            heavy: (light * 2.0).max(light + 1.0),
        }
    }

    /// Returns the thickness for a box-drawing weight.
    fn thickness(self, weight: Weight) -> f32 {
        match weight {
            Weight::Light => self.light,
            Weight::Heavy => self.heavy,
        }
    }
}

/// Returns the heavier of two optional weights.
fn heavier(a: Option<Weight>, b: Option<Weight>) -> Weight {
    if a == Some(Weight::Heavy) || b == Some(Weight::Heavy) {
        Weight::Heavy
    } else {
        Weight::Light
    }
}

/// Returns whether `c` has supported procedural box-drawing geometry.
pub(super) fn is_supported(c: char) -> bool {
    edges(c).is_some()
}

/// Computes non-overlapping, cell-local rectangles for a box-drawing glyph.
pub(super) fn rects(
    c: char,
    cell_width: f32,
    cell_height: f32,
    metrics: StrokeMetrics,
) -> SmallVec<[RectF; 3]> {
    let mut rects = SmallVec::new();
    if cell_width <= 0.0 || cell_height <= 0.0 {
        return rects;
    }

    if let Some(edges) = edges(c) {
        push_line_rects(&mut rects, edges, cell_width, cell_height, metrics);
    }

    debug_assert!(
        !rects_overlap(&rects),
        "box drawing produced overlapping rects for {c:?}"
    );
    rects
}

/// Pushes a non-degenerate cell-local rectangle.
fn push_rect(rects: &mut SmallVec<[RectF; 3]>, x0: f32, y0: f32, x1: f32, y1: f32) {
    if x1 > x0 && y1 > y0 {
        rects.push(RectF::new(vec2f(x0, y0), vec2f(x1 - x0, y1 - y0)));
    }
}

/// Returns a centered device-pixel-aligned band.
fn band(center: f32, thickness: f32) -> (f32, f32) {
    let low = (center - thickness / 2.0).round();
    (low, low + thickness)
}

/// Draws a straight glyph whose two opposing arms have different weights.
fn push_straight_transition_rects(
    rects: &mut SmallVec<[RectF; 3]>,
    edges: Edges,
    width: f32,
    height: f32,
    metrics: StrokeMetrics,
) -> bool {
    if !edges.has_horizontal() {
        if let (Some(up), Some(down)) = (edges.up, edges.down) {
            if up != down {
                let center_x = width / 2.0;
                let split_y = (height / 2.0).round();
                let (up_low, up_high) = band(center_x, metrics.thickness(up));
                let (down_low, down_high) = band(center_x, metrics.thickness(down));
                push_rect(rects, up_low.max(0.0), 0.0, up_high.min(width), split_y);
                push_rect(
                    rects,
                    down_low.max(0.0),
                    split_y,
                    down_high.min(width),
                    height,
                );
                return true;
            }
        }
    }

    if !edges.has_vertical() {
        if let (Some(left), Some(right)) = (edges.left, edges.right) {
            if left != right {
                let center_y = height / 2.0;
                let split_x = (width / 2.0).round();
                let (left_low, left_high) = band(center_y, metrics.thickness(left));
                let (right_low, right_high) = band(center_y, metrics.thickness(right));
                push_rect(rects, 0.0, left_low, split_x, left_high);
                push_rect(rects, split_x, right_low, width, right_high);
                return true;
            }
        }
    }

    false
}

/// Appends the disjoint rectangles making up a box-drawing glyph.
fn push_line_rects(
    rects: &mut SmallVec<[RectF; 3]>,
    edges: Edges,
    width: f32,
    height: f32,
    metrics: StrokeMetrics,
) {
    if push_straight_transition_rects(rects, edges, width, height, metrics) {
        return;
    }
    let center_x = width / 2.0;
    let center_y = height / 2.0;

    if edges.has_vertical() {
        let (vertical_low, vertical_high) =
            band(center_x, metrics.thickness(edges.vertical_weight()));
        let (horizontal_low, horizontal_high) =
            band(center_y, metrics.thickness(edges.horizontal_weight()));
        let top = if edges.up.is_some() {
            0.0
        } else {
            horizontal_low
        };
        let bottom = if edges.down.is_some() {
            height
        } else {
            horizontal_high
        };
        push_rect(
            rects,
            vertical_low.max(0.0),
            top,
            vertical_high.min(width),
            bottom,
        );

        // Keep horizontal arms outside the vertical band so translucent colors
        // are not composited twice at junctions.
        if edges.left.is_some() {
            push_rect(rects, 0.0, horizontal_low, vertical_low, horizontal_high);
        }
        if edges.right.is_some() {
            push_rect(rects, vertical_high, horizontal_low, width, horizontal_high);
        }
    } else if edges.has_horizontal() {
        let line_thickness = metrics.thickness(edges.horizontal_weight());
        let (horizontal_low, horizontal_high) = band(center_y, line_thickness);
        let (center_low, center_high) = band(center_x, line_thickness);
        let left = if edges.left.is_some() {
            0.0
        } else {
            center_low
        };
        let right = if edges.right.is_some() {
            width
        } else {
            center_high
        };
        push_rect(
            rects,
            left,
            horizontal_low,
            right.min(width),
            horizontal_high,
        );
    }
}

/// Returns the arms for a supported solid box-drawing glyph.
fn edges(c: char) -> Option<Edges> {
    let edge = |up, down, left, right| Some(Edges::new(up, down, left, right));
    match c as u32 {
        // Horizontals and verticals.
        0x2500 => edge(N, N, L, L), // ─
        0x2501 => edge(N, N, H, H), // ━
        0x2502 => edge(L, L, N, N), // │
        0x2503 => edge(H, H, N, N), // ┃
        // Corners, including mixed weights.
        0x250C => edge(N, L, N, L), // ┌
        0x250D => edge(N, L, N, H), // ┍
        0x250E => edge(N, H, N, L), // ┎
        0x250F => edge(N, H, N, H), // ┏
        0x2510 => edge(N, L, L, N), // ┐
        0x2511 => edge(N, L, H, N), // ┑
        0x2512 => edge(N, H, L, N), // ┒
        0x2513 => edge(N, H, H, N), // ┓
        0x2514 => edge(L, N, N, L), // └
        0x2515 => edge(L, N, N, H), // ┕
        0x2516 => edge(H, N, N, L), // ┖
        0x2517 => edge(H, N, N, H), // ┗
        0x2518 => edge(L, N, L, N), // ┘
        0x2519 => edge(L, N, H, N), // ┙
        0x251A => edge(H, N, L, N), // ┚
        0x251B => edge(H, N, H, N), // ┛
        // Pure-weight tees and crosses.
        0x251C => edge(L, L, N, L), // ├
        0x2523 => edge(H, H, N, H), // ┣
        0x2524 => edge(L, L, L, N), // ┤
        0x252B => edge(H, H, H, N), // ┫
        0x252C => edge(N, L, L, L), // ┬
        0x2533 => edge(N, H, H, H), // ┳
        0x2534 => edge(L, N, L, L), // ┴
        0x253B => edge(H, N, H, H), // ┻
        0x253C => edge(L, L, L, L), // ┼
        0x254B => edge(H, H, H, H), // ╋
        // Stubs and light/heavy transitions.
        0x2574 => edge(N, N, L, N), // ╴
        0x2575 => edge(L, N, N, N), // ╵
        0x2576 => edge(N, N, N, L), // ╶
        0x2577 => edge(N, L, N, N), // ╷
        0x2578 => edge(N, N, H, N), // ╸
        0x2579 => edge(H, N, N, N), // ╹
        0x257A => edge(N, N, N, H), // ╺
        0x257B => edge(N, H, N, N), // ╻
        0x257C => edge(N, N, L, H), // ╼
        0x257D => edge(L, H, N, N), // ╽
        0x257E => edge(N, N, H, L), // ╾
        0x257F => edge(H, L, N, N), // ╿
        _ => None,
    }
}

/// Returns whether any rectangles overlap over a positive area.
fn rects_overlap(rects: &[RectF]) -> bool {
    for (index, a) in rects.iter().enumerate() {
        for b in &rects[index + 1..] {
            let a_origin = a.origin();
            let b_origin = b.origin();
            let overlaps_x = a_origin.x().max(b_origin.x())
                < (a_origin.x() + a.width()).min(b_origin.x() + b.width());
            let overlaps_y = a_origin.y().max(b_origin.y())
                < (a_origin.y() + a.height()).min(b_origin.y() + b.height());
            if overlaps_x && overlaps_y {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "box_drawing_tests.rs"]
mod tests;
