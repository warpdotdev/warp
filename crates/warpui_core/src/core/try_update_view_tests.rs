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
