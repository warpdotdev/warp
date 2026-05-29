//! 单行 commit DAG 泳道的自定义绘制元素。
//!
//! 受渲染层限制（`Scene` 仅有矩形/文字/图标/图片图元，无 line/path/旋转），连线以
//! **正交折线**绘制：竖线/横线均为细矩形，节点圆点用 `corner_radius = 半径` 的方块。
//! 圆角折线拐角的进一步美化留待后续 polish。

use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use warpui::elements::{CornerRadius, Element, Point, Radius};
use warpui::event::DispatchedEvent;
use warpui::{AppContext, EventContext, LayoutContext, PaintContext, SizeConstraint};

use super::layout::GraphRow;

/// 单条泳道的像素宽度。
const LANE_WIDTH: f32 = 14.0;
/// 行高（与右侧文字行对齐；UniformList 以此为统一行高）。
const ROW_HEIGHT: f32 = 22.0;
/// 连线粗细。
const LINE_THICKNESS: f32 = 2.0;
/// 提交圆点直径。
const DOT_DIAMETER: f32 = 8.0;

/// 泳道配色板，按 `color_idx % LEN` 取色。Phase 5 可替换为主题 token。
/// 用结构体字面量构造（`ColorU::new` 非 const fn，无法用于 const 数组）。
const PALETTE: [ColorU; 7] = [
    ColorU { r: 0x4f, g: 0xc1, b: 0xff, a: 0xff }, // 蓝
    ColorU { r: 0x4e, g: 0xc9, b: 0x7a, a: 0xff }, // 绿
    ColorU { r: 0xff, g: 0xb0, b: 0x4f, a: 0xff }, // 橙
    ColorU { r: 0xd6, g: 0x7c, b: 0xff, a: 0xff }, // 紫
    ColorU { r: 0xff, g: 0x6e, b: 0x6e, a: 0xff }, // 红
    ColorU { r: 0x4f, g: 0xe0, b: 0xd6, a: 0xff }, // 青
    ColorU { r: 0xe6, g: 0xd2, b: 0x4f, a: 0xff }, // 黄
];

fn lane_color(idx: usize) -> ColorU {
    PALETTE[idx % PALETTE.len()]
}

/// 绘制一行图谱泳道的元素。宽度由泳道列数决定，高度固定为 [`ROW_HEIGHT`]。
pub(crate) struct GitGraphRowCanvas {
    row: GraphRow,
    /// 整张图的最大泳道数，决定本元素（也即整列对齐）的宽度。
    lane_count: usize,
    origin: Option<Point>,
    size: Option<Vector2F>,
}

impl GitGraphRowCanvas {
    pub(crate) fn new(row: GraphRow, lane_count: usize) -> Self {
        Self {
            row,
            lane_count,
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

    /// 在列 `x` 处画一段竖线（`y0..y1`）。
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

    /// 在 `y` 处画一段水平线（连接两列中心 `x0`、`x1`）。两端各延伸半个线宽以补齐直角拐角。
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

        // 穿过本行的延续泳道：整行竖线。
        for lane in &self.row.passing {
            Self::draw_vertical(
                ctx,
                Self::col_center_x(origin, lane.col),
                top,
                bot,
                lane_color(lane.color_idx),
            );
        }

        // 节点自身的上半段（由上一行延续而来时）。
        if self.row.node_continues_up {
            Self::draw_vertical(ctx, node_x, top, mid, lane_color(self.row.node_color));
        }

        // 子提交从上方汇入本节点：竖（顶→中，于子列）+ 横（子列→节点列，于中线）。
        for child in &self.row.from_children {
            let child_x = Self::col_center_x(origin, child.col);
            let color = lane_color(child.color_idx);
            Self::draw_vertical(ctx, child_x, top, mid, color);
            Self::draw_horizontal(ctx, child_x, node_x, mid, color);
        }

        // 本节点连向下方各父：横（节点列→父列，于中线）+ 竖（中→底，于父列）。
        for parent in &self.row.to_parents {
            let parent_x = Self::col_center_x(origin, parent.col);
            let color = lane_color(parent.color_idx);
            if parent.col != self.row.node_col {
                Self::draw_horizontal(ctx, node_x, parent_x, mid, color);
            }
            Self::draw_vertical(ctx, parent_x, mid, bot, color);
        }

        // 提交圆点（corner_radius = 半径 → 圆形）。
        let radius = DOT_DIAMETER / 2.0;
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(
                vec2f(node_x - radius, mid - radius),
                vec2f(DOT_DIAMETER, DOT_DIAMETER),
            ))
            .with_background(lane_color(self.row.node_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)));
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
