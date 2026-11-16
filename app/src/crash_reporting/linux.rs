use crate::crash_reporting::VirtualEnvironment;

/// Returns what virtualized environment Warp is running in, if any.
pub fn get_virtualized_environment() -> Option<VirtualEnvironment> {
    if let Ok(output) = command::blocking::Command::new("systemd-detect-virt").output() {
        if !output.status.success() {
            return None;
        } else {
            let value = std::str::from_utf8(&output.stdout).ok()?;
            if value == "none" {
                return None;
            }
            return Some(VirtualEnvironment {
                name: value.to_owned(),
            });
        }
    };

    if command::wsl::is_wsl() {
        return Some(VirtualEnvironment {
            name: "wsl".to_owned(),
        });
    }

    None
}
