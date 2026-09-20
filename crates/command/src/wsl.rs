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
#[cfg(not(target_family = "wasm"))]
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
#[cfg(not(target_family = "wasm"))]
use std::sync::{LazyLock, Mutex, MutexGuard};
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

#[cfg(not(target_family = "wasm"))]
use instant::Instant;

#[cfg(not(target_family = "wasm"))]
use crate::r#async::{Command, OutputError};

#[cfg(not(target_family = "wasm"))]
const BACKGROUND_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(not(target_family = "wasm"))]
const BACKGROUND_COMMAND_BACKOFF: Duration = Duration::from_secs(30);

#[cfg(not(target_family = "wasm"))]
static BACKGROUND_COMMANDS: LazyLock<Mutex<HashMap<String, BackgroundCommandState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(not(target_family = "wasm"))]
#[derive(Debug, Default)]
struct BackgroundCommandState {
    in_flight: bool,
    retry_after: Option<Instant>,
}

#[cfg(not(target_family = "wasm"))]
struct BackgroundCommandLease {
    distribution: String,
    completed: bool,
}

#[cfg(not(target_family = "wasm"))]
impl BackgroundCommandLease {
    fn acquire(distribution: &str) -> Result<Self, BackgroundWslCommandError> {
        let now = Instant::now();
        let mut commands = background_commands();
        let state = commands.entry(distribution.to_owned()).or_default();
        if state.in_flight {
            return Err(BackgroundWslCommandError::AlreadyRunning);
        }
        if state
            .retry_after
            .is_some_and(|retry_after| retry_after > now)
        {
            return Err(BackgroundWslCommandError::BackingOff);
        }
        state.in_flight = true;
        state.retry_after = None;
        Ok(Self {
            distribution: distribution.to_owned(),
            completed: false,
        })
    }

    fn complete(&mut self) {
        self.completed = true;
    }
}

#[cfg(not(target_family = "wasm"))]
impl Drop for BackgroundCommandLease {
    fn drop(&mut self) {
        let mut commands = background_commands();
        let state = commands.entry(self.distribution.clone()).or_default();
        state.in_flight = false;
        if !self.completed {
            state.retry_after = Some(Instant::now() + BACKGROUND_COMMAND_BACKOFF);
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn background_commands() -> MutexGuard<'static, HashMap<String, BackgroundCommandState>> {
    BACKGROUND_COMMANDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
#[cfg(not(target_family = "wasm"))]
pub fn record_distribution_timeout(distribution: &str) {
    let mut commands = background_commands();
    let state = commands.entry(distribution.to_owned()).or_default();
    state.retry_after = Some(Instant::now() + BACKGROUND_COMMAND_BACKOFF);
}

#[cfg(not(target_family = "wasm"))]
#[derive(Debug, thiserror::Error)]
pub enum BackgroundWslCommandError {
    #[error("another background WSL command is already running for this distribution")]
    AlreadyRunning,
    #[error("background WSL commands are temporarily paused after an unresponsive command")]
    BackingOff,
    #[error(transparent)]
    Output(#[from] OutputError),
}

#[cfg(not(target_family = "wasm"))]
pub async fn output_background_command(
    command: &mut Command,
    distribution: &str,
) -> Result<std::process::Output, BackgroundWslCommandError> {
    output_background_command_with_timeout(command, distribution, BACKGROUND_COMMAND_TIMEOUT).await
}

#[cfg(not(target_family = "wasm"))]
async fn output_background_command_with_timeout(
    command: &mut Command,
    distribution: &str,
    timeout: Duration,
) -> Result<std::process::Output, BackgroundWslCommandError> {
    let mut lease = BackgroundCommandLease::acquire(distribution)?;
    match command.output_with_timeout(timeout).await {
        Ok(output) => {
            lease.complete();
            Ok(output)
        }
        Err(error) => Err(error.into()),
    }
}

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
