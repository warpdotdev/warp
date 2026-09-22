use std::cell::RefCell;
use std::rc::Rc;

use pathfinder_geometry::rect::RectF;

use super::*;
use crate::elements::new_scrollable::SingleAxisConfig;
use crate::elements::{
    ClippedScrollStateHandle, EventHandler, Fill, NewScrollable, SelectableElement,
};
use crate::platform::WindowStyle;
use crate::{App, Entity, EntityIdSet, Presenter, TypedActionView, WindowInvalidation};

/// Selectable element that paints a hit-recorded rect (as `Container` does), so content inside a
/// layer-starting parent registers as covered at lower z-indexes.
#[derive(Default)]
struct LayeredSelectableProbe {
    origin: Option<Point>,
}

const PROBE_SIZE: f32 = 100.;

impl Element for LayeredSelectableProbe {
    fn layout(
        &mut self,
        _constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        vec2f(PROBE_SIZE, PROBE_SIZE)
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(origin, vec2f(PROBE_SIZE, PROBE_SIZE)))
            .with_background(Fill::None);
    }

    fn size(&self) -> Option<Vector2F> {
        Some(vec2f(PROBE_SIZE, PROBE_SIZE))
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

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self)
    }
}

impl SelectableElement for LayeredSelectableProbe {
    fn get_selection(
        &self,
        _selection_start: Vector2F,
        _selection_end: Vector2F,
        _is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        Some(vec![SelectionFragment {
            text: "probe".to_string(),
            origin: self.origin?,
        }])
    }

    fn expand_selection(
        &self,
        _absolute_point: Vector2F,
        _direction: SelectionDirection,
        _unit: SelectionType,
        _word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        None
    }

    fn is_point_semantically_before(
        &self,
        absolute_point: Vector2F,
        absolute_point_other: Vector2F,
    ) -> Option<bool> {
        Some(
            (absolute_point.y(), absolute_point.x())
                < (absolute_point_other.y(), absolute_point_other.x()),
        )
    }

    fn smart_select(
        &self,
        _absolute_point: Vector2F,
        _smart_select_fn: crate::elements::SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        None
    }

    fn calculate_clickable_bounds(&self, _current_selection: Option<Selection>) -> Vec<RectF> {
        Vec::new()
    }
}

#[derive(Default)]
struct ScrollableSelectionView {
    selection_handle: SelectionHandle,
    captured_selection: Rc<RefCell<Option<Option<String>>>>,
}

impl Entity for ScrollableSelectionView {
    type Event = ();
}

impl crate::core::View for ScrollableSelectionView {
    fn ui_name() -> &'static str {
        "selectable_area_test_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let captured_selection = self.captured_selection.clone();
        // Mirrors the element sandwich around collapsible reasoning block bodies: an
        // EventHandler wrapping a clipped scrollable.
        SelectableArea::new(
            self.selection_handle.clone(),
            move |args, _, _| {
                *captured_selection.borrow_mut() = Some(args.selection);
            },
            EventHandler::new(
                NewScrollable::vertical(
                    SingleAxisConfig::Clipped {
                        handle: ClippedScrollStateHandle::default(),
                        child: Box::new(LayeredSelectableProbe::default()),
                    },
                    Fill::None,
                    Fill::None,
                    Fill::None,
                )
                .finish(),
            )
            .finish(),
        )
        .finish()
    }
}

impl TypedActionView for ScrollableSelectionView {
    type Action = ();
}

/// A drag starting on content painted in a child's own layers must start a selection and yield
/// the child's fragments on mouse up. Covers the mouse-down hit test at the child subtree's max
/// z-index and `EventHandler`'s forwarding of selection queries.
#[test]
fn selection_starts_on_content_painted_in_child_layers() {
    App::test((), |mut app| async move {
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            ScrollableSelectionView::default()
        });
        let (selection_handle, captured_selection) = view.read(&app, |view, _| {
            (
                view.selection_handle.clone(),
                view.captured_selection.clone(),
            )
        });

        let root_view_id = view.id();
        app.update(move |ctx| {
            let mut presenter = Presenter::new(window_id);
            let mut updated = EntityIdSet::default();
            updated.insert(root_view_id);
            let invalidation = WindowInvalidation {
                updated,
                ..Default::default()
            };
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(200., 200.), 1., None, ctx);
            let presenter = Rc::new(RefCell::new(presenter));

            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(10., 10.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseDragged {
                    position: vec2f(60., 10.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(60., 10.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter,
            );
        });

        assert_eq!(
            *captured_selection.borrow(),
            Some(Some("probe".to_string())),
            "the drag should have produced a selection from the layered child",
        );
        assert!(!selection_handle.is_selecting());
    });
}
