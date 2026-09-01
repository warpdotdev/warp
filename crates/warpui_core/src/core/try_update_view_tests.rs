use std::cell::RefCell;
use std::rc::Rc;

use super::*;

#[derive(Default)]
struct TestView {
    value: usize,
}

impl Entity for TestView {
    type Event = ();
}

impl View for TestView {
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn ui_name() -> &'static str {
        "TestView"
    }
}

impl TypedActionView for TestView {
    type Action = ();
}

#[test]
fn try_update_applies_the_closure_while_the_window_is_open() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());
        let view = app.add_view(window_id, |_| TestView::default());

        let result = view.try_update(&mut app, |view, _| {
            view.value = 42;
            "updated"
        });

        assert_eq!(result, Ok("updated"));
        view.read(&app, |view, _| assert_eq!(view.value, 42));
    });
}

#[test]
fn try_update_reports_window_closed_after_the_window_closes() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());
        let view = app.add_view(window_id, |_| TestView::default());

        app.update(|ctx| ctx.simulate_window_closed(window_id));

        let closure_ran = Rc::new(RefCell::new(false));
        let closure_ran_inner = closure_ran.clone();
        let result = view.try_update(&mut app, move |_, _| {
            *closure_ran_inner.borrow_mut() = true;
        });

        assert_eq!(result, Err(ViewUpdateError::WindowClosed));
        assert!(
            !*closure_ran.borrow(),
            "the update closure must not run once the window is gone"
        );
    });
}

/// A `try_` method is expected not to panic, so a reentrant update is reported
/// rather than raised. It must stay distinct from
/// [`ViewUpdateError::WindowClosed`] because only reentrancy is a bug worth an
/// engineer's attention.
#[test]
fn try_update_reports_a_circular_update_rather_than_panicking() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());
        let view = app.add_view(window_id, |_| TestView::default());
        let reentrant = view.clone();

        let result = app.update(|ctx| {
            view.update(ctx, |_, ctx| {
                reentrant.try_update(ctx, |view, _| view.value += 1)
            })
        });

        assert_eq!(result, Err(ViewUpdateError::CircularUpdate));
        view.read(&app, |view, _| assert_eq!(view.value, 0));
    });
}

/// The panicking [`ViewHandle::update`] is deliberately left alone, so callers
/// that never expect a reentrant update still fail loudly.
#[test]
#[should_panic(expected = "Circular view update")]
fn update_still_panics_on_a_reentrant_update() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());
        let view = app.add_view(window_id, |_| TestView::default());
        let reentrant = view.clone();

        app.update(|ctx| {
            view.update(ctx, |_, ctx| {
                reentrant.update(ctx, |view, _| view.value += 1);
            });
        });
    });
}

/// Guards the reason `try_update` does not reuse `WeakViewHandle::upgrade`:
/// `upgrade` consults the window's view map, and a view is absent from that map
/// while an update against it is in flight. Building the check on it would
/// collapse the reentrant case above into
/// [`ViewUpdateError::WindowClosed`].
#[test]
fn weak_upgrade_reports_a_reentrantly_borrowed_view_as_gone() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());
        let view = app.add_view(window_id, |_| TestView::default());
        let weak = view.downgrade();

        app.update(|ctx| {
            view.update(ctx, |_, ctx| {
                assert!(weak.upgrade(ctx).is_none());
            });
        });
    });
}
