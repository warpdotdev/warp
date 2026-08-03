use super::action_log::{self, ACTION_LOG_KEY, ActionLog};
use super::artifacts::{self, ARTIFACTS_KEY, TestArtifacts};
use super::overlay::{OVERLAY_LOG_KEY, OverlayLog};
use super::step::{AssertionOutcome, StepDataMap, TestStep, run_step};
use super::video_recorder::{self, VIDEO_RECORDER_KEY, VideoRecorder};
use super::{RootDir, TestSetupUtils};

const RUNTIME_TAG_FAILED_STEP_GROUP_NAME: &str = "failed_step_group_name";
const RUNTIME_TAG_FAILED_ASSERTION_NAME: &str = "failed_assertion_name";
pub const RUNTIME_TAG_FAILURE_REASON: &str = "failure_reason";

use std::backtrace::BacktraceStatus;
use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
#[cfg(not(target_family = "wasm"))]
use std::sync::atomic::Ordering;

use anyhow::Context as _;
use futures::{Future, FutureExt};
use instant::{Duration, Instant};
use warp_errors::report_error;

use crate::r#async::FutureExt as _;
#[cfg(feature = "integration_tests")]
use crate::r#async::Timer;
use crate::integration::step::PersistedDataMap;
use crate::platform::TerminationMode;
use crate::{App, WindowId};

pub type SetupFn = Box<dyn FnMut(&mut TestSetupUtils) + 'static>;
pub type OnFinishFn = Box<
    dyn FnMut(&mut App, WindowId, &mut PersistedDataMap) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + 'static,
>;

pub struct Builder {
    use_real_display: bool,
    steps: VecDeque<TestStep>,
    should_run_test: Box<dyn FnMut() -> bool>,
    setup: Option<SetupFn>,
    cleanup: Box<dyn FnMut(&mut TestSetupUtils) + 'static>,
    /// The callback to run before the app quits (on success, failure, or cancel).
    /// Note that this cannot run if the app panics, so make sure your assertions don't panic if you rely on this.
    /// Also, this function relies on the presence of an active window after the test steps have finished.
    on_finish: Option<OnFinishFn>,
    timeout: Option<Duration>,
    root_dir: RootDirKind,
    step_group_name_to_apply_to_new_steps: Option<String>,
    static_persisted_data: PersistedDataMap,
}

impl Builder {
    pub fn new(work_dir: PathBuf) -> Self {
        let mut persisted_data = PersistedDataMap::default();
        persisted_data.insert("platform".to_string(), std::env::consts::OS.to_string());
        persisted_data.insert(
            "architecture".to_string(),
            std::env::consts::ARCH.to_string(),
        );
        Self {
            use_real_display: false,
            steps: Default::default(),
            should_run_test: Box::new(|| true),
            setup: None,
            cleanup: Box::new(|_| {}),
            on_finish: None,
            timeout: None,
            root_dir: RootDirKind::Named { work_dir },
            step_group_name_to_apply_to_new_steps: None,
            static_persisted_data: persisted_data,
        }
    }

    pub fn set_should_run_test<P>(mut self, predicate: P) -> Self
    where
        P: FnMut() -> bool + 'static,
    {
        self.should_run_test = Box::new(predicate);
        self
    }

    pub fn with_real_display(mut self) -> Self {
        self.use_real_display = true;
        self
    }

    /// Applies to every TestStep added after this call in the builder, unless
    /// TestStep already has Some step_group_name.
    pub fn with_step_group_name(mut self, step_group_name: &str) -> Self {
        self.step_group_name_to_apply_to_new_steps = Some(step_group_name.to_string());
        self
    }

    pub fn with_step(mut self, mut step: TestStep) -> Self {
        if step.step_group_name.is_none() {
            step.step_group_name = self.step_group_name_to_apply_to_new_steps.clone();
        }
        self.steps.push_back(step);
        self
    }

    pub fn with_steps(mut self, mut steps: Vec<TestStep>) -> Self {
        for step in &mut steps {
            if step.step_group_name.is_none() {
                step.step_group_name = self.step_group_name_to_apply_to_new_steps.clone();
            }
        }
        self.steps.extend(steps);
        self
    }

    pub fn with_setup<C>(mut self, callback: C) -> Self
    where
        C: FnMut(&mut TestSetupUtils) + 'static,
    {
        assert!(
            self.setup.is_none(),
            "Can only register a single callback using with_setup!"
        );
        self.setup = Some(Box::new(callback));
        self
    }

    pub fn with_static_persisted_data(mut self, data: PersistedDataMap) -> Self {
        self.static_persisted_data.extend(data);
        self
    }

    pub fn with_cleanup<C>(mut self, callback: C) -> Self
    where
        C: FnMut(&mut TestSetupUtils) + 'static,
    {
        self.cleanup = Box::new(callback);
        self
    }

    pub fn with_on_finish<C>(mut self, callback: C) -> Self
    where
        C: FnMut(
                &mut App,
                WindowId,
                &mut PersistedDataMap,
            ) -> Pin<Box<dyn Future<Output = ()> + Send>>
            + 'static,
    {
        self.on_finish = Some(Box::new(callback));
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Configures the test to run with its root directory under the /tmp
    /// directory instead of under CARGO_TARGET_TMPDIR.
    pub fn use_tmp_filesystem_for_test_root_directory(mut self) -> Self {
        self.root_dir = RootDirKind::TemporaryDirectory;
        self
    }

    pub fn build(self, test_name: &str, create_temp_dir_for_test: bool) -> TestDriver {
        let test_setup = TestSetupUtils::new(self.root_dir.into_root(test_name));

        let mut driver = TestDriver {
            steps: self.steps,
            test_name: test_name.to_string(),
            test_setup,
            should_run_test: self.should_run_test,
            setup: self.setup.unwrap_or_else(|| Box::new(|_| {})),
            cleanup: self.cleanup,
            on_finish: self.on_finish,
            timeout: self.timeout,
            persisted_data: self.static_persisted_data,
        };

        driver.setup(create_temp_dir_for_test);

        driver
    }
}

/// Configuration for the test's root directory. The home directory and user data directory
/// are set based on this.
pub enum RootDirKind {
    /// Create a directory named after the test, under the temporary working directory.
    Named { work_dir: PathBuf },
    /// Create a new, anonymous temporary directory for this test.
    TemporaryDirectory,
}

impl RootDirKind {
    fn into_root(self, test_name: &str) -> RootDir {
        match self {
            RootDirKind::Named { mut work_dir } => {
                work_dir.push(test_name);
                RootDir::Path(work_dir)
            }
            RootDirKind::TemporaryDirectory => RootDir::TempDir(
                tempfile::tempdir().expect("should not fail to create temporary directory"),
            ),
        }
    }
}

/// The TestDriver records a series of test steps and executes them against the app
/// when run_test is called.
pub struct TestDriver {
    steps: VecDeque<TestStep>,
    test_name: String,
    test_setup: TestSetupUtils,
    should_run_test: Box<dyn FnMut() -> bool>,
    setup: Box<dyn FnMut(&mut TestSetupUtils) + 'static>,
    cleanup: Box<dyn FnMut(&mut TestSetupUtils) + 'static>,
    /// The callback to run before the app quits (on success, failure, or cancel).
    /// Note that this cannot run if the app panics, so make sure your assertions don't panic if you rely on this.
    /// Also, this function relies on the presence of an active window after the test steps have finished.
    on_finish: Option<OnFinishFn>,
    timeout: Option<Duration>,
    persisted_data: PersistedDataMap,
}

pub const RERUN_EXIT_CODE: i32 = 127;

/// The result of running a single integration test step. This does not include results that panic
/// or exit the driver process (failures and cancellations).
enum StepResult {
    Success,
    PreconditionFailed,
}

impl TestDriver {
    /// Executes the test steps, performing assertions against application state,
    /// and then cleans up test-only state as necessary.
    ///
    /// In integration tests, this task is automatically spawned on the foreground
    /// executor after initializing the application.
    pub async fn run_test_and_cleanup(mut self, mut app: App) {
        if !(self.should_run_test)() {
            log::info!("Skipping test ...");
            app.as_mut()
                .terminate_app(TerminationMode::ForceTerminate, None);
            return;
        }

        // Safety: We can use `AssertUnwindSafe` here because we aren't accessing any captured data
        // and are immediately terminating the app after a panic
        let test_result = AssertUnwindSafe(self.run_steps_and_determine_rerun(&mut app))
            .catch_unwind()
            .await;

        if test_result
            .as_ref()
            .is_ok_and(|attempt_rerun| !attempt_rerun)
        {
            let window_id = app.read(|ctx| {
                let windowing_state = ctx.windows();
                windowing_state.active_window()
            });
            if let Some(window_id) = window_id {
                self.run_on_finish_and_export_tags(&mut app, window_id)
                    .await;
            }
        }

        // Make sure we perform any necessary cleanup steps in case we end up
        // calling `std::process::exit()`, which doesn't `Drop` things.
        self.cleanup();

        match test_result {
            Ok(should_rerun) => {
                if should_rerun {
                    std::process::exit(RERUN_EXIT_CODE);
                } else {
                    app.as_mut()
                        .terminate_app(TerminationMode::ForceTerminate, None);
                }
            }
            Err(panic_data) => {
                match get_panic_message(&panic_data) {
                    Some(message) => eprintln!("\n{message}\n"),
                    None => eprintln!("\nTest failed (No additional information available)\n"),
                }
                // Exit with a non-zero status so that the test function knows that we failed
                std::process::exit(1);
            }
        }
    }

    async fn run_steps_and_determine_rerun(&mut self, app: &mut App) -> bool {
        let steps: Vec<TestStep> = self.steps.drain(..).collect();
        log::info!("Spawning integration test with {} steps", steps.len());

        // Set up Ctrl+C handler to ensure on_finish runs
        let sigint_received = Arc::new(AtomicBool::new(false));
        #[cfg(not(target_family = "wasm"))]
        let sigint_received_clone = sigint_received.clone();

        #[cfg(not(target_family = "wasm"))]
        ctrlc::set_handler(move || {
            log::info!("Received Ctrl+C in test driver");
            sigint_received_clone.store(true, Ordering::Relaxed);
        })
        .expect("Error setting Ctrl-C handler");

        // If the test was configured with a timeout, spawn a thread to kill
        // the test when the timeout is reached.
        //
        // We do this with a dedicated, detached thread to ensure that no deadlocks or
        // other issues that can tie up a thread prevent this logic from running.
        if let Some(timeout) = self.timeout {
            let _ = std::thread::Builder::new()
                .name("test-timeout-watchdog".to_string())
                .spawn(move || {
                    std::thread::sleep(timeout);
                    log::warn!(
                        "Test reached timeout after {}s; terminating...",
                        timeout.as_secs()
                    );
                    std::process::exit(2);
                });
        }

        let mut step_data_map = StepDataMap::default();
        self.configure_capture_recording(app, &mut step_data_map);

        for mut step in steps {
            match self
                .run_single_step_with_retries(&mut step, app, &mut step_data_map, &sigint_received)
                .await
            {
                StepResult::Success => {
                    self.handle_post_step_capture(app, &mut step_data_map).await;
                }
                StepResult::PreconditionFailed => {
                    self.finalize_recording(&mut step_data_map).await;
                    return true;
                }
            }
        }

        self.finalize_recording(&mut step_data_map).await;
        false
    }
    fn configure_capture_recording(&self, app: &mut App, step_data_map: &mut StepDataMap) {
        #[cfg(not(feature = "integration_tests"))]
        let _ = app;
        let test_artifacts = TestArtifacts::new(&self.test_name);
        log::info!(
            "Test artifacts directory: {}",
            test_artifacts.dir().display()
        );
        step_data_map.insert(ARTIFACTS_KEY, test_artifacts);
        step_data_map.insert(VIDEO_RECORDER_KEY, VideoRecorder::new());
        step_data_map.insert(ACTION_LOG_KEY, ActionLog::new());
        step_data_map.insert(OVERLAY_LOG_KEY, OverlayLog::new());

        #[cfg(feature = "integration_tests")]
        {
            let capture_state = video_recorder::get_recorder(step_data_map)
                .map(|recorder| recorder.capture_loop_state());
            if let Some(state) = capture_state {
                let app_clone = app.clone();
                app.foreground_executor()
                    .spawn(video_recorder::run_capture_loop(app_clone, state))
                    .detach();
            }

            if let Some(scale) = app.read(|ctx| {
                let windows = ctx.windows();
                windows
                    .active_window()
                    .and_then(|id| windows.platform_window(id))
                    .map(|window| window.as_ctx().backing_scale_factor())
            }) && let Some(overlay_log) = super::overlay::get_overlay_log_mut(step_data_map)
            {
                overlay_log.set_scale_factor(scale);
            }
        }

        if video_recording_enabled_for_test(&self.test_name) {
            if let Some(recorder) = video_recorder::get_recorder_mut(step_data_map) {
                recorder.start_recording();
                log::info!(
                    "VideoRecorder: auto-started recording for '{}' via {}",
                    self.test_name,
                    video_recorder::VIDEO_ENABLED_ENV_VAR
                );
            }
            if let Some(log) = action_log::get_action_log_mut(step_data_map) {
                log.set_recording_start(Instant::now());
                log.record("Recording started (auto via env var)");
            }
        }
    }

    async fn handle_post_step_capture(&self, app: &mut App, step_data_map: &mut StepDataMap) {
        let screenshot_filename: Option<String> = step_data_map
            .get::<_, String>(video_recorder::SCREENSHOT_PATH_KEY)
            .cloned();
        let needs_capture = screenshot_filename
            .as_ref()
            .is_some_and(|filename| !filename.is_empty());

        if !needs_capture {
            return;
        }

        let window = match app.read(|ctx| {
            let windowing_state = ctx.windows();
            let wid = windowing_state.active_window();
            wid.and_then(|id| windowing_state.platform_window(id))
        }) {
            Some(window) => window,
            None => return,
        };

        let (tx, rx) = futures::channel::oneshot::channel();

        window
            .as_ctx()
            .request_frame_capture(Box::new(move |frame| {
                let _ = tx.send(frame);
            }));
        window.as_ctx().request_redraw();
        let frame = match rx.with_timeout(Duration::from_secs(5)).await {
            Ok(Ok(frame)) => frame,
            _ => {
                log::warn!("VideoRecorder: frame capture timed out after step");
                return;
            }
        };

        if let Some(filename) = screenshot_filename.filter(|filename| !filename.is_empty()) {
            let path = artifacts::get_artifacts(step_data_map)
                .map(|artifacts| artifacts.path(&filename))
                .unwrap_or_else(|| PathBuf::from(&filename));
            if let Err(e) = video_recorder::save_captured_frame_as_png(&frame, &path)
                .context("VideoRecorder: failed to save screenshot")
            {
                report_error!(e, extra: { "path" => %path.display() });
            } else {
                log::info!("VideoRecorder: screenshot saved to {}", path.display());
            }
            step_data_map.insert(video_recorder::SCREENSHOT_PATH_KEY, String::new());
        }
    }

    async fn finalize_recording(&self, step_data_map: &mut StepDataMap) {
        #[cfg(feature = "integration_tests")]
        {
            if let Some(recorder) = video_recorder::get_recorder(step_data_map) {
                recorder.stop_capture_loop();
            }
            Timer::at(Instant::now() + std::time::Duration::from_millis(50)).await;
        }

        let artifacts_dir =
            artifacts::get_artifacts(step_data_map).map(|artifacts| artifacts.dir().to_path_buf());

        let overlay_log: Option<OverlayLog> =
            step_data_map.remove::<_, OverlayLog>(OVERLAY_LOG_KEY);
        if let Some(recorder) = video_recorder::get_recorder_mut(step_data_map) {
            recorder.stop_recording();
            if recorder.frame_count() > 0
                && let Some(ref dir) = artifacts_dir
            {
                let output = dir.join("recording.mp4");
                if let Err(e) = recorder.finalize(&output, overlay_log.as_ref()) {
                    report_error!(e.context("VideoRecorder: finalization failed"));
                }
            }
        }

        if let Some(ref dir) = artifacts_dir {
            let log_output = dir.join("recording.log");
            if let Some(action_log) = action_log::get_action_log(step_data_map)
                && let Err(e) = action_log
                    .write_to_file(&log_output)
                    .context("ActionLog: finalization failed")
            {
                report_error!(e);
            }
        }
    }
    pub(crate) fn setup(&mut self, create_temp_dir_for_test: bool) {
        if create_temp_dir_for_test {
            self.test_setup.create_temp_dir_for_test();
            self.test_setup.set_home_dir_for_test();
        }
        (self.setup)(&mut self.test_setup);
    }

    pub(crate) fn cleanup(&mut self) {
        self.test_setup.cleanup_env();
        self.test_setup.cleanup_dir();
        (self.cleanup)(&mut self.test_setup);
    }

    fn export_runtime_tags(&self) {
        if let Ok(output_file) = std::env::var("RUNTIME_TAGS_OUTPUT_FILE") {
            match serde_json::to_string_pretty(&self.persisted_data)
                .context("Failed to serialize runtime tags to JSON")
            {
                Ok(json_content) => match std::fs::write(&output_file, json_content)
                    .context("Failed to write runtime tags to file")
                {
                    Ok(_) => log::info!("Runtime tags exported to: {output_file}"),
                    Err(e) => report_error!(e, extra: { "file" => %output_file }),
                },
                Err(e) => report_error!(e),
            }
        } else {
            log::debug!(
                "RUNTIME_TAGS_OUTPUT_FILE environment variable not set, skipping runtime tags export"
            );
        }
    }

    /// Runs the on_finish callback and exports runtime tags. This should be called
    /// everywhere on_finish is invoked to ensure runtime tags are always exported.
    async fn run_on_finish_and_export_tags(&mut self, app: &mut App, window_id: WindowId) {
        if let Some(ref mut on_finish) = self.on_finish {
            let future = (on_finish)(app, window_id, &mut self.persisted_data);
            future.await;
        }
        self.export_runtime_tags();
    }

    /// Run a single step of an integration test.
    ///
    /// If the step has retries configured, it will be attempted up to `retries + 1` times:
    /// * [`AssertionOutcome::Success`], [`AssertionOutcome::SuccessWithData`] succeed immediately
    /// * [`AssertionOutcome::PreconditionFailed`] ends the entire test
    /// * [`AssertionOutcome::Failure`] and [`AssertionOutcome::ImmediateFailure`] may be retried
    ///
    /// If the step succeeds or fails preconditions, this returns a [`StepResult`]. If it fails,
    /// this panics with a failure message.
    ///
    /// If the test is canceled, this exits the process immediately.
    async fn run_single_step_with_retries(
        &mut self,
        step: &mut TestStep,
        app: &mut App,
        step_data_map: &mut StepDataMap,
        sigint_received: &AtomicBool,
    ) -> StepResult {
        let (window_id, window) = app.read(|ctx| {
            let windowing_state = ctx.windows();
            let window_id = windowing_state
                .active_window()
                .expect("should be an active window in integration tests");
            let window = windowing_state
                .platform_window(window_id)
                .expect("should be a platform window");
            (window_id, window)
        });

        // Retry logic for the step
        let mut retry_attempt = 0;
        let max_attempts = step.retries() + 1; // +1 for the original attempt

        'retries: loop {
            retry_attempt += 1;

            if step.retries() > 0 {
                log::info!(
                    "Running test step '{}' on window id {:?} (attempt {}/{})",
                    step.name(),
                    window_id,
                    retry_attempt,
                    max_attempts
                );
            } else {
                log::info!(
                    "Running test step '{}' on window id {:?}",
                    step.name(),
                    window_id
                );
            }

            if let Some(log) = action_log::get_action_log_mut(step_data_map) {
                log.record(format!("Step started: {}", step.name()));
            }

            match run_step(
                step,
                app,
                window_id,
                window.as_ref(),
                step_data_map,
                &mut self.test_setup,
                sigint_received,
            )
            .await
            {
                AssertionOutcome::Success | AssertionOutcome::SuccessWithData(_) => {
                    if retry_attempt > 1 {
                        log::info!(
                            "Test step '{}' succeeded after {} attempts.",
                            step.name(),
                            retry_attempt
                        );
                    } else {
                        log::info!("Test step '{}' succeeded.", step.name());
                    }
                    if let Some(log) = action_log::get_action_log_mut(step_data_map) {
                        log.record(format!("Step succeeded: {}", step.name()));
                    }
                    return StepResult::Success;
                }
                AssertionOutcome::Failure {
                    message,
                    backtrace,
                    failed_assertion_name,
                } => {
                    if retry_attempt < max_attempts {
                        log::warn!(
                            "Test step '{}' failed (attempt {}/{}): {}. Retrying...",
                            step.name(),
                            retry_attempt,
                            max_attempts,
                            message
                        );
                        continue 'retries; // Retry the step
                    } else {
                        // All retries exhausted, fail the test
                        let backtrace_message = match backtrace.status() {
                            BacktraceStatus::Captured => format!("{backtrace}"),
                            _ => "(Backtrace disabled; run with `RUST_BACKTRACE=1` environment variable to display a backtrace)".into(),
                        };
                        let step_group_name =
                            step.step_group_name.as_deref().unwrap_or("Unspecified");
                        self.persisted_data.insert(
                            RUNTIME_TAG_FAILED_STEP_GROUP_NAME.to_owned(),
                            step_group_name.to_owned(),
                        );
                        let failed_assertion_name =
                            failed_assertion_name.unwrap_or("Unspecified".to_owned());
                        self.persisted_data.insert(
                            RUNTIME_TAG_FAILED_ASSERTION_NAME.to_owned(),
                            failed_assertion_name,
                        );
                        self.persisted_data
                            .insert(RUNTIME_TAG_FAILURE_REASON.to_owned(), message.clone());
                        self.run_on_finish_and_export_tags(app, window_id).await;
                        panic!(
                            "Test step '{}' failed after {} attempts: {message}\nFailed in step group: {step_group_name}\n{backtrace_message}",
                            step.name(),
                            max_attempts,
                        );
                    }
                }
                AssertionOutcome::ImmediateFailure {
                    message,
                    backtrace,
                    failed_assertion_name,
                } => {
                    if retry_attempt < max_attempts {
                        log::warn!(
                            "Test step '{}' failed (attempt {}/{}): {}. Retrying...",
                            step.name(),
                            retry_attempt,
                            max_attempts,
                            message
                        );
                        continue 'retries; // Retry the step
                    } else {
                        // All retries exhausted, fail the test
                        let backtrace_message = match backtrace.status() {
                            BacktraceStatus::Captured => format!("{backtrace}"),
                            _ => "(Backtrace disabled; run with `RUST_BACKTRACE=1` environment variable to display a backtrace)".into(),
                        };
                        let step_group_name =
                            step.step_group_name.as_deref().unwrap_or("Unspecified");
                        self.persisted_data.insert(
                            RUNTIME_TAG_FAILED_STEP_GROUP_NAME.to_owned(),
                            step_group_name.to_owned(),
                        );
                        let failed_assertion_name =
                            failed_assertion_name.unwrap_or("Unspecified".to_owned());
                        self.persisted_data.insert(
                            RUNTIME_TAG_FAILED_ASSERTION_NAME.to_owned(),
                            failed_assertion_name,
                        );
                        self.persisted_data
                            .insert(RUNTIME_TAG_FAILURE_REASON.to_owned(), message.clone());
                        self.run_on_finish_and_export_tags(app, window_id).await;
                        panic!(
                            "Test step '{}' failed after {} attempts: {message}\nFailed in step group: {step_group_name}\n{backtrace_message}",
                            step.name(),
                            max_attempts,
                        );
                    }
                }
                AssertionOutcome::Canceled => {
                    // Early exit on cancellation.
                    log::info!("Test step '{}' canceled, running on_finish...", step.name());
                    self.run_on_finish_and_export_tags(app, window_id).await;
                    log::info!("on_finish complete, exiting");
                    std::process::exit(0);
                }
                AssertionOutcome::PreconditionFailed(msg) => {
                    // End the test, but don't fail it.
                    log::warn!(
                        "Test step '{}' precondition failed because of '{}' - attempting a re-run.",
                        step.name(),
                        msg
                    );
                    return StepResult::PreconditionFailed;
                }
            }
        }
    }
}

/// Returns whether video recording should be enabled for the given test name.
///
/// Checks the `WARP_INTEGRATION_TEST_VIDEO` environment variable:
/// - Unset or empty → recording disabled.
/// - `"1"` or `"all"` → recording enabled for every test.
/// - Any other value → treated as a comma-separated list of test names;
///   recording is enabled only when `test_name` appears in the list.
///
/// Example:
/// ```sh
/// # Record all tests
/// WARP_INTEGRATION_TEST_VIDEO=1 cargo nextest run ...
///
/// # Record only specific tests
/// WARP_INTEGRATION_TEST_VIDEO=test_foo,test_bar cargo nextest run ...
/// ```
fn video_recording_enabled_for_test(test_name: &str) -> bool {
    let Ok(value) = std::env::var(video_recorder::VIDEO_ENABLED_ENV_VAR) else {
        return false;
    };
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if value == "1" || value == "all" {
        return true;
    }
    value.split(',').any(|name| name.trim() == test_name)
}

/// Given a value retrieved from catching an unwinding panic, returns
/// the panic message, if one is available.
fn get_panic_message(panic: &Box<dyn std::any::Any + Send>) -> Option<&str> {
    panic
        // If a panic or assert is invoked in a way that includes a format
        // string and arguments, the panic data will be an owned string.
        .downcast_ref::<String>()
        .map(String::as_str)
        // Otherwise, it might be a static string reference (if there are no values
        // that need to be interpolated at runtime).
        .or_else(|| {
            panic
                .downcast_ref::<&'static str>()
                .map(std::ops::Deref::deref)
        })
}
