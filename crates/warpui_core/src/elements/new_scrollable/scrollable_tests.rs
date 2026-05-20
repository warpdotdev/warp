use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{vec2f, Vector2F};

use crate::{
    elements::{
        Axis, ClippedScrollStateHandle, ConstrainedBox, DispatchEventResult, EventHandler, Fill,
        ParentElement, Point, Rect, SavePosition, ScrollData, ScrollStateHandle, ScrollTarget,
        ScrollToPositionMode, ScrollbarWidth, SelectableElement, SelectionFragment, Stack, ZIndex,
    },
    event::DispatchedEvent,
    platform::{TerminationMode, WindowStyle},
    text::{word_boundaries::WordBoundariesPolicy, IsRect, SelectionDirection, SelectionType},
    units::Pixels,
    AfterLayoutContext, App, AppContext, Element, Entity, EntityId, Event, EventContext,
    LayoutContext, PaintContext, Presenter, SizeConstraint, TypedActionView, View, ViewContext,
    WindowInvalidation,
};

use super::{
    AxisConfiguration, ClippedAxisConfiguration, DualAxisConfig, NewScrollable,
    NewScrollableElement, ScrollableAppearance, ScrollableAxis, SingleAxisConfig,
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
    let mut updated = HashSet::new();
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
