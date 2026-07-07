use anyhow::Result;
use warpui::{AppContext, Entity, SingletonEntity};
#[cfg(unix)]
use {
    crate::report_error, crate::terminal::local_tty::server::TerminalServer, anyhow::bail,
    std::cmp::Reverse, std::collections::HashMap, std::ffi::OsString, std::process::Child,
};

#[cfg(target_os = "windows")]
use super::PseudoConsoleChild;
use super::{PtyOptions, PtySpawnResult};
use crate::send_telemetry_from_app_ctx;
use crate::server::telemetry::{PtySpawnMode, TelemetryEvent};
use crate::terminal::local_tty::{self};
/// A handle that can be used to interact with a pty process.
pub trait PtyHandle: Send + Sync {
    /// Returns the pty's process ID.
    fn pid(&self) -> u32;

    /// Returns whether or not the child process has terminated.  This may
    /// return false for an exited child (e.g.: for a server-hosted pty), but
    /// will never return true for a living child.
    fn has_process_terminated(&mut self) -> Result<bool>;

    /// Kills the pty process and waits for its successful termination.
    fn kill(&mut self) -> Result<()>;
}

/// A handle for a pty that is a direct child of the current process.
#[cfg(unix)]
struct DirectPtyHandle {
    child: Child,
}

#[cfg(unix)]
impl PtyHandle for DirectPtyHandle {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn has_process_terminated(&mut self) -> Result<bool> {
        // If the child has exited, try_wait will return Ok(Some(exit_status)).
        self.child
            .try_wait()
            .map(|inner| inner.is_some())
            .map_err(anyhow::Error::from)
    }

    fn kill(&mut self) -> Result<()> {
        self.child.kill()?;
        match self.child.wait() {
            Ok(_) => Ok(()),
            Err(err) => bail!(err),
        }
    }
}

#[cfg(target_os = "windows")]
struct DirectPtyHandle {
    child: PseudoConsoleChild,
}

#[cfg(target_os = "windows")]
impl PtyHandle for DirectPtyHandle {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn has_process_terminated(&mut self) -> Result<bool> {
        Ok(self.child.is_terminated())
    }

    fn kill(&mut self) -> Result<()> {
        // The logic to kill the process and file handles are fully contained in
        // EventedPty::kill().
        Ok(())
    }
}
/// Invokes the provided callback function without crash reporting enabled.
fn invoke_without_crash_reporting<T>(
    is_crash_reporting_enabled: bool,
    func: impl FnOnce() -> T,
) -> T {
    // Uninitialize cocoa-sentry before spawning the shell process to avoid passing any custom state
    // (such as BSD signal handlers and mach exception handlers) into the shell process. This means
    // we lose all Cocoa crash reports from now until when the session is successfully spawned,
    // which is not ideal but allows us to fully ensure that we don't improperly leak any Sentry state
    // into the child processes.
    #[cfg(feature = "crash_reporting")]
    crate::crash_reporting::uninit_cocoa_sentry();

    let retval = func();

    // Now that the child has spawned--reinitialize cocoa sentry.
    if is_crash_reporting_enabled {
        #[cfg(feature = "crash_reporting")]
        crate::crash_reporting::init_cocoa_sentry();
    }

    retval
}

pub(super) struct PtySpawnInfo {
    pub result: PtySpawnResult,
    #[cfg(unix)]
    pub child: Child,
    #[cfg(windows)]
    pub child: PseudoConsoleChild,
}

/// A global singleton that provides the ability to spawn ptys.
///
/// This abstracts away from callers the manner in which the pty is spawned -
/// depending on configuration, the pty might be spawned as a child of the
/// current process, or it may be spawned by a subprocess that is responsible
/// for owning and managing ptys.
pub struct PtySpawner {
    #[cfg(unix)]
    server: Option<TerminalServer>,
}

impl PtySpawner {
    /// Creates a new PtySpawner.
    ///
    /// This should be called extremely early in the application startup
    /// process - we want to minimize the number of already-obtained resources
    /// that could leak into forked subprocesses (e.g.: file descriptors).
    pub fn new() -> Result<Self> {
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                let server = super::server::TerminalServer::new()?;
                Ok(Self {
                    server: Some(server),
                })
            } else if #[cfg(target_os = "windows")] {
                Ok(Self {})
            } else {
                unreachable!("Spawning a PTY is not supported on this platform.");
            }
        }
    }

    /// Creates a new PtySpanwer that is configured for unit test purposes.
    pub fn new_for_test() -> Self {
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                Self{ server: None }
            } else if #[cfg(target_os = "windows")] {
                Self {}
            } else {
                unreachable!("Spawning a PTY for tests is not supported on this platform.");
            }
        }
    }

    /// Does any work necessary to clean up state in advance of the app
    /// terminating.
    pub fn prepare_for_app_termination(&mut self) {
        // Drop the backing `TerminalServer`, if one exists, killing the child
        // process.
        #[cfg(unix)]
        if let Some(server) = self.server.take() {
            log::info!("Tearing down terminal server...");
            drop(server);
        }
    }

    /// Spawns a pty, returning information about the pty and a handle that can
    /// be used to interact with the pty process.
    pub(super) fn spawn_pty(
        &self,
        options: PtyOptions,
        is_crash_reporting_enabled: bool,
        #[cfg(windows)] event_loop_tx: super::mio_channel::Sender<
            crate::terminal::writeable_pty::Message,
        >,
        ctx: &mut AppContext,
    ) -> Result<(PtySpawnResult, Box<dyn PtyHandle>)> {
        #[cfg(not(unix))]
        let is_fallback = false;
        #[cfg(unix)]
        let mut is_fallback = false;

        #[cfg(unix)]
        if let Some(server) = &self.server {
            let result = Self::spawn_pty_via_server(server, options.clone());
            if let Err(err) = result {
                // Log env var names + sizes for any terminal server failure.
                // Large env vars are the most common cause (E2BIG on Linux,
                // socket overflow on macOS), so logging them on every failure
                // makes both cases diagnosable from the logs.
                log_env_var_diagnostics(&options.env_vars);

                // E2BIG is deterministic — retrying from the main process with
                // the same PtyOptions would hit the same limit — so fail
                // immediately rather than falling back.
                if is_e2big(&err) {
                    // err already contains the original message. We add some context on how to fix.
                    return Err(err.context(
                        "This can happen when env vars in the image or Oz secrets are \
                         too long.",
                    ));
                }
                report_error!(err.context(
                    "Failed to spawn pty via terminal server; falling back to spawning locally...",
                ));
                is_fallback = true;
            } else {
                send_telemetry_from_app_ctx!(
                    TelemetryEvent::PtySpawned {
                        mode: PtySpawnMode::TerminalServer
                    },
                    ctx
                );
                return result;
            }
        }

        let mode = if is_fallback {
            PtySpawnMode::FallbackToDirect
        } else {
            PtySpawnMode::Direct
        };
        send_telemetry_from_app_ctx!(TelemetryEvent::PtySpawned { mode }, ctx);

        Self::spawn_pty_directly(
            options,
            #[cfg(windows)]
            event_loop_tx,
            is_crash_reporting_enabled,
        )
    }

    fn spawn_pty_directly(
        options: PtyOptions,
        #[cfg(windows)] event_loop_tx: super::mio_channel::Sender<
            crate::terminal::writeable_pty::Message,
        >,
        is_crash_reporting_enabled: bool,
    ) -> Result<(PtySpawnResult, Box<dyn PtyHandle>)> {
        let pty_spawn_info =
            invoke_without_crash_reporting(is_crash_reporting_enabled, move || {
                local_tty::spawn(
                    options,
                    #[cfg(windows)]
                    event_loop_tx,
                )
            })?;
        let direct_pty_handle = Box::new(DirectPtyHandle {
            child: pty_spawn_info.child,
        });
        Ok((pty_spawn_info.result, direct_pty_handle))
    }

    #[cfg(unix)]
    fn spawn_pty_via_server(
        server: &TerminalServer,
        options: PtyOptions,
    ) -> Result<(PtySpawnResult, Box<dyn PtyHandle>)> {
        use crate::terminal::local_tty::server::ServerOwnedPtyHandle;

        let client = server.client().clone();
        let result = client.spawn_pty(options)?;
        let handle = Box::new(ServerOwnedPtyHandle {
            pid: result.pid,
            client,
        });
        Ok((result, handle))
    }
}

impl Entity for PtySpawner {
    type Event = ();
}

impl SingletonEntity for PtySpawner {}

/// Returns `true` if the error (or any cause in its chain) indicates that
/// `execve` failed with `ENAMETOOLONG` / E2BIG ("Argument list too long").
///
/// The terminal-server IPC path stringifies the raw `io::Error`, so we cannot
/// reliably downcast; instead we check the formatted error message as a
/// fallback.
#[cfg(unix)]
fn is_e2big(err: &anyhow::Error) -> bool {
    // Try the "real" io::Error in the chain first (direct-spawn path).
    if err.chain().any(|e| {
        e.downcast_ref::<std::io::Error>()
            .and_then(|e| e.raw_os_error())
            .is_some_and(|code| code == libc::E2BIG)
    }) {
        return true;
    }
    // Fall back to string matching for the IPC path where the io::Error
    // was serialised to a String before being sent back to the client.
    let msg = format!("{err:#}");
    msg.contains("os error 7") || msg.contains("Argument list too long")
}

/// Logs the names and byte-lengths (not values) of env vars passed to the
/// shell. Called on any terminal server failure to aid diagnosis of oversized
/// env var / secret configurations (E2BIG on Linux, socket overflow on macOS).
#[cfg(unix)]
fn log_env_var_diagnostics(extra_env_vars: &HashMap<OsString, OsString>) {
    log::error!("Shell spawn env var diagnostics (names and sizes only, no values):");

    // Log the additional env vars supplied via PtyOptions.
    let mut extra: Vec<(&OsString, usize)> = extra_env_vars
        .iter()
        .map(|(k, v)| (k, k.len() + v.len() + 2))
        .collect();
    extra.sort_by_key(|(_, size)| Reverse(*size));
    log::error!("  PtyOptions env_vars ({} entries):", extra_env_vars.len());
    for (key, size) in extra.iter().take(20) {
        log::error!("    {:?} — {} bytes", key, size);
    }

    // Log the largest vars from the inherited process environment.
    let mut inherited: Vec<(OsString, usize)> = std::env::vars_os()
        .map(|(k, v)| {
            let size = k.len() + v.len() + 2;
            (k, size)
        })
        .collect();
    inherited.sort_by_key(|(_, size)| Reverse(*size));
    let total: usize = inherited.iter().map(|(_, s)| s).sum();
    log::error!(
        "  Inherited process env ({} vars, ~{} bytes total):",
        inherited.len(),
        total
    );
    for (key, size) in inherited.iter().take(20) {
        log::error!("    {:?} — {} bytes", key, size);
    }
}
