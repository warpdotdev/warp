use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use instant::Instant;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use warp_features::FeatureFlag;

use super::{
    AxisConfiguration, ClippedAxisConfiguration, DualAxisConfig, NewScrollable,
    NewScrollableElement, ScrollableAppearance, ScrollableAxis, SingleAxisConfig,
};
use crate::elements::{
    Axis, ClippedScrollStateHandle, ConstrainedBox, DispatchEventResult, EventHandler, Fill, Flex,
    Hoverable, MouseStateHandle, ParentElement, Point, Rect, SavePosition, ScrollData,
    ScrollStateHandle, ScrollTarget, ScrollToPositionMode, ScrollbarWidth, SelectableElement,
    SelectionFragment, Stack, ZIndex,
};
use crate::event::{DispatchedEvent, ModifiersState};
use crate::platform::{TerminationMode, WindowStyle};
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{IsRect, SelectionDirection, SelectionType};
use crate::units::{IntoPixels, Pixels};
use crate::{
    AfterLayoutContext, App, AppContext, Element, Entity, EntityId, EntityIdSet, Event,
    EventContext, LayoutContext, PaintContext, Presenter, SizeConstraint, TypedActionView, View,
    ViewContext, WindowInvalidation,
};

const TOTAL_SCROLLABLE_SIZE: f32 = 500.;
const CHILD_EVENT_HANDLER_DIMENSION: f32 = 50.;
const CHILD_EVENT_HANDLER_COUNT: usize = 10;
const SCROLLABLE_VIEWPORT_SIZE: f32 = 250.;

fn select_entire_probe_text(
    _content: &str,
    _click_offset: string_offset::ByteOffset,
) -> Option<std::ops::Range<string_offset::ByteOffset>> {
    Some(string_offset::ByteOffset::zero()..string_offset::ByteOffset::from(1))
}

#[derive(Clone, Default)]
struct SelectableProbeState {
    get_selection_args: Rc<RefCell<Vec<(Vector2F, Vector2F, IsRect)>>>,
    expand_selection_args: Rc<RefCell<Vec<(Vector2F, SelectionDirection, SelectionType)>>>,
    semantic_order_args: Rc<RefCell<Vec<(Vector2F, Vector2F)>>>,
    smart_select_args: Rc<RefCell<Vec<Vector2F>>>,
    clickable_bounds_args: Rc<RefCell<Vec<Option<crate::elements::Selection>>>>,
}

struct SelectableProbeElement {
    state: SelectableProbeState,
    size: Vector2F,
}

impl SelectableProbeElement {
    fn new(state: SelectableProbeState) -> Self {
        Self {
            state,
            size: vec2f(400.0, 120.0),
        }
    }
}

impl Element for SelectableProbeElement {
    fn layout(
        &mut self,
        _constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        self.size
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, _origin: Vector2F, _ctx: &mut PaintContext, _app: &AppContext) {}

    fn size(&self) -> Option<Vector2F> {
        Some(self.size)
    }

    fn origin(&self) -> Option<Point> {
        Some(Point::new(0.0, 0.0, ZIndex::new(0)))
    }

    fn dispatch_event(
        &mut self,
        _event: &DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self)
    }
}

impl SelectableElement for SelectableProbeElement {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        self.state
            .get_selection_args
            .borrow_mut()
            .push((selection_start, selection_end, is_rect));
        Some(vec![SelectionFragment {
            text: "probe".to_string(),
            origin: Point::new(0.0, 0.0, ZIndex::new(0)),
        }])
    }

    fn expand_selection(
        &self,
        absolute_point: Vector2F,
        direction: SelectionDirection,
        unit: SelectionType,
        _word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        self.state
            .expand_selection_args
            .borrow_mut()
            .push((absolute_point, direction, unit));
        Some(absolute_point + vec2f(5.0, 0.0))
    }

    fn is_point_semantically_before(
        &self,
        absolute_point: Vector2F,
        absolute_point_other: Vector2F,
    ) -> Option<bool> {
        self.state
            .semantic_order_args
            .borrow_mut()
            .push((absolute_point, absolute_point_other));
        Some(absolute_point.x() < absolute_point_other.x())
    }

    fn smart_select(
        &self,
        absolute_point: Vector2F,
        _smart_select_fn: crate::elements::SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        self.state
            .smart_select_args
            .borrow_mut()
            .push(absolute_point);
        Some((absolute_point, absolute_point + vec2f(12.0, 0.0)))
    }

    fn calculate_clickable_bounds(
        &self,
        current_selection: Option<crate::elements::Selection>,
    ) -> Vec<crate::geometry::rect::RectF> {
        self.state
            .clickable_bounds_args
            .borrow_mut()
            .push(current_selection);
        Vec::new()
    }
}

fn test_clipped_horizontal_scrollable_with_probe(
    state: SelectableProbeState,
    scroll_left: f32,
) -> NewScrollable {
    let handle = ClippedScrollStateHandle::default();
    handle.scroll_to(Pixels::new(scroll_left));
    test_clipped_horizontal_scrollable_with_probe_handle(state, handle)
}

fn test_clipped_horizontal_scrollable_with_probe_handle(
    state: SelectableProbeState,
    handle: ClippedScrollStateHandle,
) -> NewScrollable {
    NewScrollable::horizontal(
        SingleAxisConfig::Clipped {
            handle,
            child: Box::new(SelectableProbeElement::new(state)),
        },
        Fill::None,
        Fill::None,
        Fill::None,
    )
}

struct ScrollableElement {
    size: Option<Vector2F>,
    origin: Option<Point>,
    scroll_top: f32,
    scroll_left: f32,
    elements: Vec<Vec<Box<dyn Element>>>,
}

impl ScrollableElement {
    fn new(scroll_top: f32, scroll_left: f32, elements: Vec<Vec<Box<dyn Element>>>) -> Self {
        Self {
            scroll_left,
            scroll_top,
            size: None,
            origin: None,
            elements,
        }
    }
}

impl Element for ScrollableElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        // The child element size should all be hard-coded. We don't need to worry about the
        // size constraint here.
        for element in self.elements.iter_mut().flatten() {
            element.layout(constraint, ctx, app);
        }
        let size = vec2f(
            constraint
                .max_along(Axis::Horizontal)
                .min(TOTAL_SCROLLABLE_SIZE),
            constraint
                .max_along(Axis::Vertical)
                .min(TOTAL_SCROLLABLE_SIZE),
        );
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let adjusted_origin = origin - vec2f(self.scroll_left, self.scroll_top);

        for i in 0..CHILD_EVENT_HANDLER_COUNT {
            for j in 0..CHILD_EVENT_HANDLER_COUNT {
                let cell_origin = adjusted_origin
                    + vec2f(
                        i as f32 * CHILD_EVENT_HANDLER_DIMENSION,
                        j as f32 * CHILD_EVENT_HANDLER_DIMENSION,
                    );
                self.elements[i][j].as_mut().paint(cell_origin, ctx, app);
            }
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
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.elements
            .iter_mut()
            .flatten()
            .any(|element| element.dispatch_event(event, ctx, app))
    }
}

impl NewScrollableElement for ScrollableElement {
    fn axis(&self) -> ScrollableAxis {
        ScrollableAxis::Both
    }

    fn axis_should_handle_scroll_wheel(&self, _axis: Axis) -> bool {
        true
    }

    fn scroll_data(&self, axis: Axis, _app: &AppContext) -> Option<ScrollData> {
        match axis {
            Axis::Horizontal => Some(ScrollData {
                scroll_start: Pixels::new(self.scroll_left),
                visible_px: Pixels::new(self.size.unwrap().x()),
                total_size: Pixels::new(TOTAL_SCROLLABLE_SIZE),
            }),
            Axis::Vertical => Some(ScrollData {
                scroll_start: Pixels::new(self.scroll_top),
                visible_px: Pixels::new(self.size.unwrap().y()),
                total_size: Pixels::new(TOTAL_SCROLLABLE_SIZE),
            }),
        }
    }

    fn scroll(&mut self, delta: Pixels, axis: Axis, ctx: &mut EventContext) {
        match axis {
            Axis::Horizontal => ctx.dispatch_action("test_view:scroll_horizontal", delta.as_f32()),
            Axis::Vertical => ctx.dispatch_action("test_view:scroll_vertical", delta.as_f32()),
        }
    }
}

#[derive(Clone)]
enum ScrollBehavior {
    Manual(ScrollStateHandle),
    Clipped(ClippedScrollStateHandle),
}

struct BasicScrollableView {
    horizontal_axis: Option<ScrollBehavior>,
    vertical_axis: Option<ScrollBehavior>,
    // maps view id to number of mouse downs
    mouse_downs: HashMap<(usize, usize), u32>,
    scroll_top: f32,
    scroll_left: f32,
}

pub fn init(app: &mut AppContext) {
    app.add_action("test_view:mouse_down", BasicScrollableView::mouse_down);
    app.add_action(
        "test_view:scroll_horizontal",
        BasicScrollableView::scroll_horizontal,
    );
    app.add_action(
        "test_view:scroll_vertical",
        BasicScrollableView::scroll_vertical,
    );
}

impl BasicScrollableView {
    fn new(horizontal_axis: Option<ScrollBehavior>, vertical_axis: Option<ScrollBehavior>) -> Self {
        Self {
            horizontal_axis,
            vertical_axis,
            scroll_left: 0.,
            scroll_top: 0.,
            mouse_downs: Default::default(),
        }
    }

    fn mouse_down(&mut self, element_id: &(usize, usize), _ctx: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_down on element_id {element_id:?}");
        let entry = self.mouse_downs.entry(*element_id).or_insert(0);
        *entry += 1;
        true
    }

    fn scroll_horizontal(&mut self, delta: &f32, ctx: &mut ViewContext<Self>) -> bool {
        log::info!("Received scroll horizontal event {}", *delta);
        self.scroll_left = (self.scroll_left - *delta).clamp(0., 257.);
        ctx.notify();
        true
    }

    fn scroll_vertical(&mut self, delta: &f32, ctx: &mut ViewContext<Self>) -> bool {
        log::info!("Received scroll vertical event {}", *delta);
        self.scroll_top = (self.scroll_top - *delta).clamp(0., 257.);
        ctx.notify();
        true
    }
}

impl Entity for BasicScrollableView {
    type Event = String;
}

impl View for BasicScrollableView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        let mut elements = Vec::new();
        for i in 0..CHILD_EVENT_HANDLER_COUNT {
            let mut row = Vec::new();
            for j in 0..CHILD_EVENT_HANDLER_COUNT {
                row.push(
                    EventHandler::new(
                        SavePosition::new(
                            ConstrainedBox::new(Rect::new().finish())
                                .with_height(CHILD_EVENT_HANDLER_DIMENSION)
                                .with_width(CHILD_EVENT_HANDLER_DIMENSION)
                                .finish(),
                            &format!("child-{i}-{j}"),
                        )
                        .finish(),
                    )
                    .on_left_mouse_down(move |evt_ctx, _ctx, _position| {
                        evt_ctx.dispatch_action("test_view:mouse_down", (i, j));
                        DispatchEventResult::StopPropagation
                    })
                    .finish(),
                );
            }
            elements.push(row);
        }

        let element = match (self.horizontal_axis.clone(), self.vertical_axis.clone()) {
            (
                Some(ScrollBehavior::Clipped(horizontal_state)),
                Some(ScrollBehavior::Clipped(vertical_state)),
            ) => {
                let axis_config = DualAxisConfig::Clipped {
                    horizontal: ClippedAxisConfiguration {
                        handle: horizontal_state,
                        max_size: None,
                        stretch_child: false,
                    },
                    vertical: ClippedAxisConfiguration {
                        handle: vertical_state,
                        max_size: None,
                        stretch_child: false,
                    },
                    child: ScrollableElement::new(self.scroll_top, self.scroll_left, elements)
                        .finish(),
                };

                NewScrollable::horizontal_and_vertical(
                    axis_config,
                    ColorU::white().into(),
                    ColorU::white().into(),
                    ColorU::new(100, 100, 100, 255).into(),
                )
                .with_horizontal_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
                .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
            }
            (Some(ScrollBehavior::Manual(horizontal)), Some(ScrollBehavior::Clipped(vertical))) => {
                let axis_config = DualAxisConfig::Manual {
                    horizontal: AxisConfiguration::Manual(horizontal),
                    vertical: AxisConfiguration::Clipped(ClippedAxisConfiguration {
                        handle: vertical,
                        max_size: None,
                        stretch_child: false,
                    }),
                    child: ScrollableElement::new(self.scroll_top, self.scroll_left, elements)
                        .finish_scrollable(),
                };

                NewScrollable::horizontal_and_vertical(
                    axis_config,
                    ColorU::white().into(),
                    ColorU::white().into(),
                    ColorU::new(100, 100, 100, 255).into(),
                )
                .with_horizontal_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
                .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
            }
            (Some(ScrollBehavior::Clipped(horizontal)), Some(ScrollBehavior::Manual(vertical))) => {
                let axis_config = DualAxisConfig::Manual {
                    horizontal: AxisConfiguration::Clipped(ClippedAxisConfiguration {
                        handle: horizontal,
                        max_size: None,
                        stretch_child: false,
                    }),
                    vertical: AxisConfiguration::Manual(vertical),
                    child: ScrollableElement::new(self.scroll_top, self.scroll_left, elements)
                        .finish_scrollable(),
                };

                NewScrollable::horizontal_and_vertical(
                    axis_config,
                    ColorU::white().into(),
                    ColorU::white().into(),
                    ColorU::new(100, 100, 100, 255).into(),
                )
                .with_horizontal_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
                .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
            }
            (Some(ScrollBehavior::Manual(horizontal)), Some(ScrollBehavior::Manual(vertical))) => {
                let axis_config = DualAxisConfig::Manual {
                    horizontal: AxisConfiguration::Manual(horizontal),
                    vertical: AxisConfiguration::Manual(vertical),
                    child: ScrollableElement::new(self.scroll_top, self.scroll_left, elements)
                        .finish_scrollable(),
                };

                NewScrollable::horizontal_and_vertical(
                    axis_config,
                    ColorU::white().into(),
                    ColorU::white().into(),
                    ColorU::new(100, 100, 100, 255).into(),
                )
                .with_horizontal_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
                .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
            }
            (Some(ScrollBehavior::Clipped(horizontal)), None) => {
                let axis_config = SingleAxisConfig::Clipped {
                    handle: horizontal,
                    child: ScrollableElement::new(self.scroll_top, self.scroll_left, elements)
                        .finish(),
                };

                NewScrollable::horizontal(
                    axis_config,
                    ColorU::white().into(),
                    ColorU::white().into(),
                    ColorU::new(100, 100, 100, 255).into(),
                )
                .with_horizontal_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
            }
            (Some(ScrollBehavior::Manual(horizontal)), None) => {
                let axis_config = SingleAxisConfig::Manual {
                    handle: horizontal,
                    child: ScrollableElement::new(self.scroll_top, self.scroll_left, elements)
                        .finish_scrollable(),
                };

                NewScrollable::horizontal(
                    axis_config,
                    ColorU::white().into(),
                    ColorU::white().into(),
                    ColorU::new(100, 100, 100, 255).into(),
                )
                .with_horizontal_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
            }
            (None, Some(ScrollBehavior::Manual(vertical))) => {
                let axis_config = SingleAxisConfig::Manual {
                    handle: vertical,
                    child: ScrollableElement::new(self.scroll_top, self.scroll_left, elements)
                        .finish_scrollable(),
                };

                NewScrollable::vertical(
                    axis_config,
                    ColorU::white().into(),
                    ColorU::white().into(),
                    ColorU::new(100, 100, 100, 255).into(),
                )
                .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
            }
            (None, Some(ScrollBehavior::Clipped(vertical))) => {
                let axis_config = SingleAxisConfig::Clipped {
                    handle: vertical,
                    child: ScrollableElement::new(self.scroll_top, self.scroll_left, elements)
                        .finish(),
                };

                NewScrollable::vertical(
                    axis_config,
                    ColorU::white().into(),
                    ColorU::white().into(),
                    ColorU::new(100, 100, 100, 255).into(),
                )
                .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
            }
            (None, None) => panic!("Invalid test configuration"),
        };

        let constrained = ConstrainedBox::new(element.finish())
            .with_height(SCROLLABLE_VIEWPORT_SIZE)
            .with_width(SCROLLABLE_VIEWPORT_SIZE);

        Stack::new()
            .with_child(Rect::new().with_background_color(ColorU::black()).finish())
            .with_child(constrained.finish())
            .finish()
    }

    fn ui_name() -> &'static str {
        "View"
    }
}

impl TypedActionView for BasicScrollableView {
    type Action = ();
}

fn render(presenter: &mut Presenter, view_id: EntityId, ctx: &mut AppContext) {
    let mut updated = EntityIdSet::default();
    updated.insert(view_id);
    let invalidation = WindowInvalidation {
        updated,
        ..Default::default()
    };

    presenter.invalidate(invalidation, ctx);
    presenter.build_scene(vec2f(1000., 1000.), 1., None, ctx);
}

#[test]
fn clipped_scrollable_selection_apis_use_viewport_coordinates() {
    let probe = SelectableProbeState::default();
    let scrollable = test_clipped_horizontal_scrollable_with_probe(probe.clone(), 64.0);
    let start = vec2f(180.0, 24.0);
    let end = vec2f(220.0, 24.0);

    let fragments = scrollable
        .get_selection(start, end, IsRect::False)
        .expect("probe selection should succeed");
    assert_eq!(fragments[0].text, "probe");
    assert_eq!(
        probe.get_selection_args.borrow().as_slice(),
        &[(start, end, IsRect::False)]
    );

    let expanded = scrollable
        .expand_selection(
            start,
            SelectionDirection::Forward,
            SelectionType::Semantic,
            &WordBoundariesPolicy::Default,
        )
        .expect("probe expansion should succeed");
    assert_eq!(expanded, start + vec2f(5.0, 0.0));
    let expand_args = probe.expand_selection_args.borrow();
    assert_eq!(expand_args.len(), 1);
    assert_eq!(expand_args[0].0, start);
    assert!(matches!(expand_args[0].1, SelectionDirection::Forward));
    assert!(matches!(expand_args[0].2, SelectionType::Semantic));

    let is_before = scrollable
        .is_point_semantically_before(start, end)
        .expect("probe semantic comparison should succeed");
    assert!(is_before);
    assert_eq!(
        probe.semantic_order_args.borrow().as_slice(),
        &[(start, end)]
    );

    let smart_selection = scrollable
        .smart_select(start, select_entire_probe_text)
        .expect("probe smart select should succeed");
    assert_eq!(smart_selection, (start, start + vec2f(12.0, 0.0)));
    assert_eq!(probe.smart_select_args.borrow().as_slice(), &[start]);
}

#[test]
fn clipped_scrollable_reanchors_existing_selection_after_horizontal_scroll() {
    let probe = SelectableProbeState::default();
    let handle = ClippedScrollStateHandle::default();
    handle.scroll_to(Pixels::new(64.0));
    let selection = crate::elements::Selection {
        start: vec2f(180.0, 24.0),
        end: vec2f(220.0, 24.0),
        is_rect: IsRect::False,
    };

    let scrollable =
        test_clipped_horizontal_scrollable_with_probe_handle(probe.clone(), handle.clone());
    scrollable
        .get_selection(selection.start, selection.end, selection.is_rect)
        .expect("initial probe selection should succeed");
    assert_eq!(
        probe.get_selection_args.borrow().last().copied(),
        Some((selection.start, selection.end, selection.is_rect))
    );

    handle.scroll_to(Pixels::new(96.0));
    let scrollable = test_clipped_horizontal_scrollable_with_probe_handle(probe.clone(), handle);
    scrollable
        .get_selection(selection.start, selection.end, selection.is_rect)
        .expect("reanchored probe selection should succeed");
    assert_eq!(
        probe.get_selection_args.borrow().last().copied(),
        Some((vec2f(148.0, 24.0), vec2f(188.0, 24.0), IsRect::False))
    );

    scrollable.calculate_clickable_bounds(Some(selection));
    let clickable_bounds_args = probe.clickable_bounds_args.borrow();
    let latest_selection = clickable_bounds_args
        .last()
        .copied()
        .flatten()
        .expect("scrollable should forward adjusted clickable-bounds selection");
    assert_eq!(latest_selection.start, vec2f(148.0, 24.0));
    assert_eq!(latest_selection.end, vec2f(188.0, 24.0));
    assert_eq!(latest_selection.is_rect, IsRect::False);
}

#[test]
fn clearing_scroll_anchor_treats_same_viewport_selection_as_new_content() {
    let probe = SelectableProbeState::default();
    let handle = ClippedScrollStateHandle::default();
    handle.scroll_to(Pixels::new(64.0));
    let selection = crate::elements::Selection {
        start: vec2f(180.0, 24.0),
        end: vec2f(220.0, 24.0),
        is_rect: IsRect::False,
    };

    let scrollable =
        test_clipped_horizontal_scrollable_with_probe_handle(probe.clone(), handle.clone());
    scrollable
        .get_selection(selection.start, selection.end, selection.is_rect)
        .expect("initial probe selection should succeed");

    handle.scroll_to(Pixels::new(96.0));
    let scrollable = test_clipped_horizontal_scrollable_with_probe_handle(probe.clone(), handle);
    scrollable.clear_selection_scroll_anchor();
    scrollable
        .get_selection(selection.start, selection.end, selection.is_rect)
        .expect("selection after anchor clear should use current viewport coordinates");

    assert_eq!(
        probe.get_selection_args.borrow().last().copied(),
        Some((selection.start, selection.end, selection.is_rect))
    );
}

#[test]
fn test_click_to_scroll_dual() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);

        let dual_configurations = [
            (
                ScrollBehavior::Clipped(Default::default()),
                ScrollBehavior::Clipped(Default::default()),
            ),
            (
                ScrollBehavior::Manual(Default::default()),
                ScrollBehavior::Clipped(Default::default()),
            ),
            (
                ScrollBehavior::Clipped(Default::default()),
                ScrollBehavior::Manual(Default::default()),
            ),
            (
                ScrollBehavior::Manual(Default::default()),
                ScrollBehavior::Manual(Default::default()),
            ),
        ];

        for (x_config, y_config) in dual_configurations {
            let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
                BasicScrollableView::new(Some(x_config), Some(y_config))
            });

            let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
            let view_id = app.root_view_id(window_id).unwrap();

            app.update(move |ctx| {
                render(&mut presenter.borrow_mut(), view_id, ctx);

                // Fire event on child (0, 0)
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 0.5,
                            CHILD_EVENT_HANDLER_DIMENSION * 0.5,
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );

                // Fire event on child (2, 1)
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 2.5,
                            CHILD_EVENT_HANDLER_DIMENSION * 1.5,
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );

                // Click on the vertical scrollbar track. This should scroll the view down.
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 5.0 - ScrollbarWidth::Auto.as_f32(),
                            CHILD_EVENT_HANDLER_DIMENSION * 4.5,
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );

                // Click on the horizontal scrollbar track. This should scroll the view right.
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 4.5,
                            CHILD_EVENT_HANDLER_DIMENSION * 5.0 - ScrollbarWidth::Auto.as_f32(),
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );
            });

            view.read(app, |view, _ctx| {
                for (coord, count) in view.mouse_downs.iter() {
                    match coord {
                        (0, 0) | (2, 1) => assert_eq!(1, *count),
                        _ => assert_eq!(0, *count),
                    }
                }

                match view.vertical_axis.clone().unwrap() {
                    ScrollBehavior::Clipped(handle) => {
                        assert!(handle.scroll_start().as_f32() > 0.)
                    }
                    ScrollBehavior::Manual(_) => assert!(view.scroll_top > 0.),
                };

                match view.horizontal_axis.clone().unwrap() {
                    ScrollBehavior::Clipped(handle) => {
                        assert!(handle.scroll_start().as_f32() > 0.)
                    }
                    ScrollBehavior::Manual(_) => assert!(view.scroll_left > 0.),
                };
            });

            app.update(|ctx| {
                ctx.windows()
                    .close_window(window_id, TerminationMode::ForceTerminate)
            });
        }
    })
}

#[test]
fn test_click_to_scroll_horizontal() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);

        let configurations = [
            ScrollBehavior::Manual(Default::default()),
            ScrollBehavior::Clipped(Default::default()),
        ];

        for config in configurations {
            let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
                BasicScrollableView::new(Some(config), None)
            });

            let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
            let view_id = app.root_view_id(window_id).unwrap();

            app.update(move |ctx| {
                render(&mut presenter.borrow_mut(), view_id, ctx);

                // Fire event on child (0, 0)
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 0.5,
                            CHILD_EVENT_HANDLER_DIMENSION * 0.5,
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );

                // Fire event on child (2, 1)
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 2.5,
                            CHILD_EVENT_HANDLER_DIMENSION * 1.5,
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );

                // Click on the vertical scrollbar track. This should NOT scroll the view down.
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 5.0 - ScrollbarWidth::Auto.as_f32(),
                            CHILD_EVENT_HANDLER_DIMENSION * 4.5,
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );

                // Click on the horizontal scrollbar track. This should scroll the view right.
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 4.5,
                            CHILD_EVENT_HANDLER_DIMENSION * 5.0 - ScrollbarWidth::Auto.as_f32(),
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );
            });

            view.read(app, |view, _ctx| {
                for (coord, count) in view.mouse_downs.iter() {
                    match coord {
                        (0, 0) | (2, 1) | (4, 4) => assert_eq!(1, *count),
                        _ => assert_eq!(0, *count),
                    }
                }

                match view.horizontal_axis.clone().unwrap() {
                    ScrollBehavior::Clipped(handle) => {
                        assert!(handle.scroll_start().as_f32() > 0.)
                    }
                    ScrollBehavior::Manual(_) => assert!(view.scroll_left > 0.),
                };
            });

            app.update(|ctx| {
                ctx.windows()
                    .close_window(window_id, TerminationMode::ForceTerminate)
            });
        }
    })
}

#[test]
fn test_click_to_scroll_vertical() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);

        let configurations = [
            ScrollBehavior::Manual(Default::default()),
            ScrollBehavior::Clipped(Default::default()),
        ];

        for config in configurations {
            let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
                BasicScrollableView::new(None, Some(config))
            });

            let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
            let view_id = app.root_view_id(window_id).unwrap();

            app.update(move |ctx| {
                render(&mut presenter.borrow_mut(), view_id, ctx);

                // Fire event on child (0, 0)
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 0.5,
                            CHILD_EVENT_HANDLER_DIMENSION * 0.5,
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );

                // Fire event on child (2, 1)
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 2.5,
                            CHILD_EVENT_HANDLER_DIMENSION * 1.5,
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );

                // Click on the vertical scrollbar track. This should scroll the view down.
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 5.0 - ScrollbarWidth::Auto.as_f32(),
                            CHILD_EVENT_HANDLER_DIMENSION * 4.5,
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );

                // Click on the horizontal scrollbar track. This should NOT scroll the view right.
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(
                            CHILD_EVENT_HANDLER_DIMENSION * 4.5,
                            CHILD_EVENT_HANDLER_DIMENSION * 5.0 - ScrollbarWidth::Auto.as_f32(),
                        ),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );
            });

            view.read(app, |view, _ctx| {
                for (coord, count) in view.mouse_downs.iter() {
                    match coord {
                        (0, 0) | (2, 1) | (4, 4) => assert_eq!(1, *count),
                        _ => assert_eq!(0, *count),
                    }
                }

                match view.vertical_axis.clone().unwrap() {
                    ScrollBehavior::Clipped(handle) => {
                        assert!(handle.scroll_start().as_f32() > 0.)
                    }
                    ScrollBehavior::Manual(_) => assert!(view.scroll_top > 0.),
                };
            });

            app.update(|ctx| {
                ctx.windows()
                    .close_window(window_id, TerminationMode::ForceTerminate)
            });
        }
    })
}

#[test]
fn test_scroll_to_position_dual() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            BasicScrollableView::new(
                Some(ScrollBehavior::Clipped(Default::default())),
                Some(ScrollBehavior::Clipped(Default::default())),
            )
        });

        let mut presenter = Presenter::new(window_id);
        let view_id = app.root_view_id(window_id).unwrap();

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let (horizontal, vertical) = get_scroll_handles(view);
            assert_eq!(horizontal.scroll_start().as_f32(), 0.);
            assert_eq!(vertical.scroll_start().as_f32(), 0.);
            vertical.scroll_to_position(ScrollTarget {
                position_id: "child-4-8".to_owned(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        });

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let (horizontal, vertical) = get_scroll_handles(view);
            assert_eq!(horizontal.scroll_start().as_f32(), 0.);
            assert_eq!(
                vertical.scroll_start().as_f32(),
                position_for_child(8, Boundary::End)
            );
            vertical.scroll_to_position(ScrollTarget {
                position_id: "child-8-2".to_owned(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        });

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let (horizontal, vertical) = get_scroll_handles(view);
            assert_eq!(horizontal.scroll_start().as_f32(), 0.);
            assert_eq!(
                vertical.scroll_start().as_f32(),
                position_for_child(2, Boundary::Start)
            );
            horizontal.scroll_to_position(ScrollTarget {
                position_id: "child-6-3".to_owned(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
            vertical.scroll_to_position(ScrollTarget {
                position_id: "child-6-3".to_owned(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        });

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let (horizontal, vertical) = get_scroll_handles(view);
            assert_eq!(
                horizontal.scroll_start().as_f32(),
                position_for_child(6, Boundary::End)
            );
            assert_eq!(
                vertical.scroll_start().as_f32(),
                position_for_child(2, Boundary::Start)
            );
        });
    })
}

fn get_scroll_handles(
    view: &BasicScrollableView,
) -> (&ClippedScrollStateHandle, &ClippedScrollStateHandle) {
    let Some((ScrollBehavior::Clipped(horizontal), ScrollBehavior::Clipped(vertical))) = view
        .horizontal_axis
        .as_ref()
        .zip(view.vertical_axis.as_ref())
    else {
        panic!("invalid test config");
    };
    (horizontal, vertical)
}

#[test]
fn test_scroll_to_position_horizontal() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            BasicScrollableView::new(Some(ScrollBehavior::Clipped(Default::default())), None)
        });

        let mut presenter = Presenter::new(window_id);
        let view_id = app.root_view_id(window_id).unwrap();

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.horizontal_axis.as_ref() else {
                panic!("invalid test config");
            };
            assert_eq!(handle.scroll_start().as_f32(), 0.);
            handle.scroll_to_position(ScrollTarget {
                position_id: "child-4-2".to_owned(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        });

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.horizontal_axis.as_ref() else {
                panic!("invalid test config");
            };
            assert_eq!(handle.scroll_start().as_f32(), 0.);
            handle.scroll_to_position(ScrollTarget {
                position_id: "child-5-2".to_owned(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        });

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.horizontal_axis.as_ref() else {
                panic!("invalid test config");
            };
            assert_eq!(
                handle.scroll_start().as_f32(),
                position_for_child(5, Boundary::End)
            );
            handle.scroll_to_position(ScrollTarget {
                position_id: "child-0-0".to_owned(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        });

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.horizontal_axis.as_ref() else {
                panic!("invalid test config");
            };
            assert_eq!(handle.scroll_start().as_f32(), 0.);
        });
    })
}

#[test]
fn test_scroll_to_position_vertical() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            BasicScrollableView::new(None, Some(ScrollBehavior::Clipped(Default::default())))
        });

        let mut presenter = Presenter::new(window_id);
        let view_id = app.root_view_id(window_id).unwrap();

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.vertical_axis.as_ref() else {
                panic!("invalid test config");
            };
            assert_eq!(handle.scroll_start().as_f32(), 0.);
            handle.scroll_to_position(ScrollTarget {
                position_id: "child-1-9".to_owned(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        });

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.vertical_axis.as_ref() else {
                panic!("invalid test config");
            };
            assert_eq!(
                handle.scroll_start().as_f32(),
                position_for_child(9, Boundary::End)
            );
            handle.scroll_to_position(ScrollTarget {
                position_id: "child-3-6".to_owned(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        });

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.vertical_axis.as_ref() else {
                panic!("invalid test config");
            };
            assert_eq!(
                handle.scroll_start().as_f32(),
                position_for_child(9, Boundary::End)
            );
            // This example is subtly different from the rest b/c child (4, 3) is partially
            // clipped on the right by the scrollbar gutter. That clipping shouldn't affect
            // vertical scrolling.
            handle.scroll_to_position(ScrollTarget {
                position_id: "child-4-3".to_owned(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        });

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.vertical_axis.as_ref() else {
                panic!("invalid test config");
            };
            assert_eq!(
                handle.scroll_start().as_f32(),
                position_for_child(3, Boundary::Start)
            );
        });
    })
}

enum Boundary {
    Start,
    End,
}

/// Returns what the scroll_start value should be to have the child square at the edge of the
/// viewport (either the start or the end).
///
/// For example, if we want to scroll the x-axis to child (6, 1) at the end, we need to set
/// scroll_start to 100px:
/// ```
/// assert_eq!(position_for_child(6, Boundary::End), 100.);
/// ```
///            Viewport
///   100px┌──────┴───────┐
///  ┌──┴──┐
///
///   0  1  2  3  4  5  6  7  8  9  
///  ┌──┬──┲━━┯━━┯━━┯━━┯━━┱──┬──┬──┐  ┐
/// 0│  │  ┃  │  │  │  │  ┃  │  │  │  │
///  ├──┼──╂──┼──┼──┼──┼──╂──┼──┼──┤  │
/// 1│  │  ┃  │  │  │  │**┃  │  │  │  │
///  ├──┼──╂──┼──┼──┼──┼──╂──┼──┼──┤  │
/// 2│  │  ┃  │  │  │  │  ┃  │  │  │  ├─Viewport
///  ├──┼──╂──┼──┼──┼──┼──╂──┼──┼──┤  │
/// 3│  │  ┃  │  │  │  │  ┃  │  │  │  │
///  ├──┼──╂──┼──┼──┼──┼──╂──┼──┼──┤  │
/// 4│  │  ┃  │  │  │  │  ┃  │  │  │  │
///  ├──┼──╄━━┿━━┿━━┿━━┿━━╃──┼──┼──┤  ┘
/// 5│  │  │  │  │  │  │  │  │  │  │
///  ├──┼──┼──┼──┼──┼──┼──┼──┼──┼──┤
/// 6│  │  │  │  │  │  │  │  │  │  │
///  ├──┼──┼──┼──┼──┼──┼──┼──┼──┼──┤
/// 7│  │  │  │  │  │  │  │  │  │  │
///  ├──┼──┼──┼──┼──┼──┼──┼──┼──┼──┤
/// 8│  │  │  │  │  │  │  │  │  │  │
///  ├──┼──┼──┼──┼──┼──┼──┼──┼──┼──┤
/// 9│  │  │  │  │  │  │  │  │  │  │
///  └──┴──┴──┴──┴──┴──┴──┴──┴──┴──┘
fn position_for_child(i: usize, boundary: Boundary) -> f32 {
    let mut pos = CHILD_EVENT_HANDLER_DIMENSION * i as f32;
    if let Boundary::End = boundary {
        pos -= SCROLLABLE_VIEWPORT_SIZE - CHILD_EVENT_HANDLER_DIMENSION;
    }
    pos.clamp(0., SCROLLABLE_VIEWPORT_SIZE)
}

fn dispatch_non_precise_wheel_down(
    ctx: &mut AppContext,
    window_id: crate::WindowId,
    presenter: Rc<RefCell<Presenter>>,
) {
    ctx.simulate_window_event(
        Event::ScrollWheel {
            position: vec2f(100., 100.),
            delta: vec2f(0., -1.),
            precise: false,
            modifiers: ModifiersState::default(),
        },
        window_id,
        presenter,
    );
}

fn dispatch_precise_wheel_down(
    ctx: &mut AppContext,
    window_id: crate::WindowId,
    presenter: Rc<RefCell<Presenter>>,
    delta_px: f32,
) {
    ctx.simulate_window_event(
        Event::ScrollWheel {
            position: vec2f(100., 100.),
            delta: vec2f(0., -delta_px),
            precise: true,
            modifiers: ModifiersState::default(),
        },
        window_id,
        presenter,
    );
}

/// Sets up a single, vertically Clipped-scrolling [`BasicScrollableView`] and returns its window
/// id, view handle, and presenter, having already rendered once (required before the scrollable
/// will handle any dispatched event).
fn setup_vertical_clipped_scrollable(
    app: &mut App,
) -> (
    crate::WindowId,
    crate::ViewHandle<BasicScrollableView>,
    Rc<RefCell<Presenter>>,
) {
    app.update(init);
    let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
        BasicScrollableView::new(None, Some(ScrollBehavior::Clipped(Default::default())))
    });
    let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
    let view_id = app.root_view_id(window_id).unwrap();
    app.update(|ctx| render(&mut presenter.borrow_mut(), view_id, ctx));
    (window_id, view, presenter)
}

fn vertical_handle(view: &BasicScrollableView) -> &ClippedScrollStateHandle {
    let Some(ScrollBehavior::Clipped(handle)) = view.vertical_axis.as_ref() else {
        panic!("invalid test config");
    };
    handle
}

/// Sets up a single, vertically Manual-scrolling [`BasicScrollableView`] and returns its window
/// id, view handle, and presenter, having already rendered once.
fn setup_vertical_manual_scrollable(
    app: &mut App,
) -> (
    crate::WindowId,
    crate::ViewHandle<BasicScrollableView>,
    Rc<RefCell<Presenter>>,
) {
    app.update(init);
    let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
        BasicScrollableView::new(None, Some(ScrollBehavior::Manual(Default::default())))
    });
    let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
    let view_id = app.root_view_id(window_id).unwrap();
    app.update(|ctx| render(&mut presenter.borrow_mut(), view_id, ctx));
    (window_id, view, presenter)
}

#[test]
fn non_precise_wheel_scroll_animates_toward_target_when_smooth_scrolling_enabled() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view, presenter) = setup_vertical_clipped_scrollable(app);

        app.update(|ctx| dispatch_non_precise_wheel_down(ctx, window_id, presenter.clone()));

        view.read(app, |view, _| {
            let handle = vertical_handle(view);
            let target = handle.scroll_target().as_f32();
            assert!(target > 0., "wheel notch should have set a positive target");
            // The animation has barely started, so the displayed position lags the target.
            assert!(handle.scroll_start().as_f32() < target);
            assert!(handle.is_animating());
        });

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

#[test]
fn non_precise_wheel_scroll_applies_immediately_when_smooth_scrolling_disabled() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(false);

    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view, presenter) = setup_vertical_clipped_scrollable(app);

        app.update(|ctx| dispatch_non_precise_wheel_down(ctx, window_id, presenter.clone()));

        view.read(app, |view, _| {
            let handle = vertical_handle(view);
            assert!(handle.scroll_start().as_f32() > 0.);
            assert_eq!(
                handle.scroll_start().as_f32(),
                handle.scroll_target().as_f32()
            );
            assert!(!handle.is_animating());
        });

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

#[test]
fn precise_wheel_scroll_applies_immediately_even_when_smooth_scrolling_enabled() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view, presenter) = setup_vertical_clipped_scrollable(app);

        app.update(|ctx| dispatch_precise_wheel_down(ctx, window_id, presenter.clone(), 40.));

        view.read(app, |view, _| {
            let handle = vertical_handle(view);
            assert!(handle.scroll_start().as_f32() > 0.);
            assert_eq!(
                handle.scroll_start().as_f32(),
                handle.scroll_target().as_f32()
            );
            assert!(!handle.is_animating());
        });

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

#[test]
fn scrollbar_drag_cancels_in_flight_smooth_scroll_animation() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view, presenter) = setup_vertical_clipped_scrollable(app);

        app.update(|ctx| dispatch_non_precise_wheel_down(ctx, window_id, presenter.clone()));
        view.read(app, |view, _| {
            assert!(vertical_handle(view).is_animating());
        });

        // Click on the vertical scrollbar track: a direct scroll operation that should cancel
        // the in-flight animation and apply immediately at the currently displayed position.
        app.update(|ctx| {
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(
                        CHILD_EVENT_HANDLER_DIMENSION * 5.0 - ScrollbarWidth::Auto.as_f32(),
                        CHILD_EVENT_HANDLER_DIMENSION * 4.5,
                    ),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
        });

        view.read(app, |view, _| {
            let handle = vertical_handle(view);
            assert!(!handle.is_animating());
            assert_eq!(
                handle.scroll_start().as_f32(),
                handle.scroll_target().as_f32()
            );
        });

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

#[test]
fn same_direction_wheel_notches_compose_into_a_larger_target() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view, presenter) = setup_vertical_clipped_scrollable(app);

        app.update(|ctx| dispatch_non_precise_wheel_down(ctx, window_id, presenter.clone()));
        let target_after_first_notch = view.read(app, |view, _| {
            vertical_handle(view).scroll_target().as_f32()
        });

        app.update(|ctx| dispatch_non_precise_wheel_down(ctx, window_id, presenter.clone()));
        view.read(app, |view, _| {
            let handle = vertical_handle(view);
            // The second notch composes with the first rather than restarting it: the target
            // grows by another notch's worth of distance.
            assert_eq!(
                handle.scroll_target().as_f32(),
                target_after_first_notch * 2.
            );
            assert!(handle.is_animating());
        });

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

#[test]
fn manual_axis_wheel_scroll_eventually_matches_immediate_scroll_distance() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view, presenter) = setup_vertical_manual_scrollable(app);

        app.update(|ctx| dispatch_non_precise_wheel_down(ctx, window_id, presenter.clone()));

        // No increment has been emitted to the child yet: the manual child's own scroll state
        // is only advanced lazily, as further events are dispatched to the scrollable.
        view.read(app, |view, _| assert_eq!(view.scroll_top, 0.));

        // Wait past the animation's duration (up to 200ms at the slow end of the inverse-delta
        // ramp), then dispatch another event (standing in for the synthetic MouseMoved the app
        // replays after each scheduled repaint) to drain it.
        std::thread::sleep(Duration::from_millis(300));
        app.update(|ctx| {
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: vec2f(100., 100.),
                    cmd: false,
                    shift: false,
                    is_synthetic: true,
                },
                window_id,
                presenter.clone(),
            );
        });

        // The final position matches exactly what an immediate (non-animated) scroll of the
        // same notch would have produced: 1 line * 40px-per-line.
        view.read(app, |view, _| assert_eq!(view.scroll_top, 40.));

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

#[test]
fn precise_input_interrupts_an_already_active_clipped_tween() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view, presenter) = setup_vertical_clipped_scrollable(app);

        app.update(|ctx| dispatch_non_precise_wheel_down(ctx, window_id, presenter.clone()));
        let displayed_at_interrupt = view.read(app, |view, _| {
            let handle = vertical_handle(view);
            assert!(handle.is_animating());
            handle.scroll_start().as_f32()
        });

        // A precise (trackpad) event arrives mid-flight: it must cancel the tween at its
        // currently displayed position, then apply its own delta immediately and exactly once.
        app.update(|ctx| dispatch_precise_wheel_down(ctx, window_id, presenter.clone(), 10.));

        view.read(app, |view, _| {
            let handle = vertical_handle(view);
            assert!(!handle.is_animating());
            // The tween keeps easing in the (small) real time between sampling
            // `displayed_at_interrupt` above and the precise event actually cancelling it, so
            // allow a small tolerance rather than expecting bit-for-bit equality.
            let expected = displayed_at_interrupt + 10.;
            assert!(
                (handle.scroll_start().as_f32() - expected).abs() < 5.,
                "expected ~{expected}, got {}",
                handle.scroll_start().as_f32()
            );
            assert_eq!(
                handle.scroll_start().as_f32(),
                handle.scroll_target().as_f32()
            );
        });

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

#[test]
fn can_scroll_delta_uses_target_not_lagging_displayed_position_for_clipped_axis() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |app| async move {
        app.read(|ctx| {
            let handle = ClippedScrollStateHandle::default();
            let child: Box<dyn Element> =
                Box::new(SelectableProbeElement::new(SelectableProbeState::default()));
            let config = SingleAxisConfig::Clipped {
                handle: handle.clone(),
                child,
            };
            // The probe child is 120px tall; constrain the viewport to 60px so the max scroll
            // position (target boundary) is exactly 60px.
            let viewport_size = vec2f(400., 60.);
            let start = Instant::now();

            handle.animate_scroll_by(60_f32.into_pixels(), start);
            // The tween has barely started: the displayed position lags the target, which has
            // already reached the boundary.
            assert!(handle.scroll_start().as_f32() < handle.scroll_target().as_f32());
            assert_eq!(handle.scroll_target().as_f32(), 60.);

            // A further same-direction notch must be reported as unable to scroll further (so
            // it propagates to a parent scrollable), even though the displayed position hasn't
            // caught up to the boundary yet.
            assert!(!config.can_scroll_delta(Axis::Vertical, viewport_size, vec2f(0., -1.), ctx));
        });
    })
}

#[test]
fn dual_axis_notches_animate_each_axis_independently_to_completion() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            BasicScrollableView::new(
                Some(ScrollBehavior::Clipped(Default::default())),
                Some(ScrollBehavior::Clipped(Default::default())),
            )
        });
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let view_id = app.root_view_id(window_id).unwrap();
        app.update(|ctx| render(&mut presenter.borrow_mut(), view_id, ctx));

        // A single dual-axis notch starts an independent animation on each axis.
        app.update(|ctx| {
            ctx.simulate_window_event(
                Event::ScrollWheel {
                    position: vec2f(100., 100.),
                    delta: vec2f(-1., -1.),
                    precise: false,
                    modifiers: ModifiersState::default(),
                },
                window_id,
                presenter.clone(),
            );
        });

        view.read(app, |view, _| {
            let (horizontal, vertical) = get_scroll_handles(view);
            assert!(horizontal.is_animating());
            assert!(vertical.is_animating());
            assert_eq!(horizontal.scroll_target().as_f32(), 40.);
            assert_eq!(vertical.scroll_target().as_f32(), 40.);
        });

        // Once both tweens finish, each axis lands exactly on its own target, independent of
        // the other. 300ms comfortably exceeds the 200ms slow end of the inverse-delta ramp.
        std::thread::sleep(Duration::from_millis(300));
        app.update(|ctx| render(&mut presenter.borrow_mut(), view_id, ctx));

        view.read(app, |view, _| {
            let (horizontal, vertical) = get_scroll_handles(view);
            assert!(!horizontal.is_animating());
            assert!(!vertical.is_animating());
            assert_eq!(horizontal.scroll_start().as_f32(), 40.);
            assert_eq!(vertical.scroll_start().as_f32(), 40.);
        });

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

/// A single-axis element that records how many times it's painted, in an `Rc<Cell<usize>>`
/// shared with the test, used to measure the actual repaint cadence achieved over the course of
/// a smooth-scroll animation.
struct PaintCountingElement {
    size: Vector2F,
    paint_count: Rc<Cell<usize>>,
}

impl Element for PaintCountingElement {
    fn layout(
        &mut self,
        _constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        self.size
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, _origin: Vector2F, _ctx: &mut PaintContext, _app: &AppContext) {
        self.paint_count.set(self.paint_count.get() + 1);
    }

    fn size(&self) -> Option<Vector2F> {
        Some(self.size)
    }

    fn origin(&self) -> Option<Point> {
        Some(Point::new(0., 0., ZIndex::new(0)))
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

#[derive(Default)]
struct PaintCountingScrollView {
    handle: ClippedScrollStateHandle,
    paint_count: Rc<Cell<usize>>,
}

impl Entity for PaintCountingScrollView {
    type Event = ();
}

impl View for PaintCountingScrollView {
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let axis_config = SingleAxisConfig::Clipped {
            handle: self.handle.clone(),
            child: Box::new(PaintCountingElement {
                size: vec2f(SCROLLABLE_VIEWPORT_SIZE, 500.),
                paint_count: self.paint_count.clone(),
            })
            .finish(),
        };
        let scrollable = NewScrollable::vertical(axis_config, Fill::None, Fill::None, Fill::None);
        ConstrainedBox::new(scrollable.finish())
            .with_height(SCROLLABLE_VIEWPORT_SIZE)
            .with_width(SCROLLABLE_VIEWPORT_SIZE)
            .finish()
    }

    fn ui_name() -> &'static str {
        "PaintCountingScrollView"
    }
}

impl TypedActionView for PaintCountingScrollView {
    type Action = ();
}

/// Measures how many distinct frames the smooth-scroll animation's own self-scheduling chain
/// (`PaintContext::repaint_after` -> `manage_delayed_repaint_timers` -> a real async timer ->
/// `request_redraw`) actually drives over one full animation, in the absence of any
/// display/vsync throttling (there is no real display in this test; the harness eagerly builds
/// the scene on every invalidation). This answers "is our own scheduling code the bottleneck":
/// if this count is healthy, any remaining steppiness on a real display is downstream of the
/// platform's actual redraw cadence, not of this code requesting repaints too infrequently.
#[test]
fn smooth_scroll_animation_drives_many_distinct_repaints_over_its_duration() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        let paint_count = Rc::new(Cell::new(0usize));
        let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, {
            let paint_count = paint_count.clone();
            move |_| PaintCountingScrollView {
                handle: Default::default(),
                paint_count,
            }
        });

        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let view_id = app.root_view_id(window_id).unwrap();
        app.update(|ctx| render(&mut presenter.borrow_mut(), view_id, ctx));
        let paints_before_scroll = paint_count.get();

        app.update(|ctx| dispatch_non_precise_wheel_down(ctx, window_id, presenter.clone()));

        // Let the animation's self-scheduled repaints run for comfortably longer than its
        // duration (this awaits real wall-clock time, letting the spawned repaint-timer tasks
        // actually fire, same as the pre-existing hover-delay tests in `hoverable_tests.rs`).
        crate::r#async::Timer::after(Duration::from_millis(200)).await;

        let paints_during_animation = paint_count.get() - paints_before_scroll;
        // At an 8ms self-requested interval over a 120ms animation, the code's own scheduling
        // asks for on the order of a dozen repaints; require a healthy fraction of that so a
        // regression that throttles or drops requests is caught, without being so strict that
        // ordinary test-timing jitter fails it.
        assert!(
            paints_during_animation >= 6,
            "expected the animation to have driven at least 6 distinct repaints via its own \
             self-scheduling, got {paints_during_animation}"
        );

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

/// A single-axis element whose size can be changed between layout passes, used to exercise
/// content shrinking mid-animation.
struct ResizableElement {
    size: Rc<Cell<Vector2F>>,
    laid_out_size: Option<Vector2F>,
}

impl Element for ResizableElement {
    fn layout(
        &mut self,
        _constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        let size = self.size.get();
        self.laid_out_size = Some(size);
        size
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, _origin: Vector2F, _ctx: &mut PaintContext, _app: &AppContext) {}

    fn size(&self) -> Option<Vector2F> {
        self.laid_out_size
    }

    fn origin(&self) -> Option<Point> {
        Some(Point::new(0., 0., ZIndex::new(0)))
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

#[derive(Default)]
struct ResizableScrollView {
    handle: ClippedScrollStateHandle,
    content_size: Rc<Cell<Vector2F>>,
}

impl Entity for ResizableScrollView {
    type Event = ();
}

impl View for ResizableScrollView {
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let axis_config = SingleAxisConfig::Clipped {
            handle: self.handle.clone(),
            child: Box::new(ResizableElement {
                size: self.content_size.clone(),
                laid_out_size: None,
            })
            .finish(),
        };
        let scrollable = NewScrollable::vertical(axis_config, Fill::None, Fill::None, Fill::None);
        ConstrainedBox::new(scrollable.finish())
            .with_height(SCROLLABLE_VIEWPORT_SIZE)
            .with_width(SCROLLABLE_VIEWPORT_SIZE)
            .finish()
    }

    fn ui_name() -> &'static str {
        "ResizableScrollView"
    }
}

impl TypedActionView for ResizableScrollView {
    type Action = ();
}

#[test]
fn content_shrink_mid_animation_reclamps_and_cancels_the_active_tween() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        let content_size = Rc::new(Cell::new(vec2f(SCROLLABLE_VIEWPORT_SIZE, 500.)));
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, {
            let content_size = content_size.clone();
            move |_| ResizableScrollView {
                handle: Default::default(),
                content_size,
            }
        });

        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let view_id = app.root_view_id(window_id).unwrap();
        app.update(|ctx| render(&mut presenter.borrow_mut(), view_id, ctx));

        // Animate close to the boundary: max scroll for a 500px-tall child in a 250px viewport
        // is 250px. Six notches of 40px each target 240px.
        for _ in 0..6 {
            app.update(|ctx| {
                ctx.simulate_window_event(
                    Event::ScrollWheel {
                        position: vec2f(100., 100.),
                        delta: vec2f(0., -1.),
                        precise: false,
                        modifiers: ModifiersState::default(),
                    },
                    window_id,
                    presenter.clone(),
                );
            });
        }

        let (target_before_shrink, displayed_before_shrink) = view.read(app, |view, _| {
            (
                view.handle.scroll_target().as_f32(),
                view.handle.scroll_start().as_f32(),
            )
        });
        assert_eq!(target_before_shrink, 240.);
        assert!(
            displayed_before_shrink < target_before_shrink,
            "animation should still be in flight"
        );

        // Shrink the content so the new max scroll (300 - 250 = 50) is well below both the
        // in-flight target and the currently displayed position.
        content_size.set(vec2f(SCROLLABLE_VIEWPORT_SIZE, 300.));
        app.update(|ctx| render(&mut presenter.borrow_mut(), view_id, ctx));

        view.read(app, |view, _| {
            assert!(
                !view.handle.is_animating(),
                "shrink should cancel the tween"
            );
            assert_eq!(view.handle.scroll_target().as_f32(), 50.);
            assert_eq!(view.handle.scroll_start().as_f32(), 50.);
        });

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

/// Regression test for a rapid burst of clicky-wheel notches through the real wheel-dispatch
/// path (not just the controller in isolation): many overlapping contributions must sum exactly
/// and clamp to the scrollable's bounds, never cancel, saturate, or drop to zero net movement.
#[test]
fn long_rapid_same_direction_burst_through_wheel_dispatch_clamps_without_losing_movement() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view, presenter) = setup_vertical_clipped_scrollable(app);

        // 25 rapid same-direction notches, a few milliseconds apart -- the input pattern a
        // clicky trackball wheel produces during a fast spin. All 25 land inside the 120ms
        // window (75ms total), so every contribution is simultaneously active at once.
        for _ in 0..25 {
            app.update(|ctx| dispatch_non_precise_wheel_down(ctx, window_id, presenter.clone()));
            std::thread::sleep(Duration::from_millis(3));
        }

        // The target must be clamped to the scrollable's max extent (500 - 250 = 250px), not
        // stuck at zero or some intermediate value from a dropped or cancelled contribution.
        view.read(app, |view, _| {
            let handle = vertical_handle(view);
            assert_eq!(handle.scroll_target().as_f32(), 250.);
        });

        // Once every tween fully settles, the displayed position matches the clamped target
        // exactly -- no movement was silently swallowed by the burst. Read `scroll_start()`
        // first: it's what settles any expired contribution into the committed baseline (as it
        // would be during a real paint), so `is_animating()` reflects the post-settle state.
        // 300ms comfortably exceeds the 200ms slow end of the inverse-delta duration ramp.
        std::thread::sleep(Duration::from_millis(300));
        view.read(app, |view, _| {
            let handle = vertical_handle(view);
            assert_eq!(handle.scroll_start().as_f32(), 250.);
            assert!(!handle.is_animating());
        });

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

/// Validates that `scroll_position_top_into_view` stabilizes after one scroll:
/// scrolling to a child whose full bounds extend past the viewport should bring
/// the child's top edge into view and not oscillate on repeated calls.
#[test]
fn test_scroll_position_top_into_view_does_not_alternate() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            BasicScrollableView::new(None, Some(ScrollBehavior::Clipped(Default::default())))
        });

        let mut presenter = Presenter::new(window_id);
        let view_id = app.root_view_id(window_id).unwrap();

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        // Scroll to child (1,9). Its top is at y=450 (row 9 * 50px).
        // Viewport is 250px. The raw delta is 450, but the next layout
        // clamps scroll_start to max_scroll = 500 - 250 = 250.
        // We need a second render pass to let layout clamping settle.
        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.vertical_axis.as_ref() else {
                panic!("invalid test config");
            };
            assert_eq!(handle.scroll_start().as_f32(), 0.);
            handle.scroll_to_position(ScrollTarget {
                position_id: "child-1-9".to_owned(),
                mode: ScrollToPositionMode::TopIntoView,
            });
        });

        // First render: paint applies the scroll. Layout on the next render
        // will clamp to max_scroll.
        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        // Second render: layout clamps scroll_start from 450 to 250.
        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        let scroll_after_settled = view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.vertical_axis.as_ref() else {
                panic!("invalid test config");
            };
            let pos = handle.scroll_start().as_f32();
            assert!(pos > 0., "should have scrolled down");
            pos
        });

        // Call scroll_to_position with TopIntoView again for the same element.
        // After clamping, the top of child (1,9) is at y = 450 - 250 = 200,
        // which is within the viewport [0, 250]. No scroll should happen.
        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.vertical_axis.as_ref() else {
                panic!("invalid test config");
            };
            handle.scroll_to_position(ScrollTarget {
                position_id: "child-1-9".to_owned(),
                mode: ScrollToPositionMode::TopIntoView,
            });
        });

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.vertical_axis.as_ref() else {
                panic!("invalid test config");
            };
            assert_eq!(
                handle.scroll_start().as_f32(),
                scroll_after_settled,
                "scroll position should not change on repeated calls"
            );
        });

        // A third call should also be stable.
        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.vertical_axis.as_ref() else {
                panic!("invalid test config");
            };
            handle.scroll_to_position(ScrollTarget {
                position_id: "child-1-9".to_owned(),
                mode: ScrollToPositionMode::TopIntoView,
            });
        });

        app.update(|ctx| {
            render(&mut presenter, view_id, ctx);
        });

        view.read(app, |view, _| {
            let Some(ScrollBehavior::Clipped(handle)) = view.vertical_axis.as_ref() else {
                panic!("invalid test config");
            };
            assert_eq!(
                handle.scroll_start().as_f32(),
                scroll_after_settled,
                "scroll position should remain stable on third call"
            );
        });
    })
}

const HOVER_TRACKING_ROW_HEIGHT: f32 = 40.;
const HOVER_TRACKING_ROW_COUNT: usize = 20;
const HOVER_TRACKING_VIEWPORT_HEIGHT: f32 = 200.;
/// Screen-space Y of the stationary pointer used by the live-tracking tests below. At
/// scroll_start = 0, this lands inside row 2 (content y in [80, 120)); at scroll_start = 40 (one
/// notch later), it lands inside row 3 (content y in [120, 160)), crossing the row-2/row-3
/// boundary at scroll_start = 20 -- the exact midpoint of a single notch's distance.
const HOVER_TRACKING_MOUSE_Y: f32 = 100.;

/// A thin wrapper that records the Y origin it was actually painted at on every paint, so a test
/// can compare hover state against exactly what was last rendered -- distinguishing "hover is
/// wrong relative to what's on screen" from "hover matches the last paint, but a fresh poll of
/// `scroll_start()` has moved on since, because painting is discrete and time is continuous".
struct PaintOffsetRecorder {
    child: Box<dyn Element>,
    last_painted_origin_y: Rc<Cell<f32>>,
}

impl Element for PaintOffsetRecorder {
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
        self.last_painted_origin_y.set(origin.y());
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
}

#[derive(Default)]
struct HoverTrackingScrollView {
    handle: ClippedScrollStateHandle,
    row_states: Vec<MouseStateHandle>,
    last_painted_origin_y: Rc<Cell<f32>>,
    /// If set, each row's `Hoverable` build_child closure snapshots `state.is_hovered()` into
    /// this per-row cell *at construction time* (mirroring real call sites like
    /// `app/src/settings_view/keybindings.rs`'s `KeybindingRow::render`, which computes its
    /// background fill once from `state.is_hovered()` inside the closure, rather than reading
    /// hover state fresh on every paint). Rebuilding the row (via `ctx.notify()` on the owning
    /// view) is required for this snapshot to reflect a later hover-state change; a plain
    /// re-paint of an already-constructed row does not.
    baked_in_hover_snapshots: Option<Rc<RefCell<Vec<bool>>>>,
}

impl Entity for HoverTrackingScrollView {
    type Event = ();
}

impl View for HoverTrackingScrollView {
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let rows = self.row_states.iter().enumerate().map(|(index, state)| {
            let baked_in = self.baked_in_hover_snapshots.clone();
            Hoverable::new(state.clone(), move |state| {
                if let Some(baked_in) = baked_in {
                    baked_in.borrow_mut()[index] = state.is_hovered();
                }
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(HOVER_TRACKING_ROW_HEIGHT)
                    .with_width(200.)
                    .finish()
            })
            .finish()
        });
        let axis_config = SingleAxisConfig::Clipped {
            handle: self.handle.clone(),
            child: Box::new(PaintOffsetRecorder {
                child: Flex::column().with_children(rows).finish(),
                last_painted_origin_y: self.last_painted_origin_y.clone(),
            }),
        };
        let scrollable = NewScrollable::vertical(axis_config, Fill::None, Fill::None, Fill::None);
        ConstrainedBox::new(scrollable.finish())
            .with_height(HOVER_TRACKING_VIEWPORT_HEIGHT)
            .with_width(200.)
            .finish()
    }

    fn ui_name() -> &'static str {
        "HoverTrackingScrollView"
    }
}

impl TypedActionView for HoverTrackingScrollView {
    type Action = ();
}

/// Which row's `Hoverable` should be considered hovered given the stationary pointer at
/// [`HOVER_TRACKING_MOUSE_Y`] and the current `scroll_start`.
fn hover_tracking_expected_row(scroll_start: f32) -> usize {
    ((HOVER_TRACKING_MOUSE_Y + scroll_start) / HOVER_TRACKING_ROW_HEIGHT).floor() as usize
}

fn hover_tracking_hovered_rows(row_states: &[MouseStateHandle]) -> Vec<usize> {
    row_states
        .iter()
        .enumerate()
        .filter(|(_, state)| state.lock().unwrap().is_hovered())
        .map(|(index, _)| index)
        .collect()
}

/// Regression test for the reported "hover still lags" bug: unlike a settled-state-only check
/// (which cannot distinguish a real fix from a highlight that is wrong throughout the motion and
/// only self-corrects once the animation stops), this samples which row is hovered at several
/// points *during* an in-flight animation, for a pointer that never moves, and compares that
/// against the row the currently *displayed* (not target, not pre-animation) scroll offset
/// actually places under the pointer at that same instant.
#[test]
fn hover_tracks_the_displayed_offset_at_intermediate_animation_frames() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        let row_states: Vec<MouseStateHandle> = (0..HOVER_TRACKING_ROW_COUNT)
            .map(|_| MouseStateHandle::default())
            .collect();
        let handle = ClippedScrollStateHandle::default();
        let last_painted_origin_y = Rc::new(Cell::new(0.0f32));
        let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, {
            let handle = handle.clone();
            let row_states = row_states.clone();
            let last_painted_origin_y = last_painted_origin_y.clone();
            move |_| HoverTrackingScrollView {
                handle,
                row_states,
                last_painted_origin_y,
                baked_in_hover_snapshots: None,
            }
        });

        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let view_id = app.root_view_id(window_id).unwrap();
        app.update(|ctx| render(&mut presenter.borrow_mut(), view_id, ctx));

        let pointer = vec2f(100., HOVER_TRACKING_MOUSE_Y);
        let real_mouse_move = Event::MouseMoved {
            position: pointer,
            cmd: false,
            shift: false,
            is_synthetic: false,
        };

        // Establish initial hover with a real (non-synthetic) mouse move, and record it as the
        // "last" mouse position so the app's redraw-driven synthetic-MouseMoved replay uses it
        // on every subsequent repaint, exactly as it would for a truly stationary physical mouse.
        app.update(|ctx| {
            ctx.simulate_window_event(real_mouse_move.clone(), window_id, presenter.clone());
            ctx.set_last_mouse_move_event(window_id, real_mouse_move);
        });

        assert_eq!(
            hover_tracking_hovered_rows(&row_states),
            vec![hover_tracking_expected_row(0.)],
            "row 2 should be hovered before any scrolling"
        );

        // One non-precise notch (40px, matching NUM_PIXELS_PER_LINE with no app-level
        // multiplier in this test harness) animates scroll_start from 0 to 40, crossing the
        // row-2/row-3 boundary at scroll_start = 20 -- the animation's exact midpoint.
        app.update(|ctx| {
            ctx.simulate_window_event(
                Event::ScrollWheel {
                    position: pointer,
                    delta: vec2f(0., -1.),
                    precise: false,
                    modifiers: ModifiersState::default(),
                },
                window_id,
                presenter.clone(),
            );
        });

        // Sample at several points strictly *during* the animation (not settled). Each time,
        // compare hover state against the offset that was actually used for the *last paint*
        // (recorded by `PaintOffsetRecorder`), not a freshly-polled `scroll_start()` -- polling
        // the controller independently always reads a slightly more-advanced position than
        // whatever was last rendered, since painting is discrete (repaint-timer-driven) while
        // the controller's position is a continuous function of wall-clock time. Comparing
        // against the true poll would conflate that expected, harmless skew with a real bug.
        for _ in 0..10 {
            crate::r#async::Timer::after(Duration::from_millis(25)).await;

            let painted_offset = -last_painted_origin_y.get();
            let expected_row = hover_tracking_expected_row(painted_offset);
            let hovered_rows = hover_tracking_hovered_rows(&row_states);

            assert_eq!(
                hovered_rows,
                vec![expected_row],
                "at last-painted scroll offset={painted_offset} (scroll_start() polled \
                 independently reads {}), expected only row {expected_row} to be hovered \
                 (pointer is stationary at y={HOVER_TRACKING_MOUSE_Y}), got {hovered_rows:?}",
                handle.scroll_start().as_f32()
            );
        }

        // Once fully settled, the same invariant holds trivially (this alone would not have
        // caught the reported bug, since a wrong-throughout-then-self-correcting highlight looks
        // identical to a correct one at this point).
        assert!(!handle.is_animating());
        let settled_row = hover_tracking_expected_row(-last_painted_origin_y.get());
        assert_eq!(hover_tracking_hovered_rows(&row_states), vec![settled_row]);

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}

/// Regression test mirroring the *real* pattern used by settings-page rows (e.g.
/// `app/src/settings_view/keybindings.rs`'s `KeybindingRow::render`): the row's background is
/// computed once, inside the `Hoverable` `build_child` closure, from `state.is_hovered()` at
/// *construction* time -- not read fresh on every paint. Making that background visually
/// up to date therefore additionally depends on the row actually getting *rebuilt* (via
/// `ctx.notify()` on the owning view triggering `View::render()` again), not merely on the
/// underlying `MouseState.is_hovered()` flag being correct (which the previous test already
/// confirms it is). This test checks the *baked-in* snapshot instead of the raw flag, at the
/// same intermediate animation instants.
#[test]
fn hover_background_baked_in_at_construction_tracks_the_displayed_offset() {
    let _flag = FeatureFlag::SmoothScrolling.override_enabled(true);

    App::test((), |mut app| async move {
        let app = &mut app;
        let row_states: Vec<MouseStateHandle> = (0..HOVER_TRACKING_ROW_COUNT)
            .map(|_| MouseStateHandle::default())
            .collect();
        let handle = ClippedScrollStateHandle::default();
        let last_painted_origin_y = Rc::new(Cell::new(0.0f32));
        let baked_in_hover_snapshots = Rc::new(RefCell::new(vec![false; HOVER_TRACKING_ROW_COUNT]));
        let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, {
            let handle = handle.clone();
            let row_states = row_states.clone();
            let last_painted_origin_y = last_painted_origin_y.clone();
            let baked_in_hover_snapshots = baked_in_hover_snapshots.clone();
            move |_| HoverTrackingScrollView {
                handle,
                row_states,
                last_painted_origin_y,
                baked_in_hover_snapshots: Some(baked_in_hover_snapshots),
            }
        });

        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let view_id = app.root_view_id(window_id).unwrap();
        app.update(|ctx| render(&mut presenter.borrow_mut(), view_id, ctx));

        let pointer = vec2f(100., HOVER_TRACKING_MOUSE_Y);
        let real_mouse_move = Event::MouseMoved {
            position: pointer,
            cmd: false,
            shift: false,
            is_synthetic: false,
        };
        app.update(|ctx| {
            ctx.simulate_window_event(real_mouse_move.clone(), window_id, presenter.clone());
            ctx.set_last_mouse_move_event(window_id, real_mouse_move);
        });

        let baked_in_hovered_rows = || -> Vec<usize> {
            baked_in_hover_snapshots
                .borrow()
                .iter()
                .enumerate()
                .filter(|&(_, &hovered)| hovered)
                .map(|(index, _)| index)
                .collect()
        };

        assert_eq!(
            baked_in_hovered_rows(),
            vec![hover_tracking_expected_row(0.)],
            "row 2's baked-in background should reflect hover before any scrolling"
        );

        app.update(|ctx| {
            ctx.simulate_window_event(
                Event::ScrollWheel {
                    position: pointer,
                    delta: vec2f(0., -1.),
                    precise: false,
                    modifiers: ModifiersState::default(),
                },
                window_id,
                presenter.clone(),
            );
        });

        for _ in 0..10 {
            crate::r#async::Timer::after(Duration::from_millis(25)).await;

            let painted_offset = -last_painted_origin_y.get();
            let expected_row = hover_tracking_expected_row(painted_offset);
            let baked_in = baked_in_hovered_rows();
            let raw_flags = hover_tracking_hovered_rows(&row_states);

            assert_eq!(
                baked_in,
                vec![expected_row],
                "at last-painted scroll offset={painted_offset}, expected only row \
                 {expected_row}'s baked-in background to show hovered (raw MouseState flags \
                 currently say {raw_flags:?}), got baked-in={baked_in:?}"
            );
        }

        assert!(!handle.is_animating());
        let settled_row = hover_tracking_expected_row(-last_painted_origin_y.get());
        assert_eq!(baked_in_hovered_rows(), vec![settled_row]);

        app.update(|ctx| {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate)
        });
    })
}
