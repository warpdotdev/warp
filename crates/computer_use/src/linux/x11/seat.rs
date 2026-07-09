//! A dedicated "agent seat": an XInput2 (MPX) master pointer/keyboard pair used to drive
//! background windows without moving the user's cursor or stealing the user's keyboard focus.
//!
//! X11 has no equivalent of macOS's `CGEventPostToPid` (delivering events directly to a
//! process). The alternatives are `XSendEvent`-style synthetic events, which most toolkits
//! (GTK, Qt, Chromium, WINE) ignore for security reasons, or real server-generated input. This
//! module takes the latter path using Multi-Pointer X: the X server supports any number of
//! independent master pointer/keyboard pairs, each with its own on-screen cursor and its own
//! keyboard focus.
//!
//! The server routes all *core* input-related requests of a client — XTEST fake input,
//! `WarpPointer`, `QueryPointer`, `SetInputFocus` — through that client's "ClientPointer"
//! master pair. By creating a private master pair and pointing a dedicated connection's
//! ClientPointer at it (`XISetClientPointer`), the existing XTEST-based mouse and keyboard code
//! drives the agent seat unchanged: events are indistinguishable from real hardware input to
//! applications, while the user's own pointer, keyboard focus, and modifier state stay put.
//!
//! Unlike most X resources, master devices are server-global and outlive the connection that
//! created them, so the pair is removed explicitly on drop and stale pairs from crashed
//! processes are reaped on creation (identified by the process id embedded in the seat name).

use std::sync::atomic::{AtomicU64, Ordering};

use x11rb::connection::Connection;
use x11rb::protocol::xinput::{
    self, ChangeMode, ConnectionExt as _, DeviceType, HierarchyChange, HierarchyChangeData,
    HierarchyChangeDataAddMaster, HierarchyChangeDataRemoveMaster,
};
use x11rb::protocol::xproto;
use x11rb::rust_connection::RustConnection;

/// The prefix of agent seat device names. A seat is named `{PREFIX}{pid}-{sequence}` and the
/// server derives the individual device names from it (e.g. "… pointer", "… keyboard",
/// "… XTEST pointer"). The embedded pid identifies the owning process for stale-seat reaping.
const SEAT_NAME_PREFIX: &str = "warp-agent-cu-";

/// The XI2 device id wildcard meaning "all devices" in `XIQueryDevice`.
const ALL_DEVICES: u16 = 0;

/// A private master pointer/keyboard pair plus the dedicated connection whose ClientPointer is
/// set to it. All core input requests issued on [`AgentSeat::conn`] act on the agent's cursor
/// and keyboard focus instead of the user's.
pub struct AgentSeat {
    conn: RustConnection,
    master_keyboard: xinput::DeviceId,
    /// The paired master pointer. Retained for the `RemoveMaster` request on drop (removing
    /// either device of a pair removes both).
    master_pointer: xinput::DeviceId,
}

impl AgentSeat {
    /// Creates the agent seat: a fresh X connection plus a private master device pair, with the
    /// connection's ClientPointer set to the new pair so all of its core input requests route
    /// through the agent devices.
    pub fn new() -> Result<Self, String> {
        let (conn, _screen_index) =
            RustConnection::connect(None).map_err(|e| format!("Failed to connect to X11: {e}"))?;

        // XI 2.0 introduced the master/slave device hierarchy used here (X server >= 1.7,
        // released 2009). The supported version must be announced before other XI2 requests.
        let version = conn
            .xinput_xi_query_version(2, 2)
            .map_err(|e| format!("XInput2 extension not available: {e}"))?
            .reply()
            .map_err(|e| format!("XInput2 extension query failed: {e}"))?;
        if version.major_version < 2 {
            return Err(format!(
                "XInput2 is required for background window control, but the server only \
                 supports XInput {}.{}.",
                version.major_version, version.minor_version
            ));
        }

        // Best-effort: reap seats leaked by crashed processes so their cursors do not
        // accumulate on the display.
        remove_stale_seats(&conn);

        let name = format!(
            "{SEAT_NAME_PREFIX}{}-{}",
            std::process::id(),
            next_sequence()
        );
        create_master_pair(&conn, &name)?;
        let (master_pointer, master_keyboard) = find_master_pair(&conn, &name)?;

        // Route this connection's core requests (XTEST fake input, WarpPointer, QueryPointer,
        // SetInputFocus) through the agent master pair instead of the user's virtual core
        // pointer and keyboard. Window `None` selects the requesting client itself.
        conn.xinput_xi_set_client_pointer(x11rb::NONE, master_pointer)
            .map_err(|e| format!("Failed to select the agent pointer: {e}"))?
            .check()
            .map_err(|e| format!("Failed to select the agent pointer: {e}"))?;

        Ok(Self {
            conn,
            master_keyboard,
            master_pointer,
        })
    }

    /// The connection whose core input requests act on the agent seat.
    pub fn conn(&self) -> &RustConnection {
        &self.conn
    }

    /// Gives the agent keyboard focus to `window` so subsequent key events are delivered there.
    ///
    /// Only the agent master keyboard is affected: the user's keyboard focus is a property of
    /// their own master keyboard and stays where it is. The target application receives a
    /// regular `FocusIn`, so it treats itself as focused (accepting input, showing its caret)
    /// while the user's focused window does the same — the X11 analog of the macOS background
    /// activation dance.
    pub fn focus_window(&self, window: xproto::Window) -> Result<(), String> {
        self.conn
            .xinput_xi_set_focus(window, x11rb::CURRENT_TIME, self.master_keyboard)
            .map_err(|e| format!("Failed to focus target window {window}: {e}"))?
            .check()
            .map_err(|e| format!("Failed to focus target window {window}: {e}"))?;
        Ok(())
    }
}

impl Drop for AgentSeat {
    fn drop(&mut self) {
        // Master devices are server-global (they outlive this connection), so remove the pair
        // explicitly. Failures are ignored: `remove_stale_seats` reaps leftovers on the next
        // seat creation.
        let _ = remove_master(&self.conn, self.master_pointer);
        let _ = self.conn.flush();
    }
}

/// Returns a process-locally unique sequence number, so concurrent computer-use sessions in one
/// process get distinct seat names.
fn next_sequence() -> u64 {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

/// Creates a master pointer/keyboard pair named "{name} pointer" / "{name} keyboard". The
/// server also creates and attaches the matching XTEST slave devices, which is what makes XTEST
/// fake input work through the new pair.
fn create_master_pair(conn: &RustConnection, name: &str) -> Result<(), String> {
    let name = name.as_bytes().to_vec();
    // A hierarchy change carries its total wire length in 4-byte units: an 8-byte fixed header
    // plus the name, padded to a multiple of 4.
    let len = (8 + name.len()).div_ceil(4) as u16;
    let change = HierarchyChange {
        len,
        data: HierarchyChangeData::AddMaster(HierarchyChangeDataAddMaster {
            // Send core events so non-XI2 applications see the agent's input.
            send_core: true,
            enable: true,
            name,
        }),
    };
    conn.xinput_xi_change_hierarchy(&[change])
        .map_err(|e| format!("Failed to create the agent input devices: {e}"))?
        .check()
        .map_err(|e| format!("Failed to create the agent input devices: {e}"))?;
    Ok(())
}

/// Removes the master pair that `master_pointer` belongs to. The pair's XTEST slave devices are
/// destroyed with it; it has no other slaves that would need reattaching.
fn remove_master(conn: &RustConnection, master_pointer: xinput::DeviceId) -> Result<(), String> {
    let change = HierarchyChange {
        // The RemoveMaster change is a fixed 12 bytes = 3 four-byte units.
        len: 3,
        data: HierarchyChangeData::RemoveMaster(HierarchyChangeDataRemoveMaster {
            deviceid: master_pointer,
            return_mode: ChangeMode::FLOAT,
            return_pointer: 0,
            return_keyboard: 0,
        }),
    };
    conn.xinput_xi_change_hierarchy(&[change])
        .map_err(|e| format!("Failed to remove the agent input devices: {e}"))?
        .check()
        .map_err(|e| format!("Failed to remove the agent input devices: {e}"))?;
    Ok(())
}

/// Finds the device ids of the master pair created under `name`.
fn find_master_pair(
    conn: &RustConnection,
    name: &str,
) -> Result<(xinput::DeviceId, xinput::DeviceId), String> {
    let pointer_name = format!("{name} pointer");
    let keyboard_name = format!("{name} keyboard");
    let reply = conn
        .xinput_xi_query_device(ALL_DEVICES)
        .map_err(|e| format!("Failed to query input devices: {e}"))?
        .reply()
        .map_err(|e| format!("Failed to query input devices: {e}"))?;

    let mut pointer = None;
    let mut keyboard = None;
    for info in &reply.infos {
        let device_name = String::from_utf8_lossy(&info.name);
        if info.type_ == DeviceType::MASTER_POINTER && device_name == pointer_name {
            pointer = Some(info.deviceid);
        } else if info.type_ == DeviceType::MASTER_KEYBOARD && device_name == keyboard_name {
            keyboard = Some(info.deviceid);
        }
    }
    match (pointer, keyboard) {
        (Some(pointer), Some(keyboard)) => Ok((pointer, keyboard)),
        _ => Err("The agent input devices were not created as expected.".to_string()),
    }
}

/// Removes agent seats whose owning process no longer exists. A crashed process never runs
/// `Drop`, and its master devices (and their on-screen cursors) would otherwise persist until
/// the X server restarts.
fn remove_stale_seats(conn: &RustConnection) {
    let Some(reply) = conn
        .xinput_xi_query_device(ALL_DEVICES)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return;
    };
    for info in &reply.infos {
        if info.type_ != DeviceType::MASTER_POINTER {
            continue;
        }
        let device_name = String::from_utf8_lossy(&info.name);
        let Some(pid) = seat_pid(&device_name) else {
            continue;
        };
        // Live seats belonging to this process are in use by concurrent sessions; skip them.
        if pid == std::process::id() || process_alive(pid) {
            continue;
        }
        let _ = remove_master(conn, info.deviceid);
    }
    let _ = conn.flush();
}

/// Parses the owning pid out of a seat master pointer name like "warp-agent-cu-1234-0 pointer".
fn seat_pid(device_name: &str) -> Option<u32> {
    let rest = device_name.strip_prefix(SEAT_NAME_PREFIX)?;
    rest.split('-').next()?.parse().ok()
}

/// Reports whether a process with the given pid exists.
fn process_alive(pid: u32) -> bool {
    // Signal `None` performs error checking only. EPERM still means the process exists.
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None) {
        Ok(()) => true,
        Err(errno) => errno == nix::errno::Errno::EPERM,
    }
}
