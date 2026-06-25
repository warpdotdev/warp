// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

//! TTY related functionality.

pub mod docker_sandbox;
pub mod event_loop;
mod mio_channel;
pub mod recorder;
#[cfg(unix)]
pub mod server;
pub mod shell;
pub mod spawner;
#[cfg(unix)]
pub mod terminal_attributes;
pub mod terminal_manager;
mod terminal_view_adaptor;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
pub mod windows;

use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use shell::ShellStarter;

pub use self::terminal_manager::{get_shell_starter, TerminalManager};
#[cfg(windows)]
pub use self::terminal_view_adaptor::shutdown_all_pty_event_loops;
pub(crate) use self::terminal_view_adaptor::{
    create_terminal_view_surface, terminal_view_restored_blocks, TerminalViewSurfaceConfig,
};
#[cfg(unix)]
pub use self::unix::*;
#[cfg(windows)]
pub use self::windows::*;
use super::SizeInfo;

/// This trait defines the behaviour needed to read and/or write to a stream.
/// It defines an abstraction over mio's interface in order to allow either one
/// read/write object or a separate read and write object.
pub trait EventedReadWrite {
    type Reader: io::Read;
    type Writer: io::Write;

    fn register(&mut self, _: &mio::Poll, _: mio::Interest) -> io::Result<()>;
    fn reregister(&mut self, _: &mio::Poll, _: mio::Interest) -> io::Result<()>;
    fn deregister(&mut self, _: &mio::Poll) -> io::Result<()>;

    fn reader(&mut self) -> &mut Self::Reader;
    fn read_token(&self) -> mio::Token;
    fn writer(&mut self) -> &mut Self::Writer;
    fn write_token(&self) -> mio::Token;
}

/// Events concerning TTY child processes.
#[derive(Debug, PartialEq, Eq)]
pub enum ChildEvent {
    /// Indicates the child has exited.
    Exited,
}

/// A pseudoterminal (or PTY).
///
/// This is a refinement of EventedReadWrite that also provides a channel through which we can be
/// notified if the PTY child process does something we care about (other than writing to the TTY).
/// In particular, this allows for race-free child exit notification on UNIX (cf. `SIGCHLD`).
pub trait EventedPty: EventedReadWrite {
    fn child_event_token(&self) -> mio::Token;

    /// Tries to retrieve an event.
    ///
    /// Returns `Some(event)` on success, or `None` if there are no events to retrieve.
    fn next_child_event(&mut self) -> Option<ChildEvent>;

    /// Resize the PTY.
    ///
    /// Tells the kernel that the window size changed with the new pixel
    /// dimensions and line/column counts.
    fn on_resize(&mut self, size: &SizeInfo);

    /// Terminate the PTY process
    fn kill(self) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyOptions {
    pub size: SizeInfo,
    pub window_id: Option<usize>,
    pub shell_starter: ShellStarter,
    pub start_dir: Option<PathBuf>,
    /// Environment variables to add/override for the spawned PTY process.
    #[serde(default)]
    pub env_vars: HashMap<OsString, OsString>,
    // Refers to the original SSH wrapper that uses ControlMaster and
    // requires overwriting the user's SSH command at the shell layer.
    pub enable_ssh_wrapper: bool,
    /// Whether the legacy SSH wrapper should attach to an existing
    /// ControlMaster for the destination host (discovered via `ssh -G` and
    /// verified with `ssh -O check`) instead of always creating its own.
    #[serde(default)]
    pub reuse_ssh_control_master: bool,
    pub shell_debug_mode: bool,
    pub honor_ps1: bool,
    /// Whether the Node.js Version context chip is enabled for this session. When
    /// `false`, the shell bootstrap skips the per-prompt `node --version` detection
    /// (gated via the `WARP_PROMPT_NODE_VERSION_ENABLED` env var).
    pub node_version_chip_enabled: bool,
    pub close_fds: bool,
}
