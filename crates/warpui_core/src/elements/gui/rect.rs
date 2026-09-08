use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{Vector2F, vec2f};

use super::{
    AfterLayoutContext, AppContext, Element, EventContext, Fill, LayoutContext, PaintContext,
    Point, SizeConstraint,
};
use crate::event::DispatchedEvent;
pub use crate::scene::{Border, CornerRadius, DropShadow};
pub struct Rect {
    background: Fill,
    drop_shadow: Option<DropShadow>,
    border: Border,
    corner_radius: CornerRadius,
    size: Option<Vector2F>,
    origin: Option<Point>,
    #[cfg(debug_assertions)]
    /// Custom panic location, set with [`Rect::set_location_for_panic_logging`]
    constructor_location: Option<&'static std::panic::Location<'static>>,
}

impl Default for Rect {
    fn default() -> Self {
        Self::new()
    }
}

impl Rect {
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn new() -> Self {
        Self {
            drop_shadow: None,
            background: Fill::None,
            border: Border::default(),
            corner_radius: CornerRadius::default(),
            size: None,
            origin: None,
            #[cfg(debug_assertions)]
            constructor_location: Some(std::panic::Location::caller()),
        }
    }

    pub fn with_corner_radius(mut self, radius: CornerRadius) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn with_background<F>(mut self, fill: F) -> Self
    where
        F: Into<Fill>,
    {
        self.background = fill.into();
        self
    }

    pub fn with_background_color(mut self, color: ColorU) -> Self {
        self.background = Fill::Solid(color);
        self
    }

    pub fn with_drop_shadow(mut self, drop_shadow: DropShadow) -> Self {
        self.drop_shadow = Some(drop_shadow);
        self
    }

    pub fn with_horizontal_background_gradient(
        mut self,
        start_color: ColorU,
        end_color: ColorU,
    ) -> Self {
        self.background = Fill::Gradient {
            start: vec2f(0.0, 0.0),
            end: vec2f(1.0, 0.0),
            start_color,
            end_color,
        };
        self
    }

    pub fn with_background_gradient(
        mut self,
        start: Vector2F,
        end: Vector2F,
        start_color: ColorU,
        end_color: ColorU,
    ) -> Self {
        self.background = Fill::Gradient {
            start,
            end,
            start_color,
            end_color,
        };
        self
    }

    pub fn with_border(mut self, border: Border) -> Self {
        self.border = border;
        self
    }
}

impl Element for Rect {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        let size = constraint.max;
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let size = self.size.unwrap();

        #[cfg(debug_assertions)]
        ctx.scene
            .set_location_for_panic_logging(self.constructor_location);

        let rect = ctx
            .scene
            .draw_rect_with_hit_recording(RectF::new(origin, size))
            .with_background(self.background)
            .with_border(self.border)
            .with_corner_radius(self.corner_radius);
        if let Some(drop_shadow) = self.drop_shadow {
            rect.with_drop_shadow(drop_shadow);
        }
    }

    fn dispatch_event(
        &mut self,
        _event: &DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    #[cfg_attr(debug_assertions, track_caller)]
    fn finish(mut self) -> Box<dyn Element>
    where
        Self: 'static + Sized,
    {
        #[cfg(debug_assertions)]
        {
            self.constructor_location = Some(std::panic::Location::caller());
        }
        Box::new(self)
    }
}
