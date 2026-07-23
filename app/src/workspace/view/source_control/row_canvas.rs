use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use warpui::elements::{CornerRadius, Element, Point, Radius};
use warpui::event::DispatchedEvent;
use warpui::{AppContext, EventContext, LayoutContext, PaintContext, SizeConstraint};

use super::layout::GraphRow;

const LANE_WIDTH: f32 = 12.;
const ROW_HEIGHT: f32 = 24.;
const LINE_THICKNESS: f32 = 1.5;
const DOT_DIAMETER: f32 = 7.;
const MAX_LANES: usize = 8;

pub struct GraphRowCanvas {
    row: GraphRow,
    lane_count: usize,
    color: ColorU,
    origin: Option<Point>,
    size: Option<Vector2F>,
}

impl GraphRowCanvas {
    pub fn new(row: GraphRow, lane_count: usize, color: ColorU) -> Self {
        Self {
            row,
            lane_count,
            color,
            origin: None,
            size: None,
        }
    }

    fn lane_x(origin: Vector2F, lane: usize) -> f32 {
        origin.x() + lane as f32 * LANE_WIDTH + LANE_WIDTH / 2.
    }

    fn draw_vertical(ctx: &mut PaintContext, x: f32, top: f32, bottom: f32, color: ColorU) {
        if bottom <= top {
            return;
        }
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(
                vec2f(x - LINE_THICKNESS / 2., top),
                vec2f(LINE_THICKNESS, bottom - top),
            ))
            .with_background(color);
    }

    fn draw_horizontal(ctx: &mut PaintContext, left: f32, right: f32, y: f32, color: ColorU) {
        let (left, right) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(
                vec2f(left - LINE_THICKNESS / 2., y - LINE_THICKNESS / 2.),
                vec2f(right - left + LINE_THICKNESS, LINE_THICKNESS),
            ))
            .with_background(color);
    }
}

impl Element for GraphRowCanvas {
    fn layout(
        &mut self,
        _constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        let lane_count = self.lane_count.clamp(1, MAX_LANES);
        let size = vec2f(lane_count as f32 * LANE_WIDTH, ROW_HEIGHT);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _ctx: &mut warpui::AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let top = origin.y();
        let middle = top + ROW_HEIGHT / 2.;
        let bottom = top + ROW_HEIGHT;

        for segment in &self.row.segments {
            let from_x = Self::lane_x(origin, segment.from_lane.min(MAX_LANES - 1));
            let to_x = Self::lane_x(origin, segment.to_lane.min(MAX_LANES - 1));
            Self::draw_vertical(ctx, from_x, top, middle, self.color);
            Self::draw_horizontal(ctx, from_x, to_x, middle, self.color);
            Self::draw_vertical(ctx, to_x, middle, bottom, self.color);
        }

        let node_x = Self::lane_x(origin, self.row.node_lane.min(MAX_LANES - 1));
        let radius = DOT_DIAMETER / 2.;
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(
                vec2f(node_x - radius, middle - radius),
                vec2f(DOT_DIAMETER, DOT_DIAMETER),
            ))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
            .with_background(self.color);
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
