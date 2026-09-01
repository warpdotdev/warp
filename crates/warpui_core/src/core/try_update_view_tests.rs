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

        assert_eq!(result, Some("updated"));
        view.read(&app, |view, _| assert_eq!(view.value, 42));
    });
}

#[test]
fn try_update_returns_none_after_the_window_closes() {
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

        assert_eq!(result, None);
        assert!(
            !*closure_ran.borrow(),
            "the update closure must not run once the window is gone"
        );
    });
}

/// A reentrant update is a programming error rather than a torn-down window, so
/// `try_update` must still surface it instead of quietly reporting `None`.
#[test]
#[should_panic(expected = "Circular view update")]
fn try_update_still_panics_on_a_reentrant_update() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.add_window(WindowStyle::NotStealFocus, |_| TestView::default());
        let view = app.add_view(window_id, |_| TestView::default());
        let reentrant = view.clone();

        app.update(|ctx| {
            view.update(ctx, |_, ctx| {
                reentrant.try_update(ctx, |view, _| view.value += 1);
            });
        });
    });
}

/// Guards the reason `try_update` exists rather than reusing
/// `WeakViewHandle::upgrade`: `upgrade` consults the window's view map, and a
/// view is absent from that map while an update against it is in flight. A
/// guard built on it would misreport the reentrant update above as a closed
/// window and swallow the panic.
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
