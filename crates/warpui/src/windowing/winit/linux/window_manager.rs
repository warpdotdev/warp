use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::process::Stdio;
use std::{env, fs, path};

use command::blocking::Command;

/// Attempt to find a running process that we believe is the window compositor.
///
/// The name comes from `/proc/$pid/comm`, and so it will be truncated to the first 15 chars of the
/// actual process name.
/// https://superuser.com/questions/567648/ps-comm-format-always-cuts-the-process-name
pub(crate) fn look_for_wayland_compositor() -> Option<String> {
    // First, try to determine the compositor by looking at the Wayland display
    // socket and seeing which process is listening on it.
    //
    // TODO(CORE-3034): Re-enable this codepath once we've understood and
    // addressed the lsof performance issues.
    // if let Some(compositor_name) = get_wayland_compositor_from_socket() {
    //     return Some(compositor_name);
    // }

    // If the above method didn't work, fallback to a less precise method. Simply use `ps
    // -u` and grep for a recognized set of names among the running processes. This may
    // have false positives, like processes that name-clash with these compositors.
    let uid = nix::unistd::getuid();
    let euid = nix::unistd::geteuid();
    if let Some(ps_output) = Command::new("ps")
        .args(["-u", &format!("{euid}"), "-U", &format!("{uid}")])
        .stdout(Stdio::piped())
        .spawn()
        .ok()
        .and_then(|output| output.stdout)
    {
        let wm_match_cmd = Command::new("grep")
            .args(
                ["-m", "1", "-o", "-F", "-i"].iter().chain(
                    WAYLAND_TILING_WM
                        .iter()
                        .flat_map(|wm_name| [&"-e", wm_name]),
                ),
            )
            .stdin(Stdio::from(ps_output))
            .output()
            .ok()
            .filter(|out| out.status.success());

        if let Some(wm_name_raw) = wm_match_cmd {
            if let Ok(wm_name) = String::from_utf8(wm_name_raw.stdout) {
                if !wm_name.is_empty() {
                    return Some(wm_name);
                }
            }
        }
    }
    None
}

/// Returns the name of the Wayland compositor by looking at the Wayland
/// display socket and seeing which process is listening on it, or [`None`] if
/// we were unable to compute it for any reason.
///
/// TODO(CORE-3034): Re-enable this codepath and remove the allow(dead_code)
/// attribute.
#[allow(dead_code)]
fn get_wayland_compositor_from_socket() -> Option<String> {
    // https://discourse.ubuntu.com/t/environment-variables-for-wayland-hackers/12750
    let xdg_runtime_dir = env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|val| !val.is_empty())?;
    let wayland_display = env::var("WAYLAND_DISPLAY")
        .ok()
        .filter(|val| !val.is_empty())
        .unwrap_or("wayland-0".to_owned());

    // Wayland compositors communicate with their clients using a UNIX socket. This path is the
    // standard location of that socket.
    let wayland_socket_path = path::Path::new(xdg_runtime_dir.as_str()).join(wayland_display);
    let socket_metadata = fs::metadata(&wayland_socket_path).ok()?;

    // Validate that this file is a socket owned by the effective user ID.
    if !socket_metadata.file_type().is_socket()
        || socket_metadata.uid() != nix::unistd::geteuid().as_raw()
    {
        return None;
    }

    let path_str = wayland_socket_path.to_str()?;

    // If we found a valid socket, try either `lsof` or `fuser` to identify the process
    // which is listening at this socket. This is the most precise method of doing this,
    // but not all Linux systems have these tools installed, and if they do they may still
    // require elevated privileges.
    let get_pid_cmd = Command::new("lsof")
        .args(["-t", path_str])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
        .or_else(|| {
            Command::new("fuser")
                .arg(path_str)
                .stderr(Stdio::null())
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| output.stdout)
        });

    // If the above method worked, lookup the name of that pid.
    if let Some(raw_pid) = get_pid_cmd {
        let pid = String::from_utf8(raw_pid).ok()?.trim().to_owned();
        // Validate that an integer pid was returned.
        pid.parse::<i32>().ok()?;
        if let Ok(wm_name_raw) = Command::new("ps")
            .args(["-p", pid.as_str(), "-o", "comm="])
            .output()
        {
            if let Ok(wm_name) = String::from_utf8(wm_name_raw.stdout) {
                return Some(wm_name);
            }
        }
    }

    None
}

/// Hand-picked tiling wayland compositors. These are the two most starred on GitHub.
const WAYLAND_TILING_WM: &[&str] = &["hyprland", "sway"];

pub(crate) fn is_tiling_window_manager(name: &str) -> bool {
    // List of X11 tiling window managers copied from Chromium repo:
    // https://source.chromium.org/chromium/chromium/src/+/6fa59a48:ui/base/x/x11_util.cc;l=374
    const X11_TILING_WM: &[&str] = &["i3", "ion3", "notion", "ratpoison", "stumpwm"];
    // Dynamic window managers can be configured to function as either tiling or stacking. It is
    // impractical for us to introspect how these are configured, so for now we copy Chrome's
    // approach to assume they are used as tiling.
    const X11_DYNAMIC_WM: &[&str] = &["awesome", "qtile", "xmonad", "wmii"];

    let normalized = name.trim().to_lowercase();
    X11_TILING_WM.contains(&normalized.as_str())
        || X11_DYNAMIC_WM.contains(&normalized.as_str())
        || WAYLAND_TILING_WM.contains(&normalized.as_str())
}
