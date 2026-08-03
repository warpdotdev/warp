mod keysym;
mod recording;
mod wayland;
mod x11;

use std::sync::OnceLock;

use async_trait::async_trait;
pub use recording::{Recorder, post_process_recording};
use warp_errors::report_error;

use crate::{ActionResult, Options, TargetedAction};

/// Returns true if a Wayland environment is available.
fn is_wayland_available() -> bool {
    std::env::var("WAYLAND_DISPLAY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Returns true if an X11 environment is available.
fn is_x11_available() -> bool {
    std::env::var("DISPLAY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

pub fn is_supported_on_current_platform() -> bool {
    is_wayland_available() || is_x11_available()
}

/// Reports whether background, per-window control is available. On X11 it is implemented with a
/// dedicated XInput2 (MPX) master device pair, so it requires an XI2-capable server; the Wayland
/// path drives inputs through XDG portals, which have no per-window targeting.
pub fn background_supported() -> bool {
    if is_wayland_available() || !is_x11_available() {
        return false;
    }
    // The probe opens an X connection; cache it since this is consulted on every agent request.
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(x11::probe_background_support)
}

/// Enumerates the on-screen windows so a caller can pick one to target. Only supported on X11;
/// returns an empty list on Wayland or when no display is reachable.
pub fn enumerate_windows() -> Vec<crate::WindowInfo> {
    if is_wayland_available() || !is_x11_available() {
        return Vec::new();
    }
    x11::enumerate_windows()
}

/// Lists on-screen windows as a formatted diagnostic string. Only supported on X11.
pub fn list_windows() -> Result<String, String> {
    if is_wayland_available() || !is_x11_available() {
        return Err("Window listing is only supported on X11.".to_string());
    }
    x11::list_windows()
}

pub struct Actor {
    inner: ActorInner,
}

enum ActorInner {
    /// Wayland environment (uses XDG portals for input and screenshots).
    Wayland(Box<wayland::Actor>),
    /// X11 environment (uses XTEST for input).
    X11(Box<x11::Actor>),
    /// No supported display server available.
    Unsupported,
}

impl Actor {
    pub fn new() -> Self {
        let inner = if is_wayland_available() {
            // On Wayland, use native XDG portals for input and screenshots.
            match wayland::Actor::new() {
                Ok(actor) => ActorInner::Wayland(Box::new(actor)),
                Err(e) => {
                    report_error!(anyhow::anyhow!(e).context("Failed to create Wayland actor"));
                    ActorInner::Unsupported
                }
            }
        } else if is_x11_available() {
            // Pure X11 environment.
            match x11::Actor::new() {
                Ok(actor) => ActorInner::X11(Box::new(actor)),
                Err(e) => {
                    report_error!(anyhow::anyhow!(e).context("Failed to create X11 actor"));
                    ActorInner::Unsupported
                }
            }
        } else {
            ActorInner::Unsupported
        };

        Self { inner }
    }
}

#[async_trait]
impl super::Actor for Actor {
    fn platform(&self) -> Option<super::Platform> {
        match &self.inner {
            ActorInner::Wayland(actor) => actor.platform(),
            ActorInner::X11(actor) => actor.platform(),
            ActorInner::Unsupported => None,
        }
    }

    async fn perform_actions(
        &mut self,
        actions: &[TargetedAction],
        options: Options,
    ) -> Result<ActionResult, String> {
        match &mut self.inner {
            ActorInner::Wayland(actor) => actor.perform_actions(actions, options).await,
            ActorInner::X11(actor) => actor.perform_actions(actions, options).await,
            ActorInner::Unsupported => Err(
                "Computer use is not available: No supported display server detected. \
                 X11 or Wayland is required."
                    .to_string(),
            ),
        }
    }
}
