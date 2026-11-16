//! Mouse input handling for X11 using XTEST.

use pathfinder_geometry::vector::Vector2I;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use crate::{MouseButton, ScrollDirection, ScrollDistance};

/// Mouse state tracking for an X11 connection.
pub struct Mouse<'a> {
    conn: &'a RustConnection,
    root_window: xproto::Window,
    held_buttons: HeldButtons,
}

impl<'a> Mouse<'a> {
    pub fn new(conn: &'a RustConnection, root_window: xproto::Window) -> Self {
        Self {
            conn,
            root_window,
            held_buttons: HeldButtons::default(),
        }
    }

    pub fn move_to(&mut self, target: Vector2I) -> Result<(), String> {
        // Use WarpPointer to move the pointer. Unlike XTEST MotionNotify, this
        // reliably updates the server's pointer position, ensuring that subsequent
        // button events are delivered to the correct window.
        self.conn
            .warp_pointer(
                x11rb::NONE,      // src_window (unconstrained)
                self.root_window, // dst_window (absolute coordinates)
                0,                // src_x (unused)
                0,                // src_y (unused)
                0,                // src_width (unused)
                0,                // src_height (unused)
                target.x() as i16,
                target.y() as i16,
            )
            .map_err(|e| format!("Failed to warp pointer: {e}"))?;

        self.conn
            .flush()
            .map_err(|e| format!("Failed to flush X11 connection: {e}"))?;

        Ok(())
    }

    pub fn button_down(&mut self, button: &MouseButton) -> Result<(), String> {
        let x11_button = mouse_button_to_x11(button);
        self.held_buttons.set_down(button, true);

        self.conn
            .xtest_fake_input(
                xproto::BUTTON_PRESS_EVENT,
                x11_button,
                x11rb::CURRENT_TIME,
                x11rb::NONE,
                0,
                0,
                0,
            )
            .map_err(|e| format!("Failed to send button down: {e}"))?;

        self.conn
            .flush()
            .map_err(|e| format!("Failed to flush X11 connection: {e}"))?;

        Ok(())
    }

    pub fn button_up(&mut self, button: &MouseButton) -> Result<(), String> {
        let x11_button = mouse_button_to_x11(button);
        self.held_buttons.set_down(button, false);

        self.conn
            .xtest_fake_input(
                xproto::BUTTON_RELEASE_EVENT,
                x11_button,
                x11rb::CURRENT_TIME,
                x11rb::NONE,
                0,
                0,
                0,
            )
            .map_err(|e| format!("Failed to send button up: {e}"))?;

        self.conn
            .flush()
            .map_err(|e| format!("Failed to flush X11 connection: {e}"))?;

        Ok(())
    }

    pub fn scroll(
        &mut self,
        direction: &ScrollDirection,
        distance: &ScrollDistance,
    ) -> Result<(), String> {
        // In X11, scroll is done via button presses. Buttons 4/5 are vertical scroll,
        // buttons 6/7 are horizontal scroll.
        let (button, count) = match (direction, distance) {
            (ScrollDirection::Up, ScrollDistance::Clicks(n)) => (4u8, *n),
            (ScrollDirection::Down, ScrollDistance::Clicks(n)) => (5u8, *n),
            (ScrollDirection::Left, ScrollDistance::Clicks(n)) => (6u8, *n),
            (ScrollDirection::Right, ScrollDistance::Clicks(n)) => (7u8, *n),
            // For pixel scrolling, approximate with clicks. X11 doesn't have native pixel scroll.
            (ScrollDirection::Up, ScrollDistance::Pixels(px)) => (4u8, (*px / 15).max(1)),
            (ScrollDirection::Down, ScrollDistance::Pixels(px)) => (5u8, (*px / 15).max(1)),
            (ScrollDirection::Left, ScrollDistance::Pixels(px)) => (6u8, (*px / 15).max(1)),
            (ScrollDirection::Right, ScrollDistance::Pixels(px)) => (7u8, (*px / 15).max(1)),
        };

        for _ in 0..count.abs() {
            // Button press.
            self.conn
                .xtest_fake_input(
                    xproto::BUTTON_PRESS_EVENT,
                    button,
                    x11rb::CURRENT_TIME,
                    x11rb::NONE,
                    0,
                    0,
                    0,
                )
                .map_err(|e| format!("Failed to send scroll button press: {e}"))?;

            // Button release.
            self.conn
                .xtest_fake_input(
                    xproto::BUTTON_RELEASE_EVENT,
                    button,
                    x11rb::CURRENT_TIME,
                    x11rb::NONE,
                    0,
                    0,
                    0,
                )
                .map_err(|e| format!("Failed to send scroll button release: {e}"))?;
        }

        self.conn
            .flush()
            .map_err(|e| format!("Failed to flush X11 connection: {e}"))?;

        Ok(())
    }

    /// Sets input focus to the deepest window under the current pointer position.
    /// This simulates the click-to-focus behavior that a window manager would
    /// normally provide. Without a WM (e.g. in Xvfb), windows do not receive
    /// focus automatically, so keyboard and some button events may not be
    /// delivered correctly.
    pub fn focus_window_under_pointer(&self) -> Result<(), String> {
        let mut window = self.root_window;

        // Walk down the window tree to find the deepest child under the pointer.
        loop {
            let reply = self
                .conn
                .query_pointer(window)
                .map_err(|e| format!("Failed to query pointer: {e}"))?
                .reply()
                .map_err(|e| format!("Failed to get pointer reply: {e}"))?;

            if reply.child == x11rb::NONE {
                break;
            }
            window = reply.child;
        }

        // Set input focus to the deepest window found.
        self.conn
            .set_input_focus(xproto::InputFocus::PARENT, window, x11rb::CURRENT_TIME)
            .map_err(|e| format!("Failed to set input focus: {e}"))?;

        self.conn
            .flush()
            .map_err(|e| format!("Failed to flush X11 connection: {e}"))?;

        Ok(())
    }

    pub fn current_position(&mut self) -> Result<Vector2I, String> {
        let reply = self
            .conn
            .query_pointer(self.root_window)
            .map_err(|e| format!("Failed to query pointer: {e}"))?
            .reply()
            .map_err(|e| format!("Failed to get pointer reply: {e}"))?;

        let pos = Vector2I::new(reply.root_x as i32, reply.root_y as i32);
        Ok(pos)
    }
}

#[derive(Clone, Copy, Default)]
struct HeldButtons {
    left: bool,
    right: bool,
    middle: bool,
    back: bool,
    forward: bool,
}

impl HeldButtons {
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

fn mouse_button_to_x11(button: &MouseButton) -> u8 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
        MouseButton::Back => 8,
        MouseButton::Forward => 9,
    }
}
