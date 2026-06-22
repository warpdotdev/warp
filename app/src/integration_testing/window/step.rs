use pathfinder_geometry::rect::RectF;
use warpui::integration::TestStep;
use warpui::platform::TerminationMode;
use warpui::windowing::WindowManager;
use warpui::{async_assert_eq, SingletonEntity};

use crate::integration_testing::step::new_step_with_default_assertions;

/// Adds a window and verifies that the new number of windows is as expected
pub fn add_window(expected_num_windows: usize) -> TestStep {
    new_step_with_default_assertions("Add a window")
        .with_action(|app, _, _| {
            app.dispatch_global_action("root_view:open_new", ());
        })
        .add_assertion(move |app, _| async_assert_eq!(app.window_ids().len(), expected_num_windows))
}

/// Adds a window and saves its ID into the step data.
pub fn add_and_save_window(window_key: impl Into<String>) -> TestStep {
    let window_key = window_key.into();
    TestStep::new("Add a window").with_action(move |app, _, data| {
        let prev_window = app.read(|ctx| {
            WindowManager::as_ref(ctx)
                .active_window()
                .expect("Should be an active window")
        });
        app.dispatch_global_action("root_view:open_new", ());
        let active_window = app.read(|ctx| {
            WindowManager::as_ref(ctx)
                .active_window()
                .expect("Should be an active window")
        });
        assert_ne!(
            prev_window, active_window,
            "Should have activated new window"
        );

        data.insert(window_key.clone(), active_window);
    })
}

/// Adds a window and checks that the window bounds are equal to the bounds of the window with the
/// given step data key
pub fn add_window_and_check_bounds<K>(expected_num_windows: usize, bounds_key: K) -> TestStep
where
    K: Into<String>,
{
    let bounds_key = bounds_key.into();
    new_step_with_default_assertions("Add a window")
        .with_action(move |app, _, data_map| {
            app.dispatch_global_action("root_view:open_new", ());
            let target_window_bounds: RectF =
                *data_map.get(&bounds_key).expect("bounds should be defined");
            let active_window = app.read(|ctx| {
                WindowManager::as_ref(ctx)
                    .active_window()
                    .expect("Should be an active window")
            });
            let active_window_bounds = app
                .window_bounds(&active_window)
                .expect("active window bounds defined");

            // Note that we do the assert immediately after adding the window
            // because the OS may move or resize the window, changing the bounds if we do
            // it async.
            assert_eq!(
                target_window_bounds, active_window_bounds,
                "Expected first window bounds {target_window_bounds:?} to be equal to third window bounds {active_window_bounds:?}"
            );
        })
        .add_assertion(move |app, _| async_assert_eq!(app.window_ids().len(), expected_num_windows))
}

/// Closes the window with the given step data key (corresponding to a WindowId).
pub fn close_window<K>(window_key: K, expected_num_windows: usize) -> TestStep
where
    K: Into<String>,
{
    let window_key = window_key.into();
    new_step_with_default_assertions("Close a window")
        .with_action(move |app, _, data_map| {
            let window_id = data_map
                .get(&window_key)
                .expect("Expected window id to be in data map");
            app.update(|ctx| {
                WindowManager::as_ref(ctx)
                    .close_window(*window_id, TerminationMode::ForceTerminate);
            });
        })
        .add_assertion(move |app, _| async_assert_eq!(app.window_ids().len(), expected_num_windows))
}
