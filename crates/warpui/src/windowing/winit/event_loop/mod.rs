mod key_events;

#[cfg(test)]
mod drag_drop_tests;

use std::collections::HashMap;
use std::mem::ManuallyDrop;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use crate::notification::RequestPermissionsOutcome;

use futures_util::future::LocalBoxFuture;
use futures_util::stream::AbortHandle;
use instant::{Duration, Instant};
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::Ime as ImeEvent;
use winit::event_loop::EventLoopProxy;
use winit::keyboard::{self, KeyCode};
use winit::window::WindowId as WinitWindowId;
use winit::{
    event::{ElementState, Event, MouseButton, StartCause, Touch, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
};

use crate::actions::StandardAction;
use crate::event::ModifiersState;
use crate::platform::NotificationInfo;
use crate::platform::OperatingSystem;
use crate::platform::{
    self,
    app::{AppCallbackDispatcher, ApproveTerminateResult},
    TerminationMode, WindowContext,
};
use crate::r#async::Timer;
use crate::rendering::wgpu::renderer;
use crate::windowing::winit::app::RequestPermissionsCallback;
use crate::windowing::winit::window::MIN_WINDOW_SIZE;
use crate::Event::{ClearMarkedText, SetMarkedText, TypedCharacters};
use crate::{AppContext, WindowId};

#[cfg(target_family = "wasm")]
use wasm_bindgen::JsCast;

use super::app::ClipboardEvent;
use super::window::DEFAULT_TITLEBAR_HEIGHT;
use super::CustomEvent;

#[cfg(windows)]
use super::windows::{add_network_connection_listener, WindowsNetworkConnectionPoint};

use self::key_events::convert_keyboard_input_event;

/// This is the time duration beyond which clicks get treated as separate single clicks instead of
/// double-click, triple-click, etc.
const MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(400);

/// The debounce timeout for drag-and-drop files. Multiple DroppedFile events
/// are received within this time window and then combined into a single DragAndDropFiles event.
/// This timeout ensures all files in a multi-file drag operation are batched together efficiently.
const DRAG_DROP_DEBOUNCE_TIMEOUT: Duration = Duration::from_millis(50);

/// Distance (in logical pixels) before a touch input is considered a drag. Flutter uses 18.
const MAX_TAP_DISTANCE: f64 = 18.;

/// Duration to hold before a touch becomes a right-click (context menu).
/// Matches the iOS and Android platform default of 500ms.
const LONG_PRESS_DURATION: Duration = Duration::from_millis(500);

/// Momentum scrolling configuration. Math is as follows:
/// Each tick (every MOMENTUM_FRAME_INTERVAL):
///
/// 1. Decay velocity: v = v * MOMENTUM_DECAY^(elapsed / MOMENTUM_DECAY_INTERVAL)
/// 2. Check stop condition: if |v| < MOMENTUM_MIN_VELOCITY, cancel the animation.
/// 3. Compute scroll delta: delta = v * elapsed
///
///  Decay is between iOS normal (0.984/8ms) and fast (0.923/8ms).
const MOMENTUM_DECAY: f32 = 0.968; // Every interval, velocity is multiplied by this factor.
const MOMENTUM_DECAY_INTERVAL: f32 = 0.008; // Time period (seconds) over which MOMENTUM_DECAY is applied
const MOMENTUM_FRAME_INTERVAL: Duration = Duration::from_millis(8); //Controls how often the momentum scroll tick fires.
                                                                    // Higher values means it fires less often (choppier)
const MOMENTUM_THRESHOLD: f32 = 50.0; // Min-velocity to start momentum scroll, Android standards
const MOMENTUM_MIN_VELOCITY: f32 = 1.0; // When velocity falls below this, scrolling stops. 1.0 is subpixel
const MOMENTUM_MAX_VELOCITY: f32 = 2000.0; // Hard cap on momentum initial velocity (px/s)
const MIN_VELOCITY_TIME_DELTA: f32 = 0.004; // Floor for time deltas to prevent spikes from batched events

/// TryFrom implementation for converting winit's `KeyCode` to
/// `crate::platform::keyboard::KeyCode`.
/// Only converts modifier keys and fails for all other keys.
fn try_from_winit_keycode(keycode: &KeyCode) -> Result<crate::platform::keyboard::KeyCode, ()> {
    match keycode {
        KeyCode::AltLeft => Ok(crate::platform::keyboard::KeyCode::AltLeft),
        KeyCode::AltRight => Ok(crate::platform::keyboard::KeyCode::AltRight),
        KeyCode::ShiftLeft => Ok(crate::platform::keyboard::KeyCode::ShiftLeft),
        KeyCode::ShiftRight => Ok(crate::platform::keyboard::KeyCode::ShiftRight),
        KeyCode::ControlLeft => Ok(crate::platform::keyboard::KeyCode::ControlLeft),
        KeyCode::ControlRight => Ok(crate::platform::keyboard::KeyCode::ControlRight),
        KeyCode::SuperLeft => Ok(crate::platform::keyboard::KeyCode::SuperLeft),
        KeyCode::SuperRight => Ok(crate::platform::keyboard::KeyCode::SuperRight),
        // Note that the Fn key is not well identified on Windows laptops (e.g.
        // winit wasn't able to identify it correctly on ThinkPad). But, if it's
        // identified, we still pass it on to the UI framework.
        KeyCode::Fn => Ok(crate::platform::keyboard::KeyCode::Fn),
        _ => Err(()),
    }
}

/// Data needed to detect double/triple-click.
struct MouseButtonPressState {
    pressed_at: Instant,
    button_pressed: MouseButton,
    click_count: u32,
}

#[derive(Debug)]
/// Purpose of the touch event.
enum TouchPurpose {
    Select,
    Scroll(Touch),
    /// A tap that hasn't yet been classified.
    /// Stores (initial touch, click count, start time for long-press detection).
    Tap(Touch, u32, Instant),
    /// Dragging the window via touch in the titlebar region.
    /// Stores the starting touch position (window-relative).
    WindowDrag {
        start_touch: PhysicalPosition<f64>,
    },
}

/// Tracks scroll velocity during active touch scrolling and momentum scrolling.
#[derive(Debug, Clone, Copy)]
struct ScrollVelocity {
    velocity: Vector2F,
    last_update: Instant,
}

/// The set of state we need to track per-window across frames.
struct WindowState {
    /// The UI framework's identifier for the window in question (not to be
    /// confused with winit's identifer for the window).
    window_id: crate::WindowId,
    /// The last known modifier key (ctr/alt/etc) state.
    modifiers: keyboard::ModifiersState,
    /// Whether the left Alt key is currently pressed. `ModifiersState` does not distinguish
    /// between left and right Alt, so we track per-side state by watching `KeyboardInput`
    /// events for `KeyCode::AltLeft`/`KeyCode::AltRight`. Used by the extra-meta-keys setting
    /// to apply the left/right-alt-as-meta preferences correctly.
    left_alt_pressed: bool,
    /// Whether the right Alt key is currently pressed. See [`Self::left_alt_pressed`].
    right_alt_pressed: bool,
    /// The currently-pressed mouse button, if any. Set back to None when the button is released.
    ///
    /// This ultimately should be a HashSet of mouse buttons, as more than one
    /// can be held down at a time.
    current_mouse_button_pressed: Option<MouseButton>,
    /// The last mouse button pressed. This persists after the button is released because it needs
    /// to keep that state to detect double/triple-click.
    last_mouse_button_pressed: Option<MouseButtonPressState>,
    /// The last known cursor position, measured in logical pixels.
    last_cursor_position: winit::dpi::LogicalPosition<f32>,
    /// Drag-and-drop files are received as separate DroppedFile events per file.
    /// We collect them and debounce them to create a single consistent DragAndDropFiles event.
    pending_drag_drop_files: Vec<String>,
    /// Track if we have a debounce timer already running for this window.
    has_pending_drag_drop_timer: bool,
    /// The purpose of the last touch event.
    last_touch_purpose: Option<TouchPurpose>,
    /// Tracks scroll velocity during active touch scrolling and momentum scrolling.
    /// Active phase determined through `momentum_scroll_abort.is_some()`.
    scroll_velocity: Option<ScrollVelocity>,
    /// Abort handle for momentum scrolling timer. Present only during the momentum phase.
    momentum_scroll_abort: Option<AbortHandle>,
    /// For touch events, stores whether soft keyboard was requested during LeftMouseDown.
    /// This is needed because touch keyboard updates are deferred to LeftMouseUp, but the
    /// UI element only requests the keyboard during LeftMouseDown.
    #[cfg(target_family = "wasm")]
    pending_soft_keyboard_request: bool,
}

impl WindowState {
    fn new(window_id: crate::WindowId) -> Self {
        Self {
            window_id,
            modifiers: Default::default(),
            left_alt_pressed: false,
            right_alt_pressed: false,
            current_mouse_button_pressed: None,
            last_mouse_button_pressed: None,
            last_cursor_position: Default::default(),
            pending_drag_drop_files: Vec::new(),
            has_pending_drag_drop_timer: false,
            last_touch_purpose: None,
            scroll_velocity: None,
            momentum_scroll_abort: None,
            #[cfg(target_family = "wasm")]
            pending_soft_keyboard_request: false,
        }
    }

    /// Cancels ongoing momentum scroll, clearing both the animation timer and velocity state.
    fn cancel_momentum_scroll(&mut self) {
        if let Some(abort_handle) = self.momentum_scroll_abort.take() {
            abort_handle.abort();
        }
        self.scroll_velocity = None;
    }

    /// When a mouse button is pressed, save it to [`Self::current_mouse_button_pressed`] so that
    /// we can detect dragging. Also save it to [`Self::last_mouse_button_pressed`] for
    /// double/triple-click. Returns the calculated click_count.
    fn determine_click_count_and_update_button_state(&mut self, button: MouseButton) -> u32 {
        self.current_mouse_button_pressed = Some(button);
        let now = Instant::now();
        // Increment the click_count if the button type is the same and the duration is faster than
        // MULTI_CLICK_INTERVAL.
        let click_count = self
            .last_mouse_button_pressed
            .take()
            .filter(|old_state| {
                old_state.button_pressed == button
                    && now.duration_since(old_state.pressed_at) <= MULTI_CLICK_INTERVAL
            })
            .map(|old_state| old_state.click_count + 1)
            .unwrap_or(1);
        let new_state = MouseButtonPressState {
            pressed_at: now,
            button_pressed: button,
            click_count,
        };
        self.last_mouse_button_pressed = Some(new_state);
        click_count
    }
}

/// An extension trait to add helpful methods to [`winit::dpi::LogicalPosition`].
trait LogicalPositionExt {
    /// Converts the [`LogicalPosition`] into a [`Vector2F`].
    fn to_vec2f(&self) -> Vector2F;
}

impl LogicalPositionExt for winit::dpi::LogicalPosition<f32> {
    fn to_vec2f(&self) -> Vector2F {
        Vector2F::new(self.x, self.y)
    }
}

/// The state that we need to track across frames in order to properly convert
/// winit events into warpui events.
#[derive(Default)]
struct State {
    windows: HashMap<winit::window::WindowId, WindowState>,
    pending_active_window_change: Option<ActiveWindowChange>,

    #[cfg(windows)]
    network_connection_listener: Option<WindowsNetworkConnectionPoint>,
}

/// This enum holds the state needed to convert multiple emitted
/// [`winit::event::WindowEvent::Focused`] events into a single
/// [`CustomEvent::ActiveWindowChanged`].
#[derive(Copy, Clone, Debug)]
enum ActiveWindowChange {
    /// When we see `WindowEvent::Focused(false)` from winit, we store this variant.
    FocusOut,
    /// When we see `WindowEvent::Focused(true)` from winit, we store this variant and save the ID
    /// of the newly focused window.
    FocusIn(winit::window::WindowId),
}

fn from_winit_modifiers_state(state: keyboard::ModifiersState) -> ModifiersState {
    ModifiersState {
        alt: state.alt_key(),
        cmd: state.super_key(),
        shift: state.shift_key(),
        ctrl: state.control_key(),
        // TODO(advait): Implement the function key for winit.
        // Note there is no function_key() function to use here.
        func: false,
    }
}

/// Handles the `TouchPhase::Started` phase of a touch event.
///
/// Emits a `LeftMouseDown` event to begin a potential tap, selection, scroll, or window drag.
fn convert_touch_started(
    touch: Touch,
    window_state: &mut WindowState,
    scale_factor: f32,
) -> Option<ConvertedEvent> {
    if window_state.last_touch_purpose.is_some() {
        return None;
    }

    // Cancel any ongoing momentum scroll when user touches screen (iOS behavior).
    window_state.cancel_momentum_scroll();

    window_state.last_cursor_position = touch.location.to_logical(scale_factor as f64);
    let click_count = window_state.determine_click_count_and_update_button_state(MouseButton::Left);
    window_state.current_mouse_button_pressed = None;

    // Store click_count and start time for double-tap and long-press detection.
    window_state.last_touch_purpose = Some(TouchPurpose::Tap(touch, click_count, Instant::now()));

    Some(ConvertedEvent::Event(crate::event::Event::LeftMouseDown {
        position: window_state.last_cursor_position.to_vec2f(),
        click_count,
        is_first_mouse: false,
        modifiers: from_winit_modifiers_state(window_state.modifiers),
    }))
}

/// Handles the `TouchPhase::Moved` phase of a touch event.
fn convert_touch_moved(
    touch: Touch,
    window_state: &mut WindowState,
    scale_factor: f32,
    titlebar_height: f32,
) -> Option<ConvertedEvent> {
    match window_state.last_touch_purpose {
        Some(TouchPurpose::Tap(last_touch, click_count, start_time)) => {
            // Compute deltas in logical pixels for consistent behavior across DPI settings.
            let current_logical: LogicalPosition<f64> =
                touch.location.to_logical(scale_factor as f64);
            let last_logical: LogicalPosition<f64> =
                last_touch.location.to_logical(scale_factor as f64);
            let delta_x = current_logical.x - last_logical.x;
            let delta_y = current_logical.y - last_logical.y;

            // Not moved enough to classify gesture yet
            if delta_x.abs() <= MAX_TAP_DISTANCE && delta_y.abs() <= MAX_TAP_DISTANCE {
                return None;
            }

            window_state.last_cursor_position = touch.location.to_logical(scale_factor as f64);

            // Double-tap + drag = text selection
            if click_count >= 2 {
                window_state.last_touch_purpose = Some(TouchPurpose::Select);
                return Some(ConvertedEvent::Event(
                    crate::event::Event::LeftMouseDragged {
                        position: window_state.last_cursor_position.to_vec2f(),
                        modifiers: from_winit_modifiers_state(window_state.modifiers),
                    },
                ));
            }

            // Touch in titlebar = window drag
            let initial_pos: winit::dpi::LogicalPosition<f32> =
                last_touch.location.to_logical(scale_factor as f64);
            if initial_pos.y < titlebar_height && !cfg!(target_family = "wasm") {
                let start_touch = last_touch.location;
                window_state.last_touch_purpose = Some(TouchPurpose::WindowDrag { start_touch });
                return Some(ConvertedEvent::MoveWindowBy {
                    current_touch: touch.location,
                    start_touch,
                });
            }

            // Single tap + swipe = scroll (default)
            window_state.last_touch_purpose = Some(TouchPurpose::Scroll(last_touch));
            let elapsed = start_time
                .elapsed()
                .as_secs_f32()
                .max(MIN_VELOCITY_TIME_DELTA);
            window_state.scroll_velocity = Some(ScrollVelocity {
                velocity: Vector2F::new(delta_x as f32, delta_y as f32) / elapsed,
                last_update: Instant::now(),
            });
            Some(ConvertedEvent::Event(crate::event::Event::ScrollWheel {
                position: window_state.last_cursor_position.to_vec2f(),
                delta: Vector2F::new(delta_x as f32, delta_y as f32),
                precise: true,
                modifiers: from_winit_modifiers_state(window_state.modifiers),
            }))
        }
        Some(TouchPurpose::Scroll(last_touch)) => {
            // Continue scrolling. Use logical pixels for consistent scroll speed across DPI.
            window_state.last_touch_purpose = Some(TouchPurpose::Scroll(touch));
            let current_logical: LogicalPosition<f64> =
                touch.location.to_logical(scale_factor as f64);
            let last_logical: LogicalPosition<f64> =
                last_touch.location.to_logical(scale_factor as f64);
            let delta_x = current_logical.x - last_logical.x;
            let delta_y = current_logical.y - last_logical.y;
            window_state.last_cursor_position = current_logical.cast();

            // Update velocity for momentum scrolling.
            let now = Instant::now();
            let delta = Vector2F::new(delta_x as f32, delta_y as f32);
            let time_delta = window_state
                .scroll_velocity
                .map(|v| now.duration_since(v.last_update).as_secs_f32())
                .unwrap_or(MOMENTUM_DECAY_INTERVAL)
                .max(MIN_VELOCITY_TIME_DELTA);
            window_state.scroll_velocity = Some(ScrollVelocity {
                velocity: delta / time_delta,
                last_update: now,
            });

            Some(ConvertedEvent::Event(crate::event::Event::ScrollWheel {
                position: window_state.last_cursor_position.to_vec2f(),
                delta,
                precise: true,
                modifiers: from_winit_modifiers_state(window_state.modifiers),
            }))
        }
        Some(TouchPurpose::Select) => {
            // Continue selecting.
            window_state.last_cursor_position = touch.location.to_logical(scale_factor as f64);
            Some(ConvertedEvent::Event(
                crate::event::Event::LeftMouseDragged {
                    position: window_state.last_cursor_position.to_vec2f(),
                    modifiers: from_winit_modifiers_state(window_state.modifiers),
                },
            ))
        }
        Some(TouchPurpose::WindowDrag { start_touch }) => Some(ConvertedEvent::MoveWindowBy {
            current_touch: touch.location,
            start_touch,
        }),
        None => None,
    }
}

/// Handles the `TouchPhase::Ended` phase of a touch event.
///
/// Note: This function intentionally does NOT clear `last_touch_purpose` for normal taps.
/// The purpose is cleared later in `handle_converted_warpui_event` after soft keyboard
/// logic runs, which needs to check if the touch was still a Tap (vs Scroll/Select/WindowDrag).
fn convert_touch_ended(
    touch: Touch,
    window_state: &mut WindowState,
    scale_factor: f32,
) -> Option<ConvertedEvent> {
    // Check the purpose but don't clear it yet - we'll clear it later
    // in handle_converted_warpui_event after checking if we need to
    // update the soft keyboard.
    let is_long_press =
        if let Some(TouchPurpose::Tap(_, _, start_time)) = &window_state.last_touch_purpose {
            start_time.elapsed() >= LONG_PRESS_DURATION
        } else {
            false
        };

    let is_window_drag = matches!(
        &window_state.last_touch_purpose,
        Some(TouchPurpose::WindowDrag { .. })
    );

    window_state.last_cursor_position = touch.location.to_logical(scale_factor as f64);
    window_state.current_mouse_button_pressed = None;

    // Long press: still in Tap state and held longer than LONG_PRESS_DURATION
    if is_long_press {
        // Clear the purpose here since we're returning early
        window_state.last_touch_purpose = None;
        return Some(ConvertedEvent::Event(crate::event::Event::RightMouseDown {
            position: window_state.last_cursor_position.to_vec2f(),
            cmd: window_state.modifiers.super_key(),
            shift: window_state.modifiers.shift_key(),
            click_count: 1,
        }));
    }

    // WindowDrag doesn't need a mouse up event
    if is_window_drag {
        // Clear the purpose here since we're returning early
        window_state.last_touch_purpose = None;
        return None;
    }

    // Don't clear last_touch_purpose yet - it will be cleared in
    // handle_converted_warpui_event after soft keyboard logic runs.
    Some(ConvertedEvent::Event(crate::event::Event::LeftMouseUp {
        position: window_state.last_cursor_position.to_vec2f(),
        modifiers: from_winit_modifiers_state(window_state.modifiers),
    }))
}

/// Handles the `TouchPhase::Cancelled` phase of a touch event.
///
/// Cancelled touches only clean up state without triggering any action events.
fn convert_touch_cancelled(
    touch: Touch,
    window_state: &mut WindowState,
    scale_factor: f32,
) -> Option<ConvertedEvent> {
    window_state.last_touch_purpose.take();
    window_state.last_cursor_position = touch.location.to_logical(scale_factor as f64);
    window_state.current_mouse_button_pressed = None;
    None
}

/// A structure to manage state
/// [`winit::event_loop::EventLoop`] and generate the appropriate callbacks into
/// the UI framework.
pub(super) struct EventLoop {
    ui_app: crate::App,
    callbacks: AppCallbackDispatcher,
    init_fn: Option<platform::app::AppInitCallbackFn>,
    window_class: Option<String>,
    state: State,
    proxy: EventLoopProxy<CustomEvent>,
    ime_enabled: bool,
    /// Whether to downrank non-NVIDIA vulkan adapters. This is set to true when we detect a DRI3
    /// error that occurs when trying to present against a non-NVIDIA Vulkan adapter when the
    /// PRIME Profile is set to "Performance" mode.  It's not fully clear why this error occurs. Our
    /// theory is that when the PRIME  performance profile is enabled (which indicates to NVIDIA
    /// Optimus that the machine should _only_ render to the NVIDIA GPU), Optimus determines that
    /// the Integrated GPU won't be rendered to and puts it in an idle / partially loaded state that
    /// will eventually trigger these DRI3 `BadMatch` errors when we attempt to render to it.
    downrank_non_nvidia_vulkan_adapters: bool,
    /// Soft keyboard manager for mobile WASM.
    #[cfg(target_family = "wasm")]
    soft_keyboard_manager: Option<std::rc::Rc<crate::platform::wasm::SoftKeyboardManager>>,
}

impl EventLoop {
    pub fn new(
        ui_app: crate::App,
        callbacks: platform::AppCallbacks,
        init_fn: impl FnOnce(&mut AppContext, LocalBoxFuture<'static, crate::App>) + 'static,
        window_class: Option<String>,
        proxy: EventLoopProxy<CustomEvent>,
    ) -> Self {
        Self {
            ui_app: ui_app.clone(),
            callbacks: AppCallbackDispatcher::new(callbacks, ui_app),
            init_fn: Some(Box::new(init_fn)),
            window_class,
            state: Default::default(),
            proxy,
            ime_enabled: false,
            downrank_non_nvidia_vulkan_adapters: false,
            #[cfg(target_family = "wasm")]
            soft_keyboard_manager: None,
        }
    }

    /// Handles a single [`winit::event::Event`].
    pub fn handle_event(&mut self, evt: Event<CustomEvent>, window_target: &ActiveEventLoop) {
        window_target.set_control_flow(ControlFlow::Wait);

        match evt {
            Event::NewEvents(StartCause::Init) => {
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                {
                    let windowing_system =
                        if winit::platform::x11::ActiveEventLoopExtX11::is_x11(window_target) {
                            crate::windowing::WindowingSystem::X11
                        } else {
                            crate::windowing::WindowingSystem::Wayland
                        };
                    log::info!("Running app with windowing system: {windowing_system:?}");
                    if let Err(err) = super::app::WINDOWING_SYSTEM.set(windowing_system) {
                        log::warn!("Could not set global static for windowing system: {err:?}");
                    }
                }

                if let Some(init_fn) = self.init_fn.take() {
                    self.callbacks.initialize_app(init_fn);
                }

                // Start listening for various platform events.
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                {
                    super::linux::watch_suspend_resume_changes(
                        self.proxy.clone(),
                        &self.ui_app.background_executor(),
                    );
                    super::linux::watch_desktop_settings_changes(
                        self.proxy.clone(),
                        &self.ui_app.background_executor(),
                    );
                    if self.callbacks.has_internet_reachability_changed_callback() {
                        super::linux::watch_network_status_changed(
                            self.proxy.clone(),
                            &self.ui_app.background_executor(),
                        );
                    }
                }

                #[cfg(windows)]
                match add_network_connection_listener(self.proxy.clone()) {
                    Ok(listener) => {
                        self.state.network_connection_listener = Some(listener);
                    }
                    Err(e) => {
                        log::warn!("Creating a network connection listener failed: {e:?}");
                    }
                }

                // Initialize soft keyboard support on mobile WASM devices.
                #[cfg(target_family = "wasm")]
                {
                    self.initialize_soft_keyboard();
                }
            }
            Event::UserEvent(CustomEvent::OpenWindow {
                window_id,
                window_options,
            }) => {
                let Some((window, is_tiling_window_manager)) = self.ui_app.update(|ctx| {
                    let window = ctx.windows().platform_window(window_id)?;
                    let is_tiling_window_manager = ctx.windows().is_tiling_window_manager();
                    Some((window, is_tiling_window_manager))
                }) else {
                    return;
                };

                let window = downcast_window(window.as_ref());

                match window.open_window(
                    window_target,
                    window_options,
                    &self.window_class,
                    is_tiling_window_manager,
                    self.downrank_non_nvidia_vulkan_adapters,
                ) {
                    Ok(winit_window_id) => {
                        let window_state = WindowState::new(window_id);
                        self.state.windows.insert(winit_window_id, window_state);
                        // Now that the window has opened and we know its
                        // actual size, notify the framework that the window
                        // size may have (almost certainly) changed.
                        self.callbacks.for_window(window).window_resized(window);
                    }
                    Err(err) => {
                        log::error!("Failed to open window: {err:#}");
                        // Tell the app that the window is "closing".
                        self.callbacks.window_will_close(window_id);
                    }
                }
            }
            Event::UserEvent(CustomEvent::RunTask(task)) => {
                let task = ManuallyDrop::into_inner(task);
                task.run();
            }
            Event::UserEvent(CustomEvent::Terminate(termination_mode)) => {
                if let ApproveTerminateResult::Terminate =
                    self.terminate_app_requested(termination_mode)
                {
                    window_target.exit();
                }
            }
            Event::UserEvent(CustomEvent::UpdateUIApp(callback)) => {
                self.ui_app.update(callback);
            }
            Event::UserEvent(CustomEvent::GlobalShortcutTriggered(shortcut)) => {
                self.callbacks.global_shortcut_triggered(shortcut)
            }
            Event::UserEvent(CustomEvent::CloseWindow {
                window_id,
                termination_mode,
            }) => {
                if let Some(winit_window_id) = self
                    .state
                    .windows
                    .iter()
                    .find(|(_, v)| v.window_id == window_id)
                    .map(|(k, _)| k)
                    .cloned()
                {
                    self.close_window_requested(
                        window_id,
                        winit_window_id,
                        termination_mode,
                        window_target,
                    )
                }
            }
            Event::UserEvent(CustomEvent::ActiveWindowChanged) => {
                let app_was_active = self
                    .ui_app
                    .read(|ctx| ctx.windows().active_window())
                    .is_some();

                let active_window_id = match self.state.pending_active_window_change.take() {
                    None => return,
                    Some(ActiveWindowChange::FocusOut) => None,
                    Some(ActiveWindowChange::FocusIn(window_id)) => Some(window_id),
                };
                let active_window_id = active_window_id
                    .and_then(|window_id| self.state.windows.get(&window_id))
                    .map(|state| state.window_id);
                self.callbacks.active_window_changed(active_window_id);

                // If the application became active or inactive, invoke the appropriate callback.
                let app_is_active = active_window_id.is_some();
                match (app_was_active, app_is_active) {
                    (false, true) => self.callbacks.app_became_active(),
                    (true, false) => self.callbacks.app_resigned_active(),
                    _ => {}
                };
            }
            Event::UserEvent(CustomEvent::RequestUserAttention { window_id }) => {
                self.ui_app.update(|ctx| {
                    if ctx.windows().active_window() == Some(window_id) {
                        // The current window is already active, early return since requesting user attention would be
                        // a noop.
                        return;
                    }

                    let Some(window) = ctx.windows().platform_window(window_id) else {
                        return;
                    };

                    let window = downcast_window(window.as_ref());
                    window.request_user_attention();

                    // To mimic the behavior on Mac, we only request user attention for 1 second before then stopping
                    // the request. This is especially needed on x11 since the app icon will bounce in perpetuity until
                    // is explicitly told to stop.
                    let event_loop_proxy = self.proxy.clone();
                    ctx.foreground_executor()
                        .spawn(async move {
                            Timer::after(Duration::from_secs(1)).await;
                            let _ = event_loop_proxy
                                .send_event(CustomEvent::StopRequestingUserAttention { window_id });
                        })
                        .detach();
                });
            }
            Event::UserEvent(CustomEvent::StopRequestingUserAttention { window_id }) => {
                self.ui_app.update(|ctx| {
                    let Some(window) = ctx.windows().platform_window(window_id) else {
                        return;
                    };

                    let window = downcast_window(window.as_ref());
                    window.stop_requesting_user_attention();
                });
            }
            Event::UserEvent(CustomEvent::Clipboard(clipboard_event)) => {
                self.handle_clipboard_event(clipboard_event);
            }
            Event::UserEvent(CustomEvent::SetCursorShape(cursor)) => {
                self.ui_app.update(|ctx| {
                    let Some(window_id) = ctx.windows().active_window() else {
                        return;
                    };

                    let Some(window) = ctx.windows().platform_window(window_id) else {
                        return;
                    };

                    let winit_window = downcast_window(window.as_ref());
                    winit_window.set_cursor_icon(cursor);
                });
            }
            Event::UserEvent(CustomEvent::ActiveCursorPositionUpdated) => {
                if self.ime_enabled {
                    self.update_ime_position();
                }
            }
            Event::UserEvent(CustomEvent::AboutToSleep) => {
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                self.prepare_for_sleep_on_linux(window_target);

                self.callbacks.cpu_will_sleep();
            }
            Event::UserEvent(CustomEvent::ResumedFromSleep) => {
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                self.resume_from_sleep_on_linux();

                self.callbacks.cpu_awakened();
            }

            Event::UserEvent(CustomEvent::InternetConnected) => {
                self.callbacks.internet_reachability_changed(true);
            }
            Event::UserEvent(CustomEvent::InternetDisconnected) => {
                self.callbacks.internet_reachability_changed(false)
            }
            Event::UserEvent(CustomEvent::SystemThemeChanged) => {
                self.callbacks.os_appearance_changed();
            }
            Event::WindowEvent {
                window_id: _,
                event: WindowEvent::ThemeChanged(_new_theme),
            } => {
                self.callbacks.os_appearance_changed();
            }
            Event::UserEvent(CustomEvent::SendNotification {
                notification_info,
                window_id,
            }) => {
                self.send_notification(notification_info, window_id);
            }
            Event::UserEvent(CustomEvent::FocusWindow { window_id }) => {
                self.focus_window(window_id);
            }
            Event::UserEvent(CustomEvent::RequestNotificationPermissions(callback)) => {
                self.request_notification_permissions(callback);
            }
            Event::UserEvent(CustomEvent::DragAndDropFilesDebounced { window_id }) => {
                self.handle_debounced_drag_drop(window_id);
            }
            #[cfg(target_family = "wasm")]
            Event::UserEvent(CustomEvent::SoftKeyboardInput(input)) => {
                self.handle_soft_keyboard_input(input);
            }
            #[cfg(target_family = "wasm")]
            Event::UserEvent(CustomEvent::VisualViewportResized { width, height }) => {
                self.handle_visual_viewport_resize(width, height);
            }
            Event::UserEvent(CustomEvent::MomentumScroll { window_id }) => {
                let Some(window_state) = self.state.windows.get_mut(&window_id) else {
                    return;
                };
                let Some(mut velocity) = window_state.scroll_velocity else {
                    return;
                };

                let now = Instant::now();
                let elapsed = now.duration_since(velocity.last_update).as_secs_f32();
                velocity.last_update = now;

                // Apply time-based decay: v_new = v_old * decay^(Δt/interval)
                let decay_factor = MOMENTUM_DECAY.powf(elapsed / MOMENTUM_DECAY_INTERVAL);
                velocity.velocity *= decay_factor;

                if velocity.velocity.length() < MOMENTUM_MIN_VELOCITY {
                    window_state.cancel_momentum_scroll();
                    return;
                }

                window_state.scroll_velocity = Some(velocity);

                // Convert velocity (px/sec) to scroll delta using elapsed time.
                let delta = velocity.velocity * elapsed;
                let position = window_state.last_cursor_position.to_vec2f();
                self.handle_converted_warpui_event(
                    window_id,
                    crate::event::Event::ScrollWheel {
                        position,
                        delta,
                        precise: true,
                        modifiers: ModifiersState::default(),
                    },
                );
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::RedrawRequested,
            } => self.redraw_window(window_id, window_target),
            Event::WindowEvent {
                window_id: winit_window_id,
                event: WindowEvent::CloseRequested,
            } => {
                if let Some(state) = self.state.windows.get(&winit_window_id) {
                    self.close_window_requested(
                        state.window_id,
                        winit_window_id,
                        TerminationMode::Cancellable,
                        window_target,
                    );
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Destroyed,
                ..
            } => {
                // TODO(vorporeal): Should we be calling approve_termination() here?
                // i.e.: should we invoke a helper shared with CustomEvent::Terminate?
                if cfg!(not(target_os = "macos")) && self.state.windows.is_empty() {
                    window_target.exit();
                }

                // When a window loses focus, winit will emit [`WindowEvent::Focused(false)`].
                // However, that doesn't fire when a window is closed. So, we trigger that code path
                // from here to make sure the app knows to update its active window.
                self.state.pending_active_window_change = Some(ActiveWindowChange::FocusOut);
                let _ = self.proxy.send_event(CustomEvent::ActiveWindowChanged);
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::Ime(evt),
            } => {
                self.handle_ime_event(window_id, evt);
            }
            Event::WindowEvent {
                window_id,
                event:
                    WindowEvent::ScaleFactorChanged {
                        scale_factor,
                        mut inner_size_writer,
                    },
            } => {
                // The following correction is only needed on Windows.
                if !cfg!(windows) {
                    return;
                }
                let Some(window_state) = self.state.windows.get(&window_id) else {
                    return;
                };
                let Some(window) = self
                    .ui_app
                    .read(|ctx| ctx.windows().platform_window(window_state.window_id))
                else {
                    return;
                };

                // There is a winit bug such that events which cause a window to switch displays to
                // one with a different scale factor resize the Warp window to an absurdly small
                // size, <157, 25> on my system when I repro it. Events include unplugging a
                // display, changing a display from extended to mirrored, and the like. We work
                // around that by listening for [`WindowEvent::ScaleFactorChanged`] and changing
                // the size back up to the minimum dimensions.
                let size = window.as_ctx().size();
                let mut size = LogicalSize::new(size.x() as f64, size.y() as f64);
                let mut request_new_size = false;
                if size.width < MIN_WINDOW_SIZE.width {
                    size.width = MIN_WINDOW_SIZE.width;
                    request_new_size = true;
                }
                if size.height < MIN_WINDOW_SIZE.height {
                    size.height = MIN_WINDOW_SIZE.height;
                    request_new_size = true;
                }
                if request_new_size {
                    if let Err(err) =
                        inner_size_writer.request_inner_size(size.to_physical(scale_factor))
                    {
                        log::warn!("unable to correct window size: {err:#}");
                    }
                }
            }
            Event::WindowEvent { window_id, event } => self.handle_window_event(window_id, event),
            Event::LoopExiting => {
                // Hide all open windows such that, if the application takes a
                // second or two to clean up before exiting, this isn't visible
                // to the end user.
                self.ui_app.update(|ctx| {
                    use crate::SingletonEntity as _;
                    crate::windowing::WindowManager::handle(ctx).update(
                        ctx,
                        |window_manager, _| {
                            window_manager.hide_app();
                        },
                    );
                });

                #[cfg(windows)]
                if let Some(network_listener) = self.state.network_connection_listener.take() {
                    network_listener.clean_up();
                }

                self.callbacks.app_will_terminate();

                // On non-web platforms, immediately terminate the process instead of returning
                // from the event loop.  This matches the behavior of
                // `[NSApp terminate]` on macOS, and may avoid some at-exit
                // crashes that produce noise in our crash reporting data.
                // On web, it's not possible to exit cleanly, so just return from the event loop
                // instead.
                #[cfg(not(target_family = "wasm"))]
                std::process::exit(0);
            }

            _ => {}
        }
    }

    fn redraw_window(
        &mut self,
        window_id: winit::window::WindowId,
        window_target: &ActiveEventLoop,
    ) {
        let Some(window_id) = self
            .state
            .windows
            .get(&window_id)
            .map(|state| state.window_id)
        else {
            log::warn!("Redraw requested for a window for which we have no state");
            return;
        };
        let Some(window) = self
            .ui_app
            .read(|ctx| ctx.windows().platform_window(window_id))
        else {
            log::warn!("Unable to retrieve platform window from app");
            return;
        };

        let window = downcast_window(window.as_ref());

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        if crate::windowing::winit::linux::take_encountered_bad_match_from_dri3_fence_from_fd() {
            log::warn!("Encountered a DRI3FenceFromFd error, forcing use of the NVIDIA GPU and recreating resources...");
            self.downrank_non_nvidia_vulkan_adapters = true;

            self.ui_app.update(|ctx| {
                for window_id in ctx.window_ids() {
                    let Some(window) = ctx.windows().platform_window(window_id) else {
                        return;
                    };

                    let winit_window = downcast_window(window.as_ref());
                    winit_window.recreate_renderer(self.downrank_non_nvidia_vulkan_adapters);
                }
            })
        }

        let render_result = (|| {
            // Before building the scene, make sure the window size is up-to-date, to ensure
            // that the scene is built at a size that matches the size we're about to render at.
            window.update_size_if_needed()?;

            let new_scene = if !window.has_scene() {
                Some(self.callbacks.for_window(window).build_scene(window))
            } else {
                None
            };

            self.callbacks
                .with_mutable_app_context(|ctx| window.render(new_scene, ctx.font_cache()))
        })();

        match render_result {
            Ok(_) => self.callbacks.for_window(window).frame_drawn(),
            Err(err) => {
                log::warn!("Failed to render frame: {err:#}");
                self.callbacks.for_window(window).frame_failed_to_draw();

                match err {
                    // If we failed to configure the surface, or...
                    renderer::Error::SurfaceConfigureError { .. }
                    // If the device was lost, or...
                    | renderer::Error::SurfaceError(renderer::GetSurfaceTextureError::Lost)
                    | renderer::Error::DeviceLost
                    // If we ran into any other wgpu error -
                    | renderer::Error::Unknown(_)=> {
                        log::warn!("Recreating the renderer in an attempt to recover...");
                        window.drop_renderer(Box::new(window_target.owned_display_handle()));
                        window.recreate_renderer(self.downrank_non_nvidia_vulkan_adapters);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Handles a [`winit::event::WindowEvent`].
    fn handle_window_event(&mut self, window_id: winit::window::WindowId, evt: WindowEvent) {
        let Some(event) = self.convert_window_event(window_id, evt) else {
            return;
        };
        let Some(window_state) = self.state.windows.get_mut(&window_id) else {
            return;
        };
        let Some(window) = self
            .ui_app
            .read(|ctx| ctx.windows().platform_window(window_state.window_id))
        else {
            return;
        };

        match event {
            ConvertedEvent::Event(event) => {
                self.handle_converted_warpui_event(window_id, event);
            }
            ConvertedEvent::Resize => {
                let window = downcast_window(window.as_ref());
                window.handle_resize();
                self.callbacks.for_window(window).window_resized(window);
                self.callbacks.window_resized();
            }
            ConvertedEvent::ModifierKeyChanged { key_code, state } => {
                let mut window_callbacks = self.callbacks.for_window(window.as_ref());
                window_callbacks.dispatch_event(crate::event::Event::ModifierKeyChanged {
                    key_code,
                    state: match state {
                        ElementState::Pressed => crate::event::KeyState::Pressed,
                        ElementState::Released => crate::event::KeyState::Released,
                    },
                });
            }
            ConvertedEvent::KeyDownWithTypedCharacters { chars, event } => {
                // To match the behavior of macOS: first try to dispatch the underlying keydown
                // event. If it was not handled (and doesn't include the cmd modifier), send a
                // `TypedCharacters` event. (We don't send `TypedCharacters` events for keypresess
                // that include the cmd key because they are assumed to be
                // intended as OS-level or application-level shortcuts. This matches the behavior
                // on macOS.)
                let cmd_pressed = match &event {
                    crate::event::Event::KeyDown { keystroke, .. } => keystroke.cmd,
                    _ => false,
                };

                let mut window_callbacks = self.callbacks.for_window(window.as_ref());
                let result = window_callbacks.dispatch_event(event);
                if !result.handled && !cmd_pressed {
                    if let Some(chars) = chars {
                        window_callbacks.dispatch_event(TypedCharacters { chars });
                    }
                }
            }
            ConvertedEvent::WindowMoved { new_position } => {
                let window = downcast_window(window.as_ref());
                let scale_factor = self.ui_app.update(|ctx| {
                    ctx.windows()
                        .platform_window(window_state.window_id)
                        .expect("window should exist")
                        .backing_scale_factor()
                });
                let position = new_position.to_logical(scale_factor.into());
                let size = window.size();
                self.callbacks
                    .for_window(window)
                    .window_moved(RectF::new(vec2f(position.x, position.y), size));
                self.callbacks.window_moved();
            }
            ConvertedEvent::MoveWindowBy {
                current_touch,
                start_touch,
            } => {
                let winit_window = downcast_window(window.as_ref());
                if let Some(current_window) = winit_window.outer_position() {
                    // target = current_window + (current_touch - start_touch)
                    let target_x =
                        (current_window.x as f64 + current_touch.x - start_touch.x) as i32;
                    let target_y =
                        (current_window.y as f64 + current_touch.y - start_touch.y) as i32;
                    winit_window.set_outer_position(PhysicalPosition::new(target_x, target_y));
                }
            }
        }
    }

    /// Converts a [`winit::event::WindowEvent`] into a [`ConvertedEvent`], returning [`None`] if
    /// there is no equivalent/the event should be ignored.
    fn convert_window_event(
        &mut self,
        window_id: winit::window::WindowId,
        evt: winit::event::WindowEvent,
    ) -> Option<ConvertedEvent> {
        let window_state = self.state.windows.get_mut(&window_id)?;
        let scale_factor = self.ui_app.update(|ctx| {
            ctx.windows()
                .platform_window(window_state.window_id)
                .expect("window should exist")
                .backing_scale_factor()
        });
        match evt {
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                window_state.modifiers = state;
                // If Alt is no longer held at all, clear both per-side flags as a safety net
                // in case a key-release event was dropped (e.g. released while the window was
                // unfocused).
                if !state.alt_key() {
                    window_state.left_alt_pressed = false;
                    window_state.right_alt_pressed = false;
                }
                Some(ConvertedEvent::Event(
                    crate::event::Event::ModifierStateChanged {
                        mouse_position: window_state.last_cursor_position.to_vec2f(),
                        modifiers: from_winit_modifiers_state(state),
                        // TODO: when we need key codes for voice input on Linux/Windows, we'll need to populate this!
                        key_code: None,
                    },
                ))
            }
            WindowEvent::CursorMoved { position, .. } => {
                window_state.last_cursor_position = position.to_logical(scale_factor as f64);
                match window_state.current_mouse_button_pressed {
                    Some(MouseButton::Left) => Some(ConvertedEvent::Event(
                        crate::event::Event::LeftMouseDragged {
                            position: window_state.last_cursor_position.to_vec2f(),
                            modifiers: from_winit_modifiers_state(window_state.modifiers),
                        },
                    )),
                    _ => Some(ConvertedEvent::Event(crate::event::Event::MouseMoved {
                        position: window_state.last_cursor_position.to_vec2f(),
                        cmd: window_state.modifiers.super_key(),
                        shift: window_state.modifiers.shift_key(),
                        is_synthetic: false,
                    })),
                }
            }
            WindowEvent::MouseInput { state, button, .. } => match state {
                ElementState::Pressed => {
                    let click_count =
                        window_state.determine_click_count_and_update_button_state(button);
                    match button {
                        MouseButton::Left => {
                            // ctrl-click should actually be registered as a right-click on mac
                            let ctrl_click = window_state.modifiers.control_key();
                            if ctrl_click && OperatingSystem::get().is_mac() {
                                Some(ConvertedEvent::Event(crate::event::Event::RightMouseDown {
                                    position: window_state.last_cursor_position.to_vec2f(),
                                    cmd: window_state.modifiers.super_key(),
                                    shift: window_state.modifiers.shift_key(),
                                    click_count,
                                }))
                            } else {
                                Some(ConvertedEvent::Event(crate::event::Event::LeftMouseDown {
                                    position: window_state.last_cursor_position.to_vec2f(),
                                    click_count,
                                    is_first_mouse: false,
                                    modifiers: from_winit_modifiers_state(window_state.modifiers),
                                }))
                            }
                        }
                        MouseButton::Right => {
                            Some(ConvertedEvent::Event(crate::event::Event::RightMouseDown {
                                position: window_state.last_cursor_position.to_vec2f(),
                                cmd: window_state.modifiers.super_key(),
                                shift: window_state.modifiers.shift_key(),
                                click_count,
                            }))
                        }
                        MouseButton::Middle => Some(ConvertedEvent::Event(
                            crate::event::Event::MiddleMouseDown {
                                position: window_state.last_cursor_position.to_vec2f(),
                                cmd: window_state.modifiers.super_key(),
                                shift: window_state.modifiers.shift_key(),
                                click_count,
                            },
                        )),
                        _ => None,
                    }
                }
                ElementState::Released => {
                    window_state.current_mouse_button_pressed = None;
                    match button {
                        MouseButton::Left => {
                            Some(ConvertedEvent::Event(crate::event::Event::LeftMouseUp {
                                position: window_state.last_cursor_position.to_vec2f(),
                                modifiers: from_winit_modifiers_state(window_state.modifiers),
                            }))
                        }
                        _ => None,
                    }
                }
            },
            // Handle and convert touch events into mouse events.
            WindowEvent::Touch(touch) => match touch.phase {
                TouchPhase::Started => convert_touch_started(touch, window_state, scale_factor),
                TouchPhase::Moved => {
                    let titlebar_height = self
                        .ui_app
                        .read(|ctx| ctx.windows().platform_window(window_state.window_id))
                        .map(|w| downcast_window(w.as_ref()).titlebar_height())
                        .unwrap_or(DEFAULT_TITLEBAR_HEIGHT);
                    convert_touch_moved(touch, window_state, scale_factor, titlebar_height)
                }
                TouchPhase::Ended => convert_touch_ended(touch, window_state, scale_factor),
                TouchPhase::Cancelled => convert_touch_cancelled(touch, window_state, scale_factor),
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let (precise, delta) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(horiz, vert) => {
                        (false, Vector2F::new(horiz, vert))
                    }
                    winit::event::MouseScrollDelta::PixelDelta(px) => {
                        (true, px.to_logical(scale_factor as f64).to_vec2f())
                    }
                };
                Some(ConvertedEvent::Event(crate::event::Event::ScrollWheel {
                    position: window_state.last_cursor_position.to_vec2f(),
                    delta,
                    precise,
                    modifiers: from_winit_modifiers_state(window_state.modifiers),
                }))
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                // Track per-side Alt press state so that the extra-meta-keys setting can
                // distinguish between left Alt and right Alt. `ModifiersState` alone does
                // not expose which side of a modifier was pressed.
                if let keyboard::PhysicalKey::Code(keycode) = &event.physical_key {
                    let is_pressed = event.state == ElementState::Pressed;
                    match keycode {
                        KeyCode::AltLeft => window_state.left_alt_pressed = is_pressed,
                        KeyCode::AltRight => window_state.right_alt_pressed = is_pressed,
                        _ => {}
                    }
                }

                // If the event is a modifier key, just by itself, we handle it specially, issuing
                // the appropriate Warp-side event (ModifierKeyChanged).
                if let (None, keyboard::PhysicalKey::Code(keycode)) =
                    (&event.text, &event.physical_key)
                {
                    if let Ok(mapped_keycode) = try_from_winit_keycode(keycode) {
                        return Some(ConvertedEvent::ModifierKeyChanged {
                            key_code: mapped_keycode,
                            state: event.state,
                        });
                    }
                }

                let event_text = event.text.as_ref().map(|text| text.to_string());
                let event_state = event.state;
                let is_unidentified_key =
                    matches!(event.logical_key, keyboard::Key::Unidentified(_));
                match convert_keyboard_input_event(event, window_state, is_synthetic) {
                    Some(warp_ui_event) => Some(ConvertedEvent::KeyDownWithTypedCharacters {
                        chars: event_text,
                        event: warp_ui_event,
                    }),
                    None if is_unidentified_key
                        && !is_synthetic
                        && event_state == ElementState::Pressed =>
                    {
                        // Fallback for synthetic WM_CHAR messages injected by non-IME input methods
                        // (e.g. Unikey/EVKey on Windows for Vietnamese Telex/VNI). These input
                        // methods hook the keyboard at a low level and inject pre-composed
                        // characters via `SendInput` instead of going through the standard IME
                        // pipeline. The resulting key event has
                        // `logical_key == Key::Unidentified(...)`
                        event_text.map(|chars| ConvertedEvent::Event(TypedCharacters { chars }))
                    }
                    None => None,
                }
            }
            WindowEvent::Resized(_) => Some(ConvertedEvent::Resize),
            WindowEvent::Focused(is_focused) => {
                // On mobile WASM, ignore focus-out events. The soft keyboard's hidden input
                // causes spurious focus events, and mobile doesn't have the concept of
                // "unfocused windows" anyway - you're either in the app or switched away entirely.
                #[cfg(target_family = "wasm")]
                if !is_focused && crate::platform::wasm::is_mobile_device() {
                    return None;
                }

                // Clear tracked per-side Alt state when we lose focus so that a release
                // event dropped while another window had focus can't leave us believing a
                // side is still held.
                if !is_focused {
                    window_state.left_alt_pressed = false;
                    window_state.right_alt_pressed = false;
                }

                // On the next tick of the event loop, notify the ui_app that focus has
                // transferred, but only if there isn't already one of these
                // [`CustomEvent::ActiveWindowChanged`] pending. This coalesces multiple
                // `WindowEvent::Focused` events into a single CustomEvent.
                if self.state.pending_active_window_change.is_none() {
                    let _ = self.proxy.send_event(CustomEvent::ActiveWindowChanged);
                }
                if is_focused {
                    self.state.pending_active_window_change =
                        Some(ActiveWindowChange::FocusIn(window_id));
                } else {
                    self.state.pending_active_window_change = Some(ActiveWindowChange::FocusOut);
                }
                None
            }
            WindowEvent::DroppedFile(path_buf) => {
                let Some(path) = path_buf.as_os_str().to_str() else {
                    log::warn!("Failed to convert dropped file path to UTF-8: {path_buf:?}");
                    return None;
                };

                // Add this file to the pending list
                window_state.pending_drag_drop_files.push(path.to_string());

                // Only schedule a debounced event if we don't already have one running
                if !window_state.has_pending_drag_drop_timer {
                    window_state.has_pending_drag_drop_timer = true;
                    let proxy = self.proxy.clone();
                    // Wait to collect all files before sending one event for all of them
                    self.ui_app.update(|ctx| {
                        ctx.foreground_executor()
                            .spawn(async move {
                                Timer::after(DRAG_DROP_DEBOUNCE_TIMEOUT).await;
                                let _ = proxy.send_event(CustomEvent::DragAndDropFilesDebounced {
                                    window_id,
                                });
                            })
                            .detach();
                    });
                }
                None // Use debounced event instead of immediate processing
            }
            WindowEvent::Moved(new_position) => Some(ConvertedEvent::WindowMoved { new_position }),
            _ => None,
        }
    }

    #[allow(unused_variables)]
    fn send_notification(&mut self, notification_info: NotificationInfo, window_id: WindowId) {
        let proxy = self.proxy.clone();
        self.ui_app.update(|ctx| {
            ctx.background_executor()
                .spawn(async move {
                    crate::windowing::winit::notifications::send_notification(
                        notification_info,
                        window_id,
                        proxy,
                    )
                    .await;
                })
                .detach()
        });
    }

    fn focus_window(&mut self, window_id: WindowId) {
        self.ui_app.update(|ctx| {
            let Some(window) = ctx.windows().platform_window(window_id) else {
                return;
            };
            let window = downcast_window(window.as_ref());
            window.focus();
        });
    }

    #[allow(unused_variables)]
    fn request_notification_permissions(&mut self, callback: RequestPermissionsCallback) {
        let proxy = self.proxy.clone();
        self.ui_app.update(|ctx| {
            ctx.background_executor()
                .spawn(async move {
                    #[cfg(target_family = "wasm")]
                    crate::windowing::winit::notifications::request_notification_permissions(
                        callback, proxy,
                    )
                    .await;

                    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                    {
                        // On Linux, there is no concept of requesting notification permissions. This
                        // logic is hard-coded to always return an outcome of "Accepted".
                        let _ = proxy.send_event(CustomEvent::UpdateUIApp(Box::new(|ctx| {
                            callback(RequestPermissionsOutcome::Accepted, ctx)
                        })));
                    }
                })
                .detach()
        });
    }

    /// Takes all pending dropped files and creates a single DragAndDropFiles event.
    fn handle_debounced_drag_drop(&mut self, window_id: winit::window::WindowId) {
        let Some(window_state) = self.state.windows.get_mut(&window_id) else {
            return;
        };

        window_state.has_pending_drag_drop_timer = false;

        if window_state.pending_drag_drop_files.is_empty() {
            return;
        }

        // Take ownership of accumulated files
        let paths = std::mem::take(&mut window_state.pending_drag_drop_files);

        let location = window_state.last_cursor_position.to_vec2f();

        // Create and dispatch the batched drag-and-drop event
        let drag_drop_event = crate::Event::DragAndDropFiles { paths, location };

        self.handle_converted_warpui_event(window_id, drag_drop_event);
    }

    /// Handles a request to close the window with the given warpui and winit
    /// IDs.
    fn close_window_requested(
        &mut self,
        window_id: crate::WindowId,
        winit_window_id: winit::window::WindowId,
        termination_mode: TerminationMode,
        window_target: &ActiveEventLoop,
    ) {
        if matches!(
            termination_mode,
            TerminationMode::ForceTerminate | TerminationMode::ContentTransferred
        ) {
            self.close_window(window_id, winit_window_id, window_target);
        } else if let ApproveTerminateResult::Terminate =
            self.callbacks.should_close_window(window_id)
        {
            self.close_window(window_id, winit_window_id, window_target);
        }
    }

    fn close_window(
        &mut self,
        window_id: crate::WindowId,
        winit_window_id: winit::window::WindowId,
        window_target: &ActiveEventLoop,
    ) {
        let window_state = self.state.windows.remove(&winit_window_id);

        // Drop the renderer before we actually clean up the window, to ensure
        // that the window outlives the `wgpu` surface that references it.
        if let Some(WindowState { window_id, .. }) = window_state {
            if let Some(window) = self
                .ui_app
                .read(|ctx| ctx.windows().platform_window(window_id))
            {
                downcast_window(window.as_ref())
                    .drop_renderer(Box::new(window_target.owned_display_handle()));
            }
        }

        self.callbacks.window_will_close(window_id)
    }

    fn terminate_app_requested(
        &mut self,
        termination_mode: TerminationMode,
    ) -> ApproveTerminateResult {
        if matches!(
            termination_mode,
            TerminationMode::ForceTerminate | TerminationMode::ContentTransferred
        ) {
            return ApproveTerminateResult::Terminate;
        }

        let approve_terminate_result = self.callbacks.should_terminate_app();
        if let ApproveTerminateResult::Terminate = approve_terminate_result {}
        approve_terminate_result
    }

    fn handle_ime_event(&mut self, winit_window_id: WinitWindowId, event: ImeEvent) {
        match event {
            winit::event::Ime::Enabled => {
                self.ime_enabled = true;
                self.ui_app
                    .update(|ctx| ctx.report_active_cursor_position_update());
            }
            winit::event::Ime::Preedit(preedit_text, cursor_position) => {
                if !self.ime_enabled {
                    return;
                }

                let Some(window_state) = self.state.windows.get_mut(&winit_window_id) else {
                    return;
                };
                let Some(window) = self
                    .ui_app
                    .read(|ctx| ctx.windows().platform_window(window_state.window_id))
                else {
                    return;
                };

                let mut window_callbacks = self.callbacks.for_window(window.as_ref());
                window_callbacks.dispatch_event(SetMarkedText {
                    marked_text: preedit_text,
                    selected_range: cursor_position
                        .map(|cursor_position| cursor_position.0..cursor_position.1)
                        .unwrap_or(0..0),
                });
            }
            winit::event::Ime::Commit(chars) => {
                let Some(window_state) = self.state.windows.get_mut(&winit_window_id) else {
                    return;
                };
                let Some(window) = self
                    .ui_app
                    .read(|ctx| ctx.windows().platform_window(window_state.window_id))
                else {
                    return;
                };

                let mut window_callbacks = self.callbacks.for_window(window.as_ref());
                // We clear the marked text state before inserting typed characters so that the Vim
                // FSA knows it can interpret the committed text as a user insertion.
                window_callbacks.dispatch_event(ClearMarkedText);
                window_callbacks.dispatch_event(TypedCharacters { chars });
            }
            winit::event::Ime::Disabled => {
                self.ime_enabled = false;
            }
        };
    }

    /// Handle events that may be handled by warpui, or maybe not in some cases, e.g. window
    /// drag-to-resize or drag-to-move.
    fn handle_converted_warpui_event(
        &mut self,
        window_id: winit::window::WindowId,
        event: crate::Event,
    ) {
        let Some(window_state) = self.state.windows.get_mut(&window_id) else {
            return;
        };
        let Some(window) = self
            .ui_app
            .read(|ctx| ctx.windows().platform_window(window_state.window_id))
        else {
            return;
        };

        let winit_window = downcast_window(window.as_ref());
        // There is some state on the [`winit::window::Window`] that needs to be kept
        // in sync with the cursor position in order for drag-resizing windows to work.
        if let crate::Event::MouseMoved { .. } = event {
            if !winit_window.is_decorated() {
                winit_window.update_drag_resize_state(window_state.last_cursor_position);
            }
        }

        // Check if we should start a window drag-resize. If so, do that instead of
        // passing the event into warpui. Skip for touch events as drag_resize_window
        // doesn't work properly with touch input on Windows.
        if let crate::event::Event::LeftMouseDown { .. } = event {
            if !winit_window.is_decorated()
                && winit_window.try_drag_resize()
                && window_state.last_touch_purpose.is_none()
            {
                // If we initiated a drag via the method
                // [`winit::window::Window::drag_resize_window`], we will not
                // receive a MouseInput event when the button is release, so we
                // pre-emptively set this back to None.
                window_state.current_mouse_button_pressed = None;
                return;
            }
        }
        let dispatch_result = self
            .callbacks
            .for_window(winit_window)
            .dispatch_event(event.clone());

        // If the app didn't handle the event, warpui might still want to do something
        // with it if it's a click within the "titlebar region" at the top.
        if !dispatch_result.handled {
            if let crate::event::Event::LeftMouseDown {
                click_count,
                position,
                ..
            } = event
            {
                // The WASM "window" does not support dragging or maximization.
                let titlebar_height = winit_window.titlebar_height();
                if position.y() < titlebar_height && !cfg!(target_family = "wasm") {
                    // Double-clicking the titlebar does maximize/restore.
                    if click_count >= 2 {
                        window.toggle_maximized();
                    } else if window_state.last_touch_purpose.is_none() {
                        // Single-click drag moves the window. Skip for touch events as
                        // drag_window doesn't work properly with touch input on Windows.
                        // We won't receive MouseInput::Released after drag_window.
                        match winit_window.drag_window() {
                            Ok(_) => window_state.current_mouse_button_pressed = None,
                            Err(err) => log::error!("error dragging window: {err:?}"),
                        }
                    }
                }
            }
        }

        // On mobile WASM, update soft keyboard state based on touch/click events.
        // This must happen synchronously within the touch event handler (user gesture context)
        // for the browser to allow focusing the hidden input element.
        //
        // For touch events, we defer keyboard updates until LeftMouseUp to avoid showing
        // the keyboard during drags/scrolls (which start with LeftMouseDown but later get
        // reclassified as scroll gestures). We only trigger the keyboard if the touch purpose
        // is still Tap (meaning it was never reclassified to Scroll, Select, or WindowDrag).
        #[cfg(target_family = "wasm")]
        {
            // First, check what kind of event we have without holding a mutable borrow.
            let touch_info = self.state.windows.get(&window_id).and_then(|ws| {
                ws.last_touch_purpose.as_ref().map(|purpose| {
                    // Check if this is still a tap (not a scroll/drag/select)
                    matches!(purpose, TouchPurpose::Tap(..))
                })
            });

            match (&event, touch_info) {
                // Regular mouse click (not touch) - update keyboard immediately.
                (crate::event::Event::LeftMouseDown { .. }, None) => {
                    self.update_soft_keyboard_state(dispatch_result.soft_keyboard_requested);
                }
                // Touch LeftMouseDown - store keyboard request for later use on LeftMouseUp.
                (crate::event::Event::LeftMouseDown { .. }, Some(_)) => {
                    if let Some(ws) = self.state.windows.get_mut(&window_id) {
                        ws.pending_soft_keyboard_request = dispatch_result.soft_keyboard_requested;
                    }
                }
                // Touch tap completed and purpose is still Tap - use stored keyboard request.
                (crate::event::Event::LeftMouseUp { .. }, Some(true)) => {
                    let should_show = self
                        .state
                        .windows
                        .get(&window_id)
                        .map(|ws| ws.pending_soft_keyboard_request)
                        .unwrap_or(false);
                    self.update_soft_keyboard_state(should_show);
                }
                _ => {}
            };
        }

        // On LeftMouseUp: clear touch state and start momentum scrolling if applicable.
        if matches!(event, crate::event::Event::LeftMouseUp { .. }) {
            let should_start_momentum = self
                .state
                .windows
                .get_mut(&window_id)
                .and_then(|window_state| {
                    let purpose = window_state.last_touch_purpose.take()?;

                    if !matches!(purpose, TouchPurpose::Scroll(_)) {
                        return None;
                    }

                    window_state.last_mouse_button_pressed = None;

                    let scroll_vel = window_state.scroll_velocity.as_mut()?;
                    // Clamp velocity to prevent excessively fast momentum scrolling
                    // from quick flick gestures or batched touch events that produce
                    // artificially large velocity spikes.
                    scroll_vel.velocity = vec2f(
                        scroll_vel
                            .velocity
                            .x()
                            .clamp(-MOMENTUM_MAX_VELOCITY, MOMENTUM_MAX_VELOCITY),
                        scroll_vel
                            .velocity
                            .y()
                            .clamp(-MOMENTUM_MAX_VELOCITY, MOMENTUM_MAX_VELOCITY),
                    );
                    (scroll_vel.velocity.length() >= MOMENTUM_THRESHOLD).then_some(())
                })
                .is_some();

            if should_start_momentum {
                let abort_handle = self.start_momentum_scroll(window_id);

                if let Some(ws) = self.state.windows.get_mut(&window_id) {
                    ws.momentum_scroll_abort = Some(abort_handle);
                }
            }
        }
    }

    fn handle_clipboard_event(&mut self, clipboard_event: ClipboardEvent) {
        let Some(active_window_id) = self.ui_app.read(|ctx| ctx.windows().active_window()) else {
            return;
        };

        match clipboard_event {
            #[allow(unused_variables)]
            ClipboardEvent::Paste(content) => {
                cfg_if::cfg_if! {
                    if #[cfg(target_family = "wasm")] {
                        self.ui_app.update(|ctx| {
                            ctx.clipboard().save(content);
                        })
                    }
                }
                self.ui_app
                    .dispatch_standard_action(active_window_id, StandardAction::Paste);
            }
        }
    }

    /// Starts a timer that triggers MomentumScroll events at a fixed interval.
    fn start_momentum_scroll(&self, window_id: winit::window::WindowId) -> AbortHandle {
        let proxy = self.proxy.clone();
        // Use abortable future so we can cancel momentum scrolling when the user touches the screen.
        // Aborting is expected since we cancelled the animation, so we can safely discard it.
        let (future, abort_handle) = futures::future::abortable(async move {
            loop {
                Timer::after(MOMENTUM_FRAME_INTERVAL).await;
                let _ = proxy.send_event(CustomEvent::MomentumScroll { window_id });
            }
        });

        self.ui_app.read(|ctx| {
            ctx.foreground_executor()
                .spawn(async move {
                    let _ = future.await;
                })
                .detach();
        });

        abort_handle
    }

    fn update_ime_position(&mut self) {
        let Some(active_window_id) = self.ui_app.read(|ctx| ctx.windows().active_window()) else {
            return;
        };
        let Some(window) = self
            .ui_app
            .update(|ctx| ctx.windows().platform_window(active_window_id))
        else {
            return;
        };

        let mut window_callbacks = self.callbacks.for_window(window.as_ref());
        let active_cursor_position = window_callbacks.get_active_cursor_position();
        if let Some(active_cursor_position) = active_cursor_position {
            let winit_window = downcast_window(window.as_ref());
            let position = LogicalPosition::new(
                active_cursor_position.position.origin_x(),
                active_cursor_position.position.origin_y()
                    + (1.2 * active_cursor_position.font_size),
            );
            // Currently the size argument is not supported on X11. We calculate it here anyway.
            let size = LogicalSize::new(
                active_cursor_position.font_size,
                active_cursor_position.font_size,
            );
            // TODO(abhishek): We make sure that the position is different than last time to prevent winit from
            // caching the old position and not properly updating on `WindowMoved` or `WindowResized` events.
            winit_window.set_ime_position(LogicalPosition::new(position.x, position.y + 1.), size);
            winit_window.set_ime_position(position, size);
        }
    }

    /// Prepares for an impending system suspend/sleep on Linux.
    ///
    /// When using a dedicated GPU, the kernel sometimes disables the device
    /// during system suspend.  When this happens, wgpu treats the device as
    /// "lost", which currently produces an unavoidable panic the next time
    /// it is accessed.
    ///
    /// To work around this, we drop all rendering resources pre-suspend, and
    /// re-create them post-resume.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn prepare_for_sleep_on_linux(&mut self, window_target: &ActiveEventLoop) {
        self.ui_app.update(|ctx| {
            for window_id in ctx.window_ids() {
                let Some(window) = ctx.windows().platform_window(window_id) else {
                    return;
                };

                let winit_window = downcast_window(window.as_ref());
                winit_window.drop_renderer(Box::new(window_target.owned_display_handle()));
            }
        });
    }

    /// Resumes from system suspend/sleep on Linux.
    ///
    /// See the [`Self::prepare_for_sleep_on_linux`] documentation for more
    /// details.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn resume_from_sleep_on_linux(&mut self) {
        self.ui_app.update(|ctx| {
            for window_id in ctx.window_ids() {
                let Some(window) = ctx.windows().platform_window(window_id) else {
                    return;
                };

                let winit_window = downcast_window(window.as_ref());
                winit_window.recreate_renderer(self.downrank_non_nvidia_vulkan_adapters);
            }
        });
    }

    /// Initializes the soft keyboard manager on mobile WASM devices.
    ///
    /// This creates the hidden input element that triggers the soft keyboard
    /// when focused. The manager is only created on mobile devices.
    #[cfg(target_family = "wasm")]
    fn initialize_soft_keyboard(&mut self) {
        use crate::platform::wasm::{is_mobile_device, SoftKeyboardInput, SoftKeyboardManager};

        if !is_mobile_device() {
            log::info!("Not a mobile device, skipping soft keyboard initialization");
            return;
        }

        log::info!("Initializing soft keyboard for mobile WASM");

        // Create a callback that handles soft keyboard input events.
        // These are forwarded to the event loop via CustomEvent, where they
        // will be dispatched to the active window as TypedCharacters/IME events.
        let proxy = self.proxy.clone();
        let on_input = Box::new(move |input: SoftKeyboardInput| {
            log::debug!("Soft keyboard callback received input: {:?}", input);
            if let Err(e) = proxy.send_event(CustomEvent::SoftKeyboardInput(input)) {
                log::error!("Failed to send SoftKeyboardInput event: {:?}", e);
            }
        });

        match SoftKeyboardManager::new(on_input) {
            Ok(manager) => {
                log::info!("Soft keyboard manager initialized successfully");
                self.soft_keyboard_manager = Some(manager);
            }
            Err(err) => {
                log::error!("Failed to initialize soft keyboard manager: {:?}", err);
            }
        }
    }

    /// Updates the soft keyboard visibility based on the dispatch result.
    ///
    /// This is called after processing touch events, while still in user gesture context.
    /// Since everything renders to a canvas, the browser can't detect taps "outside" the
    /// keyboard input, so we must explicitly show/hide based on what the app requested.
    #[cfg(target_family = "wasm")]
    fn update_soft_keyboard_state(&mut self, requested: bool) {
        let Some(manager) = &self.soft_keyboard_manager else {
            return;
        };

        if requested {
            log::debug!("App requested soft keyboard, showing it");
            manager.show_keyboard();
        } else {
            log::debug!("App did not request soft keyboard, hiding it");
            manager.hide_keyboard();
        }
    }

    /// Attempts to refocus the main canvas element.
    ///
    /// This is needed to restore app interactivity after the soft keyboard is dismissed,
    /// particularly on iOS Safari which can leave the app in a "blurred" state.
    ///
    /// We defer the focus to the next frame using setTimeout(0) because calling focus()
    /// synchronously during event processing may not work reliably on iOS Safari.
    #[cfg(target_family = "wasm")]
    fn refocus_canvas() {
        use wasm_bindgen::{prelude::Closure, JsCast};

        // Defer focus to next frame to ensure we're outside the current event processing.
        let callback = Closure::once(Box::new(|| {
            if let Some(canvas) = gloo::utils::document()
                .query_selector("canvas")
                .ok()
                .flatten()
            {
                if let Ok(html_element) = canvas.dyn_into::<web_sys::HtmlElement>() {
                    let _ = html_element.focus();
                }
            }
        }) as Box<dyn FnOnce()>);

        let _ = gloo::utils::window().set_timeout_with_callback(callback.as_ref().unchecked_ref());
        // Prevent the closure from being dropped immediately
        callback.forget();
    }

    /// Handles input events from the soft keyboard on mobile WASM.
    ///
    /// Converts `SoftKeyboardInput` events into warpui `Event`s and dispatches
    /// them to the active window, similar to how `handle_ime_event` works.
    #[cfg(target_family = "wasm")]
    fn handle_soft_keyboard_input(&mut self, input: crate::platform::wasm::SoftKeyboardInput) {
        use crate::platform::wasm::SoftKeyboardInput;

        // On WASM, get the first (and typically only) window if there's no "active" window
        let window_id = self.ui_app.read(|ctx| {
            ctx.windows()
                .active_window()
                .or_else(|| ctx.window_ids().next())
        });

        let Some(window_id) = window_id else {
            log::debug!("No window for soft keyboard input");
            return;
        };
        let Some(window) = self
            .ui_app
            .read(|ctx| ctx.windows().platform_window(window_id))
        else {
            log::debug!("Could not get platform window for soft keyboard input");
            return;
        };

        let mut window_callbacks = self.callbacks.for_window(window.as_ref());

        match input {
            SoftKeyboardInput::TextInserted(text) => {
                window_callbacks.dispatch_event(TypedCharacters { chars: text });
            }
            SoftKeyboardInput::Backspace => {
                window_callbacks.dispatch_event(crate::Event::KeyDown {
                    keystroke: crate::keymap::Keystroke {
                        ctrl: false,
                        alt: false,
                        shift: false,
                        cmd: false,
                        meta: false,
                        key: "backspace".to_string(),
                    },
                    chars: String::new(),
                    details: crate::event::KeyEventDetails::default(),
                    is_composing: false,
                });
            }
            SoftKeyboardInput::KeyboardDismissed => {
                // The keyboard was dismissed (user tapped elsewhere or pressed "Done").
                // Refocus the canvas to restore interactivity.
                log::debug!("Soft keyboard was dismissed, refocusing canvas");
                Self::refocus_canvas();
            }
            SoftKeyboardInput::KeyDown(key) => {
                // Map special key names to their control characters (e.g., Enter → "\r")
                // so the terminal's key_down handler can process them.
                let chars = match key.to_lowercase().as_str() {
                    "enter" => "\r".to_string(),
                    _ => String::new(),
                };
                window_callbacks.dispatch_event(crate::Event::KeyDown {
                    keystroke: crate::keymap::Keystroke {
                        ctrl: false,
                        alt: false,
                        shift: false,
                        cmd: false,
                        meta: false,
                        key: key.to_lowercase(),
                    },
                    chars,
                    details: crate::event::KeyEventDetails::default(),
                    is_composing: false,
                });
            }
        }
    }

    /// Resizes container when visual viewport changes (e.g., soft keyboard appears).
    #[cfg(target_family = "wasm")]
    fn handle_visual_viewport_resize(&mut self, _width: f32, height: f32) {
        log::debug!("Visual viewport resized, height = {}px", height);

        if let Some(container) = gloo::utils::document().get_element_by_id("wasm-container") {
            if let Some(html_element) = container.dyn_ref::<web_sys::HtmlElement>() {
                let _ = html_element
                    .style()
                    .set_property("height", &format!("{}px", height));
            }
        }
    }
}

/// Set of possible UI-framework events that have been converted from a [`winit::event::Event`].
#[derive(Debug)]
enum ConvertedEvent {
    Event(crate::Event),
    /// A keydown event with the actual text of keydown event. We separate the characters from the
    /// underlying event so that we can produce a `TypedCharacters` event if the initial `event` is
    /// not handled by the UI framework.
    KeyDownWithTypedCharacters {
        chars: Option<String>,
        event: crate::Event,
    },
    Resize,
    WindowMoved {
        new_position: PhysicalPosition<i32>,
    },
    ModifierKeyChanged {
        key_code: crate::platform::keyboard::KeyCode,
        state: ElementState,
    },
    /// Move the window for touch-based window dragging.
    MoveWindowBy {
        /// Current touch position (window-relative).
        current_touch: PhysicalPosition<f64>,
        /// Touch position when drag started (window-relative).
        start_touch: PhysicalPosition<f64>,
    },
}

/// Convert the platform-independent trait object Window into a concrete, platform-specific Window.
fn downcast_window(window: &dyn platform::Window) -> &super::Window {
    window
        .as_any()
        .downcast_ref::<super::Window>()
        .expect("Should not fail to downcast the platform window to its concrete type")
}
