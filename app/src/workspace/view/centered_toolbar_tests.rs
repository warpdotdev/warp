use std::cell::RefCell;
use std::rc::Rc;

use pathfinder_geometry::vector::{Vector2F, vec2f};
use warpui::elements::{ConstrainedBox, Empty, Point};
use warpui::event::DispatchedEvent;
use warpui::platform::WindowStyle;
use warpui::{
    AfterLayoutContext, App, AppContext, Element, Entity, EntityIdSet, EventContext, LayoutContext,
    PaintContext, Presenter, SizeConstraint, TypedActionView, View, WindowInvalidation,
};

use super::{CenteredToolbar, centered_toolbar_geometry};

#[test]
fn center_stays_at_full_row_midpoint_with_asymmetric_sides() {
    let geometry = centered_toolbar_geometry(1_000., 56., 16., 336., 110.);

    assert_eq!(geometry.center_origin_x + geometry.center_width / 2., 500.);
}

#[test]
fn center_shrinks_symmetrically_before_overlapping_the_wider_side() {
    let geometry = centered_toolbar_geometry(500., 56., 16., 336., 110.);

    assert_eq!(geometry.center_width, 280.);
    assert_eq!(geometry.center_origin_x, 110.);
    assert_eq!(geometry.right_origin_x, 390.);
}

#[test]
fn center_continues_smoothly_across_zero_symmetric_space() {
    let just_above = centered_toolbar_geometry(220.1, 80., 16., 336., 110.);
    let at_zero = centered_toolbar_geometry(220., 80., 16., 336., 110.);

    assert_eq!(just_above.center_width, 16.);
    assert_eq!(at_zero.center_width, 16.);
    assert!((just_above.center_origin_x - at_zero.center_origin_x - 0.1).abs() < 0.001);
}

#[test]
fn center_uses_physical_gap_when_symmetric_space_is_exhausted() {
    let geometry = centered_toolbar_geometry(200., 80., 16., 336., 110.);

    assert_eq!(geometry.center_width, 10.);
    assert_eq!(geometry.center_origin_x, 80.);
    assert_eq!(geometry.right_origin_x, 90.);
}

#[test]
fn fixed_sides_remain_edge_anchored_when_they_overlap() {
    let geometry = centered_toolbar_geometry(150., 80., 16., 336., 110.);

    assert_eq!(geometry.center_width, 0.);
    assert_eq!(geometry.center_origin_x, 75.);
    assert_eq!(geometry.right_origin_x, 40.);
}

#[derive(Default)]
struct CenterProbeState {
    layout_max_width: Option<f32>,
    paint_origin_x: Option<f32>,
    visible_width: Option<f32>,
}

struct OversizedCenterProbe {
    state: Rc<RefCell<CenterProbeState>>,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl OversizedCenterProbe {
    fn new(state: Rc<RefCell<CenterProbeState>>) -> Self {
        Self {
            state,
            size: None,
            origin: None,
        }
    }
}

impl Element for OversizedCenterProbe {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _: &mut LayoutContext,
        _: &AppContext,
    ) -> Vector2F {
        self.state.borrow_mut().layout_max_width = Some(constraint.max.x());
        let size = vec2f(16., 20.);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _: &AppContext) {
        let origin = Point::from_vec2f(origin, ctx.scene.z_index());
        self.origin = Some(origin);
        let mut state = self.state.borrow_mut();
        state.paint_origin_x = Some(origin.xy().x());
        state.visible_width = ctx
            .scene
            .visible_rect(
                origin,
                self.size.expect("probe must be laid out before paint"),
            )
            .map(|bounds| bounds.width());
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }
}

struct CenteredToolbarTestView {
    center_state: Rc<RefCell<CenterProbeState>>,
}

impl Entity for CenteredToolbarTestView {
    type Event = ();
}

impl View for CenteredToolbarTestView {
    fn ui_name() -> &'static str {
        "centered_toolbar_test_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        ConstrainedBox::new(
            CenteredToolbar::new(
                ConstrainedBox::new(Empty::new().finish())
                    .with_width(80.)
                    .with_height(20.)
                    .finish(),
                OversizedCenterProbe::new(self.center_state.clone()).finish(),
                ConstrainedBox::new(Empty::new().finish())
                    .with_width(110.)
                    .with_height(20.)
                    .finish(),
                16.,
                336.,
            )
            .finish(),
        )
        .with_width(200.)
        .with_height(20.)
        .finish()
    }
}

impl TypedActionView for CenteredToolbarTestView {
    type Action = ();
}

#[test]
fn center_paint_is_clipped_to_its_allocated_width() {
    App::test((), |mut app| async move {
        let center_state = Rc::new(RefCell::new(CenterProbeState::default()));
        let center_state_for_view = center_state.clone();
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, move |_| {
            CenteredToolbarTestView {
                center_state: center_state_for_view,
            }
        });
        let mut presenter = Presenter::new(window_id);
        let mut updated = EntityIdSet::default();
        updated.insert(
            app.root_view_id(window_id)
                .expect("test window must have a root view"),
        );

        app.update(move |ctx| {
            presenter.invalidate(
                WindowInvalidation {
                    updated,
                    ..Default::default()
                },
                ctx,
            );
            presenter.build_scene(vec2f(200., 20.), 1., None, ctx);
        });

        let center_state = center_state.borrow();
        assert_eq!(center_state.layout_max_width, Some(10.));
        assert_eq!(center_state.paint_origin_x, Some(80.));
        assert_eq!(center_state.visible_width, Some(10.));
    });
}
