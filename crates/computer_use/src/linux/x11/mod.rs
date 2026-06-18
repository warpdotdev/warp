//! X11 implementation of computer use actions using the XTEST extension.

mod keyboard;
mod mouse;
mod screenshot;

use async_trait::async_trait;
use pathfinder_geometry::vector::Vector2I;
use warpui_core::r#async::Timer;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use crate::{Action, ActionResult, Options};

/// An actor that performs computer use actions on X11.
pub struct Actor {
    conn: RustConnection,
    screen_index: usize,
    /// Cached keyboard mapping for this connection. This avoids querying the server on every
    /// character typed.
    keyboard_mapping: xproto::GetKeyboardMappingReply,
}

impl Actor {
    pub fn new() -> Result<Self, String> {
        let (conn, screen_index) =
            RustConnection::connect(None).map_err(|e| format!("Failed to connect to X11: {e}"))?;

        // Verify XTEST extension is available. XTEST is part of the Xorg server and is
        // typically present by default. On Wayland or X servers without XTEST, this will
        // fail and computer use will not be available on X11.
        conn.xtest_get_version(2, 2)
            .map_err(|e| format!("XTEST extension not available: {e}"))?
            .reply()
            .map_err(|e| format!("XTEST extension query failed: {e}"))?;

        // Pre-fetch and cache the keyboard mapping for this connection to avoid
        // round-trips for every character typed.
        let setup = conn.setup();
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;
        let keyboard_mapping = conn
            .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)
            .map_err(|e| format!("Failed to get keyboard mapping: {e}"))?
            .reply()
            .map_err(|e| format!("Failed to get keyboard mapping reply: {e}"))?;

        Ok(Self {
            conn,
            screen_index,
            keyboard_mapping,
        })
    }

    fn root_window(&self) -> xproto::Window {
        self.conn.setup().roots[self.screen_index].root
    }

    fn screen(&self) -> &xproto::Screen {
        &self.conn.setup().roots[self.screen_index]
    }
}

#[async_trait]
impl crate::Actor for Actor {
    fn platform(&self) -> Option<crate::Platform> {
        Some(crate::Platform::LinuxX11)
    }

    async fn perform_actions(
        &mut self,
        actions: &[Action],
        options: Options,
    ) -> Result<ActionResult, String> {
        let mut mouse = mouse::Mouse::new(&self.conn, self.root_window());
        let mut keyboard = keyboard::Keyboard::new(&self.conn, &self.keyboard_mapping);
        let mut last_mouse_position: Option<Vector2I> = None;

        for action in actions {
            match action {
                Action::Wait(duration) => {
                    Timer::after(*duration).await;
                }
                Action::MouseDown { button, at } => {
                    mouse.move_to(*at)?;
                    mouse.focus_window_under_pointer()?;
                    mouse.button_down(button)?;
                    last_mouse_position = Some(*at);
                }
                Action::MouseUp { button } => {
                    mouse.button_up(button)?;
                }
                Action::MouseMove { to } => {
                    mouse.move_to(*to)?;
                    last_mouse_position = Some(*to);
                }
                Action::MouseWheel {
                    at,
                    direction,
                    distance,
                } => {
                    mouse.move_to(*at)?;
                    mouse.scroll(direction, distance)?;
                    last_mouse_position = Some(*at);
                }
                Action::TypeText { text } => {
                    keyboard.type_text(text)?;
                }
                Action::KeyDown { key } => {
                    keyboard.key_down(key)?;
                }
                Action::KeyUp { key } => {
                    keyboard.key_up(key)?;
                }
            }
        }

        let screenshot = if let Some(params) = options.screenshot_params {
            Some(screenshot::take(
                &self.conn,
                self.screen(),
                self.root_window(),
                params,
            )?)
        } else {
            None
        };

        // Get the final mouse position.
        let cursor_position = if let Some(pos) = last_mouse_position {
            Some(pos)
        } else {
            Some(mouse.current_position()?)
        };

        Ok(ActionResult {
            screenshot,
            cursor_position,
        })
    }
}
