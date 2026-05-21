use super::{
    AfterLayoutContext, AppContext, DropShadow, Element, EventContext, Fill, LayoutContext, Margin,
    Overdraw, Padding, PaintContext, Point, SelectableElement, Selection, SelectionFragment,
    SizeConstraint,
};
pub use crate::scene::{Border, CornerRadius, Radius};
use crate::{
    event::DispatchedEvent,
    text::{word_boundaries::WordBoundariesPolicy, IsRect, SelectionDirection, SelectionType},
    ClipBounds, Gradient,
};
use pathfinder_color::ColorU;
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};

pub struct Container {
    margin: Margin,
    padding: Padding,
    overdraw: Overdraw,
    background: Fill,
    border: Border,
    corner_radius: CornerRadius,
    drop_shadow: Option<DropShadow>,
    foreground_overlay: Option<Fill>,
    child: Box<dyn Element>,
    size: Option<Vector2F>,
    origin: Option<Point>,
    #[cfg(debug_assertions)]
    /// Captures the location of the constructor call site. This is used for debugging purposes.
    construction_location: Option<&'static std::panic::Location<'static>>,
}

impl Container {
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn new(child: Box<dyn Element>) -> Self {
        Self {
            margin: Margin::default(),
            padding: Padding::default(),
            overdraw: Overdraw::default(),
            background: Fill::None,
            border: Border::default(),
            corner_radius: CornerRadius::default(),
            foreground_overlay: None,
            drop_shadow: None,
            child,
            size: None,
            origin: None,
            #[cfg(debug_assertions)]
            construction_location: Some(std::panic::Location::caller()),
        }
    }

    pub fn with_drop_shadow(mut self, drop_shadow: DropShadow) -> Self {
        self.drop_shadow = Some(drop_shadow);
        self
    }

    pub fn with_foreground_overlay<F>(mut self, overlay: F) -> Self
    where
        F: Into<Fill>,
    {
        self.foreground_overlay = Some(overlay.into());
        self
    }

    pub fn with_margin_top(mut self, margin: f32) -> Self {
        self.margin.top = margin;
        self
    }

    pub fn with_margin_bottom(mut self, margin: f32) -> Self {
        self.margin.bottom = margin;
        self
    }

    pub fn with_margin_left(mut self, margin: f32) -> Self {
        self.margin.left = margin;
        self
    }

    pub fn with_margin_right(mut self, margin: f32) -> Self {
        self.margin.right = margin;
        self
    }

    pub fn with_uniform_margin(mut self, margin: f32) -> Self {
        self.margin = Margin {
            top: margin,
            left: margin,
            bottom: margin,
            right: margin,
        };
        self
    }

    pub fn with_uniform_padding(mut self, padding: f32) -> Self {
        self.padding = Padding {
            top: padding,
            left: padding,
            bottom: padding,
            right: padding,
        };
        self
    }

    pub fn with_padding_right(mut self, padding: f32) -> Self {
        self.padding.right = padding;
        self
    }

    /// Sets the horizontal margin (`margin_left` and `margin_right`) to that of `margin`.
    pub fn with_horizontal_margin(mut self, margin: f32) -> Self {
        self.margin.left = margin;
        self.margin.right = margin;
        self
    }

    /// Sets the vertical margin (`margin_top` and `margin_bottom`) to that of `margin`.
    pub fn with_vertical_margin(mut self, margin: f32) -> Self {
        self.margin.top = margin;
        self.margin.bottom = margin;
        self
    }

    /// Sets the horizontal padding (`padding_left` and `padding_right`) to that of `padding`.
    pub fn with_horizontal_padding(mut self, padding: f32) -> Self {
        self.padding.left = padding;
        self.padding.right = padding;
        self
    }

    /// Sets the vertical padding (`padding_top` and `padding_bottom`) to that of `padding`.
    pub fn with_vertical_padding(mut self, padding: f32) -> Self {
        self.padding.top = padding;
        self.padding.bottom = padding;
        self
    }

    pub fn with_padding_left(mut self, padding: f32) -> Self {
        self.padding.left = padding;
        self
    }

    pub fn with_padding_bottom(mut self, padding: f32) -> Self {
        self.padding.bottom = padding;
        self
    }

    pub fn with_padding_top(mut self, padding: f32) -> Self {
        self.padding.top = padding;
        self
    }

    pub fn with_padding(mut self, padding: Padding) -> Self {
        self.padding = padding;
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
        gradient: Gradient,
    ) -> Self {
        self.background = Fill::Gradient {
            start,
            end,
            start_color: gradient.start,
            end_color: gradient.end,
        };
        self
    }

    pub fn with_border(mut self, border: impl Into<Border>) -> Self {
        self.border = border.into();
        self
    }

    pub fn with_overdraw_bottom(mut self, overdraw: f32) -> Self {
        self.overdraw.bottom = overdraw;
        self
    }

    pub fn with_overdraw_left(mut self, overdraw: f32) -> Self {
        self.overdraw.left = overdraw;
        self
    }

    pub fn with_vertical_overdraw(mut self, overdraw: f32) -> Self {
        self.overdraw.top = overdraw;
        self.overdraw.bottom = overdraw;
        self
    }

    pub fn with_corner_radius(mut self, radius: CornerRadius) -> Self {
        self.corner_radius = radius;
        self
    }

    fn margin_size(&self) -> Vector2F {
        vec2f(
            self.margin.left + self.margin.right,
            self.margin.top + self.margin.bottom,
        )
    }

    fn padding_size(&self) -> Vector2F {
        vec2f(
            self.padding.left + self.padding.right,
            self.padding.top + self.padding.bottom,
        )
    }

    fn border_size(&self) -> Vector2F {
        let mut x = 0.0;
        if self.border.left {
            x += self.border.width;
        }
        if self.border.right {
            x += self.border.width;
        }

        let mut y = 0.0;
        if self.border.top {
            y += self.border.width;
        }
        if self.border.bottom {
            y += self.border.width;
        }

        vec2f(x, y)
    }
}

impl Element for Container {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let size_buffer = self.margin_size() + self.padding_size() + self.border_size();

        let child_constraint = SizeConstraint {
            min: (constraint.min - size_buffer).max(Vector2F::zero()),
            max: (constraint.max - size_buffer).max(Vector2F::zero()),
        };
        let child_size = self.child.layout(child_constraint, ctx, app);
        let size = child_size + size_buffer;
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let size = self.size.unwrap() - self.margin_size()
            + vec2f(self.overdraw.right, self.overdraw.bottom);
        let origin = origin + vec2f(self.margin.left, self.margin.top)
            - vec2f(self.overdraw.left, self.overdraw.top);

        #[cfg(debug_assertions)]
        ctx.scene
            .set_location_for_panic_logging(self.construction_location);

        let rect = ctx
            .scene
            .draw_rect_with_hit_recording(RectF::new(origin, size))
            .with_background(self.background)
            .with_border(self.border)
            .with_corner_radius(self.corner_radius);
        if let Some(drop_shadow) = self.drop_shadow {
            rect.with_drop_shadow(drop_shadow);
        }

        let mut child_origin = origin
            + vec2f(self.overdraw.left, self.overdraw.top)
            + vec2f(self.padding.left, self.padding.top);
        if self.border.left {
            child_origin.set_x(child_origin.x() + self.border.width);
        }
        if self.border.top {
            child_origin.set_y(child_origin.y() + self.border.width);
        }
        self.child.paint(child_origin, ctx, app);

        // Start a new layer on top of the current container to render the foreground overlay.
        if let Some(overlay) = self.foreground_overlay {
            ctx.scene.start_layer(ClipBounds::ActiveLayer);
            ctx.scene.set_active_layer_click_through();

            #[cfg(debug_assertions)]
            ctx.scene
                .set_location_for_panic_logging(self.construction_location);

            ctx.scene
                .draw_rect_with_hit_recording(RectF::new(origin, size))
                .with_background(overlay)
                .with_corner_radius(self.corner_radius);
            ctx.scene.stop_layer();
        }
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self as &dyn SelectableElement)
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        self.child.debug_text_content()
    }
}

impl SelectableElement for Container {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.get_selection(selection_start, selection_end, is_rect)
            })
    }

    fn expand_selection(
        &self,
        point: Vector2F,
        direction: SelectionDirection,
        unit: SelectionType,
        word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.expand_selection(point, direction, unit, word_boundaries_policy)
            })
    }

    fn is_point_semantically_before(
        &self,
        absolute_point: Vector2F,
        absolute_point_other: Vector2F,
    ) -> Option<bool> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.is_point_semantically_before(absolute_point, absolute_point_other)
            })
    }

    fn smart_select(
        &self,
        absolute_point: Vector2F,
        smart_select_fn: crate::elements::SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.smart_select(absolute_point, smart_select_fn)
            })
    }

    fn calculate_clickable_bounds(&self, current_selection: Option<Selection>) -> Vec<RectF> {
        self.child
            .as_selectable_element()
            .map(|selectable_child| selectable_child.calculate_clickable_bounds(current_selection))
            .unwrap_or_default()
    }
}

#[cfg(test)]
#[path = "container_tests.rs"]
mod tests;
