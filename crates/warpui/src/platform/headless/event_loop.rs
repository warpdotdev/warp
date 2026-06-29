use std::mem::ManuallyDrop;
use std::sync::mpsc::{Receiver, Sender};

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

/// Run a simple, blocking event loop that processes AppEvent messages until termination.
pub(super) fn run(
    mut ui_app: crate::App,
    callbacks: &mut AppCallbackDispatcher,
    init_fn: platform::app::AppInitCallbackFn,
    receiver: Receiver<AppEvent>,
    sender: Sender<AppEvent>,
) -> TerminationResult {
    // Set up Ctrl-C handler to gracefully terminate the app
    setup_signal_handler(sender);

    // First, initialize the app.
    callbacks.initialize_app(init_fn);

    // Then, process events until termination.
    for event in receiver.iter() {
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
                    break;
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
    }

    // Drop the receiver so the Ctrl+C signal handler's channel send will fail,
    // causing it to fall through to `process::exit(130)`. Without this, the
    // send succeeds (since the receiver is still in scope) but nobody is reading
    // from the channel, making Ctrl+C ineffective during shutdown.
    drop(receiver);

    callbacks.app_will_terminate();

    ui_app.termination_result().unwrap_or(Ok(()))
}

/// Set up a signal handler for Ctrl-C (SIGINT) to gracefully terminate the app.
///
/// When Ctrl-C is received, this will send a Terminate event to the event loop,
/// allowing the app to shut down gracefully via the existing termination logic.
#[cfg(not(target_family = "wasm"))]
fn setup_signal_handler(sender: Sender<AppEvent>) {
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
            // If we can't send the event, force exit
            std::process::exit(130); // 128 + SIGINT (2) = 130
        }
    });

    if let Err(e) = result {
        log::warn!("Failed to set up Ctrl-C handler: {e}");
    }
}

#[cfg(target_family = "wasm")]
fn setup_signal_handler(_sender: Sender<AppEvent>) {
    // No signal handling on WASM
}
