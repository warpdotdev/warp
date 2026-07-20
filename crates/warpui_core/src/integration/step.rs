use std::any::Any;
use std::backtrace::Backtrace;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use instant::Instant;
use warp_errors::report_error;

use super::{TestSetupUtils, action_log, overlay};
use crate::r#async::Timer;
use crate::event::{Event, KeyEventDetails};
use crate::keymap::{Keystroke, PerPlatformKeystroke};
use crate::platform::{OperatingSystem, Window};
use crate::{App, WindowId};

const MAX_WAKEUPS_PER_SECOND: u64 = 60;
const THROTTLE_PERIOD: Duration = Duration::from_micros(1000 * 1000 / MAX_WAKEUPS_PER_SECOND);

/// Used for data that is used from step to step.
#[derive(Default)]
pub struct StepDataMap {
    inner: HashMap<String, Box<dyn Any>>,
}

impl StepDataMap {
    pub fn get<K, V>(&self, key: K) -> Option<&V>
    where
        K: AsRef<str>,
        V: 'static,
    {
        let boxed = self.inner.get(key.as_ref())?;
        boxed.as_ref().downcast_ref::<V>()
    }

    pub fn get_mut<K, V>(&mut self, key: K) -> Option<&mut V>
    where
        K: AsRef<str>,
        V: 'static,
    {
        let boxed = self.inner.get_mut(key.as_ref())?;
        boxed.as_mut().downcast_mut::<V>()
    }

    pub fn insert<K, V>(&mut self, key: K, value: V)
    where
        K: Into<String>,
        V: Any + 'static,
    {
        self.insert_step_data(StepData::new(key, value));
    }

    pub fn remove<K, V>(&mut self, key: K) -> Option<V>
    where
        K: AsRef<str>,
        V: 'static,
    {
        let boxed = self.inner.remove(key.as_ref())?;
        boxed.downcast::<V>().ok().map(|b| *b)
    }

    fn insert_step_data(&mut self, step_data: StepData) {
        self.inner.insert(step_data.key, step_data.data);
    }
}

fn record_overlay_kind(kind: overlay::OverlayKind, step_data_map: &mut StepDataMap) {
    let is_recording = super::video_recorder::get_recorder(step_data_map)
        .is_some_and(super::video_recorder::VideoRecorder::is_recording);
    if !is_recording {
        return;
    }
    if let Some(ol) = overlay::get_overlay_log_mut(step_data_map) {
        ol.record(kind);
    }
}

/// Data to pass from one step to the next.
pub struct StepData {
    /// The name (key) of the data.
    pub key: String,

    /// The data itself.  Can be any type.
    pub data: Box<dyn Any>,
}

impl StepData {
    pub fn new<K, V>(key: K, data: V) -> Self
    where
        K: Into<String>,
        V: Any + 'static,
    {
        let data: Box<dyn Any> = Box::new(data);
        Self {
            key: key.into(),
            data,
        }
    }
}

pub type PersistedDataMap = HashMap<String, String>;

/// The result of an assertion.  Use this rather than a normal assertion
/// when the thing you are testing may take up to a timeout to be true.
#[must_use = "AssertionOutcome must be returned to the test runner to allow retrying"]
pub enum AssertionOutcome {
    /// The step succeeded.
    Success,

    // The test successfully completed, and we want to return some data to the
    // next step or persist it.
    SuccessWithData(StepData),

    /// The step failed.  Stores a backtrace from where the failure happened.
    Failure {
        message: String,
        backtrace: Backtrace,
        failed_assertion_name: Option<String>,
    },

    /// The step failed, and we should not wait until the timeout.
    /// Use this if you need to do things in the app on test failure (like export information) -
    /// the test will fail, but the app remains running for any export steps.
    /// If you don't need to export data from the app on failure, use a normal assertion instead.
    ImmediateFailure {
        message: String,
        backtrace: Backtrace,
        failed_assertion_name: Option<String>,
    },

    /// Return this when there is a timing condition that prevents us from
    /// running the rest of the test - we don't treat this as a failure
    /// but instead skip the rest of the steps and log the flake.
    PreconditionFailed(String),

    /// The test was canceled by user (e.g. Ctrl+C)
    Canceled,
}

impl AssertionOutcome {
    /// Creates a failure outcome with a stacktrace.
    pub fn failure(message: String) -> Self {
        AssertionOutcome::Failure {
            message,
            backtrace: Backtrace::capture(),
            failed_assertion_name: None,
        }
    }

    pub fn immediate_failure(message: String) -> Self {
        AssertionOutcome::ImmediateFailure {
            message,
            backtrace: Backtrace::capture(),
            failed_assertion_name: None,
        }
    }

    pub fn as_failure_message(&self) -> Option<&str> {
        match self {
            AssertionOutcome::Failure { message, .. }
            | AssertionOutcome::ImmediateFailure { message, .. } => Some(message.as_str()),
            AssertionOutcome::Success
            | AssertionOutcome::SuccessWithData(_)
            | AssertionOutcome::PreconditionFailed(_)
            | AssertionOutcome::Canceled => None,
        }
    }
}

/// An assertion callback checks the state of the app and last presenter
/// (current element tree and last scene) for the given window_id.
/// It should be idempotent because it can be called multiple times until
/// the timeout is reached.  
pub type AssertionCallback = Box<dyn FnMut(&mut App, WindowId) -> AssertionOutcome>;

/// An assertion callback checks the state of the app and last presenter
/// (current element tree and last scene) for the given window_id.
/// It should be idempotent because it can be called multiple times until
/// the timeout is reached.  This variant also passes in a map of data from prior steps.
pub type AssertionWithDataCallback =
    Box<dyn FnMut(&mut App, WindowId, &mut StepDataMap) -> AssertionOutcome>;

enum CallbackType {
    Assertion(AssertionCallback),
    AssertionWithData(AssertionWithDataCallback),
}

struct Assertion {
    name: Option<String>,
    callback: CallbackType,
}

pub type SavedPositionFn = Box<dyn Fn(&mut App, WindowId) -> String>;
pub type EventFn = Box<dyn Fn(&mut App, WindowId) -> Event>;

/// A TestStep can include integration events that are handled
/// before asserting some state of the app.
pub enum IntegrationTestEvent {
    /// A plain-old-event to be processed.
    /// Note that the event will be created at build time.
    WithEvent(Event),
    /// Given a callback that produces an Event, this allows you to
    /// create an event at runtime using the state of the app at the time
    /// of the step.
    WithEventFn(EventFn),
    /// Similar to WithEvent, but used to dispatch an event at the saved position.
    WithSavedPosition(String, MouseEvent),
    /// Similar to WithEventFn, but used to dispatch an event at the saved position.
    WithSavedPositionFn(SavedPositionFn, MouseEvent),
}

pub enum MouseEvent {
    ClickOnce,
    RightClickOnce,
    Hover,
}

pub type IntegrationTestActionFn = Box<dyn Fn(&mut App, WindowId, &mut StepDataMap)>;
pub type IntegrationTestSetupFn = Box<dyn Fn(&mut TestSetupUtils)>;

/// A test step consists of
/// 1) A queue of setup functions, that might e.g. modify the filesystem.
/// 2) A queue of events to dispatch against the active window
/// 3) Queue of actions called *before* any assertions are checked. Since they receive &mut App, it
///    is possible that the action modifies the app state (ie. by dispatching a global action).
/// 4) An assertion callback that verifies the state of the app and last frame
/// 5) An optional timeout for the assertion callback - the app will continue
///    to test the assertion until the timeout is reached or the assertion succeeds
pub struct TestStep {
    name: String,
    setup_functions: VecDeque<IntegrationTestSetupFn>,
    events: VecDeque<IntegrationTestEvent>,
    actions: VecDeque<IntegrationTestActionFn>,
    assertions: Vec<Assertion>,
    timeout: Duration,
    post_step_pause: Option<Duration>,

    /// This causes the test to wait after a failure rather than immediately
    /// panicking - can be useful in combination with running with a real delegate
    /// to observe the state that the app is in when failure happens.
    pause_on_failure: Option<Duration>,

    /// An optional final assertion that is run when the timeout has hit.
    /// If omitted the test fails.
    on_failure_handler: Option<Assertion>,

    /// The name of the group this step belongs to, used for failure reporting.
    pub(super) step_group_name: Option<String>,

    /// Number of times to retry this step if it fails (defaults to 0, meaning no retries)
    retries: u32,
}

const DEFAULT_STEP_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_POST_STEP_PAUSE: Duration = Duration::from_secs(3);
const DEFAULT_PAUSE_ON_FAILURE: Duration = Duration::from_secs(1000);

impl TestStep {
    pub fn new(name: &str) -> Self {
        // Enable these two pauses for better local debugging
        let pause_on_failure = if std::env::var("WARPUI_PAUSE_INTEGRATION_TEST_ON_FAILURE").is_ok()
        {
            Some(DEFAULT_PAUSE_ON_FAILURE)
        } else {
            None
        };

        let post_step_pause =
            if std::env::var("WARPUI_PAUSE_INTEGRATION_TEST_AT_EVERY_STEP").is_ok() {
                Some(DEFAULT_POST_STEP_PAUSE)
            } else {
                None
            };

        Self {
            name: name.to_owned(),
            setup_functions: Default::default(),
            events: Default::default(),
            actions: Default::default(),
            assertions: Default::default(),
            timeout: DEFAULT_STEP_TIMEOUT,
            post_step_pause,
            pause_on_failure,
            on_failure_handler: None,
            step_group_name: None,
            retries: 0,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_step_group_name(mut self, name: &str) -> Self {
        self.step_group_name = Some(name.to_string());
        self
    }

    pub fn add_named_assertion<N, F>(mut self, name: N, callback: F) -> Self
    where
        N: Into<String>,
        F: FnMut(&mut App, WindowId) -> AssertionOutcome + 'static,
    {
        self.assertions.push(Assertion {
            name: Some(name.into()),
            callback: CallbackType::Assertion(Box::new(callback)),
        });
        self
    }

    /// Adds a named assertion with a callback that expects a map of data from prior steps.
    pub fn add_named_assertion_with_data_from_prior_step<N, F>(
        mut self,
        name: N,
        callback: F,
    ) -> Self
    where
        N: Into<String>,
        F: FnMut(&mut App, WindowId, &mut StepDataMap) -> AssertionOutcome + 'static,
    {
        self.assertions.push(Assertion {
            name: Some(name.into()),
            callback: CallbackType::AssertionWithData(Box::new(callback)),
        });
        self
    }

    pub fn add_assertion<F>(mut self, callback: F) -> Self
    where
        F: FnMut(&mut App, WindowId) -> AssertionOutcome + 'static,
    {
        self.assertions.push(Assertion {
            name: None,
            callback: CallbackType::Assertion(Box::new(callback)),
        });
        self
    }

    pub fn set_on_failure_handler<N, F>(mut self, name: N, callback: F) -> Self
    where
        N: Into<String>,
        F: FnMut(&mut App, WindowId) -> AssertionOutcome + 'static,
    {
        self.on_failure_handler = Some(Assertion {
            name: Some(name.into()),
            callback: CallbackType::Assertion(Box::new(callback)),
        });
        self
    }

    pub fn set_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn set_post_step_pause(mut self, pause: Duration) -> Self {
        self.post_step_pause = Some(pause);
        self
    }

    pub fn set_pause_on_failure(mut self, pause: Duration) -> Self {
        self.pause_on_failure = Some(pause);
        self
    }

    pub fn set_retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    pub(super) fn retries(&self) -> u32 {
        self.retries
    }

    pub fn with_input_string(self, input: &str, extra_keystrokes: Option<&[&str]>) -> Self {
        let v: Vec<String> = input
            .chars()
            .map(|x| {
                if x.is_ascii_uppercase() {
                    format!("shift-{x}")
                } else {
                    x.to_string()
                }
            })
            .collect();
        let v2: Vec<&str> = v.iter().map(|s| &**s).collect();

        self.with_keystrokes(v2.as_slice())
            .with_keystrokes(extra_keystrokes.unwrap_or(&[]))
    }

    pub fn with_per_platform_keystroke(self, keystrokes: PerPlatformKeystroke) -> Self {
        let keystroke = if OperatingSystem::get().is_mac() {
            keystrokes.mac
        } else {
            keystrokes.linux_and_windows
        };

        self.with_keystrokes(&[keystroke])
    }

    pub fn with_keystrokes(mut self, keystrokes: &[impl AsRef<str>]) -> Self {
        for keystroke in keystrokes
            .iter()
            .map(|keystroke| Keystroke::parse(keystroke).expect("failed to parse keystroke"))
        {
            // Match macOS by mapping control/special keys to their ASCII characters.
            // This covers common characters, but is non-exhaustive (it's mainly
            // missing arrow and function keys).
            let chars = match (&keystroke, keystroke.key.as_str()) {
                (Keystroke { ctrl: true, .. }, "c") => "\u{3}".to_string(),
                (_, "enter") => "\r".to_string(),
                (Keystroke { shift: true, .. }, "tab") => "\u{19}".to_string(),
                (_, "tab") => "\t".to_string(),
                (_, "backspace") => "\u{7f}".to_string(),
                (_, "numpadenter") => "\u{3}".to_string(),
                (_, "escape") => "\u{1b}".to_string(),
                (keystroke, _) => keystroke.key.clone(),
            };

            self.events
                .push_back(IntegrationTestEvent::WithEvent(Event::KeyDown {
                    chars,
                    keystroke,
                    details: KeyEventDetails::default(),
                    is_composing: false,
                }));
        }
        self
    }

    pub fn with_keystrokes_in_composing(mut self, keystrokes: &[&str]) -> Self {
        for keystroke in keystrokes
            .iter()
            .map(|keystroke| Keystroke::parse(keystroke).expect("failed to parse keystroke"))
        {
            let chars = if keystroke.ctrl && keystroke.key.as_str() == "c" {
                "\u{3}".to_string()
            } else {
                keystroke.key.clone()
            };
            self.events
                .push_back(IntegrationTestEvent::WithEvent(Event::KeyDown {
                    chars,
                    keystroke,
                    details: KeyEventDetails::default(),
                    is_composing: true,
                }));
        }
        self
    }

    pub fn with_typed_characters(mut self, characters: &[&str]) -> Self {
        for character in characters.iter() {
            self.events
                .push_back(IntegrationTestEvent::WithEvent(Event::TypedCharacters {
                    chars: String::from(*character),
                }));
        }
        self
    }

    pub fn with_event(mut self, event: Event) -> Self {
        self.events
            .push_back(IntegrationTestEvent::WithEvent(event));
        self
    }

    pub fn with_event_fn<F>(mut self, event_fn: F) -> Self
    where
        F: Fn(&mut App, WindowId) -> Event + 'static,
    {
        self.events
            .push_back(IntegrationTestEvent::WithEventFn(Box::new(event_fn)));
        self
    }

    pub fn with_click_on_saved_position_fn<F>(mut self, position_fn: F) -> Self
    where
        F: Fn(&mut App, WindowId) -> String + 'static,
    {
        self.events
            .push_back(IntegrationTestEvent::WithSavedPositionFn(
                Box::new(position_fn),
                MouseEvent::ClickOnce,
            ));
        self
    }

    pub fn with_click_on_saved_position<S: Into<String>>(mut self, position_id: S) -> Self {
        self.events
            .push_back(IntegrationTestEvent::WithSavedPosition(
                position_id.into(),
                MouseEvent::ClickOnce,
            ));
        self
    }

    pub fn with_right_click_on_saved_position_fn<F>(mut self, position_fn: F) -> Self
    where
        F: Fn(&mut App, WindowId) -> String + 'static,
    {
        self.events
            .push_back(IntegrationTestEvent::WithSavedPositionFn(
                Box::new(position_fn),
                MouseEvent::RightClickOnce,
            ));
        self
    }

    pub fn with_right_click_on_saved_position<S: Into<String>>(mut self, position_id: S) -> Self {
        self.events
            .push_back(IntegrationTestEvent::WithSavedPosition(
                position_id.into(),
                MouseEvent::RightClickOnce,
            ));
        self
    }

    pub fn with_hover_on_saved_position_fn<F>(mut self, position_fn: F) -> Self
    where
        F: Fn(&mut App, WindowId) -> String + 'static,
    {
        self.events
            .push_back(IntegrationTestEvent::WithSavedPositionFn(
                Box::new(position_fn),
                MouseEvent::Hover,
            ));
        self
    }

    pub fn with_hover_over_saved_position<S: Into<String>>(mut self, position_id: S) -> Self {
        self.events
            .push_back(IntegrationTestEvent::WithSavedPosition(
                position_id.into(),
                MouseEvent::Hover,
            ));
        self
    }

    pub fn with_action<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut App, WindowId, &mut StepDataMap) + 'static,
    {
        self.actions.push_back(Box::new(callback));
        self
    }

    /// Adds an action that captures a screenshot and saves it to the artifacts
    /// directory with the given filename (e.g. `"after_bootstrap.png"`).
    pub fn with_take_screenshot(self, filename: impl Into<String>) -> Self {
        let filename = filename.into();
        self.with_action(move |_app, _window_id, step_data_map| {
            step_data_map.insert(super::video_recorder::SCREENSHOT_PATH_KEY, filename.clone());
        })
    }

    /// Adds an action that starts video recording.
    pub fn with_start_recording(self) -> Self {
        self.with_action(|_app, _window_id, step_data_map| {
            if let Some(recorder) = super::video_recorder::get_recorder_mut(step_data_map) {
                recorder.start_recording();
                log::info!("VideoRecorder: recording started");
                #[cfg(feature = "integration_tests")]
                let recording_start = recorder.recording_start();
                #[cfg(not(feature = "integration_tests"))]
                let recording_start: Option<instant::Instant> = None;
                if let Some(log) = super::action_log::get_action_log_mut(step_data_map) {
                    if let Some(start) = recording_start {
                        log.set_recording_start(start);
                    }
                    log.record("Recording started");
                }
            }
        })
    }

    /// Adds an action that stops video recording.
    pub fn with_stop_recording(self) -> Self {
        self.with_action(|_app, _window_id, step_data_map| {
            if let Some(recorder) = super::video_recorder::get_recorder_mut(step_data_map) {
                recorder.stop_recording();
                log::info!("VideoRecorder: recording stopped");
                if let Some(log) = super::action_log::get_action_log_mut(step_data_map) {
                    log.record("Recording stopped");
                }
            }
        })
    }

    /// Add a setup function which runs before any events, actions, or
    /// assertions in the test step.
    ///
    /// This is a good place for any filesystem operations relevant for a
    /// test step.
    pub fn with_setup<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut TestSetupUtils) + 'static,
    {
        self.setup_functions.push_back(Box::new(callback));
        self
    }
}

pub(super) async fn run_step(
    step: &mut TestStep,
    app: &mut App,
    window_id: WindowId,
    window: &dyn Window,
    step_data_map: &mut StepDataMap,
    test_setup_utils: &mut TestSetupUtils,
    sigint_received: &AtomicBool,
) -> AssertionOutcome {
    let deadline = Instant::now() + step.timeout;

    let original_frame_count = {
        let presenter_rc = app.presenter(window_id).expect("Invalid window id");
        let presenter = presenter_rc.borrow();
        presenter.frame_count()
    };

    log::info!(
        "Running test step '{}' at frame {} with {} events",
        step.name,
        original_frame_count,
        step.events.len()
    );

    for setup in step.setup_functions.iter() {
        if sigint_received.load(Ordering::Relaxed) {
            return AssertionOutcome::Canceled;
        }
        setup(test_setup_utils);
    }

    for e in &step.events {
        if sigint_received.load(Ordering::Relaxed) {
            return AssertionOutcome::Canceled;
        }

        // TODO would be cool to move it under IntegrationTestEvent
        match e {
            IntegrationTestEvent::WithEvent(..) | IntegrationTestEvent::WithEventFn(..) => {
                let event = if let IntegrationTestEvent::WithEvent(e) = e {
                    e.clone()
                } else if let IntegrationTestEvent::WithEventFn(f) = e {
                    f(app, window_id)
                } else {
                    unreachable!("only handling WithEvent variants here")
                };

                log::info!("Dispatching event {event:?}");
                if let Some(log) = action_log::get_action_log_mut(step_data_map) {
                    log.record(format!("Event: {}", action_log::event_description(&event)));
                }
                record_overlay_event_for_event(&event, step_data_map);
                let dispatch_result =
                    app.update(|ctx| (window.callbacks().event_callback)(event.clone(), ctx));
                if !dispatch_result.handled
                    && let Event::KeyDown {
                        chars,
                        is_composing,
                        ..
                    } = event
                    && !is_composing
                {
                    // The input system expects a TypedCharacters event to follow keydown
                    // in order to update the editor's input unless is_composing is set
                    app.update(|ctx| {
                        (window.callbacks().event_callback)(
                            Event::TypedCharacters {
                                chars: chars.clone(),
                            },
                            ctx,
                        )
                    });
                }
            }
            IntegrationTestEvent::WithSavedPosition(_, mouse_event)
            | IntegrationTestEvent::WithSavedPositionFn(_, mouse_event) => {
                let presenter = app.presenter(window_id).expect("Invalid window id");
                let position_id = match e {
                    IntegrationTestEvent::WithSavedPosition(position_id, _) => {
                        position_id.to_string()
                    }
                    IntegrationTestEvent::WithSavedPositionFn(position_id_fn, _) => {
                        position_id_fn(app, window_id)
                    }
                    IntegrationTestEvent::WithEvent(..) | IntegrationTestEvent::WithEventFn(..) => {
                        unreachable!("already handled WithEvent variants")
                    }
                };
                let bounds = {
                    let presenter_ref = presenter.borrow();
                    presenter_ref.position_cache().get_position(&position_id)
                };

                // Note we are not using unwrap_or_else here because async closures
                // are experimental and we await in the failure case.
                if bounds.is_none() {
                    if let Some(pause) = step.pause_on_failure {
                        report_error!(
                            "Test step failed to find saved position, pausing",
                            extra: { "step" => %step.name, "position" => %position_id }
                        );
                        Timer::at(Instant::now() + pause).await;
                    }
                    return AssertionOutcome::failure(format!("No position for {position_id}"));
                }
                let bounds = bounds.unwrap();

                let center = bounds.center();
                match mouse_event {
                    MouseEvent::ClickOnce => {
                        record_overlay_kind(
                            overlay::OverlayKind::MouseDown {
                                x: center.x(),
                                y: center.y(),
                            },
                            step_data_map,
                        );
                        let mouse_down = Event::LeftMouseDown {
                            position: center,
                            modifiers: Default::default(),
                            click_count: 1,
                            is_first_mouse: false,
                        };
                        let mouse_up = Event::LeftMouseUp {
                            position: center,
                            modifiers: Default::default(),
                        };
                        for event in [mouse_down, mouse_up] {
                            app.update(|ctx| (window.callbacks().event_callback)(event, ctx));
                        }
                        record_overlay_kind(
                            overlay::OverlayKind::MouseUp {
                                x: center.x(),
                                y: center.y(),
                            },
                            step_data_map,
                        );
                    }
                    MouseEvent::RightClickOnce => {
                        record_overlay_kind(
                            overlay::OverlayKind::MouseDown {
                                x: center.x(),
                                y: center.y(),
                            },
                            step_data_map,
                        );
                        app.update(|ctx| {
                            (window.callbacks().event_callback)(
                                Event::RightMouseDown {
                                    position: center,
                                    cmd: false,
                                    shift: false,
                                    click_count: 1,
                                },
                                ctx,
                            )
                        });
                        record_overlay_kind(
                            overlay::OverlayKind::MouseUp {
                                x: center.x(),
                                y: center.y(),
                            },
                            step_data_map,
                        );
                    }
                    MouseEvent::Hover => {
                        app.update(|ctx| {
                            (window.callbacks().event_callback)(
                                Event::MouseMoved {
                                    position: center,
                                    cmd: false,
                                    shift: false,
                                    is_synthetic: false,
                                },
                                ctx,
                            )
                        });
                    }
                }
            }
        }

        if let Err(err) = maybe_render_frame(app, window_id, deadline).await {
            return err;
        }
    }

    for action in step.actions.iter() {
        if sigint_received.load(Ordering::Relaxed) {
            return AssertionOutcome::Canceled;
        }

        action(app, window_id, step_data_map);
        if let Some(log) = action_log::get_action_log_mut(step_data_map) {
            log.record("Action executed");
        }

        if let Err(err) = maybe_render_frame(app, window_id, deadline).await {
            return err;
        }
    }

    let mut last_failure = None;
    let mut last_assertion_name = None;
    'outer: for assertion in step.assertions.iter_mut() {
        // We loop through until the assertion is true or the timeout is reached.
        // If the timeout is reached in a failure state we panic and fail the test.
        // Regardless of the assertion timeout, always run the assertion loop at least once
        // for each assertion.
        let mut idx = 0;
        while idx == 0 || Instant::now() < deadline {
            // Check for Ctrl+C before running the assertion
            if sigint_received.load(Ordering::Relaxed) {
                log::info!(
                    "Test interrupted by Ctrl+C during assertion '{}'",
                    assertion
                        .name
                        .as_ref()
                        .map_or("unnamed", |name| name.as_str())
                );
                return AssertionOutcome::Canceled;
            }

            Timer::at(Instant::now() + THROTTLE_PERIOD).await;
            if idx == 0 {
                let name = assertion
                    .name
                    .as_ref()
                    .map_or("unnamed", |name| name.as_str());
                last_assertion_name = Some(name);
                log::info!("entering assertion loop for '{name}'");
                if let Some(log) = action_log::get_action_log_mut(step_data_map) {
                    log.record(format!("Assertion started: {name}"));
                }
            }
            idx += 1;
            let res = match &mut assertion.callback {
                CallbackType::Assertion(cb) => cb(app, window_id),
                CallbackType::AssertionWithData(cb) => cb(app, window_id, step_data_map),
            };
            match res {
                AssertionOutcome::Success => {
                    if let Some(log) = action_log::get_action_log_mut(step_data_map) {
                        let name = assertion.name.as_deref().unwrap_or("unnamed");
                        log.record(format!("Assertion passed: {name}"));
                    }
                    continue 'outer;
                }
                AssertionOutcome::SuccessWithData(step_data) => {
                    if let Some(log) = action_log::get_action_log_mut(step_data_map) {
                        let name = assertion.name.as_deref().unwrap_or("unnamed");
                        log.record(format!("Assertion passed: {name}"));
                    }
                    step_data_map.insert_step_data(step_data);
                    continue 'outer;
                }
                AssertionOutcome::PreconditionFailed(s) => {
                    // Early exit if we've flaked.
                    return AssertionOutcome::PreconditionFailed(s);
                }
                AssertionOutcome::Failure {
                    message,
                    backtrace,
                    failed_assertion_name,
                } => {
                    last_failure = Some(AssertionOutcome::Failure {
                        message,
                        backtrace,
                        failed_assertion_name: failed_assertion_name.or(assertion.name.clone()),
                    });
                }
                AssertionOutcome::ImmediateFailure {
                    message,
                    backtrace,
                    failed_assertion_name,
                } => {
                    return AssertionOutcome::ImmediateFailure {
                        message,
                        backtrace,
                        failed_assertion_name: failed_assertion_name.or(assertion.name.clone()),
                    };
                }
                AssertionOutcome::Canceled => {
                    return AssertionOutcome::Canceled;
                }
            }
        }

        // If we are here, the timer for the current assertion has elapsed without hitting success.
        // Check if there is a final assertion to run, and if so run it.
        if let Some(mut final_assertion) = step.on_failure_handler.take() {
            // Log the timed-out assertion's failure message before running the final assertion
            if let Some(AssertionOutcome::Failure { message, .. }) = &last_failure {
                report_error!(
                    "Assertion timed out",
                    extra: {
                        "assertion" => %last_assertion_name.unwrap_or("unknown"),
                        "message" => %message
                    }
                );
            }
            let res = match &mut final_assertion.callback {
                CallbackType::Assertion(cb) => cb(app, window_id),
                CallbackType::AssertionWithData(cb) => cb(app, window_id, step_data_map),
            };
            match res {
                AssertionOutcome::Success => {
                    continue 'outer;
                }
                AssertionOutcome::SuccessWithData(step_data) => {
                    step_data_map.insert_step_data(step_data);
                    continue 'outer;
                }
                AssertionOutcome::PreconditionFailed(s) => {
                    // Early exit if we've flaked.
                    return AssertionOutcome::PreconditionFailed(s);
                }
                AssertionOutcome::Failure {
                    message,
                    backtrace,
                    failed_assertion_name,
                } => {
                    last_failure = Some(AssertionOutcome::Failure {
                        message,
                        backtrace,
                        failed_assertion_name: failed_assertion_name
                            .or(final_assertion.name.clone()),
                    });
                }
                AssertionOutcome::ImmediateFailure {
                    message,
                    backtrace,
                    failed_assertion_name,
                } => {
                    return AssertionOutcome::ImmediateFailure {
                        message,
                        backtrace,
                        failed_assertion_name: failed_assertion_name
                            .or(final_assertion.name.clone()),
                    };
                }
                AssertionOutcome::Canceled => {
                    return AssertionOutcome::Canceled;
                }
            }
        }

        // We only get this far in the case of a test failure.
        let last_failure = last_failure.expect("last_failure should be set");
        if let Some(msg) = last_failure.as_failure_message().map(str::to_owned)
            && let Some(log) = action_log::get_action_log_mut(step_data_map)
        {
            let name = last_assertion_name.unwrap_or("unknown");
            log.record(format!("Assertion failed: {name}: {msg}"));
        }
        if let Some(pause) = step.pause_on_failure {
            let AssertionOutcome::Failure { message, .. } = &last_failure else {
                panic!("last_failure should be a failure assertion");
            };
            report_error!(
                "Test step failed, pausing",
                extra: { "step" => %step.name, "message" => %message }
            );
            Timer::at(Instant::now() + pause).await;
        }
        // Mostly logging to get a timestamp - the test driver will panic right
        // after this.
        report_error!(
            "Test step failed",
            extra: { "step" => %step.name, "assertion" => %last_assertion_name.unwrap_or("unknown") }
        );
        return last_failure;
    }
    if let Some(pause) = step.post_step_pause {
        Timer::at(Instant::now() + pause).await;
    }
    AssertionOutcome::Success
}

/// Renders a frame for the window, if necessary.
///
/// Returns a `Err(AssertionOutcome)` if an error occurred that should cause
/// the test to fail.
async fn maybe_render_frame(
    app: &mut App,
    window_id: WindowId,
    deadline: Instant,
) -> Result<(), AssertionOutcome> {
    let Some(initial_frame_count) = frame_count(app, window_id) else {
        // If we can't compute the frame count, the window has been closed, in
        // which case we should skip rendering the frame and move on.
        return Ok(());
    };
    if app.has_window_invalidations(window_id) {
        log::info!("app needs to render a frame");

        // Allow at least one frame to pass if the app needs to redraw the window
        let mut rerendered = false;
        while Instant::now() < deadline {
            Timer::at(Instant::now() + THROTTLE_PERIOD).await;
            let Some(next_frame_count) = frame_count(app, window_id) else {
                // If we can't compute the frame count, the window has been
                // closed, in which case we should skip rendering the frame
                // and move on.
                return Ok(());
            };
            if next_frame_count > initial_frame_count {
                log::info!("at least one frame has passed, moving on.");
                rerendered = true;
                break;
            }
        }

        if !rerendered {
            return Err(AssertionOutcome::failure(
                "Test step failed because no frames were rendered".to_string(),
            ));
        }
    } else {
        log::debug!("not checking for a frame to pass");
    }

    Ok(())
}

/// Returns the total number of frames rendered in the given window.
fn frame_count(app: &mut App, window_id: WindowId) -> Option<usize> {
    let presenter_rc = app.presenter(window_id)?;
    let presenter = presenter_rc.borrow();
    Some(presenter.frame_count())
}

fn record_overlay_event_for_event(event: &Event, step_data_map: &mut StepDataMap) {
    let kind = match event {
        Event::LeftMouseDown { position, .. }
        | Event::RightMouseDown { position, .. }
        | Event::MiddleMouseDown { position, .. }
        | Event::ForwardMouseDown { position, .. }
        | Event::BackMouseDown { position, .. } => Some(overlay::OverlayKind::MouseDown {
            x: position.x(),
            y: position.y(),
        }),
        Event::LeftMouseDragged { position, .. } => Some(overlay::OverlayKind::MouseMove {
            x: position.x(),
            y: position.y(),
        }),
        Event::LeftMouseUp { position, .. } => Some(overlay::OverlayKind::MouseUp {
            x: position.x(),
            y: position.y(),
        }),
        Event::KeyDown { keystroke, .. } => {
            let text = overlay::keystroke_display_text(keystroke);
            Some(overlay::OverlayKind::KeyPress { display_text: text })
        }
        _ => None,
    };
    if let Some(kind) = kind {
        record_overlay_kind(kind, step_data_map);
    }
}
