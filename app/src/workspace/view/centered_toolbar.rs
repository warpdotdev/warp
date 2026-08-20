use pathfinder_geometry::vector::{Vector2F, vec2f};
use warpui::elements::Point;
use warpui::event::DispatchedEvent;
use warpui::{
    AfterLayoutContext, AppContext, ClipBounds, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

#[derive(Clone, Copy, Debug, PartialEq)]
struct CenteredToolbarGeometry {
    center_width: f32,
    center_origin_x: f32,
    right_origin_x: f32,
}

fn centered_toolbar_geometry(
    width: f32,
    left_width: f32,
    center_min_width: f32,
    center_max_width: f32,
    right_width: f32,
) -> CenteredToolbarGeometry {
    let side_clearance = left_width.max(right_width);
    let symmetric_center_width = (width - 2. * side_clearance).max(0.);
    let physical_center_width = (width - left_width - right_width).max(0.);
    let center_min_width = center_min_width.min(center_max_width);
    let center_width = if symmetric_center_width >= center_min_width {
        center_max_width.min(symmetric_center_width)
    } else {
        center_min_width.min(physical_center_width)
    };
    let center_origin_x = if center_width > 0. {
        let centered_origin_x = (width - center_width) / 2.;
        centered_origin_x.clamp(left_width, width - right_width - center_width)
    } else {
        width / 2.
    };

    CenteredToolbarGeometry {
        center_width,
        center_origin_x,
        right_origin_x: width - right_width,
    }
}

/// Lays out a toolbar whose middle child stays centered against the full row.
///
/// The left and right children keep their intrinsic widths. The middle child
/// first shrinks symmetrically. Below its minimum useful width, it shifts
/// continuously into the remaining physical gap before shrinking again.
pub(super) struct CenteredToolbar {
    left: Box<dyn Element>,
    center: Box<dyn Element>,
    right: Box<dyn Element>,
    center_min_width: f32,
    center_max_width: f32,
    size: Option<Vector2F>,
    origin: Option<Point>,
    child_origins: Option<[Vector2F; 3]>,
    center_clip_size: Option<Vector2F>,
    center_was_painted: bool,
}

impl CenteredToolbar {
    pub(super) fn new(
        left: Box<dyn Element>,
        center: Box<dyn Element>,
        right: Box<dyn Element>,
        center_min_width: f32,
        center_max_width: f32,
    ) -> Self {
        Self {
            left,
            center,
            right,
            center_min_width,
            center_max_width,
            size: None,
            origin: None,
            child_origins: None,
            center_clip_size: None,
            center_was_painted: false,
        }
    }
}

impl Element for CenteredToolbar {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let intrinsic_constraint =
            SizeConstraint::new(Vector2F::zero(), vec2f(f32::INFINITY, constraint.max.y()));
        let left_size = self.left.layout(intrinsic_constraint, ctx, app);
        let right_size = self.right.layout(intrinsic_constraint, ctx, app);

        let width = if constraint.max.x().is_finite() {
            constraint.max.x().max(constraint.min.x())
        } else {
            (2. * left_size.x().max(right_size.x()) + self.center_max_width).max(constraint.min.x())
        };
        let geometry = centered_toolbar_geometry(
            width,
            left_size.x(),
            self.center_min_width,
            self.center_max_width,
            right_size.x(),
        );
        let center_size = self.center.layout(
            SizeConstraint::new(
                Vector2F::zero(),
                vec2f(geometry.center_width, constraint.max.y()),
            ),
            ctx,
            app,
        );

        let height = left_size
            .y()
            .max(center_size.y())
            .max(right_size.y())
            .max(constraint.min.y())
            .min(constraint.max.y());
        self.child_origins = Some([
            vec2f(0., (height - left_size.y()) / 2.),
            vec2f(geometry.center_origin_x, (height - center_size.y()) / 2.),
            vec2f(geometry.right_origin_x, (height - right_size.y()) / 2.),
        ]);
        self.center_clip_size = Some(vec2f(geometry.center_width, center_size.y()));
        self.center_was_painted = false;

        let size = vec2f(width, height);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.left.after_layout(ctx, app);
        self.center.after_layout(ctx, app);
        self.right.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let [left_origin, center_origin, right_origin] = self
            .child_origins
            .expect("toolbar must be laid out before paint");
        self.left.paint(origin + left_origin, ctx, app);
        let center_clip_size = self
            .center_clip_size
            .expect("toolbar must be laid out before paint");
        self.center_was_painted = false;
        if center_clip_size.x() > 0.
            && center_clip_size.y() > 0.
            && let Some(bounds) = ctx.scene.visible_rect(
                Point::from_vec2f(origin + center_origin, ctx.scene.z_index()),
                center_clip_size,
            )
        {
            ctx.scene.start_layer(ClipBounds::BoundedBy(bounds));
            self.center.paint(origin + center_origin, ctx, app);
            ctx.scene.stop_layer();
            self.center_was_painted = true;
        }
        self.right.paint(origin + right_origin, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let mut handled = self.left.dispatch_event(event, ctx, app);
        if self.center_was_painted {
            handled |= self.center.dispatch_event(event, ctx, app);
        }
        handled |= self.right.dispatch_event(event, ctx, app);
        handled
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        let text = [
            self.left.debug_text_content(),
            self.center.debug_text_content(),
            self.right.debug_text_content(),
        ]
        .into_iter()
        .flatten()
        .collect::<String>();
        (!text.is_empty()).then_some(text)
    }
}

#[cfg(test)]
#[path = "centered_toolbar_tests.rs"]
mod tests;
