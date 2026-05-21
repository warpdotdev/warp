#[cfg(unix)]
use anyhow::Context;
use anyhow::Result;
use warpui::{AppContext, Entity, SingletonEntity};

use crate::terminal::local_tty;
#[cfg(unix)]
use crate::terminal::local_tty::shell::{ShellStarter, ShellStarterSourceOrWslName};

#[cfg(target_os = "windows")]
use super::PseudoConsoleChild;
use super::{PtyOptions, PtySpawnResult};
#[cfg(unix)]
use {
    crate::report_error, crate::terminal::local_tty::server::TerminalServer, anyhow::bail,
    std::process::Child,
};
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

#[cfg(unix)]
pub fn run_local_terminal_smoke() -> Result<()> {
    use crate::terminal::{available_shells::AvailableShell, SizeInfo};
    use std::{collections::HashMap, ffi::OsString};

    let mut spawner = PtySpawner::new().context("failed to create terminal server spawner")?;
    let shell_starter: ShellStarter = ShellStarter::init(AvailableShell::default())
        .and_then(|starter| match starter {
            ShellStarterSourceOrWslName::Source(source) => Some(source.into()),
            ShellStarterSourceOrWslName::WSLName { .. } => None,
        })
        .context("failed to resolve a local Unix shell starter")?;
    let options = local_tty::PtyOptions {
        size: SizeInfo::new_without_font_metrics(24, 80),
        window_id: None,
        shell_starter,
        start_dir: Some(std::env::current_dir().context("failed to read current directory")?),
        env_vars: HashMap::<OsString, OsString>::new(),
        enable_ssh_wrapper: false,
        shell_debug_mode: false,
        honor_ps1: true,
        close_fds: true,
    };

    let (spawn_result, mut handle) = spawner
        .spawn_pty_without_app_context(options)
        .context("failed to spawn local terminal session")?;
    unsafe {
        libc::close(spawn_result.leader_fd);
    }
    handle
        .kill()
        .context("failed to stop local terminal session")?;
    spawner.prepare_for_app_termination();

    println!(
        "WARPER-001 local terminal smoke spawned and stopped pid {}",
        spawn_result.pid
    );
    Ok(())
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
        #[cfg(windows)] event_loop_tx: super::mio_channel::Sender<
            crate::terminal::writeable_pty::Message,
        >,
        _ctx: &mut AppContext,
    ) -> Result<(PtySpawnResult, Box<dyn PtyHandle>)> {
        #[cfg(unix)]
        if let Some(server) = &self.server {
            let result = Self::spawn_pty_via_server(server, options.clone()).context(
                "Failed to spawn pty via terminal server; falling back to spawning locally...",
            );
            if let Err(err) = result {
                report_error!(err);
            } else {
                return result;
            }
        }

        Self::spawn_pty_directly(
            options,
            #[cfg(windows)]
            event_loop_tx,
        )
    }

    #[cfg(unix)]
    fn spawn_pty_without_app_context(
        &self,
        options: PtyOptions,
    ) -> Result<(PtySpawnResult, Box<dyn PtyHandle>)> {
        if let Some(server) = &self.server {
            return Self::spawn_pty_via_server(server, options);
        }

        Self::spawn_pty_directly(options)
    }

    fn spawn_pty_directly(
        options: PtyOptions,
        #[cfg(windows)] event_loop_tx: super::mio_channel::Sender<
            crate::terminal::writeable_pty::Message,
        >,
    ) -> Result<(PtySpawnResult, Box<dyn PtyHandle>)> {
        let pty_spawn_info = local_tty::spawn(
            options,
            #[cfg(windows)]
            event_loop_tx,
        )?;
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
