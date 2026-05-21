use std::time::Duration;

use instant::Instant;
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{
    CGEvent, CGEventSource, CGEventSourceStateID, CGEventTapLocation, CGEventType, CGMouseButton,
    CGScrollEventUnit,
};
use pathfinder_geometry::vector::Vector2I;
use warpui::r#async::Timer;

use super::util::main_display_scale_factor;
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
}

impl Mouse {
    pub fn new() -> Self {
        Self {
            held_buttons: HeldButtons::default(),
        }
    }

    pub async fn move_to(&mut self, target: Vector2I) -> Result<(), String> {
        let (event_type, cg_button) = if let Some(held) = self.held_buttons.primary_down() {
            (mouse_dragged_event_type(&held), (&held).into())
        } else {
            (CGEventType::MouseMoved, CGMouseButton::Left)
        };

        self.post_event(event_type, to_cgpoint(target), cg_button)?;
        self.wait_for_position(target).await
    }

    pub fn button_down(&mut self, button: &MouseButton) -> Result<(), String> {
        let point = self.current_position_cgpoint()?;
        self.held_buttons.set_down(button, true);
        self.post_event(mouse_down_event_type(button), point, button.into())
    }

    pub fn button_up(&mut self, button: &MouseButton) -> Result<(), String> {
        let point = self.current_position_cgpoint()?;
        self.held_buttons.set_down(button, false);
        self.post_event(mouse_up_event_type(button), point, button.into())
    }

    pub fn current_position(&mut self) -> Result<Vector2I, String> {
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

        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        Ok(())
    }
}

// Private implementation details.
impl Mouse {
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

    fn post_event(
        &mut self,
        event_type: CGEventType,
        point: CGPoint,
        button: CGMouseButton,
    ) -> Result<(), String> {
        let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);

        let event = CGEvent::new_mouse_event(source.as_deref(), event_type, point, button)
            .ok_or_else(|| {
                format!(
                    "Failed to create mouse event (type={:?}, position=({}, {}), button={:?}). \
                     The cause is unknown.",
                    event_type, point.x, point.y, button
                )
            })?;

        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        Ok(())
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
