//! The TUI backend's main event loop.
//!
//! This is modeled on `headless/event_loop.rs` but adds two TUI-specific event
//! kinds:
//!
//! * [`AppEvent::Redraw`] — rebuild the active window's [`Scene`] via
//!   `callbacks.for_window(window).build_scene(window)` and rasterize it to the
//!   terminal with the [`TerminalRenderer`].
//! * [`AppEvent::TerminalInput`] — a raw crossterm event (key/mouse/resize) read
//!   off the input thread, to be translated (see [`super::input`]) and
//!   dispatched via `callbacks.for_window(window).dispatch_event(..)` / resized
//!   via `window_resized(..)`.
//!
//! The `Redraw`/`TerminalInput` handling below is intentionally left as a
//! scaffold; it is fleshed out by the `eventloop` child agent.
//!
//! [`Scene`]: crate::Scene

use std::mem::ManuallyDrop;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyModifiers};

use super::render::TerminalRenderer;
use super::windowing::Window;
use crate::platform::app::{AppCallbackDispatcher, ApproveTerminateResult, TerminationResult};
use crate::platform::{self, TerminationMode};
use crate::{AppContext, WindowId};

/// Minimum interval between rendered frames. The GUI backends throttle repaints
/// to the display's refresh rate (vsync); the TUI has no such limiter, so
/// without this a redraw that re-triggers itself (animations, streamed output,
/// periodic model notifications) would spin the loop at an unbounded frame rate,
/// pegging the CPU and growing memory until the system OOMs.
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16); // ~60 FPS

/// Application events handled on the TUI platform's main thread.
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
    /// Rebuild and repaint the active window to the terminal.
    Redraw,
    /// A raw terminal input event (key, mouse, resize) read off the input thread.
    TerminalInput(Event),
}

/// Run a simple, blocking event loop that processes [`AppEvent`] messages until
/// termination.
pub(super) fn run(
    mut ui_app: crate::App,
    callbacks: &mut AppCallbackDispatcher,
    init_fn: platform::app::AppInitCallbackFn,
    receiver: Receiver<AppEvent>,
    sender: Sender<AppEvent>,
    renderer: TerminalRenderer,
) -> TerminationResult {
    // Set up Ctrl-C handler to gracefully terminate the app.
    setup_signal_handler(sender);

    // The renderer owns the terminal; dropping it (when this function returns)
    // restores the terminal out of raw mode / alternate screen.
    let mut renderer = renderer;

    // First, initialize the app. This opens the TUI window + root view.
    log::info!("[DEBUG] TUI backend entered; initializing app");
    callbacks.initialize_app(init_fn);

    // The active window is the one we paint and route input to. It is updated
    // whenever the window manager reports a focus change.
    let mut active_window_id: Option<WindowId> = ui_app.read(|ctx| ctx.windows().active_window());

    // Timestamp of the last rendered frame, used to throttle redraws to at most
    // one per `MIN_FRAME_INTERVAL`.
    let mut last_frame: Option<Instant> = None;

    // Opening the window during init also queued an `ActiveWindowChanged` event
    // that drives the first paint once the loop starts. Paint one frame here
    // directly too, so any ordering gap in that path can't leave the terminal
    // sitting blank.
    log::info!(
        "[DEBUG] TUI event loop starting: active_window={active_window_id:?}, term_size={:?}",
        crossterm::terminal::size()
    );
    if let Some(window_id) = active_window_id {
        redraw(&ui_app, callbacks, &mut renderer, window_id);
        last_frame = Some(Instant::now());
    }

    // Then, process events until termination.
    for event in receiver.iter() {
        match event {
            AppEvent::RunCallback(callback) => ui_app.update(callback),
            AppEvent::RunTask(task) => {
                let task = ManuallyDrop::into_inner(task);
                task.run();
            }
            AppEvent::Terminate(termination_mode) => {
                let should_terminate = match termination_mode {
                    TerminationMode::Cancellable => matches!(
                        callbacks.should_terminate_app(),
                        ApproveTerminateResult::Terminate
                    ),
                    TerminationMode::ForceTerminate | TerminationMode::ContentTransferred => true,
                };
                if should_terminate {
                    break;
                }
            }
            AppEvent::CloseWindow(window_id) => callbacks.window_will_close(window_id),
            AppEvent::ActiveWindowChanged(window_id) => {
                log::info!("[DEBUG] TUI ActiveWindowChanged: {window_id:?}");
                active_window_id = window_id;
                callbacks.active_window_changed(window_id);
                // Make sure a newly-focused window paints at least one frame.
                if let Some(window) =
                    window_id.and_then(|id| ui_app.read(|ctx| ctx.windows().platform_window(id)))
                {
                    window.as_ctx().request_redraw();
                }
            }
            AppEvent::Redraw => {
                let Some(window_id) = active_window_id else {
                    log::info!("[DEBUG] TUI Redraw dropped: no active window");
                    continue;
                };
                // Throttle to a minimum frame interval. `request_redraw`
                // already coalesces pending redraws to one in-flight
                // `Redraw` (see `windowing::Window::request_redraw`); this
                // bounds how fast we process them so a self-retriggering
                // redraw can't free-run and exhaust CPU/memory.
                if let Some(remaining) =
                    last_frame.and_then(|last| MIN_FRAME_INTERVAL.checked_sub(last.elapsed()))
                {
                    std::thread::sleep(remaining);
                }
                redraw(&ui_app, callbacks, &mut renderer, window_id);
                last_frame = Some(Instant::now());
            }
            AppEvent::TerminalInput(terminal_event) => {
                // Raw mode suppresses SIGINT, so the ctrlc handler never fires;
                // treat Ctrl+C as a quit here instead. Checked before the
                // active-window guard so the user can always exit, even if no
                // window ever activated.
                if let Event::Key(key) = &terminal_event {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        log::info!("[DEBUG] TUI received Ctrl+C; terminating");
                        break;
                    }
                }
                let Some(window_id) = active_window_id else {
                    continue;
                };
                let Some(window) = ui_app.read(|ctx| ctx.windows().platform_window(window_id))
                else {
                    continue;
                };
                match terminal_event {
                    Event::Key(key) => {
                        if let Some(event) = super::input::translate_key(key) {
                            callbacks.for_window(window.as_ref()).dispatch_event(event);
                            // Repaint to reflect any state the key event changed.
                            window.as_ctx().request_redraw();
                        }
                    }
                    Event::Resize(cols, rows) => {
                        if let Some(tui_window) = window.as_any().downcast_ref::<Window>() {
                            tui_window.set_cell_size((cols, rows));
                        }
                        // `window_resized` refreshes the framework's cached bounds
                        // and itself requests a redraw at the new size.
                        callbacks
                            .for_window(window.as_ref())
                            .window_resized(window.as_ctx());
                    }
                    // Mouse, focus, and paste events are not yet handled.
                    _ => {}
                }
            }
        }
    }

    // Drop the receiver so the Ctrl+C signal handler's channel send will fail,
    // causing it to fall through to `process::exit(130)`.
    drop(receiver);

    callbacks.app_will_terminate();

    ui_app.termination_result().unwrap_or(Ok(()))
}

/// Rebuilds the active window's [`Scene`](crate::Scene), rasterizes it to the
/// terminal, and notifies the framework that the frame was drawn.
fn redraw(
    ui_app: &crate::App,
    callbacks: &mut AppCallbackDispatcher,
    renderer: &mut TerminalRenderer,
    window_id: WindowId,
) {
    let Some(window) = ui_app.read(|ctx| ctx.windows().platform_window(window_id)) else {
        return;
    };
    let Some(tui_window) = window.as_any().downcast_ref::<Window>() else {
        return;
    };

    // Clear the coalescing flag before building so a redraw requested *during*
    // scene construction enqueues a fresh frame.
    tui_window.clear_redraw_queued();
    let (cols, rows) = tui_window.cell_size();
    if cols == 0 || rows == 0 {
        log::info!("[DEBUG] TUI redraw skipped: zero terminal size ({cols}x{rows})");
        return;
    }

    let scene = callbacks
        .for_window(window.as_ref())
        .build_scene(window.as_ctx());
    let mut layer_count = 0usize;
    let mut rect_count = 0usize;
    let mut glyph_count = 0usize;
    for layer in scene.layers() {
        layer_count += 1;
        rect_count += layer.rects.len();
        glyph_count += layer.glyphs.len();
    }
    log::info!(
        "[DEBUG] TUI redraw: {cols}x{rows}, layers={layer_count}, rects={rect_count}, glyphs={glyph_count}"
    );
    if let Err(err) = renderer.render(&scene, cols, rows) {
        log::warn!("Failed to render TUI frame: {err:#}");
    }
    callbacks.for_window(window.as_ref()).frame_drawn();
}

/// Set up a signal handler for Ctrl-C (SIGINT) to gracefully terminate the app.
#[cfg(not(target_family = "wasm"))]
fn setup_signal_handler(sender: Sender<AppEvent>) {
    let result = ctrlc::set_handler(move || {
        log::info!("Received Ctrl-C signal in TUI mode, terminating application");
        if sender
            .send(AppEvent::Terminate(TerminationMode::ForceTerminate))
            .is_err()
        {
            // If we can't send the event, force exit.
            std::process::exit(130); // 128 + SIGINT (2) = 130
        }
    });

    if let Err(e) = result {
        log::warn!("Failed to set up Ctrl-C handler: {e}");
    }
}

#[cfg(target_family = "wasm")]
fn setup_signal_handler(_sender: Sender<AppEvent>) {}
