mod keyboard;
mod keycode_cache;
mod mouse;
mod post;
mod screenshot;
mod skylight;
mod util;
mod window;

use async_trait::async_trait;
use warpui_core::r#async::Timer;

use post::PostTarget;

use crate::{Action, ActionResult, Options};

pub fn is_supported_on_current_platform() -> bool {
    true
}

/// Experimental: lists on-screen windows (number, owner PID/name, layer, bounds) for
/// diagnosing PID/window targeting.
pub fn list_windows() -> String {
    let mut out = String::from("window#  owner_pid  layer  bounds(x,y,w,h)  owner_name\n");
    for w in window::list_windows() {
        out.push_str(&format!(
            "{:<7}  {:<9}  {:<5}  ({:.0},{:.0},{:.0},{:.0})  {}\n",
            w.number,
            w.owner_pid,
            w.layer,
            w.x,
            w.y,
            w.width,
            w.height,
            w.owner_name.as_deref().unwrap_or("<unknown>"),
        ));
    }
    out
}

pub struct Actor {
    keyboard: keyboard::Keyboard,
    mouse: mouse::Mouse,
}

impl Actor {
    pub fn new() -> Self {
        // Experimental: when COMPUTER_USE_TARGET_PID is set, events are delivered directly to
        // that process via CGEventPostToPid instead of the system-wide HID event tap. This
        // avoids moving the real cursor and stealing focus, but is less reliable (especially
        // for mouse events). See `post::PostTarget` for details.
        let target = PostTarget::from_env();
        if let PostTarget::Pid(pid) = target {
            log::info!("Computer use: routing events directly to PID {pid} (experimental).");
        }
        Self {
            keyboard: keyboard::Keyboard::new(target),
            mouse: mouse::Mouse::new(target),
        }
    }
}

#[async_trait]
impl super::Actor for Actor {
    fn platform(&self) -> Option<super::Platform> {
        Some(super::Platform::Mac)
    }

    async fn perform_actions(
        &mut self,
        actions: &[Action],
        options: Options,
    ) -> Result<ActionResult, String> {
        for action in actions {
            match action {
                Action::Wait(duration) => {
                    Timer::after(*duration).await;
                }
                Action::MouseDown { button, at } => {
                    self.mouse.move_to(*at).await?;
                    self.mouse.button_down(button)?;
                }
                Action::MouseUp { button } => self.mouse.button_up(button)?,
                Action::MouseMove { to } => self.mouse.move_to(*to).await?,
                Action::MouseWheel {
                    at,
                    direction,
                    distance,
                } => {
                    self.mouse.move_to(*at).await?;
                    self.mouse.scroll(direction, distance)?;
                }
                Action::TypeText { text } => {
                    self.keyboard.type_text(text)?;
                }
                Action::KeyDown { key } => {
                    self.keyboard.key_down(key)?;
                }
                Action::KeyUp { key } => {
                    self.keyboard.key_up(key)?;
                }
            }
        }

        // Experimental: optionally restore the user's previous input focus after the batch of
        // actions completes, undoing the focus-without-raise. Gated so it does not break flows
        // that span multiple invocations (e.g. click in one call, type in the next).
        if std::env::var_os("COMPUTER_USE_RESTORE_FOCUS").is_some() {
            self.mouse.restore_focus();
        }

        let screenshot = if let Some(params) = options.screenshot_params {
            Some(screenshot::take(params)?)
        } else {
            None
        };

        Ok(ActionResult {
            screenshot,
            cursor_position: Some(self.mouse.current_position()?),
        })
    }
}
