mod keyboard;
mod keycode_cache;
mod mouse;
mod screenshot;
mod util;

use async_trait::async_trait;
use warpui_core::r#async::Timer;

use crate::{Action, ActionResult, Options};

pub fn is_supported_on_current_platform() -> bool {
    true
}

pub struct Actor {
    keyboard: keyboard::Keyboard,
    mouse: mouse::Mouse,
}

impl Actor {
    pub fn new() -> Self {
        Self {
            keyboard: keyboard::Keyboard::new(),
            mouse: mouse::Mouse::new(),
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
