//! Windows implementation of computer use actions using the Win32 SendInput
//! API for input and GDI for screenshots.

mod dpi;
mod keyboard;
mod mouse;
mod screenshot;

use async_trait::async_trait;
use warpui_core::r#async::Timer;
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, HDESK, OpenInputDesktop,
};

// Video recording is not yet implemented on Windows; reuse the no-op recorder.
pub use crate::noop::Recorder;
use crate::{Action, ActionResult, Options, TargetedAction};

/// Returns whether computer_use can drive input on this machine right now.
///
/// Reports `false` when there is no accessible input desktop (e.g., the process is running under
/// Session 0 as a Windows service, the workstation is locked, or the user has switched to a
/// different secure desktop). In those cases `SendInput` silently no-ops and GDI desktop capture
/// fails, so we'd rather fail fast here than surface the error mid-action.
pub fn is_supported_on_current_platform() -> bool {
    probe_input_desktop_available()
}

/// Reports whether background, per-window control is available. The Windows input stack drives the
/// screen / foreground window, so per-window background control is unsupported.
pub fn background_supported() -> bool {
    false
}

/// Shared probe used by both [`is_supported_on_current_platform`] and [`Actor::new`] so the
/// "can we drive input right now?" logic lives in one place. This still runs the probe on each
/// call (it's a cheap `OpenInputDesktop` / `CloseDesktop` round-trip) — we don't cache it because
/// availability can change at runtime (workstation lock, secure desktop swap, Remote Desktop
/// reconnect).
fn probe_input_desktop_available() -> bool {
    InputDesktop::acquire().is_some()
}

/// RAII wrapper for an `HDESK` returned by `OpenInputDesktop`. Guarantees the handle is closed
/// (or at least that a close attempt is made and logged on failure) even if the caller returns
/// early. Modeled after the GDI handle guards in `screenshot.rs`.
struct InputDesktop(HDESK);

impl InputDesktop {
    fn acquire() -> Option<Self> {
        // SAFETY: `OpenInputDesktop` has no preconditions. We pass `false` for inheritance and
        // request no specific access (just probing for existence).
        let handle =
            unsafe { OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_ACCESS_FLAGS(0)) };
        handle.ok().map(Self)
    }
}

impl Drop for InputDesktop {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid HDESK returned by `OpenInputDesktop` and has not been
        // closed yet.
        unsafe {
            if let Err(e) = CloseDesktop(self.0) {
                log::warn!("CloseDesktop failed in InputDesktop::drop: {e}");
            }
        }
    }
}

/// Actor holds Keyboard/Mouse state unconditionally — both are cheap to construct and have no
/// side effects — so a `perform_actions` call can recover as soon as an input desktop is
/// reachable again, even if `Actor::new` ran while the desktop was temporarily inaccessible
/// (workstation locked at startup, RDP disconnect, etc.). Supportability is decided per call by
/// [`probe_input_desktop_available`] rather than being cached in the actor's shape.
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

impl Default for Actor {
    fn default() -> Self {
        Self::new()
    }
}

/// Error returned by `perform_actions` when the input desktop is inaccessible at call time
/// (workstation lock, secure desktop swap, RDP reconnect, Session 0 service, …).
const NO_INPUT_DESKTOP_ERROR: &str = "Computer use is not available: no accessible input desktop";

#[async_trait]
impl super::Actor for Actor {
    fn platform(&self) -> Option<super::Platform> {
        // Live probe so callers can use `platform().is_some()` as a current "can drive input"
        // signal. Matches the Linux `Unsupported`-returns-None convention.
        if probe_input_desktop_available() {
            Some(super::Platform::Windows)
        } else {
            None
        }
    }

    async fn perform_actions(
        &mut self,
        actions: &[TargetedAction],
        options: Options,
    ) -> Result<ActionResult, String> {
        // Probe at the top of every call so transient loss of the input desktop (workstation
        // lock, secure desktop swap, RDP reconnect) surfaces as a descriptive error instead of
        // letting `SendInput` silently no-op. Cheap `OpenInputDesktop`/`CloseDesktop` round-trip.
        if !probe_input_desktop_available() {
            return Err(NO_INPUT_DESKTOP_ERROR.to_string());
        }
        let keyboard = &mut self.keyboard;
        let mouse = &mut self.mouse;

        for targeted in actions {
            // Per-window targeting is not supported on Windows; act on the screen / foreground
            // window regardless of the requested target.
            let action: &Action = &targeted.action;
            match action {
                Action::Wait(duration) => {
                    Timer::after(*duration).await;
                }
                Action::MouseDown { button, at } => {
                    mouse.move_to(*at)?;
                    mouse.button_down(button)?;
                }
                Action::MouseUp { button } => mouse.button_up(button)?,
                Action::MouseMove { to } => mouse.move_to(*to)?,
                Action::MouseWheel {
                    at,
                    direction,
                    distance,
                } => {
                    mouse.move_to(*at)?;
                    mouse.scroll(direction, distance)?;
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
            Some(screenshot::take(params)?)
        } else {
            None
        };

        Ok(ActionResult::legacy(
            screenshot,
            Some(mouse.current_position()?),
        ))
    }
}
