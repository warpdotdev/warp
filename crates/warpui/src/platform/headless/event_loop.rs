use std::mem::ManuallyDrop;
use std::ops::ControlFlow;
#[cfg(target_os = "macos")]
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::{self, Receiver, SendError};

#[cfg(target_os = "macos")]
use objc2_core_foundation::{
    CFRetained, CFRunLoop, CFRunLoopSource, CFRunLoopSourceContext, kCFRunLoopDefaultMode,
};

use crate::platform::app::{
    AppCallbackDispatcher, ApproveTerminateResult, TerminationRequestSource, TerminationResult,
};
use crate::platform::{self, TerminationMode};
use crate::{AppContext, WindowId};

/// Application events handled on the headless platform's main thread.
pub(super) enum AppEvent {
    /// Run the wrapped task on the main thread.
    RunTask(ManuallyDrop<async_task::Runnable>),
    /// Run a synchronous callback on the main thread.
    RunCallback(Box<dyn FnOnce(&mut AppContext) + Send + Sync>),
    /// Close a window.
    CloseWindow(WindowId),
    /// Active window changed.
    ActiveWindowChanged(Option<WindowId>),
    /// Exit the event loop, terminating the application.
    Terminate(TerminationMode),
}

#[derive(Clone)]
pub(super) struct EventSender {
    sender: mpsc::Sender<AppEvent>,
    #[cfg(target_os = "macos")]
    run_loop_signal: Arc<RunLoopSignal>,
}

impl EventSender {
    pub(super) fn send(&self, event: AppEvent) -> Result<(), SendError<AppEvent>> {
        self.sender.send(event)?;
        #[cfg(target_os = "macos")]
        self.run_loop_signal.signal();
        Ok(())
    }
}

pub(super) struct EventReceiver {
    receiver: Receiver<AppEvent>,
    #[cfg(target_os = "macos")]
    _run_loop_signal: Arc<RunLoopSignal>,
}

pub(super) fn channel() -> (EventSender, EventReceiver) {
    let (sender, receiver) = mpsc::channel();

    #[cfg(target_os = "macos")]
    {
        let run_loop_signal = Arc::new(RunLoopSignal::new());
        (
            EventSender {
                sender,
                run_loop_signal: run_loop_signal.clone(),
            },
            EventReceiver {
                receiver,
                _run_loop_signal: run_loop_signal,
            },
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        (EventSender { sender }, EventReceiver { receiver })
    }
}

#[cfg(target_os = "macos")]
struct RunLoopSignal {
    run_loop: CFRetained<CFRunLoop>,
    source: CFRetained<CFRunLoopSource>,
}

#[cfg(target_os = "macos")]
impl RunLoopSignal {
    fn new() -> Self {
        objc2::MainThreadMarker::new()
            .expect("the macOS headless event loop must run on the process main thread");

        let run_loop = CFRunLoop::current().expect("the current thread must have a run loop");
        let mut context = CFRunLoopSourceContext {
            version: 0,
            info: std::ptr::null_mut(),
            retain: None,
            release: None,
            copyDescription: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: Some(stop_run_loop),
        };
        // SAFETY: Core Foundation copies the context during this call. The context has no
        // associated data, and its callback does not access the context pointer.
        let source = unsafe { CFRunLoopSource::new(None, 0, &mut context) }
            .expect("Core Foundation should have successfully created the run-loop source");
        // SAFETY: The source and mode are valid for the lifetime of the headless event loop.
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopDefaultMode });

        Self { run_loop, source }
    }

    fn signal(&self) {
        self.source.signal();
        self.run_loop.wake_up();
    }
}

// SAFETY: `RunLoopSignal` only exposes the thread-safe `CFRunLoopSourceSignal` and
// `CFRunLoopWakeUp` operations to other threads. The source is installed and serviced only by
// the process main thread.
#[cfg(target_os = "macos")]
unsafe impl Send for RunLoopSignal {}

// SAFETY: See the `Send` implementation. Concurrent calls only signal and wake the run loop.
#[cfg(target_os = "macos")]
unsafe impl Sync for RunLoopSignal {}

#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn stop_run_loop(_info: *mut std::ffi::c_void) {
    CFRunLoop::current()
        .expect("the run-loop source callback should be running on a thread with a run loop")
        .stop();
}

/// Run a simple, blocking event loop that processes AppEvent messages until termination.
pub(super) fn run(
    mut ui_app: crate::App,
    callbacks: &mut AppCallbackDispatcher,
    init_fn: platform::app::AppInitCallbackFn,
    receiver: EventReceiver,
    sender: EventSender,
) -> TerminationResult {
    // Set up the Ctrl-C handler to gracefully terminate the app.
    setup_signal_handler(sender);

    // Initialize the app before processing events.
    callbacks.initialize_app(init_fn);

    // Process events until termination.
    #[cfg(target_os = "macos")]
    {
        'event_loop: loop {
            CFRunLoop::run();
            loop {
                match receiver.receiver.try_recv() {
                    Ok(event) => {
                        if process_event(event, &mut ui_app, callbacks).is_break() {
                            break 'event_loop;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break 'event_loop,
                }
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        for event in receiver.receiver.iter() {
            if process_event(event, &mut ui_app, callbacks).is_break() {
                break;
            }
        }
    }

    // Drop the receiver so the Ctrl+C signal handler's channel send will fail,
    // causing it to fall through to `process::exit(130)`. Without this, the
    // send succeeds (since the receiver is still in scope) but nobody is reading
    // from the channel, making Ctrl+C ineffective during shutdown.
    drop(receiver);

    callbacks.app_will_terminate();

    ui_app.termination_result().unwrap_or(Ok(()))
}

fn process_event(
    event: AppEvent,
    ui_app: &mut crate::App,
    callbacks: &mut AppCallbackDispatcher,
) -> ControlFlow<()> {
    match event {
        AppEvent::RunCallback(callback) => ui_app.update(callback),
        AppEvent::RunTask(task) => {
            // Poll a task on the main thread.
            let task = ManuallyDrop::into_inner(task);
            task.run();
        }
        AppEvent::Terminate(termination_mode) => {
            let should_terminate = match termination_mode {
                TerminationMode::Cancellable => {
                    matches!(
                        callbacks.should_terminate_app(TerminationRequestSource::User),
                        ApproveTerminateResult::Terminate
                    )
                }
                TerminationMode::ForceTerminate | TerminationMode::ContentTransferred => true,
            };
            if should_terminate {
                return ControlFlow::Break(());
            }
        }
        AppEvent::CloseWindow(window_id) => {
            // Notify the app that a window is closing. The app will then remove the window
            // from WindowManager.
            callbacks.window_will_close(window_id);
        }
        AppEvent::ActiveWindowChanged(window_id) => {
            callbacks.active_window_changed(window_id);
        }
    }
    ControlFlow::Continue(())
}

/// Set up a signal handler for Ctrl-C (SIGINT) to gracefully terminate the app.
///
/// When Ctrl-C is received, this will send a Terminate event to the event loop,
/// allowing the app to shut down gracefully via the existing termination logic.
#[cfg(not(target_family = "wasm"))]
fn setup_signal_handler(sender: EventSender) {
    let result = ctrlc::set_handler(move || {
        log::info!("Received Ctrl-C signal in headless mode, terminating application");
        // Send a ForceTerminate event to ensure the app exits cleanly.
        // We use ForceTerminate rather than Cancellable to ensure the app exits
        // even if there are unsaved changes or other conditions that might prevent shutdown.
        if sender
            .send(AppEvent::Terminate(TerminationMode::ForceTerminate))
            .is_err()
        {
            log::warn!("Failed to send termination event - event loop may have already stopped");
            // If the event cannot be sent, force an exit.
            std::process::exit(130); // 128 + SIGINT (2) = 130
        }
    });

    if let Err(e) = result {
        log::warn!("Failed to set up Ctrl-C handler: {e}");
    }
}

#[cfg(target_family = "wasm")]
fn setup_signal_handler(_sender: EventSender) {
    // Signal handling is unavailable on WASM.
}
