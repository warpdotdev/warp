//! Builds and caches the system command search PATH so that child processes
//! spawned by Warp can find user-installed binaries even when Warp is
//! launched from a GUI (Dock, Spotlight, Start menu) with a minimal
//! inherited PATH.
//!
//! Currently macOS-only: reads `/etc/paths` and `/etc/paths.d/*`, the same
//! mechanism macOS's `path_helper` uses for login shells. Other platforms
//! are a no-op for now (Windows inherits the full registry PATH
//! automatically; Linux support can be added later).

#[cfg(not(target_family = "wasm"))]
use std::sync::OnceLock;

/// Cached system PATH. Set once at startup via [`set_path`]; read by
/// consumers like `warp_util::git::run_git_command`.
#[cfg(not(target_family = "wasm"))]
static SYSTEM_PATH: OnceLock<String> = OnceLock::new();

/// Returns the cached system PATH, if one has been set.
#[cfg(not(target_family = "wasm"))]
pub fn path() -> Option<&'static str> {
    SYSTEM_PATH.get().map(|s| s.as_str())
}

/// Sets the system PATH that consumers will read. Only the first call
/// takes effect (`OnceLock`).
#[cfg(not(target_family = "wasm"))]
pub fn set_path(path: String) {
    let _ = SYSTEM_PATH.set(path);
}

/// Reads `/etc/paths` and `/etc/paths.d/*` to build the macOS system PATH,
/// the same way `path_helper` does for login shells. Appends the current
/// process PATH so nothing the launcher provided is lost.
///
/// Returns `None` if no entries were found (unlikely on a real macOS system).
#[cfg(target_os = "macos")]
pub fn build_macos_system_path() -> Option<String> {
    use std::fs;

    let mut dirs: Vec<String> = Vec::new();

    // /etc/paths — one directory per line
    if let Ok(contents) = fs::read_to_string("/etc/paths") {
        for line in contents.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                dirs.push(trimmed.to_string());
            }
        }
    }

    // /etc/paths.d/* — each file has one directory per line
    if let Ok(entries) = fs::read_dir("/etc/paths.d") {
        let mut paths_d_files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        paths_d_files.sort_by_key(|e| e.file_name());
        for entry in paths_d_files {
            if let Ok(contents) = fs::read_to_string(entry.path()) {
                for line in contents.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !dirs.contains(&trimmed.to_string()) {
                        dirs.push(trimmed.to_string());
                    }
                }
            }
        }
    }

    if dirs.is_empty() {
        return None;
    }

    // Append any entries from the inherited process PATH that aren't
    // already covered, so we never lose anything the launcher provided.
    if let Ok(current) = std::env::var("PATH") {
        for entry in current.split(':') {
            if !entry.is_empty() && !dirs.iter().any(|d| d == entry) {
                dirs.push(entry.to_string());
            }
        }
    }

    Some(dirs.join(":"))
}
