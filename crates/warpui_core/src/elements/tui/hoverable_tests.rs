use std::cell::Cell;
use std::rc::Rc;

use super::TuiHoverable;
use crate::elements::tui::test_support::{dispatch_presented_event, with_event_context};
use crate::elements::tui::{
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPoint, TuiScreenPoint, TuiScreenPosition, TuiSize, TuiText,
};
use crate::elements::MouseStateHandle;
use crate::event::ModifiersState;
use crate::presenter::tui::TuiPresenter;
use crate::{App, AppContext};

fn left_mouse_down(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    }
}

fn left_mouse_up(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseUp {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
    }
}

fn mouse_moved(x: u16, y: u16) -> TuiEvent {
    TuiEvent::MouseMoved {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        is_synthetic: false,
    }
}

#[test]
fn pointer_dispatch_before_paint_is_unhandled() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut hoverable =
                TuiHoverable::new(MouseStateHandle::default(), TuiText::new("hello").finish())
                    .on_click(|_, _| panic!("unpainted hoverable must not click"));
            with_event_context(|event_ctx| {
                assert!(!hoverable.dispatch_event(&left_mouse_down(0, 0), event_ctx, app_ctx,));
            });
        });
    });
}

#[test]
fn mouse_moves_toggle_hover_state_and_notify_without_consuming_the_event() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let handle = MouseStateHandle::default();
            let hoverable = TuiHoverable::new(handle.clone(), TuiText::new("0123456789").finish());
            let mut presenter = TuiPresenter::new();
            presenter.present_element(
                hoverable.finish(),
                crate::elements::tui::TuiRect::new(0, 0, 10, 1),
                app_ctx,
            );

            assert_eq!(
                dispatch_presented_event(&mut presenter, &mouse_moved(2, 0), app_ctx),
                (false, 1)
            );
            assert!(handle.lock().unwrap().is_hovered());

            assert_eq!(
                dispatch_presented_event(&mut presenter, &mouse_moved(4, 0), app_ctx),
                (false, 0)
            );
            assert!(handle.lock().unwrap().is_hovered());

            assert_eq!(
                dispatch_presented_event(&mut presenter, &mouse_moved(4, 3), app_ctx),
                (false, 1)
            );
            assert!(!handle.lock().unwrap().is_hovered());
        });
    });
}

#[test]
fn click_fires_on_release_after_press_inside_bounds() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let state = MouseStateHandle::default();
            let hoverable = TuiHoverable::new(state.clone(), TuiText::new("xxxx").finish())
                .on_click(move |_, _| counter.set(counter.get() + 1));
            let mut presenter = TuiPresenter::new();
            presenter.present_element(
                hoverable.finish(),
                crate::elements::tui::TuiRect::new(0, 0, 4, 1),
                app_ctx,
            );

            assert!(dispatch_presented_event(&mut presenter, &left_mouse_down(1, 0), app_ctx).0);
            assert!(state.lock().unwrap().is_clicked());
            assert_eq!(hits.get(), 0);

            assert!(dispatch_presented_event(&mut presenter, &left_mouse_up(1, 0), app_ctx).0);
            assert!(!state.lock().unwrap().is_clicked());
            assert_eq!(hits.get(), 1);

            assert!(!dispatch_presented_event(&mut presenter, &left_mouse_down(10, 10), app_ctx).0);
            assert!(!dispatch_presented_event(&mut presenter, &left_mouse_up(1, 0), app_ctx).0);
            assert_eq!(hits.get(), 1);
        });
    });
}

#[test]
fn release_outside_bounds_cancels_the_click() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let state = MouseStateHandle::default();
            let hoverable = TuiHoverable::new(state.clone(), TuiText::new("xxxx").finish())
                .on_click(move |_, _| counter.set(counter.get() + 1));
            let mut presenter = TuiPresenter::new();
            presenter.present_element(
                hoverable.finish(),
                crate::elements::tui::TuiRect::new(0, 0, 4, 1),
                app_ctx,
            );

            assert!(dispatch_presented_event(&mut presenter, &left_mouse_down(1, 0), app_ctx).0);
            assert!(!dispatch_presented_event(&mut presenter, &left_mouse_up(10, 10), app_ctx).0);
            assert!(!state.lock().unwrap().is_clicked());
            assert_eq!(hits.get(), 0);
        });
    });
}

#[test]
fn hit_testing_is_bounded_to_the_child_laid_out_size() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let handle = MouseStateHandle::default();
            let hoverable = TuiHoverable::new(handle.clone(), TuiText::new("hello").finish())
                .on_click(move |_, _| counter.set(counter.get() + 1));
            let mut presenter = TuiPresenter::new();
            presenter.present_element(
                hoverable.finish(),
                crate::elements::tui::TuiRect::new(0, 0, 10, 1),
                app_ctx,
            );

            dispatch_presented_event(&mut presenter, &mouse_moved(2, 0), app_ctx);
            assert!(handle.lock().unwrap().is_hovered());
            assert!(dispatch_presented_event(&mut presenter, &left_mouse_down(2, 0), app_ctx).0);
            assert!(dispatch_presented_event(&mut presenter, &left_mouse_up(2, 0), app_ctx).0);
            assert_eq!(hits.get(), 1);

            dispatch_presented_event(&mut presenter, &mouse_moved(7, 0), app_ctx);
            assert!(!handle.lock().unwrap().is_hovered());
            assert!(!dispatch_presented_event(&mut presenter, &left_mouse_down(7, 0), app_ctx).0);
            assert_eq!(hits.get(), 1);
        });
    });
}

struct AlwaysHandles {
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiElement for AlwaysHandles {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let size = constraint.clamp(TuiSize::new(1, 1));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        _surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        _event: &TuiEvent,
        _event_ctx: &mut TuiEventContext<'_>,
        _app: &AppContext,
    ) -> bool {
        true
    }
}

#[test]
fn child_consumes_the_event_before_the_click_handler() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let outer_hits = Rc::new(Cell::new(0u32));
            let outer_counter = outer_hits.clone();
            let child = AlwaysHandles {
                size: None,
                origin: None,
            };
            let hoverable = TuiHoverable::new(MouseStateHandle::default(), child.finish())
                .on_click(move |_, _| outer_counter.set(outer_counter.get() + 1));
            let mut presenter = TuiPresenter::new();
            presenter.present_element(
                hoverable.finish(),
                crate::elements::tui::TuiRect::new(0, 0, 1, 1),
                app_ctx,
            );

            assert!(dispatch_presented_event(&mut presenter, &left_mouse_down(0, 0), app_ctx).0);
            assert_eq!(outer_hits.get(), 0);
        });
    });
}
