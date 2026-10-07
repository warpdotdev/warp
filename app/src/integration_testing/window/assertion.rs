use warpui::integration::{AssertionCallback, AssertionOutcome, StepData};
use warpui::windowing::WindowManager;
use warpui::{SingletonEntity, async_assert_eq};

/// Saves the active window id with the given step data key.
pub fn save_active_window_id<K>(window_key: K) -> AssertionCallback
where
    K: Into<String>,
{
    let window_key = window_key.into();
    Box::new(move |app, _| {
        let window_id = app.read(|ctx| WindowManager::as_ref(ctx).active_window());
        AssertionOutcome::SuccessWithData(StepData::new(
            window_key.clone(),
            window_id.expect("window id present"),
        ))
    })
}

/// Asserts the number of windows that are open.
pub fn assert_num_windows_open(num_windows: usize) -> AssertionCallback {
    Box::new(move |app, _| async_assert_eq!(app.window_ids().len(), num_windows))
}
