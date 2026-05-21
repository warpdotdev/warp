use super::*;
use crate::elements::*;
use crate::platform::WindowStyle;

#[derive(Default)]
struct ZoomTestView;

impl Entity for ZoomTestView {
    type Event = ();
}

impl super::View for ZoomTestView {
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn ui_name() -> &'static str {
        "ZoomTestView"
    }
}

impl TypedActionView for ZoomTestView {
    type Action = ();
}

#[test]
fn test_per_window_zoom_factor_invariants() {
    App::test((), |mut app| async move {
        let app = &mut app;

        let (window_a, _) = app.add_window(WindowStyle::NotStealFocus, |_| ZoomTestView);
        let (window_b, _) = app.add_window(WindowStyle::NotStealFocus, |_| ZoomTestView);

        // Invariant 1: no override → effective is the app-wide default (1.0).
        app.read(|ctx| {
            assert_eq!(ctx.window_zoom_factor(window_a).as_f32(), 1.0);
            assert_eq!(ctx.window_zoom_factor(window_b).as_f32(), 1.0);
        });

        // Invariant 2: per-window override only affects the target window.
        app.update(|ctx| {
            ctx.set_window_zoom_factor(window_a, 1.5);
        });
        app.read(|ctx| {
            assert_eq!(ctx.window_zoom_factor(window_a).as_f32(), 1.5);
            assert_eq!(ctx.window_zoom_factor(window_b).as_f32(), 1.0);
        });

        // Invariant 3: changing the global default leaves overridden windows
        // alone but updates non-overridden ones.
        app.update(|ctx| {
            ctx.set_zoom_factor(1.3);
        });
        app.read(|ctx| {
            assert_eq!(ctx.window_zoom_factor(window_a).as_f32(), 1.5);
            assert_eq!(ctx.window_zoom_factor(window_b).as_f32(), 1.3);
        });

        // Invariant 4: reset clears the override so the window follows the
        // global default again.
        app.update(|ctx| {
            ctx.reset_window_zoom_factor(window_a);
        });
        app.read(|ctx| {
            assert_eq!(ctx.window_zoom_factor(window_a).as_f32(), 1.3);
            assert_eq!(ctx.window_zoom_factor(window_b).as_f32(), 1.3);
        });

        // Invariant 5: a missing window_id is a silent noop on set/reset and
        // window_zoom_factor falls back to the global default.
        app.update(|ctx| {
            let bogus = WindowId::from_usize(99_999);
            ctx.set_window_zoom_factor(bogus, 1.7);
            ctx.reset_window_zoom_factor(bogus);
            assert_eq!(ctx.window_zoom_factor(bogus).as_f32(), 1.3);
        });
    });
}

#[test]
fn test_set_window_zoom_factor_clamps_to_supported_range() {
    App::test((), |mut app| async move {
        let app = &mut app;

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| ZoomTestView);

        // Below the minimum: clamped up to 0.5.
        app.update(|ctx| ctx.set_window_zoom_factor(window_id, 0.1));
        app.read(|ctx| {
            assert_eq!(ctx.window_zoom_factor(window_id).as_f32(), 0.5);
        });

        // Above the maximum: clamped down to 4.0.
        app.update(|ctx| ctx.set_window_zoom_factor(window_id, 5.0));
        app.read(|ctx| {
            assert_eq!(ctx.window_zoom_factor(window_id).as_f32(), 4.0);
        });
    });
}
