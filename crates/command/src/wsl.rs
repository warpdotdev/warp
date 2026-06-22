//! WSL detection and Linux-side binary resolution for subprocess
//! invocations made through this crate's [`Command`](crate::r#async::Command)
//! and [`Command`](crate::blocking::Command) wrappers.
//!
//! Warp ships as a Linux ELF that users routinely run inside WSL.
//! WSL's default `appendWindowsPath = true` (in `/etc/wsl.conf`)
//! puts directories like `/mnt/c/Program Files/Git/cmd/` on `PATH`,
//! so a bare `Command::new("git")` can resolve to Windows `git.exe`
//! through WSL interop. That path is dramatically slower, can
//! mishandle Linux paths, and breaks Linux-side hooks.
//!
//! [`translate_program_for_spawn`] is invoked from the wrappers'
//! `new` constructors and transparently substitutes the program
//! string when (a) we're inside WSL and (b) the program is a bare
//! name in [`KNOWN_NAMES`]. Path-qualified or unknown programs are
//! passed through unchanged. Resolution is cached for the life of
//! the process — PATH is effectively static for the host process.
//!
//! The same `/mnt/*` filtering precedent is used for `compgen` in
//! `app/src/terminal/model/session/command_executor/wsl_command_executor.rs`.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg(test)]
#[path = "wsl_tests.rs"]
mod tests;

/// Bare program names whose resolution Warp wants to override under
/// WSL. Anything not in this list is passed through unchanged.
const KNOWN_NAMES: &[&str] = &["git", "gh"];

/// True when the current process is running inside a WSL guest.
/// Cached for the life of the process.
pub fn is_wsl() -> bool {
    static IS_WSL: OnceLock<bool> = OnceLock::new();
    *IS_WSL.get_or_init(|| Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists())
}

/// Translate a `Command::new` program string. On WSL, bare names in
/// [`KNOWN_NAMES`] are resolved to the first executable on `PATH`
/// outside `/mnt/*`; everything else is returned unchanged so the OS
/// performs its normal lookup at spawn time.
pub(crate) fn translate_program_for_spawn(program: &OsStr) -> OsString {
    if !is_wsl() {
        return program.to_owned();
    }
    let Some(name) = known_bare_name(program) else {
        return program.to_owned();
    };
    cached_resolve(name).to_owned()
}

/// Returns the program's name when it's a bare entry in
/// [`KNOWN_NAMES`]. Paths and absolute names are filtered out so a
/// caller that already specified `/usr/bin/git` is not touched.
fn known_bare_name(program: &OsStr) -> Option<&'static str> {
    let s = program.to_str()?;
    if s.contains('/') || s.contains('\\') {
        return None;
    }
    KNOWN_NAMES.iter().copied().find(|&n| n == s)
}

fn cached_resolve(name: &'static str) -> &'static OsString {
    static GIT: OnceLock<OsString> = OnceLock::new();
    static GH: OnceLock<OsString> = OnceLock::new();
    let cell = match name {
        "git" => &GIT,
        "gh" => &GH,
        // `KNOWN_NAMES` is exhaustive over the static cells above; if
        // a new name is added there it must also be wired here.
        _ => unreachable!("unknown name {name:?}"),
    };
    cell.get_or_init(|| {
        let path_env = std::env::var_os("PATH");
        match resolve_binary_in_wsl_safe_path(name, path_env.as_deref(), true) {
            Some(p) => p.into_os_string(),
            None => {
                log::warn!(
                    "wsl: no Linux-side `{name}` found on PATH (excluding /mnt/*); \
                     falling back to bare `{name}` which may resolve to a Windows .exe"
                );
                OsString::from(name)
            }
        }
    })
}

/// Returns the first executable named `name` on `path_env`, skipping
/// any PATH entry under `/mnt/` when `is_wsl` is true. Returns `None`
/// if no acceptable match exists. Pure — exposed for unit testing
/// without depending on a real WSL host.
pub fn resolve_binary_in_wsl_safe_path(
    name: &str,
    path_env: Option<&OsStr>,
    is_wsl: bool,
) -> Option<PathBuf> {
    let path_env = path_env?;
    for dir in std::env::split_paths(path_env) {
        if is_wsl && dir.starts_with("/mnt") {
            continue;
        }
        let candidate = dir.join(name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    match std::fs::metadata(path) {
        Ok(md) => md.is_file() && (md.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable_file(_path: &Path) -> bool {
    // The WSL-safe resolver only runs on Linux. Other targets short-
    // circuit through `is_wsl() == false`, so this stub is unreachable
    // in practice — present only to keep the crate compiling.
    false
}
