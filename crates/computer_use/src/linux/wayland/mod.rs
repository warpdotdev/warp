//! Wayland implementation of computer use actions.
//!
//! This module handles the Wayland environment using native XDG portals:
//! - RemoteDesktop portal for input injection (keyboard, mouse)
//! - ScreenCast portal for absolute pointer positioning (stream IDs)
//! - Screenshot portal for taking screenshots

mod keyboard;
mod mouse;
mod screenshot;
mod session;

use async_trait::async_trait;
use keyboard::Keyboard;
use mouse::Mouse;
use pathfinder_geometry::vector::Vector2I;
use session::PortalSession;
use warpui_core::r#async::Timer;

use crate::{Action, ActionResult, Options, TargetedAction};

/// An actor that performs computer use actions on Wayland via XDG portals.
pub struct Actor {
    /// The portal session, created lazily on first use.
    session: Option<PortalSession<'static>>,
    /// Keyboard state for tracking shift and other modifiers.
    keyboard: Keyboard,
    /// Mouse state for tracking position.
    mouse: Mouse,
}

impl Actor {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            session: None,
            keyboard: Keyboard::new(),
            mouse: Mouse::new(),
        })
    }

    /// Ensures a portal session is available, creating one if needed.
    async fn ensure_session(&mut self) -> Result<(), String> {
        if self.session.is_none() {
            self.session = Some(PortalSession::new().await?);
            // Wait for the permission dialog to fully dismiss before returning.
            Timer::after(std::time::Duration::from_millis(500)).await;
        }
        Ok(())
    }
}

#[async_trait]
impl crate::Actor for Actor {
    fn platform(&self) -> Option<crate::Platform> {
        Some(crate::Platform::LinuxWayland)
    }

    async fn perform_actions(
        &mut self,
        actions: &[TargetedAction],
        options: Options,
    ) -> Result<ActionResult, String> {
        // Ensure we have an active session before processing actions.
        // This may show a permission dialog on first use.
        self.ensure_session().await?;

        let mut last_mouse_position: Option<Vector2I> = None;

        for targeted in actions {
            // Per-window targeting is not supported on Wayland; act on the screen / focused
            // surface regardless of the requested target.
            let action: &Action = &targeted.action;
            // Re-acquire session reference each iteration (borrow checker workaround).
            let session = self
                .session
                .as_ref()
                .expect("session must exist after ensure_session");
            let remote_desktop = session.remote_desktop();
            let portal_session = session.session();
            let stream_id = session.stream_id();

            match action {
                Action::Wait(duration) => {
                    Timer::after(*duration).await;
                }
                Action::MouseDown { button, at } => {
                    session.require_pointer()?;
                    self.mouse
                        .move_to(remote_desktop, portal_session, stream_id, *at)
                        .await?;
                    self.mouse
                        .button_down(remote_desktop, portal_session, button)
                        .await?;
                    last_mouse_position = Some(*at);
                }
                Action::MouseUp { button } => {
                    session.require_pointer()?;
                    self.mouse
                        .button_up(remote_desktop, portal_session, button)
                        .await?;
                }
                Action::MouseMove { to } => {
                    session.require_pointer()?;
                    self.mouse
                        .move_to(remote_desktop, portal_session, stream_id, *to)
                        .await?;
                    last_mouse_position = Some(*to);
                }
                Action::MouseWheel {
                    at,
                    direction,
                    distance,
                } => {
                    session.require_pointer()?;
                    self.mouse
                        .move_to(remote_desktop, portal_session, stream_id, *at)
                        .await?;
                    self.mouse
                        .scroll(remote_desktop, portal_session, direction, distance)
                        .await?;
                    last_mouse_position = Some(*at);
                }
                Action::TypeText { text } => {
                    session.require_keyboard()?;
                    self.keyboard
                        .type_text(remote_desktop, portal_session, text)
                        .await?;
                }
                Action::KeyDown { key } => {
                    session.require_keyboard()?;
                    self.keyboard
                        .key_down(remote_desktop, portal_session, key)
                        .await?;
                }
                Action::KeyUp { key } => {
                    session.require_keyboard()?;
                    self.keyboard
                        .key_up(remote_desktop, portal_session, key)
                        .await?;
                }
            }
        }

        // Take screenshot if requested.
        let screenshot = if let Some(params) = options.screenshot_params {
            Some(screenshot::take(params).await?)
        } else {
            None
        };

        // Get the final cursor position.
        let cursor_position = if let Some(pos) = last_mouse_position {
            Some(pos)
        } else {
            self.mouse.last_position()
        };

        Ok(ActionResult::legacy(screenshot, cursor_position))
    }
}
