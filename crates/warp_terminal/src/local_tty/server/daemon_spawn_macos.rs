//! Launches the terminal daemon as its own TCC responsible process, so macOS
//! privacy prompts for the daemon and its shells are not attributed to the app
//! instance that started it.

use std::ffi::{CStr, CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;

use anyhow::{Context, Result};
use libc::{c_char, c_int};

/// `int responsibility_spawnattrs_setdisclaim(posix_spawnattr_t *, int)`, a
/// private libSystem function.
pub(crate) type SetDisclaimFn =
    unsafe extern "C" fn(attr: *mut libc::posix_spawnattr_t, disclaim: c_int) -> c_int;

const SET_DISCLAIM_SYMBOL: &CStr = c"responsibility_spawnattrs_setdisclaim";
const DEV_NULL: &CStr = c"/dev/null";

/// Resolves `responsibility_spawnattrs_setdisclaim` at runtime, since it is
/// not part of the public SDK.
pub(crate) fn set_disclaim_fn() -> Option<SetDisclaimFn> {
    // SAFETY: the symbol name is a valid C string.
    let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, SET_DISCLAIM_SYMBOL.as_ptr()) };
    if symbol.is_null() {
        return None;
    }
    // SAFETY: the symbol is a function with the signature of `SetDisclaimFn`.
    Some(unsafe { std::mem::transmute::<*mut libc::c_void, SetDisclaimFn>(symbol) })
}

/// Spawns `program` with `args`, the current environment, null stdio, and no
/// other inherited fds, disclaiming responsibility for it, and waits for it.
pub(crate) fn spawn_disclaimed_and_wait(
    program: &Path,
    args: &[&OsStr],
    set_disclaim: SetDisclaimFn,
) -> Result<ExitStatus> {
    let program = to_cstring(program.as_os_str())?;
    let mut owned_args = vec![program.clone()];
    for arg in args {
        owned_args.push(to_cstring(arg)?);
    }
    let owned_env = std::env::vars_os()
        .map(|(key, value)| {
            let mut entry = key;
            entry.push("=");
            entry.push(value);
            to_cstring(&entry)
        })
        .collect::<Result<Vec<_>>>()?;
    let argv = null_terminated(&owned_args);
    let envp = null_terminated(&owned_env);

    let mut attr = SpawnAttr::new()?;
    let flags = libc::POSIX_SPAWN_CLOEXEC_DEFAULT
        | libc::POSIX_SPAWN_SETSIGMASK
        | libc::POSIX_SPAWN_SETSIGDEF;
    // SAFETY: `attr` is initialized; the signal sets are valid for writes.
    unsafe {
        check(
            libc::posix_spawnattr_setflags(&mut attr.0, flags as libc::c_short),
            "posix_spawnattr_setflags",
        )?;
        let mut empty: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut empty);
        check(
            libc::posix_spawnattr_setsigmask(&mut attr.0, &empty),
            "posix_spawnattr_setsigmask",
        )?;
        let mut defaults: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut defaults);
        libc::sigaddset(&mut defaults, libc::SIGPIPE);
        check(
            libc::posix_spawnattr_setsigdefault(&mut attr.0, &defaults),
            "posix_spawnattr_setsigdefault",
        )?;
        check(
            set_disclaim(&mut attr.0, 1),
            "responsibility_spawnattrs_setdisclaim",
        )?;
    }

    let mut actions = FileActions::new()?;
    for (fd, flags) in [
        (libc::STDIN_FILENO, libc::O_RDONLY),
        (libc::STDOUT_FILENO, libc::O_WRONLY),
        (libc::STDERR_FILENO, libc::O_WRONLY),
    ] {
        // SAFETY: `actions` is initialized and `DEV_NULL` is a valid C string.
        check(
            unsafe {
                libc::posix_spawn_file_actions_addopen(
                    &mut actions.0,
                    fd,
                    DEV_NULL.as_ptr(),
                    flags,
                    0,
                )
            },
            "posix_spawn_file_actions_addopen",
        )?;
    }

    let mut pid: libc::pid_t = 0;
    // SAFETY: every pointer is valid, and `argv` and `envp` are
    // null-terminated arrays of C strings that outlive the call.
    check(
        unsafe {
            libc::posix_spawn(
                &mut pid,
                program.as_ptr(),
                &actions.0,
                &attr.0,
                argv.as_ptr(),
                envp.as_ptr(),
            )
        },
        "posix_spawn",
    )?;
    wait_for(pid)
}

fn wait_for(pid: libc::pid_t) -> Result<ExitStatus> {
    let mut status: c_int = 0;
    loop {
        // SAFETY: `status` is valid for writes.
        if unsafe { libc::waitpid(pid, &mut status, 0) } == pid {
            return Ok(ExitStatus::from_raw(status));
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err).context("waitpid failed");
        }
    }
}

fn to_cstring(value: &OsStr) -> Result<CString> {
    CString::new(value.as_bytes()).context("argument contains a NUL byte")
}

fn null_terminated(values: &[CString]) -> Vec<*mut c_char> {
    values
        .iter()
        .map(|value| value.as_ptr().cast_mut())
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect()
}

fn check(result: c_int, what: &str) -> Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(result)).with_context(|| format!("{what} failed"))
    }
}

struct SpawnAttr(libc::posix_spawnattr_t);

impl SpawnAttr {
    fn new() -> Result<Self> {
        let mut attr: libc::posix_spawnattr_t = std::ptr::null_mut();
        // SAFETY: `attr` is valid for writes.
        check(
            unsafe { libc::posix_spawnattr_init(&mut attr) },
            "posix_spawnattr_init",
        )?;
        Ok(Self(attr))
    }
}

impl Drop for SpawnAttr {
    fn drop(&mut self) {
        // SAFETY: `self.0` was initialized by `posix_spawnattr_init`.
        unsafe { libc::posix_spawnattr_destroy(&mut self.0) };
    }
}

struct FileActions(libc::posix_spawn_file_actions_t);

impl FileActions {
    fn new() -> Result<Self> {
        let mut actions: libc::posix_spawn_file_actions_t = std::ptr::null_mut();
        // SAFETY: `actions` is valid for writes.
        check(
            unsafe { libc::posix_spawn_file_actions_init(&mut actions) },
            "posix_spawn_file_actions_init",
        )?;
        Ok(Self(actions))
    }
}

impl Drop for FileActions {
    fn drop(&mut self) {
        // SAFETY: `self.0` was initialized by `posix_spawn_file_actions_init`.
        unsafe { libc::posix_spawn_file_actions_destroy(&mut self.0) };
    }
}

#[cfg(test)]
#[path = "daemon_spawn_macos_tests.rs"]
mod tests;
