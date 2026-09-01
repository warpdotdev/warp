use pathfinder_geometry::vector::Vector2F;

use super::*;

/// Regression test for https://github.com/warpdotdev/warp/issues/15379: reopening a closed
/// window applies the caller-supplied background blur settings.
#[test]
fn test_reopen_closed_window_applies_caller_supplied_background_blur_settings() {
    #[derive(Default)]
    struct TestView;

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

    App::test((), |mut app| async move {
        // Open the window with settings that differ from what will be supplied at reopen
        // time, so the test can't pass by accident if the original values were reused.
        let (window_id, _) = app.update(|ctx| {
            ctx.add_window(
                AddWindowOptions {
                    background_blur_radius_pixels: Some(1),
                    background_blur_texture: false,
                    window_bounds: WindowBounds::ExactPosition(RectF::new(
                        Vector2F::new(0., 0.),
                        Vector2F::new(800., 600.),
                    )),
                    ..Default::default()
                },
                |_ctx| TestView,
            )
        });

        let closed_data = app
            .update(|ctx| ctx.handle_window_closed(window_id))
            .expect("closing an open window should produce ClosedWindowData");

        // Simulate the caller (the app crate) passing the user's currently configured
        // window settings, as it does via `WindowSettings`.
        app.update(|ctx| {
            ctx.reopen_closed_window(closed_data, Some(64), true);
        });

        app.read(|ctx| {
            let platform_window = ctx
                .windows()
                .platform_window(window_id)
                .expect("window should have been reopened under the same window ID");
            let test_window = platform_window
                .as_any()
                .downcast_ref::<crate::platform::test::TestWindow>()
                .expect("test platform window should be reachable via as_any");
            assert_eq!(
                test_window.background_blur_radius_pixels(),
                Some(64),
                "reopened window should use the caller-supplied background blur radius"
            );
            assert!(
                test_window.background_blur_texture(),
                "reopened window should use the caller-supplied background blur texture flag"
            );
        });
    });
}
