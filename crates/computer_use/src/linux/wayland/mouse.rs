//! Mouse input handling for Wayland via the RemoteDesktop portal.

use ashpd::desktop::Session;
use ashpd::desktop::remote_desktop::{Axis, KeyState, RemoteDesktop};
use pathfinder_geometry::vector::Vector2I;

use crate::{MouseButton, ScrollDirection, ScrollDistance};

/// Linux evdev button codes.
const BTN_LEFT: i32 = 0x110;
const BTN_RIGHT: i32 = 0x111;
const BTN_MIDDLE: i32 = 0x112;
const BTN_SIDE: i32 = 0x113; // Back.
const BTN_EXTRA: i32 = 0x114; // Forward.

/// Mouse state for the Wayland portal.
pub struct Mouse {
    /// The last known mouse position (for returning in ActionResult).
    last_position: Option<Vector2I>,
}

impl Mouse {
    pub fn new() -> Self {
        Self {
            last_position: None,
        }
    }

    /// Moves the mouse to an absolute position.
    pub async fn move_to<'a>(
        &mut self,
        remote_desktop: &RemoteDesktop<'a>,
        session: &Session<'a, RemoteDesktop<'a>>,
        stream_id: u32,
        target: Vector2I,
    ) -> Result<(), String> {
        remote_desktop
            .notify_pointer_motion_absolute(
                session,
                stream_id,
                target.x() as f64,
                target.y() as f64,
            )
            .await
            .map_err(|e| format!("Failed to move mouse: {e}"))?;

        self.last_position = Some(target);
        Ok(())
    }

    /// Presses a mouse button down.
    pub async fn button_down<'a>(
        &self,
        remote_desktop: &RemoteDesktop<'a>,
        session: &Session<'a, RemoteDesktop<'a>>,
        button: &MouseButton,
    ) -> Result<(), String> {
        let evdev_button = mouse_button_to_evdev(button);

        remote_desktop
            .notify_pointer_button(session, evdev_button, KeyState::Pressed)
            .await
            .map_err(|e| format!("Failed to press mouse button: {e}"))
    }

    /// Releases a mouse button.
    pub async fn button_up<'a>(
        &self,
        remote_desktop: &RemoteDesktop<'a>,
        session: &Session<'a, RemoteDesktop<'a>>,
        button: &MouseButton,
    ) -> Result<(), String> {
        let evdev_button = mouse_button_to_evdev(button);

        remote_desktop
            .notify_pointer_button(session, evdev_button, KeyState::Released)
            .await
            .map_err(|e| format!("Failed to release mouse button: {e}"))
    }

    /// Performs a scroll action.
    pub async fn scroll<'a>(
        &self,
        remote_desktop: &RemoteDesktop<'a>,
        session: &Session<'a, RemoteDesktop<'a>>,
        direction: &ScrollDirection,
        distance: &ScrollDistance,
    ) -> Result<(), String> {
        match distance {
            ScrollDistance::Clicks(clicks) => {
                // Use discrete scrolling for click-based scrolling.
                let (axis, steps) = match direction {
                    ScrollDirection::Up => (Axis::Vertical, -*clicks),
                    ScrollDirection::Down => (Axis::Vertical, *clicks),
                    ScrollDirection::Left => (Axis::Horizontal, -*clicks),
                    ScrollDirection::Right => (Axis::Horizontal, *clicks),
                };

                remote_desktop
                    .notify_pointer_axis_discrete(session, axis, steps)
                    .await
                    .map_err(|e| format!("Failed to scroll: {e}"))
            }
            ScrollDistance::Pixels(pixels) => {
                // Use smooth scrolling for pixel-based scrolling.
                let (dx, dy) = match direction {
                    ScrollDirection::Up => (0.0, -(*pixels as f64)),
                    ScrollDirection::Down => (0.0, *pixels as f64),
                    ScrollDirection::Left => (-(*pixels as f64), 0.0),
                    ScrollDirection::Right => (*pixels as f64, 0.0),
                };

                remote_desktop
                    .notify_pointer_axis(session, dx, dy, true)
                    .await
                    .map_err(|e| format!("Failed to scroll: {e}"))
            }
        }
    }

    /// Returns the last known mouse position.
    pub fn last_position(&self) -> Option<Vector2I> {
        self.last_position
    }
}

/// Converts a MouseButton to a Linux evdev button code.
fn mouse_button_to_evdev(button: &MouseButton) -> i32 {
    match button {
        MouseButton::Left => BTN_LEFT,
        MouseButton::Right => BTN_RIGHT,
        MouseButton::Middle => BTN_MIDDLE,
        MouseButton::Back => BTN_SIDE,
        MouseButton::Forward => BTN_EXTRA,
    }
}
