use crate::elements::AxisOrientation;
use crate::event::DispatchedEvent;
use crate::ClipBounds;

use super::cross_axis_size;
use super::AppContext;
use super::Axis;
use super::CrossAxisAlignment;
use super::Element;
use super::EventContext;
use super::LayoutContext;
use super::MainAxisSize;
use super::PaintContext;
use super::Point;
use super::SizeConstraint;
use super::Vector2FExt;
use crate::elements::flex::{main_axis_size, size_along_axis, LayoutState};
use crate::elements::MainAxisAlignment;
use ordered_float::OrderedFloat;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};

/// An element that positions its children in horizontal or vertical runs, leaving space in between
/// each run.
///
/// This element can be thought of as a bare-bones version of a flex element with the `flex-wrap`
/// property set in CSS. Children are laid out greedily until they can no longer fit on the current
/// run, in which case a new run is created with the child as the first element. If a child exceeds
/// the incoming size constraints it is clamped to the constraint max and clipped during painting.
/// Children that can't fit in any run along the cross axis are not laid out or painted.
pub struct Wrap {
    axis: Axis,
    orientation: AxisOrientation,
    children: Vec<WrapChild>,
    size: Option<Vector2F>,
    origin: Option<Point>,
    spacing: f32,
    runs: Vec<Run>,
    run_spacing: f32,
    main_axis_alignment: MainAxisAlignment,
    main_axis_size: MainAxisSize,
    cross_axis_alignment: CrossAxisAlignment,
}

impl Wrap {
    pub fn new(axis: Axis) -> Self {
        Self {
            axis,
            orientation: AxisOrientation::Normal,
            children: vec![],
            size: None,
            origin: None,
            spacing: 0.,
            runs: vec![],
            run_spacing: 0.,
            main_axis_alignment: MainAxisAlignment::Start,
            main_axis_size: MainAxisSize::Max,
            cross_axis_alignment: CrossAxisAlignment::Start,
        }
    }

    pub fn row() -> Self {
        Self::new(Axis::Horizontal)
    }

    pub fn column() -> Self {
        Self::new(Axis::Vertical)
    }

    pub fn with_reverse_orientation(mut self) -> Self {
        self.orientation = AxisOrientation::Reverse;
        self
    }

    pub fn with_spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Use the specified amount of `spacing` between each run when positioning children.
    pub fn with_run_spacing(mut self, spacing: f32) -> Self {
        self.run_spacing = spacing;
        self
    }

    fn size_along_cross_axis(runs: &[Run], run_spacing: f32) -> f32 {
        let run_height: f32 = runs.iter().map(|run| run.size_along_cross_axis).sum();
        run_height + run_spacing * (runs.len().saturating_sub(1)) as f32
    }

    /// Specifies the strategy to render children in each run when there is remaining space.
    pub fn with_main_axis_alignment(mut self, alignment: MainAxisAlignment) -> Self {
        self.main_axis_alignment = alignment;
        self
    }

    /// Specifies the strategy to size the overall element when there is remaining space after
    /// runs.
    pub fn with_main_axis_size(mut self, size: MainAxisSize) -> Self {
        self.main_axis_size = size;
        self
    }

    pub fn with_cross_axis_alignment(mut self, alignment: CrossAxisAlignment) -> Self {
        self.cross_axis_alignment = alignment;
        self
    }
}

impl Extend<Box<dyn Element>> for Wrap {
    fn extend<T: IntoIterator<Item = Box<dyn Element>>>(&mut self, iter: T) {
        self.children.extend(iter.into_iter().map(WrapChild::new));
    }
}

impl Element for Wrap {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.children.iter_mut().for_each(WrapChild::reset);
        self.runs.clear();

        let max_constraint_along_cross_axis = constraint.max_along(self.axis.invert());
        let max_constraint_along_main_axis = constraint.max_along(self.axis);

        let mut current_run = RunBuilder::default();

        for child in &mut self.children {
            let child_constraint = match child.data() {
                Some(child_data) if child_data.fill_run() => {
                    // If the child expands/shrinks based on the remaining space, then lay it out
                    // with _that_ as the max constraint along its main axis, rather than an
                    // infinite max constraint.

                    let mut remaining_space_along_main_axis =
                        max_constraint_along_main_axis - current_run.size_along_main_axis;

                    let should_create_new_run = match child_data {
                        WrapParentData::FillRemainingSpaceInRun { min_space, .. } => {
                            // If there's insufficient space along the main axis, start a new run rather
                            // than trying to lay out the child with the remaining space. This prevents
                            // calling child.layout() with a maximum constraint that's less than whatever
                            // minimum it might've set.
                            remaining_space_along_main_axis < min_space
                        }
                        WrapParentData::FillEntireRun => true,
                    };

                    if should_create_new_run {
                        let mut new_run = RunBuilder::default();
                        std::mem::swap(&mut new_run, &mut current_run);
                        self.runs.push(new_run.build(
                            self.spacing,
                            max_constraint_along_main_axis,
                            self.main_axis_alignment,
                            self.axis,
                        ));
                        remaining_space_along_main_axis = max_constraint_along_main_axis;
                    }

                    // Let the child expand along the cross axis as well.
                    let remaining_space_along_cross_axis = max_constraint_along_cross_axis
                        - Self::size_along_cross_axis(self.runs.as_slice(), self.run_spacing);

                    match self.axis {
                        Axis::Horizontal => SizeConstraint::new(
                            vec2f(0., constraint.min.y()),
                            vec2f(
                                remaining_space_along_main_axis,
                                remaining_space_along_cross_axis,
                            ),
                        ),
                        Axis::Vertical => SizeConstraint::new(
                            vec2f(constraint.min.x(), 0.),
                            vec2f(
                                remaining_space_along_cross_axis,
                                remaining_space_along_main_axis,
                            ),
                        ),
                    }
                }
                // Lay out the child so that it has an infinite max constraint along its main axis. The
                // incoming max size constraint is respected along the cross axis.
                _ => SizeConstraint::child_constraint_along_axis(self.axis, constraint),
            };
            let size = child.layout(child_constraint, ctx, app);

            // If the child individually exceeds the incoming size constraints, clamp it
            // to the constraint max so it doesn't overflow the container. We continue
            // laying out subsequent children rather than stopping entirely.
            let size = vec2f(
                size.x().min(constraint.max.x()),
                size.y().min(constraint.max.y()),
            );

            let child_size_along_main_axis = size.along(self.axis);
            let child_size_along_cross_axis = size.along(self.axis.invert());

            // The child doesn't fit in the current run--create a new run.
            if child_size_along_main_axis + current_run.size_along_main_axis
                > max_constraint_along_main_axis
            {
                let mut new_run = RunBuilder::default();
                std::mem::swap(&mut new_run, &mut current_run);
                self.runs.push(new_run.build(
                    self.spacing,
                    max_constraint_along_main_axis,
                    self.main_axis_alignment,
                    self.axis,
                ));
            }

            if child_size_along_cross_axis > current_run.size_along_cross_axis {
                // If the new size would cause the element to exceed the max size along the
                // cross axis--don't add the item to the run and immediately break.
                let total_run_size_on_cross_axis = child_size_along_cross_axis
                    + Self::size_along_cross_axis(self.runs.as_slice(), self.run_spacing);
                if total_run_size_on_cross_axis > max_constraint_along_cross_axis {
                    break;
                }
                current_run.size_along_cross_axis = child_size_along_cross_axis;
            }

            current_run.num_children += 1;
            current_run.size_along_main_axis += child_size_along_main_axis;
            // Add the spacing between the child and the next child (were we to add one).
            current_run.size_along_main_axis += self.spacing;
        }

        if current_run.num_children > 0 {
            self.runs.push(current_run.build(
                self.spacing,
                max_constraint_along_main_axis,
                self.main_axis_alignment,
                self.axis,
            ))
        }

        let size_along_cross_axis = Self::size_along_cross_axis(&self.runs, self.run_spacing);
        let size_along_main_axis = match self.main_axis_size {
            MainAxisSize::Min => {
                // Use the largest run along the main axis as the overall element width.
                self.runs
                    .iter()
                    .map(|run| OrderedFloat(run.size_along_main_axis))
                    .max()
                    .unwrap_or_default()
                    .0
            }
            MainAxisSize::Max => constraint.max_along(self.axis),
        };

        let size = match self.axis {
            Axis::Horizontal => vec2f(size_along_main_axis, size_along_cross_axis),
            Axis::Vertical => vec2f(size_along_cross_axis, size_along_main_axis),
        };

        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut crate::AfterLayoutContext, app: &crate::AppContext) {
        for child in &mut self.children {
            child.after_layout(ctx, app)
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let mut num_children_painted = 0;
        let original_origin = origin;

        let wrap_size = self.size.expect("size should exist at paint time");
        let clip_bounds = RectF::new(origin, wrap_size);

        // Clip children to the wrap's own bounds so oversized items don't overflow.
        ctx.scene
            .start_layer(ClipBounds::BoundedByActiveLayerAnd(clip_bounds));

        let mut origin = origin;

        // If the axis is reversed, offset the origin position by the length of the flex along its main axis,
        if let AxisOrientation::Reverse = self.orientation {
            let size_shift = size_along_axis(main_axis_size(wrap_size, self.axis), self.axis);
            origin += size_shift;
        };

        for run in &self.runs {
            let mut run_origin = match self.orientation {
                AxisOrientation::Normal => origin + run.layout_state.leading_space,
                AxisOrientation::Reverse => origin - run.layout_state.leading_space,
            };

            for child in self
                .children
                .iter_mut()
                .skip(num_children_painted)
                .take(run.num_children)
            {
                let child_size = child.size().expect("child size should exist at paint time");
                let child_cross_size = cross_axis_size(child_size, self.axis);

                let child_cross_shift = match self.cross_axis_alignment {
                    CrossAxisAlignment::Center => {
                        run.size_along_cross_axis / 2. - child_cross_size / 2.
                    }
                    CrossAxisAlignment::Start => 0.,
                    CrossAxisAlignment::End => run.size_along_cross_axis - child_cross_size,
                    CrossAxisAlignment::Stretch => 0.,
                };

                // Paint the child and offset the origin by the size of the child along the main
                // axis.
                match self.orientation {
                    AxisOrientation::Normal => {
                        child.paint(
                            run_origin + size_along_axis(child_cross_shift, self.axis.invert()),
                            ctx,
                            app,
                        );
                        if let Some(child_size) = child.size() {
                            run_origin +=
                                size_along_axis(main_axis_size(child_size, self.axis), self.axis);
                        }
                        run_origin += run.layout_state.between_space;
                    }
                    AxisOrientation::Reverse => {
                        if let Some(child_size) = child.size() {
                            run_origin -=
                                size_along_axis(main_axis_size(child_size, self.axis), self.axis);
                        }
                        child.paint(run_origin, ctx, app);
                        run_origin -= run.layout_state.between_space;
                    }
                };
            }
            num_children_painted += run.num_children;

            // We're finished painting the run. Update the origin to be at the start of the new run.
            origin += size_along_axis(
                run.size_along_cross_axis + self.run_spacing,
                self.axis.invert(),
            );
        }

        ctx.scene.stop_layer();
        self.origin = Some(Point::from_vec2f(original_origin, ctx.scene.z_index()));
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
        let mut handled = false;
        for child in &mut self.children {
            let child_dispatch = child.dispatch_event(event, ctx, app);
            handled |= child_dispatch;
        }
        handled
    }
}

#[derive(Clone, Copy)]
enum WrapParentData {
    FillRemainingSpaceInRun {
        /// If `true`, the child element will be laid out with the run's remaining space, rather than
        /// infinite width. This allows children to expand to fill runs.
        fill_run: bool,

        /// The minimum space along the main axis that this child needs. Generally, child elements
        /// should reserve required space in their [`Element::layout`] implementations instead.
        /// However, for flexible children, we sometimes need a minimum here.
        min_space: f32,
    },
    FillEntireRun,
}

/// Convenience wrapper for a [`Wrap`] child that must consume the entire run.
///
/// When a child is wrapped in `WrapFillEntireRun`, the `Wrap` layout will place that child alone
/// on its own run and treat it as occupying all remaining main-axis space for that run. This is
/// useful for elements like wide cards or chips that should expand to the full width of the
/// current row instead of sharing the row with other wrapped children.
pub struct WrapFillEntireRun(WrapFill);

impl WrapFillEntireRun {
    pub fn new(child: Box<dyn Element>) -> Self {
        Self(WrapFill {
            parent_data: WrapParentData::FillEntireRun,
            child,
        })
    }

    pub fn finish(self) -> Box<dyn Element> {
        self.0.finish()
    }
}

/// Marker for children of a [`Wrap`] element that preferentially expand to fill the current
/// row/column before starting a new row/column.
pub struct WrapFill {
    parent_data: WrapParentData,
    child: Box<dyn Element>,
}

impl WrapFill {
    pub fn new(min_space: f32, child: Box<dyn Element>) -> Self {
        Self {
            parent_data: WrapParentData::FillRemainingSpaceInRun {
                fill_run: true,
                min_space,
            },
            child,
        }
    }
}

impl WrapParentData {
    fn fill_run(&self) -> bool {
        match self {
            WrapParentData::FillRemainingSpaceInRun { fill_run, .. } => *fill_run,
            WrapParentData::FillEntireRun => true,
        }
    }
}

impl Element for WrapFill {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut crate::AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    fn parent_data(&self) -> Option<&dyn std::any::Any> {
        Some(&self.parent_data)
    }
}

/// Helper struct to encapsulate a child of a `Wrap` element that may not be painted or laid out
/// depending on the number of elements that fit into the `Wrap` given incoming size constraints.
struct WrapChild {
    element: Box<dyn Element>,
    is_laid_out: bool,
    is_painted: bool,
}

impl WrapChild {
    fn new(element: Box<dyn Element>) -> Self {
        Self {
            element,
            is_laid_out: false,
            is_painted: false,
        }
    }

    fn data(&self) -> Option<WrapParentData> {
        self.element
            .parent_data()
            .and_then(|data| data.downcast_ref())
            .copied()
    }

    fn reset(&mut self) {
        self.is_laid_out = false;
        self.is_painted = false;
    }
}

impl Element for WrapChild {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.is_laid_out = true;
        self.element.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut crate::AfterLayoutContext, app: &crate::AppContext) {
        if self.is_laid_out {
            self.element.after_layout(ctx, app);
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        if self.is_laid_out {
            self.element.paint(origin, ctx, app);
            self.is_painted = true;
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.element.size()
    }

    fn origin(&self) -> Option<Point> {
        self.element.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.is_painted {
            self.element.dispatch_event(event, ctx, app)
        } else {
            false
        }
    }
}

/// A given run of a `Wrap` element.
#[derive(Debug)]
struct Run {
    /// The size along the cross axis of the run. This is functionally the max size on the cross
    /// axis of the elements within the run.
    size_along_cross_axis: f32,
    /// The size along the main axis of the run. This is the sum of each element within the run's
    /// size, plus the leading space and space between each child.
    size_along_main_axis: f32,
    /// The number of children of the parent `Wrap` element that are rendered within this run.
    num_children: usize,
    /// Metadata used to layout the run. This is used to properly respect `MainAxisAlignment` within
    /// each run.
    layout_state: LayoutState,
}

/// Builder type to construct a `Run`.
#[derive(Debug, Default)]
struct RunBuilder {
    /// The size along the cross axis of the run. This is functionally the max size on the cross
    /// axis of the elements within the run.
    size_along_cross_axis: f32,
    /// The main axis size along the run.
    size_along_main_axis: f32,
    /// The number of children of the parent `Wrap` element that are rendered within this run.
    num_children: usize,
}

impl RunBuilder {
    fn build(
        self,
        spacing: f32,
        max_constraint_along_main_axis: f32,
        main_axis_alignment: MainAxisAlignment,
        axis: Axis,
    ) -> Run {
        // We added spacing after every child, but we only want spacing _between_ children,
        // so subtract the (extra) spacing after the last child.
        let size_along_main_axis = self.size_along_main_axis - spacing;

        let layout_state = LayoutState::compute(
            self.num_children,
            spacing,
            max_constraint_along_main_axis - size_along_main_axis,
            main_axis_alignment,
            axis,
        );

        let size_along_main_axis = size_along_main_axis
            + layout_state.leading_space.along(axis)
            + layout_state.between_space.along(axis) * (self.num_children as f32 - 1.);

        Run {
            size_along_cross_axis: self.size_along_cross_axis,
            size_along_main_axis,
            num_children: self.num_children,
            layout_state,
        }
    }
}

#[cfg(test)]
#[path = "wrap_tests.rs"]
mod tests;
