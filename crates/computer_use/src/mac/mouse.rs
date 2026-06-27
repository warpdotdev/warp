use std::time::Duration;

use instant::Instant;
use objc2::rc::Retained;
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventSource, CGEventSourceStateID, CGEventType, CGMouseButton,
    CGScrollEventUnit,
};
use pathfinder_geometry::vector::Vector2I;
use warpui_core::r#async::Timer;

use super::post::PostTarget;
use super::util::main_display_scale_factor;
use super::window;
use crate::{MouseButton, ScrollDirection, ScrollDistance};

const POSITION_POLL_INTERVAL: Duration = Duration::from_micros(500);
const POSITION_TIMEOUT: Duration = Duration::from_millis(100);

/// Converts physical coordinates to CGEvent point coordinates.
///
/// On Retina/HiDPI displays, physical coordinates differ from the "point" coordinates
/// used by macOS APIs like CGEvent. This function scales physical coordinates down
/// by the display's backing scale factor.
pub fn to_cgpoint(target: Vector2I) -> CGPoint {
    let scale = main_display_scale_factor();
    CGPoint {
        x: target.x() as f64 / scale,
        y: target.y() as f64 / scale,
    }
}

/// Converts CGEvent point coordinates to physical coordinates.
pub fn from_cgpoint(point: CGPoint) -> Vector2I {
    let scale = main_display_scale_factor();
    Vector2I::new((point.x * scale) as i32, (point.y * scale) as i32)
}

/// Manages mouse state and posts mouse events to the system.
pub struct Mouse {
    held_buttons: HeldButtons,
    /// Where synthesized events are delivered.
    target: PostTarget,
    /// The most recently requested cursor position, in CGEvent point coordinates.
    ///
    /// When delivering events directly to a PID, `CGEventPostToPid` does not move the real
    /// cursor, so the global cursor position cannot be used to locate clicks. We track the
    /// intended position here and use it as the location for button and move events.
    virtual_position: CGPoint,
}

impl Mouse {
    pub fn new(target: PostTarget) -> Self {
        Self {
            held_buttons: HeldButtons::default(),
            target,
            virtual_position: CGPoint { x: 0.0, y: 0.0 },
        }
    }

    /// Sets where subsequent synthesized events are delivered. Called per-action so a batch can
    /// drive the HID tap for some actions and a specific process for others.
    pub fn set_target(&mut self, target: PostTarget) {
        self.target = target;
    }

    pub async fn move_to(&mut self, target: Vector2I) -> Result<(), String> {
        let (event_type, cg_button) = if let Some(held) = self.held_buttons.primary_down() {
            (mouse_dragged_event_type(&held), (&held).into())
        } else {
            (CGEventType::MouseMoved, CGMouseButton::Left)
        };

        let point = to_cgpoint(target);
        self.virtual_position = point;
        // A drag is part of an active click, so it carries the click state; a plain move does
        // not.
        let click_state = if self.held_buttons.primary_down().is_some() {
            1
        } else {
            0
        };
        self.post_event(event_type, point, cg_button, click_state)?;

        // `CGEventPostToPid` does not move the real cursor, so polling the global cursor
        // position would always time out. Only wait when injecting through the HID tap.
        if self.target.is_pid_targeted() {
            Ok(())
        } else {
            self.wait_for_position(target).await
        }
    }

    pub fn button_down(&mut self, button: &MouseButton) -> Result<(), String> {
        let point = self.event_location()?;
        self.held_buttons.set_down(button, true);
        self.post_event(mouse_down_event_type(button), point, button.into(), 1)
    }

    pub fn button_up(&mut self, button: &MouseButton) -> Result<(), String> {
        let point = self.event_location()?;
        self.held_buttons.set_down(button, false);
        self.post_event(mouse_up_event_type(button), point, button.into(), 1)
    }

    pub fn current_position(&mut self) -> Result<Vector2I, String> {
        // In PID-targeted mode the real cursor is never moved, so report the tracked virtual
        // position instead of the (unrelated) global cursor location.
        if self.target.is_pid_targeted() {
            return Ok(from_cgpoint(self.virtual_position));
        }
        let cg_point = self.current_position_cgpoint()?;
        Ok(from_cgpoint(cg_point))
    }

    /// Scrolls the mouse wheel in the given direction by the given distance.
    pub fn scroll(
        &mut self,
        direction: &ScrollDirection,
        distance: &ScrollDistance,
    ) -> Result<(), String> {
        let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);

        // Determine scroll unit and amount based on distance type.
        let (unit, amount) = match distance {
            ScrollDistance::Pixels(pixels) => (CGScrollEventUnit::Pixel, *pixels),
            ScrollDistance::Clicks(clicks) => (CGScrollEventUnit::Line, *clicks),
        };

        // Determine which axis and sign to use based on direction.
        // Positive values scroll up/left, negative values scroll down/right.
        let (wheel1, wheel2) = match direction {
            ScrollDirection::Up => (amount, 0),
            ScrollDirection::Down => (-amount, 0),
            ScrollDirection::Left => (0, amount),
            ScrollDirection::Right => (0, -amount),
        };

        // The function signature is:
        // new_scroll_wheel_event2(source, units, wheel_count, wheel1, wheel2, wheel3)
        // wheel_count indicates how many wheel values are valid (1, 2, or 3).
        let wheel_count = if wheel2 != 0 { 2 } else { 1 };
        let event = CGEvent::new_scroll_wheel_event2(
            source.as_deref(),
            unit,
            wheel_count,
            wheel1,
            wheel2,
            0,
        )
        .ok_or_else(|| {
            format!(
                "Failed to create scroll wheel event (direction={:?}, distance={:?}). \
                     The cause is unknown.",
                direction, distance
            )
        })?;

        self.target.post(&event);
        Ok(())
    }
}

// Private implementation details.
impl Mouse {
    /// Returns the location to use for a button event.
    ///
    /// In HID mode this is the real cursor position; in PID-targeted mode the real cursor is
    /// never moved, so the tracked virtual position is used instead.
    fn event_location(&mut self) -> Result<CGPoint, String> {
        if self.target.is_pid_targeted() {
            Ok(self.virtual_position)
        } else {
            self.current_position_cgpoint()
        }
    }

    /// Waits for the mouse to reach the target position, polling until it arrives
    /// or times out.
    async fn wait_for_position(&mut self, target: Vector2I) -> Result<(), String> {
        let start = Instant::now();

        loop {
            let current = self.current_position()?;
            if current == target {
                return Ok(());
            }
            if start.elapsed() >= POSITION_TIMEOUT {
                log::warn!(
                    "Mouse position wait timed out. Target: ({}, {}), Current: ({}, {})",
                    target.x(),
                    target.y(),
                    current.x(),
                    current.y()
                );
                return Err(format!(
                    "Timed out waiting for mouse to move to ({}, {}). Current position: ({}, {})",
                    target.x(),
                    target.y(),
                    current.x(),
                    current.y()
                ));
            }
            Timer::after(POSITION_POLL_INTERVAL).await;
        }
    }

    fn current_position_cgpoint(&mut self) -> Result<CGPoint, String> {
        let event = CGEvent::new(None)
            .ok_or("Failed to query current cursor position. The cause is unknown.")?;
        let pos = CGEvent::location(Some(&event));
        Ok(pos)
    }

    /// Posts a mouse event.
    ///
    /// `click_state` is the click count (1 for a single click, 2 for a double click, etc.) and
    /// should be 0 for non-button events like plain moves. Many applications ignore synthetic
    /// clicks that lack a non-zero click state, so it is set for button-down, button-up, and
    /// drag events.
    fn post_event(
        &mut self,
        event_type: CGEventType,
        point: CGPoint,
        button: CGMouseButton,
        click_state: i64,
    ) -> Result<(), String> {
        // For a PID target with an owned window under the point, deliver a window-targeted event
        // directly to the owning process via `CGEventPostToPid`, without raising the window or
        // moving the cursor. Falls back to a plain CGEvent via the configured target when there
        // is no PID target or no owned window under the point.
        if let Some(pid) = self.target.pid()
            && let Some(info) = window::window_at(pid, point.x, point.y)
        {
            let is_down = matches!(
                event_type,
                CGEventType::LeftMouseDown
                    | CGEventType::RightMouseDown
                    | CGEventType::OtherMouseDown
            );
            let is_move = matches!(event_type, CGEventType::MouseMoved);
            // Activate the target window in the background on a hover or a press (not on drags),
            // so the pre-click mouse-moved and the press both land on an active window. This is
            // idempotent per window and does not raise the window or steal the user's frontmost
            // focus.
            if is_down || is_move {
                super::activation::ensure_activated(pid, &info);
            }
            post_window_mouse_event(
                pid,
                &info,
                event_type,
                button,
                point,
                click_state,
                event_pressure(event_type),
            );
            return Ok(());
        }

        // Fallback: no PID target, or no owned window under the point. Post a plain event via the
        // configured target (HID tap for screen targets, CGEventPostToPid for a PID target).
        let event = build_plain_mouse_event(event_type, point, button, click_state)?;
        self.target.post(&event);
        Ok(())
    }
}

/// Builds and posts a mouse event targeted at `info` (the window under `global_point`) directly
/// to the owning process via `CGEventPostToPid`.
///
/// On the `postToPid` path the WindowServer does not run its normal hit-testing, so the event
/// must carry the fields AppKit reads to route and interpret it: the click state, the pressure,
/// the target pid, the window-under-pointer number, the private window-addressing fields, and
/// the window-local location.
pub(super) fn post_window_mouse_event(
    pid: libc::pid_t,
    info: &window::WindowInfo,
    event_type: CGEventType,
    button: CGMouseButton,
    global_point: CGPoint,
    click_state: i64,
    pressure: f64,
) {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);
    let Some(event) = CGEvent::new_mouse_event(source.as_deref(), event_type, global_point, button)
    else {
        log::warn!("Failed to create window-targeted mouse event (type={event_type:?}).");
        return;
    };

    // `CGEventSetWindowLocation` wants window-local coordinates with a top-left origin (the
    // global screen point translated by the window origin).
    let window_local = CGPoint {
        x: global_point.x - info.x,
        y: global_point.y - info.y,
    };

    if click_state > 0 {
        CGEvent::set_integer_value_field(
            Some(&event),
            CGEventField::MouseEventClickState,
            click_state,
        );
    }
    CGEvent::set_double_value_field(Some(&event), CGEventField::MouseEventPressure, pressure);
    CGEvent::set_integer_value_field(
        Some(&event),
        CGEventField::EventTargetUnixProcessID,
        pid as i64,
    );
    CGEvent::set_integer_value_field(
        Some(&event),
        CGEventField::MouseEventWindowUnderMousePointer,
        info.number,
    );
    CGEvent::set_integer_value_field(
        Some(&event),
        CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent,
        info.number,
    );
    set_window_addressing_fields(&event, info.number);
    set_window_location(&event, window_local);

    CGEvent::post_to_pid(pid, Some(&event));
}

/// Stamps the private window-addressing fields the WindowServer uses to route an event to a
/// specific window on the `postToPid` path: field 51 carries the target window number, and field
/// 58 flags that the window number is valid.
pub(super) fn set_window_addressing_fields(event: &CGEvent, window_number: i64) {
    CGEvent::set_integer_value_field(Some(event), CGEventField(51), window_number);
    CGEvent::set_integer_value_field(Some(event), CGEventField(58), 1);
}

/// Returns the pressure value for a mouse event: full pressure while a button is held (a press
/// or drag) and zero otherwise (a move or release), mirroring what a real device reports.
fn event_pressure(event_type: CGEventType) -> f64 {
    match event_type {
        CGEventType::LeftMouseDown
        | CGEventType::RightMouseDown
        | CGEventType::OtherMouseDown
        | CGEventType::LeftMouseDragged
        | CGEventType::RightMouseDragged
        | CGEventType::OtherMouseDragged => 1.0,
        _ => 0.0,
    }
}

/// Builds a plain CGEvent mouse event at the global `point`, stamping the click state. Used by the
/// non-window fallback delivery path.
fn build_plain_mouse_event(
    event_type: CGEventType,
    point: CGPoint,
    button: CGMouseButton,
    click_state: i64,
) -> Result<Retained<CGEvent>, String> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);
    let event = CGEvent::new_mouse_event(source.as_deref(), event_type, point, button).ok_or_else(
        || {
            format!(
                "Failed to create mouse event (type={event_type:?}, position=({}, {}), \
                 button={button:?}). The cause is unknown.",
                point.x, point.y
            )
        },
    )?;
    if click_state > 0 {
        CGEvent::set_integer_value_field(
            Some(&event),
            CGEventField::MouseEventClickState,
            click_state,
        );
    }
    Ok(event.into())
}

/// Sets the window-local location on a `CGEvent` via the private `CGEventSetWindowLocation`.
///
/// There is no public setter for this field, which AppKit reads on the `postToPid` delivery
/// path. The symbol is resolved once at runtime. `location` is window-local, top-left origin.
fn set_window_location(event: &CGEvent, location: CGPoint) {
    use std::ffi::c_void;
    use std::sync::OnceLock;

    type SetWindowLocationFn = unsafe extern "C" fn(*mut c_void, CGPoint);
    // The macOS value of `RTLD_DEFAULT`, used to search all loaded images for the symbol.
    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

    static RESOLVED: OnceLock<Option<SetWindowLocationFn>> = OnceLock::new();
    let resolved = RESOLVED.get_or_init(|| unsafe {
        let sym = libc::dlsym(RTLD_DEFAULT, c"CGEventSetWindowLocation".as_ptr());
        if sym.is_null() {
            None
        } else {
            Some(std::mem::transmute::<*mut c_void, SetWindowLocationFn>(sym))
        }
    });

    match resolved {
        Some(set_window_location) => {
            let event_ptr = event as *const CGEvent as *mut c_void;
            unsafe { set_window_location(event_ptr, location) };
        }
        None => {
            log::warn!(
                "CGEventSetWindowLocation could not be resolved; background clicks may not land."
            );
        }
    }
}

// ----------------------------------------------------------------------------
// Button state tracking
// ----------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct HeldButtons {
    left: bool,
    right: bool,
    middle: bool,
    back: bool,
    forward: bool,
}

impl HeldButtons {
    /// Returns the "primary" held button (preferring left > right > middle).
    fn primary_down(self) -> Option<MouseButton> {
        if self.left {
            Some(MouseButton::Left)
        } else if self.right {
            Some(MouseButton::Right)
        } else if self.middle {
            Some(MouseButton::Middle)
        } else if self.back {
            Some(MouseButton::Back)
        } else if self.forward {
            Some(MouseButton::Forward)
        } else {
            None
        }
    }

    fn set_down(&mut self, button: &MouseButton, down: bool) {
        match button {
            MouseButton::Left => self.left = down,
            MouseButton::Right => self.right = down,
            MouseButton::Middle => self.middle = down,
            MouseButton::Back => self.back = down,
            MouseButton::Forward => self.forward = down,
        }
    }
}

// ----------------------------------------------------------------------------
// Event type helpers
// ----------------------------------------------------------------------------

impl From<&MouseButton> for CGMouseButton {
    fn from(button: &MouseButton) -> Self {
        match button {
            MouseButton::Left => CGMouseButton::Left,
            MouseButton::Right => CGMouseButton::Right,
            MouseButton::Middle => CGMouseButton::Center,
            MouseButton::Back => CGMouseButton(3),
            MouseButton::Forward => CGMouseButton(4),
        }
    }
}

fn mouse_down_event_type(button: &MouseButton) -> CGEventType {
    match button {
        MouseButton::Left => CGEventType::LeftMouseDown,
        MouseButton::Right => CGEventType::RightMouseDown,
        MouseButton::Middle | MouseButton::Back | MouseButton::Forward => {
            CGEventType::OtherMouseDown
        }
    }
}

fn mouse_up_event_type(button: &MouseButton) -> CGEventType {
    match button {
        MouseButton::Left => CGEventType::LeftMouseUp,
        MouseButton::Right => CGEventType::RightMouseUp,
        MouseButton::Middle | MouseButton::Back | MouseButton::Forward => CGEventType::OtherMouseUp,
    }
}

fn mouse_dragged_event_type(button: &MouseButton) -> CGEventType {
    match button {
        MouseButton::Left => CGEventType::LeftMouseDragged,
        MouseButton::Right => CGEventType::RightMouseDragged,
        MouseButton::Middle | MouseButton::Back | MouseButton::Forward => {
            CGEventType::OtherMouseDragged
        }
    }
}
