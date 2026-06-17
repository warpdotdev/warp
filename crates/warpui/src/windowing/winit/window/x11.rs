use anyhow::anyhow;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, MonitorInfo};
use x11rb::protocol::xproto::{self, AtomEnum, ConnectionExt};
use x11rb::rust_connection::RustConnection;

pub(super) type PhysicalMonitorBounds = (PhysicalPosition<i16>, PhysicalSize<u16>);

/// Holds a mapping of field names to "atoms" in X11.
///
/// In X11, "atoms" are basically enums. They are integers that map to strings, primarily to save
/// network bandwidth (X11 does not assume the GUI and the server are running on the same host).
struct Atoms {
    /// For specifying the `UTF8_STRING` type. Confusingly, this is different from
    /// [`AtomEnum::STRING`].
    utf8_string: u32,
    /// For the `_NET_ACTIVE_WINDOW` property on the root window.
    net_active_window: u32,
    /// For targeting `_NET_SUPPORTING_WM_CHECK` window.
    net_supporting_wm_check: u32,
    /// For the `_NET_WM_NAME` property.
    net_wm_name: u32,
}

/// An X11 client so that we can talk to an Xorg server for more advanced functionality from a
/// desktop environment.
pub(super) struct X11Manager {
    conn: RustConnection,
    /// The index among a list of available screens which we are displaying on.
    ///
    /// A "screen" in X11 parlance is not the concept of a monitor as we typically consider it.
    /// Rather, if there are multiple monitors plugged in, they get pooled into a single, shared
    /// coordinate space called a "screen". This allows windows to span multiple displays, as X11
    /// does not assume that any window belongs to one monitor.
    /// https://docs.google.com/drawings/d/1XeYRd9I7liQMj9w17QQZoeHSNYBJ_U0wEQh-pS_4eKM
    screen_index: usize,
    atoms: Atoms,
}

impl X11Manager {
    pub(super) fn new() -> anyhow::Result<Self> {
        let (conn, screen_index) = RustConnection::connect(None)?;

        let utf8_string = conn.intern_atom(true, b"UTF8_STRING")?.reply()?.atom;
        let net_active_window = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")?
            .reply()?
            .atom;
        let net_supporting_wm_check = conn
            .intern_atom(true, b"_NET_SUPPORTING_WM_CHECK")?
            .reply()?
            .atom;
        let net_wm_name = conn.intern_atom(true, b"_NET_WM_NAME")?.reply()?.atom;

        Ok(Self {
            conn,
            screen_index,
            atoms: Atoms {
                net_active_window,
                net_supporting_wm_check,
                net_wm_name,
                utf8_string,
            },
        })
    }

    /// Determines the index among a list of monitors for the "active" monitor. It also returns
    /// metadata for that active monitor.
    ///
    /// "Active" here means the monitor which the focused window is on. This may not be a window of
    /// your application, but another app's window. Note that windows may span multiple monitors.
    /// In that case, we pick the monitor which has the most overlap with the focused window.
    pub(super) fn get_active_monitor(&self) -> anyhow::Result<(usize, PhysicalMonitorBounds)> {
        // This logic is ported from `xdotool`
        // https://github.com/jordansissel/xdotool/blob/7e02cef5d9216bd0ce69b44f62217b587cc7c31e/xdo.c#L208
        let active_window_id = self.get_active_window()?;

        // This determines if the active window is the child of another window, or a child of the
        // "root". Indeed, windows in X11 are hierarchical.
        let tree_reply = xproto::query_tree(&self.conn, active_window_id)?.reply()?;

        // The meaning of "get_geometry" depends on this window's position in the hierarchy. This
        // call gives us the "true" position only if the window is a child of the "root". If not,
        // it gives us an offset position from its parent window.
        // https://tronche.com/gui/x/xlib/window-information/XGetGeometry.html
        let active_window_geometry = xproto::get_geometry(&self.conn, active_window_id)?.reply()?;

        // If this window is a child of the "root", return the reported position.
        let absolute_window_origin = if tree_reply.parent == tree_reply.root {
            vec2f(
                active_window_geometry.x as f32,
                active_window_geometry.y as f32,
            )
        } else {
            // Otherwise, "flatten" or "translate" the coordinates to be relative to the root.
            // https://tronche.com/gui/x/xlib/window-information/XTranslateCoordinates.html
            let translate_reply =
                xproto::translate_coordinates(&self.conn, active_window_id, tree_reply.root, 0, 0)?
                    .reply()?;
            vec2f(translate_reply.dst_x as f32, translate_reply.dst_y as f32)
        };

        let active_window_bounds = RectF::new(
            absolute_window_origin,
            vec2f(
                active_window_geometry.width as f32,
                active_window_geometry.height as f32,
            ),
        );

        // Get the full list of monitors and calculate which one overlaps with the active window
        // the most.
        let monitors = self.get_monitors(active_window_id)?;
        let (i, monitor_bounds) = monitors
            .iter()
            .map(monitor_info_to_physical_bounds)
            .enumerate()
            .max_by(|(_, bounds_a), (_, bounds_b)| {
                let intersection_a = active_window_bounds
                    .intersection(physical_bounds_to_rect(bounds_a, 1.))
                    .unwrap_or_default();
                let intersection_b = active_window_bounds
                    .intersection(physical_bounds_to_rect(bounds_b, 1.))
                    .unwrap_or_default();
                rect_area(intersection_a).total_cmp(&rect_area(intersection_b))
            })
            .ok_or(anyhow!(
                "active window position doesn't fall on any windows"
            ))?;

        Ok((i, monitor_bounds))
    }

    pub(super) fn list_monitor_bounds(&self) -> anyhow::Result<Box<[PhysicalMonitorBounds]>> {
        let active_window_id = self.get_active_window()?;
        let mut monitors = self.get_monitors(active_window_id)?;
        // Ensure the primary display is first. This is not
        monitors.sort_by(|a, b| b.primary.cmp(&a.primary));
        Ok(monitors
            .iter()
            .map(monitor_info_to_physical_bounds)
            .collect())
    }

    fn get_monitors(&self, window: xproto::Window) -> anyhow::Result<Vec<MonitorInfo>> {
        // For most X11 calls, we reuse `self.conn` for the request. However, the response for
        // `get_monitors` gets cached for the client. Subsequest calls just read the cached value,
        // which doesn't seem to ever get invalidated. To ensure we read a fresh value, we
        // construct a fresh connection client for every request.
        let (conn, _) = RustConnection::connect(None)?;
        let monitors = randr::get_monitors(&conn, window, false)?.reply()?.monitors;
        Ok(monitors)
    }

    pub(super) fn os_window_manager_name(&self) -> anyhow::Result<String> {
        let wm_check = xproto::get_property(
            &self.conn,
            false,
            self.screen().root,
            self.atoms.net_supporting_wm_check,
            AtomEnum::WINDOW,
            0,
            1024,
        )?
        .reply()?
        .value32()
        .ok_or(anyhow!(
            "Error getting _NET_SUPPORTING_WM_CHECK. Invalid response format."
        ))?
        // X protocol responses are always iterators, even if the response is a single value.
        .next()
        .ok_or(anyhow!(
            "Error getting _NET_SUPPORTING_WM_CHECK. Received empty response."
        ))?;

        let wm_name_prop = xproto::get_property(
            &self.conn,
            false,
            wm_check,
            self.atoms.net_wm_name,
            self.atoms.utf8_string,
            0,
            1024,
        )?
        .reply()?;

        let wm_name = String::from_utf8(wm_name_prop.value)?;
        Ok(wm_name)
    }

    fn screen(&self) -> &xproto::Screen {
        &self.conn.setup().roots[self.screen_index]
    }

    /// Returns X11's window ID for the active window.
    ///
    /// The "active" window is the one which has keyboard focus.
    fn get_active_window(&self) -> anyhow::Result<xproto::Window> {
        let active_window_reply = xproto::get_property(
            &self.conn,
            false,
            self.screen().root,
            self.atoms.net_active_window,
            AtomEnum::WINDOW,
            0,
            1024,
        )?
        .reply()?;

        let active_window = active_window_reply
            .value32()
            .ok_or(anyhow!(
                "Error getting active window. Invalid response format."
            ))?
            .next();

        active_window.ok_or(anyhow!(
            "Error getting active window. Received empty response."
        ))
    }
}

fn monitor_info_to_physical_bounds(monitor: &MonitorInfo) -> PhysicalMonitorBounds {
    let origin = PhysicalPosition::new(monitor.x, monitor.y);
    let size = PhysicalSize::new(monitor.width, monitor.height);
    (origin, size)
}

pub(super) fn physical_bounds_to_rect(bounds: &PhysicalMonitorBounds, scale_factor: f32) -> RectF {
    let (origin, size) = bounds;
    let origin = vec2f(origin.x as f32, origin.y as f32) / scale_factor;
    let size = vec2f(size.width as f32, size.height as f32) / scale_factor;
    RectF::new(origin, size)
}

/// Computes the area of a [`RectF`].
fn rect_area(rect: RectF) -> f32 {
    rect.width() * rect.height()
}
