use pathfinder_geometry::vector::{Vector2F, vec2f};
use warpui::elements::Point;
use warpui::event::DispatchedEvent;
use warpui::{
    AfterLayoutContext, AppContext, ClipBounds, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

/// Returns the `(origin_x, width)` of the center slot in a row of `row_width`.
///
/// The slot gets at most the gap between the side groups, so it shrinks with the
/// window instead of disappearing or overlapping the side controls. Within that
/// gap it is placed as close to the row's midpoint as possible.
fn center_slot(row_width: f32, left_width: f32, right_width: f32, center_width: f32) -> (f32, f32) {
    let gap = (row_width - left_width - right_width).max(0.);
    let width = center_width.min(gap);
    let max_origin_x = (row_width - right_width - width).max(left_width);
    let origin_x = ((row_width - width) / 2.).clamp(left_width, max_origin_x);
    (origin_x, width)
}

/// A title-bar row whose center child is centered against the full row width
/// rather than against the space left over between the side groups.
///
/// The left and right children keep their intrinsic widths and stay anchored to
/// their edges. The center child is constrained to the gap between them (see
/// [`center_slot`]) and clipped to that allocation.
pub(super) struct CenteredTitleBar {
    left: Box<dyn Element>,
    center: Box<dyn Element>,
    right: Box<dyn Element>,
    size: Option<Vector2F>,
    origin: Option<Point>,
    child_origins: [Vector2F; 3],
    center_size: Vector2F,
    center_painted: bool,
}

impl CenteredTitleBar {
    pub(super) fn new(
        left: Box<dyn Element>,
        center: Box<dyn Element>,
        right: Box<dyn Element>,
    ) -> Self {
        Self {
            left,
            center,
            right,
            size: None,
            origin: None,
            child_origins: [Vector2F::zero(); 3],
            center_size: Vector2F::zero(),
            center_painted: false,
        }
    }
}

impl Element for CenteredTitleBar {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let row_width = constraint.max.x();
        let side_constraint = SizeConstraint::new(Vector2F::zero(), constraint.max);
        let left_size = self.left.layout(side_constraint, ctx, app);
        let right_size = self.right.layout(side_constraint, ctx, app);

        let gap = (row_width - left_size.x() - right_size.x()).max(0.);
        let center_size = self.center.layout(
            SizeConstraint::new(Vector2F::zero(), vec2f(gap, constraint.max.y())),
            ctx,
            app,
        );
        let (center_x, center_width) =
            center_slot(row_width, left_size.x(), right_size.x(), center_size.x());

        let height = left_size
            .y()
            .max(center_size.y())
            .max(right_size.y())
            .clamp(constraint.min.y(), constraint.max.y());
        self.child_origins = [
            vec2f(0., (height - left_size.y()) / 2.),
            vec2f(center_x, (height - center_size.y()) / 2.),
            vec2f(row_width - right_size.x(), (height - right_size.y()) / 2.),
        ];
        self.center_size = vec2f(center_width, center_size.y());

        let size = vec2f(row_width, height);
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
        let [left_origin, center_origin, right_origin] = self.child_origins;
        self.left.paint(origin + left_origin, ctx, app);

        // Clip the center child to its allocation so it can never draw over (or
        // receive clicks meant for) the side controls.
        let center_origin = origin + center_origin;
        self.center_painted = false;
        if self.center_size.x() > 0.
            && let Some(bounds) = ctx.scene.visible_rect(
                Point::from_vec2f(center_origin, ctx.scene.z_index()),
                self.center_size,
            )
        {
            ctx.scene.start_layer(ClipBounds::BoundedBy(bounds));
            self.center.paint(center_origin, ctx, app);
            ctx.scene.stop_layer();
            self.center_painted = true;
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
        if self.center_painted {
            handled |= self.center.dispatch_event(event, ctx, app);
        }
        handled |= self.right.dispatch_event(event, ctx, app);
        handled
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        let text = [&self.left, &self.center, &self.right]
            .into_iter()
            .filter_map(|child| child.debug_text_content())
            .collect::<String>();
        (!text.is_empty()).then_some(text)
    }
}

#[cfg(test)]
#[path = "centered_title_bar_tests.rs"]
mod tests;
