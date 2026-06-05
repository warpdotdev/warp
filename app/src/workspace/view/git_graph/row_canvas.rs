//! Custom drawing element for a single row of commit-DAG lanes.
//!
//! Because of rendering-layer limitations (`Scene` only has rect / text / icon /
//! image primitives — no line / path / rotation), connections are drawn as
//! **orthogonal polylines**: vertical and horizontal segments are both thin
//! rectangles, and the commit dot is a square with `corner_radius = radius`.
//! Further beautification of rounded polyline corners is left for later polish.

use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use warpui::elements::{Border, CornerRadius, Element, Point, Radius};
use warpui::event::DispatchedEvent;
use warpui::{AppContext, EventContext, LayoutContext, PaintContext, SizeConstraint};

use super::layout::GraphRow;

/// Pixel width of a single lane.
const LANE_WIDTH: f32 = 14.0;
/// Row height (aligned with the text rows on the right; UniformList uses this as
/// the uniform row height).
const ROW_HEIGHT: f32 = 22.0;
/// Line thickness.
const LINE_THICKNESS: f32 = 2.0;
/// Commit dot diameter.
const DOT_DIAMETER: f32 = 8.0;

/// Lane color palette, indexed by `color_idx % LEN`. Phase 5 may replace this
/// with theme tokens.
/// Built with struct literals (`ColorU::new` is not a const fn and can't be used
/// in a const array).
const PALETTE: [ColorU; 7] = [
    ColorU {
        r: 0x4f,
        g: 0xc1,
        b: 0xff,
        a: 0xff,
    }, // blue
    ColorU {
        r: 0x4e,
        g: 0xc9,
        b: 0x7a,
        a: 0xff,
    }, // green
    ColorU {
        r: 0xff,
        g: 0xb0,
        b: 0x4f,
        a: 0xff,
    }, // orange
    ColorU {
        r: 0xd6,
        g: 0x7c,
        b: 0xff,
        a: 0xff,
    }, // purple
    ColorU {
        r: 0xff,
        g: 0x6e,
        b: 0x6e,
        a: 0xff,
    }, // red
    ColorU {
        r: 0x4f,
        g: 0xe0,
        b: 0xd6,
        a: 0xff,
    }, // cyan
    ColorU {
        r: 0xe6,
        g: 0xd2,
        b: 0x4f,
        a: 0xff,
    }, // yellow
];

fn lane_color(idx: usize) -> ColorU {
    PALETTE[idx % PALETTE.len()]
}

/// Element that draws one row of graph lanes. Its width is determined by the
/// number of lane columns, and its height is fixed at [`ROW_HEIGHT`].
pub(crate) struct GitGraphRowCanvas {
    row: GraphRow,
    /// The graph's maximum lane count, which determines this element's width (and
    /// thus the alignment of the whole column).
    lane_count: usize,
    /// Draw the node as a hollow ring instead of a filled dot — used only for
    /// the synthetic "uncommitted changes" row (which reads as "not a real
    /// commit"). The HEAD "you are here" marker is drawn next to the branch pill
    /// instead (see `render_ref_badge`), not on the lane.
    hollow: bool,
    origin: Option<Point>,
    size: Option<Vector2F>,
}

impl GitGraphRowCanvas {
    pub(crate) fn new(row: GraphRow, lane_count: usize, hollow: bool) -> Self {
        Self {
            row,
            lane_count,
            hollow,
            origin: None,
            size: None,
        }
    }

    fn width(&self) -> f32 {
        self.lane_count.max(1) as f32 * LANE_WIDTH
    }

    fn col_center_x(origin: Vector2F, col: usize) -> f32 {
        origin.x() + col as f32 * LANE_WIDTH + LANE_WIDTH / 2.0
    }

    /// Draw a vertical line segment at column position `x` (`y0..y1`).
    fn draw_vertical(ctx: &mut PaintContext, x: f32, y0: f32, y1: f32, color: ColorU) {
        let height = y1 - y0;
        if height <= 0.0 {
            return;
        }
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(
                vec2f(x - LINE_THICKNESS / 2.0, y0),
                vec2f(LINE_THICKNESS, height),
            ))
            .with_background(color);
    }

    /// Draw a horizontal line segment at `y` (connecting two column centers `x0`
    /// and `x1`). Each end extends by half the line width to fill in the
    /// right-angle corners.
    fn draw_horizontal(ctx: &mut PaintContext, x0: f32, x1: f32, y: f32, color: ColorU) {
        let (lo, hi) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        let width = hi - lo + LINE_THICKNESS;
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(
                vec2f(lo - LINE_THICKNESS / 2.0, y - LINE_THICKNESS / 2.0),
                vec2f(width, LINE_THICKNESS),
            ))
            .with_background(color);
    }
}

impl Element for GitGraphRowCanvas {
    fn layout(
        &mut self,
        _constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        let size = vec2f(self.width(), ROW_HEIGHT);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _ctx: &mut warpui::AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        let top = origin.y();
        let mid = origin.y() + ROW_HEIGHT / 2.0;
        let bot = origin.y() + ROW_HEIGHT;
        let node_x = Self::col_center_x(origin, self.row.node_col);
        let radius = DOT_DIAMETER / 2.0;

        // Continuing lanes passing through this row: a full-height vertical line.
        for lane in &self.row.passing {
            Self::draw_vertical(
                ctx,
                Self::col_center_x(origin, lane.col),
                top,
                bot,
                lane_color(lane.color_idx),
            );
        }

        // The node's own upper half (when it continues from the previous row).
        // A hollow ring has a transparent center, so the segment is trimmed to the
        // ring's top edge — otherwise it would show through the hollow center and
        // make the ring read as a filled dot.
        if self.row.node_continues_up {
            let y1 = if self.hollow { mid - radius } else { mid };
            Self::draw_vertical(ctx, node_x, top, y1, lane_color(self.row.node_color));
        }

        // Child commits merging into this node from above: vertical (top -> mid,
        // at the child column) + horizontal (child column -> node column, at the
        // midline).
        for child in &self.row.from_children {
            let child_x = Self::col_center_x(origin, child.col);
            let color = lane_color(child.color_idx);
            Self::draw_vertical(ctx, child_x, top, mid, color);
            Self::draw_horizontal(ctx, child_x, node_x, mid, color);
        }

        // This node connecting down to each parent: horizontal (node column ->
        // parent column, at the midline) + vertical (mid -> bottom, at the parent
        // column).
        for parent in &self.row.to_parents {
            let parent_x = Self::col_center_x(origin, parent.col);
            let color = lane_color(parent.color_idx);
            if parent.col != self.row.node_col {
                Self::draw_horizontal(ctx, node_x, parent_x, mid, color);
                Self::draw_vertical(ctx, parent_x, mid, bot, color);
            } else {
                // Straight-down lane at the node column: trim it to the ring's
                // bottom edge for a hollow node so it doesn't bleed through the
                // transparent center.
                let y0 = if self.hollow { mid + radius } else { mid };
                Self::draw_vertical(ctx, parent_x, y0, bot, color);
            }
        }

        // Commit dot (corner_radius = radius -> a circle). The synthetic
        // uncommitted row draws a ring (border only) instead — reads as "not a
        // real commit" — and its transparent center lets the row's
        // hover/selection highlight show through.
        let color = lane_color(self.row.node_color);
        let dot = ctx.scene.draw_rect_with_hit_recording(RectF::new(
            vec2f(node_x - radius, mid - radius),
            vec2f(DOT_DIAMETER, DOT_DIAMETER),
        ));
        dot.with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)));
        if self.hollow {
            dot.with_border(Border::all(LINE_THICKNESS).with_border_fill(color));
        } else {
            dot.with_background(color);
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        _event: &DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }
}
