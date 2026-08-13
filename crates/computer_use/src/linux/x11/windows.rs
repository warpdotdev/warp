//! Window enumeration and geometry helpers for X11 background computer use.
//!
//! Windows are enumerated via the EWMH client list maintained by the window manager, falling
//! back to walking the root window's children on window-manager-less servers (e.g. bare Xvfb in
//! cloud environments). All coordinates are physical pixels; window-local coordinates have their
//! origin at the top-left of the window's content box.

use pathfinder_geometry::vector::Vector2I;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    self, AtomEnum, ConnectionExt as _, MapState, StackMode, WindowClass,
};
use x11rb::rust_connection::RustConnection;

/// The geometry of a window's content box, with `x`/`y` in root-window (screen) coordinates.
#[derive(Clone, Copy, Debug)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
    /// The window's border width in pixels. Needed by window capture: the composite backing
    /// pixmap includes the border, so the content box starts at `(border_width, border_width)`.
    pub border_width: u16,
}

/// The EWMH atoms used for window enumeration, interned once per enumeration.
struct Atoms {
    net_client_list_stacking: xproto::Atom,
    net_client_list: xproto::Atom,
    net_wm_name: xproto::Atom,
    utf8_string: xproto::Atom,
    net_wm_pid: xproto::Atom,
}

impl Atoms {
    fn new(conn: &RustConnection) -> Result<Self, String> {
        // Send all intern requests before reading any reply to avoid serial round-trips.
        let cookies = [
            "_NET_CLIENT_LIST_STACKING",
            "_NET_CLIENT_LIST",
            "_NET_WM_NAME",
            "UTF8_STRING",
            "_NET_WM_PID",
        ]
        .map(|name| conn.intern_atom(false, name.as_bytes()));
        let mut atoms = [0 as xproto::Atom; 5];
        for (atom, cookie) in atoms.iter_mut().zip(cookies) {
            *atom = cookie
                .map_err(|e| format!("Failed to intern atom: {e}"))?
                .reply()
                .map_err(|e| format!("Failed to intern atom: {e}"))?
                .atom;
        }
        let [
            net_client_list_stacking,
            net_client_list,
            net_wm_name,
            utf8_string,
            net_wm_pid,
        ] = atoms;
        Ok(Self {
            net_client_list_stacking,
            net_client_list,
            net_wm_name,
            utf8_string,
            net_wm_pid,
        })
    }
}

/// Enumerates the on-screen top-level windows as crate-level [`crate::WindowInfo`] records, so
/// the agent can pick a window to target. Ordered front-to-back.
pub fn enumerate_windows(conn: &RustConnection, root: xproto::Window) -> Vec<crate::WindowInfo> {
    list_windows(conn, root)
        .into_iter()
        .map(|w| crate::WindowInfo {
            window_id: w.window,
            pid: w.pid,
            app_name: w.app_name,
            title: w.title,
            // X11 has no per-window layer concept for normal client windows; report the normal
            // application layer for every window.
            layer: 0,
        })
        .collect()
}

/// A description of an on-screen window, for enumeration and diagnostics.
pub struct WindowDescription {
    pub window: xproto::Window,
    pub pid: i32,
    pub app_name: String,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
}

/// Lists the viewable top-level windows, front-to-back, with their metadata and geometry.
pub fn list_windows(conn: &RustConnection, root: xproto::Window) -> Vec<WindowDescription> {
    let Ok(atoms) = Atoms::new(conn) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for window in top_level_windows(conn, root, &atoms) {
        // Only include viewable windows: pointer events can only be delivered to mapped
        // windows, and the agent should not target iconified or withdrawn windows.
        let Some(attributes) = conn
            .get_window_attributes(window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        else {
            continue;
        };
        if attributes.map_state != MapState::VIEWABLE || attributes.class == WindowClass::INPUT_ONLY
        {
            continue;
        }
        let Ok(geometry) = geometry(conn, root, window) else {
            continue;
        };
        out.push(WindowDescription {
            window,
            pid: window_pid(conn, &atoms, window).unwrap_or(0),
            app_name: window_class(conn, window).unwrap_or_default(),
            title: window_title(conn, &atoms, window).unwrap_or_default(),
            x: geometry.x,
            y: geometry.y,
            width: geometry.width,
            height: geometry.height,
        });
    }
    out
}

/// Returns candidate top-level client windows, ordered front-to-back.
fn top_level_windows(
    conn: &RustConnection,
    root: xproto::Window,
    atoms: &Atoms,
) -> Vec<xproto::Window> {
    // Prefer the EWMH client lists maintained by the window manager, which contain the client
    // windows themselves (not the WM's frame windows). `_NET_CLIENT_LIST_STACKING` is
    // bottom-to-top stacking order; `_NET_CLIENT_LIST` is initial-mapping order, for which
    // reversal is still the better front-to-back approximation.
    for atom in [atoms.net_client_list_stacking, atoms.net_client_list] {
        let Some(reply) = read_property(conn, root, atom, AtomEnum::WINDOW.into()) else {
            continue;
        };
        let Some(values) = reply.value32() else {
            continue;
        };
        let mut list: Vec<xproto::Window> = values.collect();
        if !list.is_empty() {
            list.reverse();
            return list;
        }
    }

    // No EWMH window manager (e.g. bare Xvfb): fall back to the root window's direct children,
    // which `QueryTree` returns bottom-to-top. Override-redirect windows (menus, tooltips) are
    // excluded to mirror what an EWMH client list would contain.
    let Some(reply) = conn
        .query_tree(root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return Vec::new();
    };
    let mut children = reply.children;
    children.reverse();
    children.retain(|&window| {
        conn.get_window_attributes(window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|attributes| !attributes.override_redirect)
    });
    children
}

/// The maximum property length requested by [`read_property`], in 32-bit units (1 MiB total).
/// The properties read during enumeration (client lists, titles, `WM_CLASS`, `_NET_WM_PID`) are
/// far smaller in practice; the cap keeps a window with a pathologically large property from
/// forcing large allocations.
const MAX_PROPERTY_LENGTH: u32 = 256 * 1024;

/// Reads a property, returning `None` when the property is missing or the request fails.
/// Reads at most [`MAX_PROPERTY_LENGTH`] 32-bit units; anything beyond that is truncated.
fn read_property(
    conn: &RustConnection,
    window: xproto::Window,
    property: xproto::Atom,
    type_: xproto::Atom,
) -> Option<xproto::GetPropertyReply> {
    let reply = conn
        .get_property(false, window, property, type_, 0, MAX_PROPERTY_LENGTH)
        .ok()?
        .reply()
        .ok()?;
    (reply.format != 0).then_some(reply)
}

/// Reads the window title, preferring the UTF-8 EWMH title over the legacy `WM_NAME`.
fn window_title(conn: &RustConnection, atoms: &Atoms, window: xproto::Window) -> Option<String> {
    if let Some(reply) = read_property(conn, window, atoms.net_wm_name, atoms.utf8_string)
        && reply.format == 8
    {
        return Some(String::from_utf8_lossy(&reply.value).into_owned());
    }
    let reply = read_property(conn, window, AtomEnum::WM_NAME.into(), AtomEnum::ANY.into())?;
    (reply.format == 8).then(|| String::from_utf8_lossy(&reply.value).into_owned())
}

/// Reads the owning process id from `_NET_WM_PID`, if the application published one.
fn window_pid(conn: &RustConnection, atoms: &Atoms, window: xproto::Window) -> Option<i32> {
    let reply = read_property(conn, window, atoms.net_wm_pid, AtomEnum::CARDINAL.into())?;
    reply.value32()?.next().map(|pid| pid as i32)
}

/// Reads the application name from `WM_CLASS` (the class part, falling back to the instance).
fn window_class(conn: &RustConnection, window: xproto::Window) -> Option<String> {
    let reply = read_property(
        conn,
        window,
        AtomEnum::WM_CLASS.into(),
        AtomEnum::STRING.into(),
    )?;
    if reply.format != 8 {
        return None;
    }
    // WM_CLASS holds two null-terminated strings: the instance name, then the class name.
    let mut parts = reply.value.split(|&b| b == 0).filter(|s| !s.is_empty());
    let instance = parts.next();
    let class = parts.next().or(instance)?;
    Some(String::from_utf8_lossy(class).into_owned())
}

/// Returns the geometry of `window`'s content box, with its origin in root coordinates.
pub fn geometry(
    conn: &RustConnection,
    root: xproto::Window,
    window: xproto::Window,
) -> Result<WindowGeometry, String> {
    let geometry = conn
        .get_geometry(window)
        .map_err(|e| format!("Failed to query geometry of window {window}: {e}"))?
        .reply()
        .map_err(|e| format!("Failed to query geometry of window {window}: {e}"))?;
    // `GetGeometry` coordinates are relative to the window's parent (e.g. a WM frame), so
    // translate the content-box origin into root coordinates.
    let translated = conn
        .translate_coordinates(window, root, 0, 0)
        .map_err(|e| format!("Failed to locate window {window}: {e}"))?
        .reply()
        .map_err(|e| format!("Failed to locate window {window}: {e}"))?;
    Ok(WindowGeometry {
        x: i32::from(translated.dst_x),
        y: i32::from(translated.dst_y),
        width: geometry.width,
        height: geometry.height,
        border_width: geometry.border_width,
    })
}

/// Translates window-local pixel coordinates to root (screen) coordinates, validating that the
/// point lies inside the window's content box. Unlike macOS's process-targeted event posting,
/// X11 pointer events are delivered by screen position, so an out-of-bounds point would land on
/// an unrelated window; reject it instead.
pub fn window_local_to_root(
    conn: &RustConnection,
    root: xproto::Window,
    window: xproto::Window,
    point: Vector2I,
) -> Result<Vector2I, String> {
    let geometry = geometry(conn, root, window)?;
    if point.x() < 0
        || point.y() < 0
        || point.x() >= i32::from(geometry.width)
        || point.y() >= i32::from(geometry.height)
    {
        return Err(format!(
            "Coordinates ({}, {}) are outside target window {window}'s {}x{} bounds.",
            point.x(),
            point.y(),
            geometry.width,
            geometry.height
        ));
    }
    Ok(Vector2I::new(
        geometry.x + point.x(),
        geometry.y + point.y(),
    ))
}

/// Reports whether a pointer event at `point` (root coordinates) would be delivered to `window`
/// or one of its descendants.
///
/// The X server picks the destination of a pointer event by walking the chain of mapped windows
/// containing the point from the root down; this performs the same walk and checks whether the
/// target window is part of the chain (which also handles window-manager frames that reparent
/// the target).
pub fn window_hit_at_point(
    conn: &RustConnection,
    root: xproto::Window,
    window: xproto::Window,
    point: Vector2I,
) -> Result<bool, String> {
    let mut current = root;
    // The chain cannot be deeper than the window tree; bound the walk defensively in case the
    // tree mutates while we walk it.
    for _ in 0..1024 {
        if current == window {
            return Ok(true);
        }
        let reply = conn
            .translate_coordinates(root, current, point.x() as i16, point.y() as i16)
            .map_err(|e| format!("Failed to hit-test window {window}: {e}"))?
            .reply()
            .map_err(|e| format!("Failed to hit-test window {window}: {e}"))?;
        if reply.child == x11rb::NONE {
            return Ok(false);
        }
        current = reply.child;
    }
    Ok(false)
}

/// Raises `window` to the top of the stacking order without changing any keyboard focus.
///
/// Under a window manager the request is redirected to the WM, which generally honors client
/// raise requests (focus-stealing prevention may deny it, so callers must re-check visibility
/// afterwards). Without a WM the server restacks the window directly.
pub fn raise(conn: &RustConnection, window: xproto::Window) -> Result<(), String> {
    conn.configure_window(
        window,
        &xproto::ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    )
    .map_err(|e| format!("Failed to raise window {window}: {e}"))?;
    conn.flush()
        .map_err(|e| format!("Failed to flush X11 connection: {e}"))?;
    Ok(())
}
