mod wrap;

pub use wrap::*;

use crate::{
    event::DispatchedEvent,
    text::{word_boundaries::WordBoundariesPolicy, IsRect, SelectionDirection, SelectionType},
};

use super::{
    AfterLayoutContext, AppContext, Axis, AxisOrientation, Element, EventContext, LayoutContext,
    PaintContext, Point, SelectableElement, Selection, SelectionFragment, SizeConstraint,
    Vector2FExt,
};
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use std::any::Any;

pub struct Flex {
    axis: Axis,
    orientation: AxisOrientation,
    children: Vec<Box<dyn Element>>,
    size: Option<Vector2F>,
    origin: Option<Point>,
    main_axis_size: MainAxisSize,
    main_axis_alignment: MainAxisAlignment,
    cross_axis_alignment: CrossAxisAlignment,
    spacing: f32,
    layout_state: Option<LayoutState>,
    constrain_horizontal_bounds_to_parent: bool,
    #[cfg(debug_assertions)]
    container_constructor_location: Option<&'static std::panic::Location<'static>>,
    #[cfg(debug_assertions)]
    child_locations: Vec<&'static std::panic::Location<'static>>,
}

#[derive(Debug)]
struct LayoutState {
    /// Space between each child element.
    between_space: Vector2F,
    /// Space between the first and last element.
    leading_space: Vector2F,
}

impl LayoutState {
    fn compute(
        children_count: usize,
        spacing: f32,
        remaining_space: f32,
        axis_alignment: MainAxisAlignment,
        axis: Axis,
    ) -> Self {
        let (between_space, leading_space) = match axis_alignment {
            MainAxisAlignment::Start => (0., 0.),
            MainAxisAlignment::SpaceBetween => {
                if children_count <= 1 {
                    (0., 0.)
                } else {
                    let between_space = remaining_space / ((children_count - 1) as f32);
                    (between_space, 0.)
                }
            }
            MainAxisAlignment::SpaceEvenly => {
                if children_count == 0 {
                    (0., 0.)
                } else {
                    // Divide the remaining space between all the gaps between the children
                    // (`children_count - 1`) plus the beginning and end.
                    let even_space = remaining_space / ((children_count + 1) as f32);
                    (even_space, even_space)
                }
            }
            MainAxisAlignment::Center => (0.0, remaining_space / 2.0),
            MainAxisAlignment::End => (0.0, remaining_space),
        };

        Self {
            between_space: size_along_axis(spacing + between_space, axis),
            leading_space: size_along_axis(leading_space, axis),
        }
    }
}

impl Flex {
    pub fn new(axis: Axis) -> Self {
        Self {
            axis,
            orientation: AxisOrientation::Normal,
            children: Vec::new(),
            size: None,
            origin: None,
            main_axis_size: MainAxisSize::Min,
            main_axis_alignment: MainAxisAlignment::Start,
            cross_axis_alignment: CrossAxisAlignment::Start,
            spacing: 0.0,
            layout_state: None,
            constrain_horizontal_bounds_to_parent: false,
            #[cfg(debug_assertions)]
            container_constructor_location: None,
            #[cfg(debug_assertions)]
            child_locations: Vec::new(),
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

    fn child_flex(child: &dyn Element) -> Option<f32> {
        child
            .parent_data()
            .and_then(|d| d.downcast_ref::<FlexParentData>())
            .map(|data| data.flex)
    }

    fn child_flex_fit(child: &dyn Element) -> Option<FlexFit> {
        child
            .parent_data()
            .and_then(|d| d.downcast_ref::<FlexParentData>())
            .map(|data| data.fit)
    }

    pub fn with_main_axis_size(mut self, main_axis_size: MainAxisSize) -> Self {
        self.main_axis_size = main_axis_size;
        self
    }

    pub fn with_main_axis_alignment(mut self, alignment: MainAxisAlignment) -> Self {
        self.main_axis_alignment = alignment;
        self
    }

    pub fn with_cross_axis_alignment(mut self, alignment: CrossAxisAlignment) -> Self {
        self.cross_axis_alignment = alignment;
        self
    }

    pub fn with_spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn with_constrain_horizontal_bounds_to_parent(
        mut self,
        constrain_horizontal_bounds_to_parent: bool,
    ) -> Self {
        self.constrain_horizontal_bounds_to_parent = constrain_horizontal_bounds_to_parent;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

impl Extend<Box<dyn Element>> for Flex {
    #[cfg_attr(debug_assertions, track_caller)]
    fn extend<T: IntoIterator<Item = Box<dyn Element>>>(&mut self, children: T) {
        #[cfg(debug_assertions)]
        {
            let children: Vec<_> = children.into_iter().collect();
            let count = children.len();
            let location = std::panic::Location::caller();
            for _ in 0..count {
                self.child_locations.push(location);
            }
            self.children.extend(children);
        }
        #[cfg(not(debug_assertions))]
        self.children.extend(children);
    }
}

impl Element for Flex {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        if self.main_axis_size == MainAxisSize::Max {
            // See https://www.notion.so/warpdev/Debugging-Flex-acc03383be5644a8af29d9c52b1142bd?pvs=4#fff43263616d8008b3e3efe280686886
            #[cfg(debug_assertions)]
            let location_info = self
                .container_constructor_location
                .map(|loc| {
                    format!(
                        " (flex created at {}:{}:{})",
                        loc.file(),
                        loc.line(),
                        loc.column()
                    )
                })
                .unwrap_or_default();
            #[cfg(not(debug_assertions))]
            let location_info = "";

            debug_assert!(
                constraint.max_along(self.axis).is_finite(),
                "A flex that should expand to a max space can't be rendered in an infinite max constraint\n{location_info}
See https://www.notion.so/warpdev/Debugging-Flex-acc03383be5644a8af29d9c52b1142bd?pvs=4#fff43263616d8008b3e3efe280686886 for troubleshooting steps"
            );
            if constraint.max_along(self.axis).is_infinite() {
                log::error!("A flex that should expand to a max space can't be rendered in an infinite max constraint\n{location_info}");
            }
        }

        let mut total_flex = 0.0;
        let mut fixed_space = self.spacing * (self.children.len().saturating_sub(1)) as f32;

        let cross_axis = self.axis.invert();
        let mut cross_axis_max: f32 = 0.0;
        // Follow the algorithm specified in Flutter to layout flex elements: https://api.flutter.dev/flutter/widgets/Flex-class.html.
        // At a high level, we take all non-flexible children and render them at an unbounded size
        // along the main axis (keeping the size constraint the cross axis). We use the remaining
        // space to fill all the `Shrinkable` and `Expanded` elements, respecting the `flex` to determine
        // how much space each element should take up. For example, if there's 60 pixels of
        // remaining space, and two `Expanded` elements (one with `flex` of 1.0 and the other with
        // 2.0), we would render first element with a size of 20 pixels and the second with a size
        // of 40 pixels.
        for child in &mut self.children {
            // If the element is flexible add to the total flex but don't lay out the child.
            if let Some(flex) = Self::child_flex(child.as_ref()) {
                total_flex += flex;
            } else {
                // The child is not flexible. In this case, we want the main axis size constraint
                // going from 0 to infinity, and:
                let child_constraint = if self.cross_axis_alignment == CrossAxisAlignment::Stretch {
                    // The cross axis constraint should be exactly the Flex's width or height.
                    SizeConstraint::tight_on_cross_axis(self.axis, constraint)
                } else {
                    // The cross axis constraint should be inherited from the Flex.
                    SizeConstraint::child_constraint_along_axis(self.axis, constraint)
                };

                let size = child.layout(child_constraint, ctx, app);
                fixed_space += size.along(self.axis);
                let cross_axis_size = size.along(cross_axis);
                if cross_axis_size.is_finite() {
                    cross_axis_max = cross_axis_max.max(size.along(cross_axis));
                }
            }
        }

        let mut size = if total_flex > 0.0 {
            // See https://www.notion.so/warpdev/Debugging-Flex-acc03383be5644a8af29d9c52b1142bd?pvs=4#057b1e4ba7b844f7ad2e69433b295363
            #[cfg(debug_assertions)]
            let location_info = self
                .container_constructor_location
                .map(|loc| {
                    format!(
                        " (flex created at {}:{}:{})",
                        loc.file(),
                        loc.line(),
                        loc.column()
                    )
                })
                .unwrap_or_default();
            #[cfg(not(debug_assertions))]
            let location_info = "";

            debug_assert!(
                constraint.max_along(self.axis).is_finite(),
                "flex contains flexible children but has an infinite constraint along the flex axis{location_info}
See https://www.notion.so/warpdev/Debugging-Flex-acc03383be5644a8af29d9c52b1142bd?pvs=4#057b1e4ba7b844f7ad2e69433b295363 for troubleshooting steps"
            );
            if constraint.max_along(self.axis).is_infinite() {
                log::error!("flex contains flexible children but has an infinite constraint along the flex axis{location_info}");
            }

            let mut remaining_space = (constraint.max_along(self.axis) - fixed_space).max(0.);
            let mut remaining_flex = total_flex;
            for child in &mut self.children {
                let space_per_flex = remaining_space / remaining_flex;
                if let Some(flex) = Self::child_flex(child.as_ref()) {
                    let child_max = space_per_flex * flex;
                    let child_min = match Self::child_flex_fit(child.as_ref()) {
                        Some(FlexFit::Loose) | None => 0.0,
                        Some(FlexFit::Tight) => child_max,
                    };

                    let child_constraint = match self.axis {
                        Axis::Horizontal => SizeConstraint::new(
                            vec2f(child_min, constraint.min.y()),
                            vec2f(child_max, constraint.max.y()),
                        ),
                        Axis::Vertical => SizeConstraint::new(
                            vec2f(constraint.min.x(), child_min),
                            vec2f(constraint.max.x(), child_max),
                        ),
                    };
                    let child_size = child.layout(child_constraint, ctx, app);
                    remaining_space -= child_size.along(self.axis);
                    remaining_flex -= flex;

                    let cross_axis_size = child_size.along(cross_axis);
                    if cross_axis_size.is_finite() {
                        cross_axis_max = cross_axis_max.max(child_size.along(cross_axis));
                    }
                }
            }

            // If children should stretch along the cross axis, perform another
            // layout pass on any which ended up having an infinite size along
            // the cross axis (so that they can stretch to the size of the
            // largest finite child).
            if self.cross_axis_alignment == CrossAxisAlignment::Stretch {
                let mut constraint = constraint;
                match cross_axis {
                    Axis::Horizontal => constraint.max.set_x(cross_axis_max),
                    Axis::Vertical => constraint.max.set_y(cross_axis_max),
                }
                for child in &mut self.children {
                    if let Some(size) = child.size() {
                        if size.along(cross_axis).is_infinite() {
                            child.layout(
                                SizeConstraint::tight_on_cross_axis(self.axis, constraint),
                                ctx,
                                app,
                            );
                        }
                    }
                }
            }

            match self.axis {
                Axis::Horizontal => vec2f(constraint.max.x() - remaining_space, cross_axis_max),
                Axis::Vertical => vec2f(cross_axis_max, constraint.max.y() - remaining_space),
            }
        } else {
            match (self.axis, self.constrain_horizontal_bounds_to_parent) {
                (Axis::Horizontal, true) => {
                    vec2f(constraint.max.x().min(fixed_space), cross_axis_max)
                }
                (Axis::Horizontal, false) => vec2f(fixed_space, cross_axis_max),
                (Axis::Vertical, _) => vec2f(cross_axis_max, fixed_space),
            }
        };

        let max_constraint_size = constraint.max.along(self.axis);
        let allocated_size = size.along(self.axis);

        // Expand out the size of the element to the max constraint size iff the max constraint is
        // finite since we need an actual size to compute how elements should be laid out.
        let actual_size =
            if max_constraint_size.is_finite() && self.main_axis_size == MainAxisSize::Max {
                max_constraint_size
            } else {
                allocated_size
            };

        let remaining_space = (actual_size - allocated_size).max(0.);

        // If the axis size is set to max--ensure the flex takes up the max possible size it can
        // while still respecting size constraints.
        if self.main_axis_size == MainAxisSize::Max {
            match self.axis {
                Axis::Horizontal => size.set_x(max_constraint_size),
                Axis::Vertical => size.set_y(max_constraint_size),
            }
        }

        if constraint.min.x().is_finite() {
            size.set_x(size.x().max(constraint.min.x()));
        }
        if constraint.min.y().is_finite() {
            size.set_y(size.y().max(constraint.min.y()));
        }

        self.layout_state = Some(LayoutState::compute(
            self.children.len(),
            self.spacing,
            remaining_space,
            self.main_axis_alignment,
            self.axis,
        ));

        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        for child in &mut self.children {
            child.after_layout(ctx, app);
        }
    }

    fn paint(&mut self, mut origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        #[cfg(debug_assertions)]
        let location_info = self
            .container_constructor_location
            .map(|loc| {
                format!(
                    " (flex created at {}:{}:{})",
                    loc.file(),
                    loc.line(),
                    loc.column()
                )
            })
            .unwrap_or_default();
        #[cfg(not(debug_assertions))]
        let location_info = "";

        let layout_state = self
            .layout_state
            .as_ref()
            .unwrap_or_else(|| panic!("layout state should exist at paint time{location_info}"));

        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        // If the axis is reversed, offset the origin position by the length of the flex along its main axis,
        match self.orientation {
            AxisOrientation::Normal => {
                origin += layout_state.leading_space;
            }
            AxisOrientation::Reverse => {
                let size_shift = size_along_axis(
                    main_axis_size(
                        self.size.unwrap_or_else(|| {
                            panic!("size should exist at paint time{location_info}")
                        }),
                        self.axis,
                    ),
                    self.axis,
                );
                origin += size_shift - layout_state.leading_space;
            }
        };

        let parent_cross_size = cross_axis_size(
            self.size
                .unwrap_or_else(|| panic!("size should exist at paint time{location_info}")),
            self.axis,
        );

        for (child_idx, child) in self.children.iter_mut().enumerate() {
            #[cfg(debug_assertions)]
            let child_location_info = self
                .child_locations
                .get(child_idx)
                .map(|loc| {
                    format!(
                        " (child {} added at {}:{}:{})",
                        child_idx,
                        loc.file(),
                        loc.line(),
                        loc.column()
                    )
                })
                .unwrap_or_else(|| format!(" (flex container at {location_info})"));
            #[cfg(not(debug_assertions))]
            let (child_location_info, _) = ("", child_idx);

            let child_size = child.size().unwrap_or_else(|| {
                panic!("child size should exist at paint time{child_location_info}")
            });
            let child_cross_size = cross_axis_size(child_size, self.axis);

            let child_cross_size = match self.cross_axis_alignment {
                CrossAxisAlignment::Center => parent_cross_size / 2. - child_cross_size / 2.,
                CrossAxisAlignment::Start => 0.,
                CrossAxisAlignment::End => parent_cross_size - child_cross_size,
                CrossAxisAlignment::Stretch => 0.,
            };

            match self.orientation {
                AxisOrientation::Normal => {
                    child.paint(
                        origin + size_along_axis(child_cross_size, self.axis.invert()),
                        ctx,
                        app,
                    );
                    origin += size_along_axis(main_axis_size(child_size, self.axis), self.axis);
                    origin += layout_state.between_space;
                }
                AxisOrientation::Reverse => {
                    origin -= size_along_axis(main_axis_size(child_size, self.axis), self.axis);
                    child.paint(
                        origin + size_along_axis(child_cross_size, self.axis.invert()),
                        ctx,
                        app,
                    );
                    origin -= layout_state.between_space;
                }
            }
        }
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

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self as &dyn SelectableElement)
    }

    #[cfg_attr(debug_assertions, track_caller)]
    fn finish(self) -> Box<dyn Element>
    where
        Self: 'static + Sized,
    {
        #[cfg(debug_assertions)]
        {
            let mut s = self;
            s.container_constructor_location = Some(std::panic::Location::caller());
            Box::new(s)
        }
        #[cfg(not(debug_assertions))]
        Box::new(self)
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        let texts: Vec<String> = self
            .children
            .iter()
            .filter_map(|child| child.debug_text_content())
            .collect();
        if texts.is_empty() {
            None
        } else {
            let separator = if self.axis == Axis::Vertical {
                "\n"
            } else {
                " "
            };
            Some(texts.join(separator))
        }
    }
}

impl SelectableElement for Flex {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        let mut selection_fragments: Vec<SelectionFragment> = Vec::new();
        for child in self.children.iter() {
            if let Some(selectable_child) = child.as_selectable_element() {
                if let Some(child_fragments) =
                    selectable_child.get_selection(selection_start, selection_end, is_rect)
                {
                    // If we're adding new selection fragments from a new child in a Flex,
                    // add a separator between the previous child's and this child's selected text.
                    if let Some(last_fragment) = selection_fragments.last() {
                        let separator = if self.axis == Axis::Vertical {
                            "\n"
                        } else {
                            " "
                        };
                        selection_fragments.push(SelectionFragment {
                            text: separator.to_string(),
                            origin: last_fragment.origin,
                        });
                    }
                    selection_fragments.extend(child_fragments);
                }
            }
        }
        if !selection_fragments.is_empty() {
            return Some(selection_fragments);
        }
        None
    }

    fn expand_selection(
        &self,
        point: Vector2F,
        direction: SelectionDirection,
        unit: SelectionType,
        word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        let mut expanded_selection = None;
        for child in self.children.iter() {
            if let Some(selectable_child) = child.as_selectable_element() {
                if let Some(selection) = selectable_child.expand_selection(
                    point,
                    direction,
                    unit,
                    word_boundaries_policy,
                ) {
                    match direction {
                        // If we're expanding backward, take the first child's expansion.
                        SelectionDirection::Backward => return Some(selection),
                        // Otherwise if we're expanding forward, take the last child's expansion.
                        SelectionDirection::Forward => {
                            expanded_selection = Some(selection);
                        }
                    }
                }
            }
        }
        expanded_selection
    }

    fn is_point_semantically_before(
        &self,
        absolute_point: Vector2F,
        absolute_point_other: Vector2F,
    ) -> Option<bool> {
        for child in self.children.iter() {
            if let Some(selectable_child) = child.as_selectable_element() {
                if let Some(is_point_semantically_before) = selectable_child
                    .is_point_semantically_before(absolute_point, absolute_point_other)
                {
                    return Some(is_point_semantically_before);
                }
            }
        }
        None
    }

    fn smart_select(
        &self,
        absolute_point: Vector2F,
        smart_select_fn: crate::elements::SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        for child in self.children.iter() {
            if let Some(selectable_child) = child.as_selectable_element() {
                if let Some(selection) =
                    selectable_child.smart_select(absolute_point, smart_select_fn)
                {
                    return Some(selection);
                }
            }
        }
        None
    }

    fn calculate_clickable_bounds(&self, current_selection: Option<Selection>) -> Vec<RectF> {
        let mut clickable_bounds = Vec::new();
        for child in self.children.iter() {
            if let Some(selectable_child) = child.as_selectable_element() {
                clickable_bounds
                    .append(&mut selectable_child.calculate_clickable_bounds(current_selection));
            }
        }
        clickable_bounds
    }
}

fn cross_axis_size(size: Vector2F, axis: Axis) -> f32 {
    match axis {
        Axis::Horizontal => size.y(),
        Axis::Vertical => size.x(),
    }
}

fn main_axis_size(size: Vector2F, axis: Axis) -> f32 {
    match axis {
        Axis::Horizontal => size.x(),
        Axis::Vertical => size.y(),
    }
}

/// Converts a coordinate to a `Vector2F` point given the axis on which the coordinate lies.
fn size_along_axis(coordinate: f32, axis: Axis) -> Vector2F {
    match axis {
        Axis::Horizontal => vec2f(coordinate, 0.),
        Axis::Vertical => vec2f(0., coordinate),
    }
}

struct FlexParentData {
    flex: f32,
    fit: FlexFit,
}

#[derive(Debug, Clone, Copy)]
enum FlexFit {
    Tight,
    Loose,
}

/// Strategies to render children within a Flex when there is remaining space. This is _heavily_
/// inspired from Flutter, see https://api.flutter.dev/flutter/widgets/Flex/mainAxisAlignment.html.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MainAxisAlignment {
    /// Place elements as close to the start of the Flex as possible.
    Start,
    /// Place the free space evenly between the children.
    SpaceBetween,
    /// Place the free space evenly between the children, as well as before and after the first and
    /// last child.
    SpaceEvenly,
    /// Place as close the center of the Flex as possible.
    Center,
    /// Place elements as close to the end of the Flex as possible.
    End,
}

/// How much space a Flex element should occupy along the main axis.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MainAxisSize {
    /// Minimize the amount of free space along the main axis, subject to incoming size constraints.
    Min,
    /// Maximize the amount of free space along the main axis, subject to the incoming size
    /// constraints. If there is any remaining space within the element, `MainAxisAlignment` is used
    /// to determine how free space is distributed within the element.
    Max,
}

/// Where children should be placed along the cross axis in a Flex.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CrossAxisAlignment {
    /// Place the children so that their centers align with the middle of the cross axis.
    Center,
    /// Place the children with their start edge aligned with the start side of the cross axis.
    Start,
    /// Place the children as close to the end of the cross axis as possible.
    End,
    /// Require the children to fill the cross axis.
    Stretch,
}

pub struct Shrinkable {
    parent_data: FlexParentData,
    child: Box<dyn Element>,
}

impl Shrinkable {
    pub fn new(flex: f32, child: Box<dyn Element>) -> Self {
        Shrinkable {
            parent_data: FlexParentData {
                flex,
                fit: FlexFit::Loose,
            },
            child,
        }
    }
}

impl Element for Shrinkable {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app);
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
        self.child.size()
    }

    fn parent_data(&self) -> Option<&dyn Any> {
        Some(&self.parent_data)
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self as &dyn SelectableElement)
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        self.child.debug_text_content()
    }
}

impl SelectableElement for Shrinkable {
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

/// A flexible child that will take up all available space in a `Flex`, regardless of whether its contained element
/// wants to grow. Behaves identically to a `Shrinkable` containing an element with infinite width.
pub struct Expanded {
    parent_data: FlexParentData,
    child: Box<dyn Element>,
}

impl Expanded {
    pub fn new(flex: f32, child: Box<dyn Element>) -> Self {
        Expanded {
            parent_data: FlexParentData {
                flex,
                fit: FlexFit::Tight,
            },
            child,
        }
    }
}

impl Element for Expanded {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app);
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
        self.child.size()
    }

    fn parent_data(&self) -> Option<&dyn Any> {
        Some(&self.parent_data)
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self as &dyn SelectableElement)
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        self.child.debug_text_content()
    }
}

impl SelectableElement for Expanded {
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
#[path = "mod_tests.rs"]
mod tests;
