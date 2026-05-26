mod keyboard;
mod keycode_cache;
mod mouse;
mod post;
mod screenshot;
mod skylight;
mod util;
mod window;

use async_trait::async_trait;
use pathfinder_geometry::vector::Vector2I;
use warpui::r#async::Timer;

use post::PostTarget;
use util::main_display_scale_factor;

use crate::{Action, ActionResult, Options, Target, TargetedAction};

pub fn is_supported_on_current_platform() -> bool {
    true
}

/// Reports whether background, per-window control is available. On macOS the background input
/// stack (focus-without-raise + window-targeted posting) is present, so this is always true.
pub fn background_supported() -> bool {
    true
}

/// Enumerates the on-screen windows as crate-level [`crate::WindowInfo`] records.
pub fn enumerate_windows() -> Vec<crate::WindowInfo> {
    window::enumerate_windows()
}

/// Maps a computer-use [`Target`] to the lower-level [`PostTarget`] used for event delivery.
fn post_target_for(target: Target) -> PostTarget {
    match target {
        Target::Screen => PostTarget::HidTap,
        Target::Window { pid, .. } => PostTarget::Pid(pid as libc::pid_t),
    }
}

/// Returns a copy of `action` with its coordinates remapped for the given target.
///
/// For a `Window` target the incoming coordinates are window-local pixels in the captured window
/// screenshot's space; they are translated to global physical pixels that the mouse pipeline
/// expects. This assumes the window shares the main display's backing scale factor (single-
/// display); multi-display / mixed-scale handling is a follow-up. `Screen` targets and windows
/// that cannot be resolved are left unchanged (legacy global-pixel behavior).
fn remap_action_for_target(action: &Action, target: Target) -> Action {
    let Target::Window { window_id, .. } = target else {
        return action.clone();
    };
    let Some(info) = window::window_by_id(window_id) else {
        return action.clone();
    };
    let scale = main_display_scale_factor();
    // Diagnostics for the agent-driven coordinate-conversion investigation. Gated on
    // COMPUTER_USE_DEBUG and routed through `log` so it lands in the app's log file. The incoming
    // coordinate is treated as a window-local pixel in the captured-window image's space; note we
    // do NOT apply any inverse of the screenshot downscale here (that is the suspected gap).
    let log_remap = std::env::var_os("COMPUTER_USE_DEBUG").is_some();
    let remap = |p: Vector2I| {
        let global = Vector2I::new(
            (info.x * scale) as i32 + p.x(),
            (info.y * scale) as i32 + p.y(),
        );
        if log_remap {
            log::info!(
                "[computer_use] remap window#={window_id} in_coord=({},{}) \
                 window_bounds_pt=({:.1},{:.1},{:.1},{:.1}) display_scale={scale:.3} \
                 window_local_px=({},{}) -> global_px=({},{}) [no downscale inverse applied]",
                p.x(),
                p.y(),
                info.x,
                info.y,
                info.width,
                info.height,
                p.x(),
                p.y(),
                global.x(),
                global.y(),
            );
        }
        global
    };
    match action {
        Action::MouseMove { to } => Action::MouseMove { to: remap(*to) },
        Action::MouseDown { button, at } => Action::MouseDown {
            button: button.clone(),
            at: remap(*at),
        },
        Action::MouseWheel {
            at,
            direction,
            distance,
        } => Action::MouseWheel {
            at: remap(*at),
            direction: *direction,
            distance: *distance,
        },
        other => other.clone(),
    }
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
        // The post target now defaults to the HID event tap (legacy screen/frontmost behavior)
        // and is overridden per-action when an action targets a specific window.
        Self {
            keyboard: keyboard::Keyboard::new(PostTarget::HidTap),
            mouse: mouse::Mouse::new(PostTarget::HidTap),
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
        actions: &[TargetedAction],
        options: Options,
    ) -> Result<ActionResult, String> {
        // When background computer use is disabled, force the legacy full-screen path: ignore any
        // window target, deliver events through the HID tap, and treat coordinates as global
        // pixels. This keeps behavior byte-identical to the pre-existing implementation.
        let background = options.background_enabled;
        for targeted in actions {
            let target = if background {
                targeted.target
            } else {
                Target::Screen
            };

            // Route this action to its target: the HID tap for screen actions, or directly to the
            // owning process for a window action (without raising it or moving the cursor).
            let post_target = post_target_for(target);
            self.mouse.set_target(post_target);
            self.keyboard.set_target(post_target);

            // For a window target, translate window-local coordinates to global physical pixels.
            let action = remap_action_for_target(&targeted.action, target);
            match &action {
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

        let (screenshot, captured_window) = match options.screenshot_params {
            Some(mut params) => {
                // With background computer use disabled, never capture a specific window: force the
                // legacy main-display capture, which returns no captured-window metadata.
                if !background {
                    params.target = Target::Screen;
                }
                let (screenshot, captured) = screenshot::take(params)?;
                (Some(screenshot), captured)
            }
            None => (None, None),
        };

        Ok(ActionResult {
            screenshot,
            cursor_position: Some(self.mouse.current_position()?),
            // Refresh the window list so the caller has up-to-date targets to choose from. When
            // background computer use is disabled, omit it so the result matches the legacy shape.
            windows: if background {
                window::enumerate_windows()
            } else {
                Vec::new()
            },
            captured_window,
        })
    }
}
