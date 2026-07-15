//! X11 implementation of computer use actions using the XTEST extension.
//!
//! Screen-targeted actions drive the user's core pointer and keyboard, exactly like the
//! pre-existing implementation. Window-targeted actions (background computer use) drive a
//! private "agent seat" — a second master pointer/keyboard pair (see [`seat`]) — so a specific
//! window can be controlled without moving the user's cursor or stealing the user's keyboard
//! focus.

mod keyboard;
mod mouse;
mod screenshot;
mod seat;
pub(crate) mod windows;

use std::time::Duration;

use async_trait::async_trait;
use instant::Instant;
use pathfinder_geometry::vector::Vector2I;
use warpui_core::r#async::Timer;
use x11rb::connection::Connection;
use x11rb::protocol::xinput::ConnectionExt as _;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use crate::{Action, ActionResult, Options, Target, TargetedAction};

/// How often to re-check whether a raise took effect. Raising is redirected to the window
/// manager (when one is running), which restacks asynchronously.
const RAISE_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long to wait for a raise to take effect before failing the action.
const RAISE_TIMEOUT: Duration = Duration::from_millis(500);

/// An actor that performs computer use actions on X11.
pub struct Actor {
    conn: RustConnection,
    screen_index: usize,
    /// Cached keyboard mapping for this connection. This avoids querying the server on every
    /// character typed.
    keyboard_mapping: xproto::GetKeyboardMappingReply,
    /// The agent seat used for background window targets, created lazily by the first
    /// window-targeted action and removed when the actor is dropped.
    agent: Option<seat::AgentSeat>,
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
            agent: None,
        })
    }

    fn root_window(&self) -> xproto::Window {
        self.conn.setup().roots[self.screen_index].root
    }

    fn screen(&self) -> &xproto::Screen {
        &self.conn.setup().roots[self.screen_index]
    }
}

/// Probes whether the display supports the XInput2 device hierarchy required for background,
/// per-window control. Any X server since 1.7 (2009), including Xvfb, supports it.
pub fn probe_background_support() -> bool {
    let Ok((conn, _screen_index)) = RustConnection::connect(None) else {
        return false;
    };
    conn.xinput_xi_query_version(2, 2)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|version| version.major_version >= 2)
}

/// Enumerates the on-screen windows over a short-lived connection, for callers outside an
/// action batch. Returns an empty list when the display is unreachable.
pub fn enumerate_windows() -> Vec<crate::WindowInfo> {
    match RustConnection::connect(None) {
        Ok((conn, screen_index)) => {
            let root = conn.setup().roots[screen_index].root;
            windows::enumerate_windows(&conn, root)
        }
        Err(_) => Vec::new(),
    }
}

/// Lists the on-screen windows (id, pid, bounds, class, title) as a human-readable table for
/// CLI diagnostics.
pub fn list_windows() -> Result<String, String> {
    let (conn, screen_index) =
        RustConnection::connect(None).map_err(|e| format!("Failed to connect to X11: {e}"))?;
    let root = conn.setup().roots[screen_index].root;
    let mut out = String::from("window#     pid     bounds(x,y,w,h)  class  title\n");
    for w in windows::list_windows(&conn, root) {
        out.push_str(&format!(
            "{:<10}  {:<6}  ({},{},{},{})  {}  {}\n",
            w.window, w.pid, w.x, w.y, w.width, w.height, w.app_name, w.title,
        ));
    }
    Ok(out)
}

/// Ensures a button or scroll event at `point` (root coordinates) will be delivered to
/// `window`.
///
/// X11 delivers pointer events to the topmost window under the pointer — there is no
/// macOS-style "post directly to a background window" path — so if another window covers that
/// point, the target is raised (without giving it the user's keyboard focus) and the check is
/// retried until the restack takes effect.
async fn ensure_window_clickable_at(
    conn: &RustConnection,
    root: xproto::Window,
    window: xproto::Window,
    point: Vector2I,
) -> Result<(), String> {
    if windows::window_hit_at_point(conn, root, window, point)? {
        return Ok(());
    }

    windows::raise(conn, window)?;
    // Under a window manager the raise request is redirected to and executed by the WM, so
    // poll briefly for the restack to take effect.
    let start = Instant::now();
    loop {
        if windows::window_hit_at_point(conn, root, window, point)? {
            return Ok(());
        }
        if start.elapsed() >= RAISE_TIMEOUT {
            return Err(format!(
                "Target window {window} is covered by another window at ({}, {}) and raising it \
                 did not take effect.",
                point.x(),
                point.y(),
            ));
        }
        Timer::after(RAISE_POLL_INTERVAL).await;
    }
}

#[async_trait]
impl crate::Actor for Actor {
    fn platform(&self) -> Option<crate::Platform> {
        Some(crate::Platform::LinuxX11)
    }

    async fn perform_actions(
        &mut self,
        actions: &[TargetedAction],
        options: Options,
    ) -> Result<ActionResult, String> {
        // When background computer use is disabled, force the legacy full-screen path: ignore
        // any window target, drive the user's core pointer/keyboard, and capture only the root
        // window. This keeps behavior identical to the pre-existing implementation.
        let background = options.background_enabled;

        // Validate every target and create the agent seat up front, so a failure cannot leave
        // the batch half-applied.
        let mut needs_agent_seat = false;
        for targeted in actions {
            let target = if background {
                targeted.target
            } else {
                Target::Screen
            };
            match target {
                // A window target must carry a concrete window id. `0` is the "unknown"
                // sentinel produced by the CLI default and by unparseable wire ids; reject it
                // here rather than failing later in window resolution with an opaque message.
                Target::Window { window_id: 0, .. } => {
                    return Err(
                        "A window target requires a non-zero window id. Select a window from \
                         the enumerated window list."
                            .to_string(),
                    );
                }
                Target::Window { .. } => needs_agent_seat = true,
                Target::Screen => {}
            }
        }
        if needs_agent_seat && self.agent.is_none() {
            let agent_seat = seat::AgentSeat::new()
                .map_err(|e| format!("Background window control is unavailable: {e}"))?;
            self.agent = Some(agent_seat);
        }

        let root = self.root_window();
        // Input state for screen-targeted actions, driving the user's core pointer/keyboard.
        let mut screen_mouse = mouse::Mouse::new(&self.conn, root);
        let mut screen_keyboard = keyboard::Keyboard::new(&self.conn, &self.keyboard_mapping);
        // Input state for window-targeted actions, driving the agent seat. Keycode resolution
        // reuses the core keyboard mapping, which is what receiving applications use to
        // interpret keycodes regardless of the source device.
        let mut agent_io = self.agent.as_ref().map(|agent_seat| {
            (
                agent_seat,
                mouse::Mouse::new(agent_seat.conn(), root),
                keyboard::Keyboard::new(agent_seat.conn(), &self.keyboard_mapping),
            )
        });
        let mut last_mouse_position: Option<Vector2I> = None;

        for targeted in actions {
            let target = if background {
                targeted.target
            } else {
                Target::Screen
            };
            let action: &Action = &targeted.action;
            match target {
                Target::Screen => match action {
                    Action::Wait(duration) => {
                        Timer::after(*duration).await;
                    }
                    Action::MouseDown { button, at } => {
                        screen_mouse.move_to(*at)?;
                        screen_mouse.focus_window_under_pointer()?;
                        screen_mouse.button_down(button)?;
                        last_mouse_position = Some(*at);
                    }
                    Action::MouseUp { button } => {
                        screen_mouse.button_up(button)?;
                    }
                    Action::MouseMove { to } => {
                        screen_mouse.move_to(*to)?;
                        last_mouse_position = Some(*to);
                    }
                    Action::MouseWheel {
                        at,
                        direction,
                        distance,
                    } => {
                        screen_mouse.move_to(*at)?;
                        screen_mouse.scroll(direction, distance)?;
                        last_mouse_position = Some(*at);
                    }
                    Action::TypeText { text } => {
                        screen_keyboard.type_text(text)?;
                    }
                    Action::KeyDown { key } => {
                        screen_keyboard.key_down(key)?;
                    }
                    Action::KeyUp { key } => {
                        screen_keyboard.key_up(key)?;
                    }
                },
                Target::Window { window_id, .. } => {
                    // The seat was created above for any batch containing a window target.
                    let Some((agent_seat, agent_mouse, agent_keyboard)) = agent_io.as_mut() else {
                        return Err("Agent input devices are missing.".to_string());
                    };
                    // Window-target coordinates are window-local pixels; the agent pointer
                    // operates in root coordinates.
                    match action {
                        Action::Wait(duration) => {
                            Timer::after(*duration).await;
                        }
                        Action::MouseDown { button, at } => {
                            let at =
                                windows::window_local_to_root(&self.conn, root, window_id, *at)?;
                            ensure_window_clickable_at(&self.conn, root, window_id, at).await?;
                            // Mirror a real click's focus effect on the agent seat only, so the
                            // window accepts subsequent input while the user's focus stays put.
                            agent_seat.focus_window(window_id)?;
                            agent_mouse.move_to(at)?;
                            agent_mouse.button_down(button)?;
                            last_mouse_position = Some(at);
                        }
                        Action::MouseUp { button } => {
                            agent_mouse.button_up(button)?;
                        }
                        Action::MouseMove { to } => {
                            let to =
                                windows::window_local_to_root(&self.conn, root, window_id, *to)?;
                            agent_mouse.move_to(to)?;
                            last_mouse_position = Some(to);
                        }
                        Action::MouseWheel {
                            at,
                            direction,
                            distance,
                        } => {
                            let at =
                                windows::window_local_to_root(&self.conn, root, window_id, *at)?;
                            // Scroll events are delivered by pointer position just like button
                            // events, so the target must be on top at the scroll point too.
                            ensure_window_clickable_at(&self.conn, root, window_id, at).await?;
                            agent_mouse.move_to(at)?;
                            agent_mouse.scroll(direction, distance)?;
                            last_mouse_position = Some(at);
                        }
                        Action::TypeText { text } => {
                            agent_seat.focus_window(window_id)?;
                            agent_keyboard.type_text(text)?;
                        }
                        Action::KeyDown { key } => {
                            agent_seat.focus_window(window_id)?;
                            agent_keyboard.key_down(key)?;
                        }
                        // `KeyUp` always follows a `KeyDown` that already focused the window,
                        // and a lone `KeyUp` with no prior `KeyDown` is a no-op regardless.
                        Action::KeyUp { key } => {
                            agent_keyboard.key_up(key)?;
                        }
                    }
                }
            }
        }

        let (screenshot, captured_window) = match options.screenshot_params {
            Some(mut params) => {
                // With background computer use disabled, never capture a specific window: force
                // the legacy root-window capture, which returns no captured-window metadata.
                if !background {
                    params.target = Target::Screen;
                }
                match params.target {
                    Target::Window { window_id: 0, .. } => {
                        return Err(
                            "A window target requires a non-zero window id. Select a window \
                             from the enumerated window list."
                                .to_string(),
                        );
                    }
                    Target::Window { window_id, .. } => {
                        let (screenshot, captured) =
                            screenshot::take_window(&self.conn, root, window_id, params)?;
                        (Some(screenshot), captured)
                    }
                    Target::Screen => (
                        Some(screenshot::take(&self.conn, self.screen(), root, params)?),
                        None,
                    ),
                }
            }
            None => (None, None),
        };

        // Get the final mouse position.
        let cursor_position = if let Some(pos) = last_mouse_position {
            Some(pos)
        } else {
            Some(screen_mouse.current_position()?)
        };

        Ok(ActionResult {
            screenshot,
            cursor_position,
            // Refresh the window list so the caller has up-to-date targets to choose from. When
            // background computer use is disabled, omit it so the result matches the legacy
            // shape.
            windows: if background {
                windows::enumerate_windows(&self.conn, root)
            } else {
                Vec::new()
            },
            captured_window,
        })
    }
}
