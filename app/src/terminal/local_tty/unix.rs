// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

//! TTY related functionality.
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{DirBuilder, File};
use std::mem::MaybeUninit;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::{io, ptr};

use anyhow::{Context as _, Error, Result};
use command::blocking::Command;
use itertools::Itertools;
use libc::{self, c_int, winsize, TIOCSCTTY};
use mio::unix::SourceFd;
use mio::Interest;
use nix::pty::openpty;
use nix::sys::termios::{self, InputFlags, SetArg};
use serde::{Deserialize, Serialize};
use signal_hook_mio::v1_0::Signals;
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warp_core::safe_error;
use warp_errors::{report_error, report_if_error};
use warpui::{AppContext, SingletonEntity};

use super::event_loop::{PTY_TOKEN, SIGNALS_TOKEN};
use super::spawner::{PtyHandle, PtySpawnInfo, PtySpawner};
use super::{ChildEvent, EventedPty, EventedReadWrite, PtyOptions, SizeInfo};
use crate::terminal::bootstrap::raw_init_shell_script_for_shell;
use crate::terminal::cli_agent_sessions::event::current_protocol_version;
use crate::terminal::local_tty::docker_sandbox::{
    DockerSandboxShellStarter, DOCKER_SANDBOX_HOME_DIR,
};
use crate::terminal::local_tty::shell::{
    extra_path_entries, ssh_socket_dir, DirectShellStarter, ShellStarter,
};
use crate::terminal::model::session::command_executor::shell_escape_single_quotes;
use crate::terminal::shell::ShellType;
use crate::ASSETS;

const BASH_HISTORY_SIZE_SENTINEL: &str = "57265949261";

/// Get raw fds for leader/follower ends of a new PTY.
fn make_pty(size: winsize) -> Result<(RawFd, RawFd)> {
    let mut win_size = size;
    win_size.ws_xpixel = 0;
    win_size.ws_ypixel = 0;

    let ends = openpty(Some(&win_size), None).context("openpty failed")?;
    // Configure the two new file descriptors to be closed on exec.  This keeps
    // us from leaking tty fds into spawned shells.  FD_CLOEXEC is _not_ shared
    // across duplicated fds, so when we call `libc::dup2()` below, those fds
    // will _not_ be closed when we exec the shell.
    unsafe {
        libc::fcntl(ends.master, libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(ends.slave, libc::F_SETFD, libc::FD_CLOEXEC);
    }

    Ok((ends.master, ends.slave))
}

fn docker_sandbox_run_args(starter: &DockerSandboxShellStarter) -> Vec<std::ffi::OsString> {
    let init_dir = starter.init_dir();
    let init_path = starter.init_path();
    let workspace_dir = starter.workspace_dir();
    let mount_arg = format!("{}:ro", init_dir.display());
    // Single-quote the init path in the bash command so paths containing
    // spaces (e.g. macOS's `~/Library/Application Support/...`) don't get
    // split on whitespace when bash parses the `-c <cmd>` string.
    let init_path_quoted = format!(
        "'{}'",
        shell_escape_single_quotes(&init_path.to_string_lossy(), ShellType::Bash)
    );
    let bash_cmd = format!(
        "cd {DOCKER_SANDBOX_HOME_DIR} && exec bash --rcfile {init_path_quoted} --noprofile",
    );

    let mut args = vec![std::ffi::OsString::from("run")];
    // Override sbx's default agent image with the environment's base image
    // when one is provided. `None` means "use sbx's default image".
    if let Some(base_image) = starter.base_image() {
        args.push(std::ffi::OsString::from("--template"));
        args.push(std::ffi::OsString::from(base_image));
    }
    args.extend([
        std::ffi::OsString::from("--name"),
        std::ffi::OsString::from(starter.sandbox_name()),
        std::ffi::OsString::from("shell"),
        workspace_dir.into_os_string(),
        std::ffi::OsString::from(mount_arg),
        std::ffi::OsString::from("--"),
        std::ffi::OsString::from("-c"),
        std::ffi::OsString::from(bash_cmd),
    ]);
    args
}

/// The current user's password-database record, resolved for shell/session
/// setup. Fields are owned so the record outlives any transient lookup buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CurrentUser {
    pub(super) name: String,
    pub(super) dir: String,
    pub(super) shell: String,
}

/// Resolves the current uid's passwd record for shell/session setup.
///
/// Resolution order, stopping at the first hit:
/// 1. In-process passwd lookup (`getpwuid_r`, via nix) — fast, and correct
///    wherever the process can resolve users itself (local `/etc/passwd`, or a
///    glibc-dynamic build that can load NSS plugins).
/// 2. `getent passwd <uid>` — delegates to the host's own NSS stack, so
///    centrally-managed users (SSSD/LDAP/AD) still resolve even from a
///    static/musl binary that can't `dlopen` glibc NSS plugins in-process.
/// 3. `/etc/passwd` — last resort for minimal hosts that lack `getent`.
///
/// Returns `None` only if all three fail; callers then fall back to the ambient
/// environment (`$HOME`/`$USER`) or built-in shell defaults.
pub(super) fn resolve_current_user() -> Option<CurrentUser> {
    let uid = nix::unistd::getuid();
    current_user_via_getpwuid(uid)
        .or_else(|| current_user_via_getent(uid.as_raw()))
        .or_else(|| current_user_from_passwd_file(uid.as_raw()))
}

/// Resolve the current user with an in-process passwd lookup via nix's
/// safe [`nix::unistd::User::from_uid`] wrapper (backed by `getpwuid_r`).
fn current_user_via_getpwuid(uid: nix::unistd::Uid) -> Option<CurrentUser> {
    match nix::unistd::User::from_uid(uid) {
        Ok(Some(user)) => Some(CurrentUser {
            name: user.name,
            dir: user.dir.to_string_lossy().into_owned(),
            shell: user.shell.to_string_lossy().into_owned(),
        }),
        // No passwd entry for this uid — e.g. a static/musl binary that can't
        // resolve directory-service users in-process. Fall through to the
        // host-delegated lookups.
        Ok(None) => None,
        Err(err) => {
            safe_error!(
                safe: ("passwd entry lookup failed for uid {uid}: {err}"),
                full: ("passwd entry lookup failed")
            );
            None
        }
    }
}

/// `getent` is the host's own (typically glibc-dynamic) binary, so it consults
/// the host's full NSS configuration — including SSSD/LDAP/AD — which a
/// static/musl Warp binary cannot do in-process.
fn current_user_via_getent(uid: u32) -> Option<CurrentUser> {
    let output = Command::new("getent")
        .arg("passwd")
        .arg(uid.to_string())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    stdout.lines().find_map(|line| parse_passwd_line(line, uid))
}

fn current_user_from_passwd_file(uid: u32) -> Option<CurrentUser> {
    let contents = std::fs::read_to_string("/etc/passwd").ok()?;
    contents
        .lines()
        .find_map(|line| parse_passwd_line(line, uid))
}

/// Parse a single `passwd(5)`-format line
/// (`name:passwd:uid:gid:gecos:dir:shell`) and return it as a [`CurrentUser`]
/// iff its uid field equals `uid`.
fn parse_passwd_line(line: &str, uid: u32) -> Option<CurrentUser> {
    // Strip leading blanks and ignore blank / comment lines, as glibc's passwd
    // reader does before parsing fields.
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    // `splitn(7, ':')` leaves the shell field (the 7th) as the rest of the
    // line, including any embedded colons — matching glibc's `pw_shell = line`.
    let mut fields = line.splitn(7, ':');
    let name = fields.next()?;
    let _passwd = fields.next()?;
    let line_uid: u32 = fields.next()?.parse().ok()?;
    let _gid = fields.next()?;
    let _gecos = fields.next()?;
    let dir = fields.next()?;
    let shell = fields.next()?;
    if line_uid != uid {
        return None;
    }
    Some(CurrentUser {
        name: name.to_owned(),
        dir: dir.to_owned(),
        shell: shell.to_owned(),
    })
}

pub struct Pty {
    pty_handle: Box<dyn PtyHandle>,
    fd: File,
    token: mio::Token,
    signals: Signals,
    signals_token: mio::Token,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtySpawnResult {
    pub pid: u32,
    pub leader_fd: i32,
}

pub(super) fn spawn(options: PtyOptions) -> Result<PtySpawnInfo> {
    // Docker sandbox sessions require sandbox-specific preparation
    // (writing the init script, creating workspace dirs) and a very
    // different `Command` shape (`sbx run` launching a container).
    // Dispatch early; both paths converge on `spawn_command_in_pty`
    // for the shared PTY/pre_exec setup.
    if let ShellStarter::DockerSandbox(docker_starter) = &options.shell_starter {
        let docker_starter = docker_starter.clone();
        return spawn_docker_sandbox(options, docker_starter);
    }

    let PtyOptions {
        size,
        window_id,
        shell_starter,
        start_dir,
        env_vars,
        enable_ssh_wrapper,
        reuse_ssh_control_master,
        shell_debug_mode,
        honor_ps1,
        node_version_chip_enabled,
        close_fds,
    } = options;
    let shell_starter = match shell_starter {
        ShellStarter::Direct(shell_starter) => shell_starter,
        _ => {
            return Err(Error::msg(
                "Given invalid shell starter on Unix-based system",
            ));
        }
    };

    let command = build_host_shell_command(
        shell_starter,
        window_id,
        env_vars,
        start_dir,
        enable_ssh_wrapper,
        reuse_ssh_control_master,
        shell_debug_mode,
        honor_ps1,
        node_version_chip_enabled,
    );

    spawn_command_in_pty(command, &size, close_fds)
}

/// Builds the `Command` for a host-shell PTY session: executable, args,
/// environment variables, and startup directory.
///
/// Does not perform any PTY-level setup; hand the returned `Command`
/// to [`spawn_command_in_pty`].
#[allow(clippy::too_many_arguments)]
fn build_host_shell_command(
    shell_starter: DirectShellStarter,
    window_id: Option<usize>,
    env_vars: HashMap<OsString, OsString>,
    start_dir: Option<PathBuf>,
    enable_ssh_wrapper: bool,
    reuse_ssh_control_master: bool,
    shell_debug_mode: bool,
    honor_ps1: bool,
    node_version_chip_enabled: bool,
) -> Command {
    let pw = resolve_current_user();

    log::info!(
        "Starting shell {}",
        shell_starter.logical_shell_path().display()
    );

    let mut builder = Command::new(shell_starter.logical_shell_path());
    for arg in shell_starter.args() {
        builder.arg(arg);
    }

    // Support an overridden home directory for integration tests, which
    // should execute in a more hermetic environment than one where the home
    // directory contains whatever happens to already exist there.
    let home_dir = std::env::var("HOME")
        .ok()
        .or_else(|| pw.as_ref().map(|pw| pw.dir.to_owned()))
        .unwrap_or_else(|| "/".to_owned());

    // Unfortunately process::Command has no facility for using the same fd for in/out/err.
    // The issue is that Stdio wants to close its fd. Previously we tried Stdio::from_raw_fd(follower)
    // for all 3 fds, and hoped that the error on close would be ignored.
    // Unfortunately this triggers a race: due to fd reuse the second and third
    // calls to close() might close a random fd. In practice this caused hangs
    // in the tests. Therefore we do NOT set stdin, stdout, stderr here; instead we
    // do it in the pre_exec hook.
    // Setup shell environment.
    if let Some(user_name) = pw
        .as_ref()
        .map(|pw| pw.name.to_owned())
        .or_else(|| std::env::var("USER").ok())
        .or_else(|| std::env::var("LOGNAME").ok())
    {
        builder.env("LOGNAME", &user_name);
        builder.env("USER", &user_name);
    }
    builder.env("HOME", &home_dir);

    // Specify terminal name and capabilities.
    builder.env("TERM", "xterm-256color");
    builder.env("TERM_PROGRAM", "WarpTerminal");
    // Advertise 24-bit color support.
    builder.env("COLORTERM", "truecolor");

    // Prevent child processes from inheriting startup notification env.
    // See: https://specifications.freedesktop.org/startup-notification-spec/startup-notification-latest.txt
    builder.env_remove("DESKTOP_STARTUP_ID");

    if let Some(version) = ChannelState::app_version() {
        builder.env("TERM_PROGRAM_VERSION", version);

        // We also insert this warp-specific version so that
        // plugins can do warp-specific version checks without worrying
        // that the version env var might be coming from a different terminal
        // (for ex., in the ssh case).
        builder.env("WARP_CLIENT_VERSION", version);
    } else {
        // Local builds don't have GIT_RELEASE_TAG, so app_version() is None.
        // Use "local" so plugins can still distinguish this from a missing value.
        builder.env("WARP_CLIENT_VERSION", "local");
    }

    // Set the `SHELL` environment variable to match the path of the shell we are using.
    // Traditionally, `$SHELL` is meant to match the user's default shell in the passwd database,
    // however we set it to the current shell that is to be `exec`ed. This behavior also matches
    // that of iTerm.
    builder.env("SHELL", shell_starter.logical_shell_path());

    if let Some(window_id) = window_id {
        builder.env("WINDOWID", format!("{window_id}"));
    }

    // Set whether or not we should utilize the SSH wrapper in this shell.
    if enable_ssh_wrapper {
        builder.env("WARP_USE_SSH_WRAPPER", "1");
    } else {
        builder.env("WARP_USE_SSH_WRAPPER", "0");
    }

    // Whether the SSH wrapper should attach to an existing ControlMaster
    // for the destination host instead of always creating its own.
    builder.env(
        "WARP_SSH_REUSE_CONTROL_MASTER",
        if reuse_ssh_control_master { "1" } else { "0" },
    );

    // For integration tests, put SSH control master sockets under the actual
    // home directory, as the length of the path to sockets placed in the
    // integration test home directory can exceed length limits.
    // See: https://stackoverflow.com/questions/35970686
    builder.env("SSH_SOCKET_DIR", ssh_socket_dir());

    // We currently don't support bootstrapping recursive SSH sessions so we will only run the SSH
    // logic if this flag is set.
    builder.env("WARP_IS_LOCAL_SHELL_SESSION", "1");

    // Only advertise the protocol version when the HOA notifications feature is enabled.
    // Without it, Warp can't render structured CLI agent notifications,
    // so the plugin should fall back to legacy notifications.
    if FeatureFlag::HOANotifications.is_enabled() {
        builder.env(
            "WARP_CLI_AGENT_PROTOCOL_VERSION",
            current_protocol_version().to_string(),
        );
    }

    if shell_debug_mode {
        builder.env("WARP_SHELL_DEBUG_MODE", "1");
    }
    if honor_ps1 {
        builder.env("WARP_HONOR_PS1", "1");
    } else {
        builder.env("WARP_HONOR_PS1", "0");
    }

    // Gate the shell's per-prompt `node --version` detection on whether the
    // Node.js Version chip is enabled. The bootstrap treats any value other than
    // "0" as enabled, so we only ever set "0" to disable it.
    builder.env(
        "WARP_PROMPT_NODE_VERSION_ENABLED",
        if node_version_chip_enabled { "1" } else { "0" },
    );

    // Pass through any additional entries to add to PATH.
    let path_append = extra_path_entries()
        .map(|p| p.to_string_lossy().into_owned())
        .join(":");
    builder.env("WARP_PATH_APPEND", path_append);

    if matches!(shell_starter.shell_type(), ShellType::Bash) {
        // Set initial very large values so bash imports the user's existing
        // history without truncating the file or in-memory list on startup.
        builder.env("HISTFILESIZE", BASH_HISTORY_SIZE_SENTINEL);
        builder.env("HISTSIZE", BASH_HISTORY_SIZE_SENTINEL);
        // Set second environment variables that we can use to know whether
        // the user rcfiles set these variables or not.
        builder.env("WARP_INITIAL_HISTFILESIZE", BASH_HISTORY_SIZE_SENTINEL);
        builder.env("WARP_INITIAL_HISTSIZE", BASH_HISTORY_SIZE_SENTINEL);
    }

    // Pass the desired initial working directory as an environment variable
    // and cd at the beginning of bootstrap.  We use this instead of
    // setting the process's initial working directory, as the spawn() will
    // fail if that directory doesn't exist.
    //
    // We could check the validity of the directory here, but that would be
    // a blocking filesystem call on the main thread, and the initial
    // directory could be on a network filesystem; deferring the `cd` to
    // shell bootstrap avoids that.
    if let Some(start_dir) = start_dir {
        builder.env("WARP_INITIAL_WORKING_DIR", start_dir);
    }

    // Apply any caller-provided environment overrides last, so they win.
    for (key, value) in env_vars {
        builder.env(key, value);
    }

    // Set the initial working directory to the user's home directory.  If
    // `start_dir` is Some, we'll attempt to cd to that directory at the
    // start of bootstrap.
    builder.current_dir(home_dir);

    builder
}

/// Shared PTY setup used by both host-shell and Docker-sandbox sessions.
///
/// Takes a fully-built `Command` and wraps it in the PTY/`pre_exec`
/// setup: creates the pty pair, applies termios, installs the child
/// process setup hook (signals, stdio, controlling terminal, close_fds,
/// Linux OOM rebias), and spawns the command.
///
/// The `pre_exec` hook has accumulated years of subtle bug fixes
/// (signal mask handling, TIOCSCTTY cast, etc.); keeping a single copy
/// ensures future fixes automatically apply to every session type.
fn spawn_command_in_pty(
    mut command: Command,
    size: &SizeInfo,
    close_fds: bool,
) -> Result<PtySpawnInfo> {
    let (leader, follower) = make_pty(size.to_winsize())?;

    // Close the follower at the end of this function.
    // We need to keep it alive long enough for fork().
    let _file = unsafe { File::from_raw_fd(follower) };

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Ok(mut termios) = termios::tcgetattr(leader) {
        // Set character encoding to UTF-8.
        termios.input_flags.set(InputFlags::IUTF8, true);
        let _ = termios::tcsetattr(leader, SetArg::TCSANOW, &termios);
    }

    // Detect isolation platform outside pre_exec, since detect() is not async-signal-safe.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let is_isolated = warp_isolation_platform::detect().is_some();

    unsafe {
        let fdlimit = libc::sysconf(libc::_SC_OPEN_MAX) as i32;

        command.pre_exec(move || {
            // IMPORTANT: THIS FUNCTION IS RUN AFTER FORK.
            // It must only use async-safe functions. No allocating memory,
            // taking a lock, or anything like that.
            //
            // If any errors are encountered while preparing the child
            // process, we convert them from a platform-level error code to
            // a rust Error, and return that.  Internally, Command passes
            // the error code back to the parent process and exposes that
            // as the result of calling spawn().

            use self::utils::cvt;

            // Reset all signal handlers to their defaults.
            for signal in 1..32 {
                // SIGKILL and SIGSTOP cannot be modified, so skip them.
                if signal == libc::SIGKILL || signal == libc::SIGSTOP {
                    continue;
                }

                // Set the signal handler to the default and check for errors.
                if libc::signal(signal, libc::SIG_DFL) == libc::SIG_ERR {
                    return Err(std::io::Error::last_os_error());
                }
            }

            // Unmask (unblock) all signals.  We need to use MaybeUninit because
            // some platforms define `libc::sigset_t` as an integer type, and
            // others define it as a structure.  The only way we can safely
            // initialize it on all platforms is through `libc::sigemptyset()`.
            let mut signals: MaybeUninit<libc::sigset_t> = MaybeUninit::uninit();
            libc::sigemptyset(signals.as_mut_ptr());
            let signals: libc::sigset_t = signals.assume_init();
            libc::sigprocmask(libc::SIG_SETMASK, &signals, ptr::null_mut());

            // Set up stdin/stdout/stderr.
            cvt(libc::dup2(follower, libc::STDIN_FILENO))?;
            cvt(libc::dup2(follower, libc::STDOUT_FILENO))?;
            cvt(libc::dup2(follower, libc::STDERR_FILENO))?;

            // Create a new process group.
            cvt(libc::setsid())?;

            // Set the controlling terminal.
            // TIOSCTTY changes based on platform and the `ioctl` call is different
            // based on architecture (32/64). So a generic cast is used to make sure
            // there are no issues. To allow such a generic cast the clippy warning
            // is disabled.
            #[allow(clippy::cast_lossless)]
            cvt(libc::ioctl(follower, TIOCSCTTY as _, 0))?;

            // Close all other FDs to avoid leaking any other non-pty FDs
            // into the shell process.  Don't propagate up errors, as most
            // of these won't be active file descriptors, and attempting to
            // close() them produces EINVAL.
            if close_fds {
                for fd in 3..fdlimit {
                    libc::close(fd);
                }
            }

            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            if is_isolated {
                // If running in a sandbox on Linux, adjust the OOM score
                // to make the child process more likely to be killed than the parent process
                // in case of OOM. If the Warp process is killed while hosting an ambient
                // agent, its shared session will abruptly end with no user-visible error.
                // Instead, we want to kill whatever process the agent spawned that's using
                // lots of memory. This gives the agent a chance to gracefully fail.
                //
                // Try to open /proc/self/oom_score_adj and set it to a positive value.
                // Valid values are between -1000 and 1000, where lower values are less likely
                // to be killed. Don't propagate errors, as this is best-effort.
                //
                // For Docker sandbox sessions this is effectively a no-op: the
                // container enforces memory limits via its own cgroup, so this
                // rebias on the host-side `sbx` process does not influence the
                // in-container OOM killer. We leave it in the shared path anyway
                // so the fork-side setup stays identical across session types.
                let oom_score_path = c"/proc/self/oom_score_adj";
                let fd = libc::open(oom_score_path.as_ptr(), libc::O_WRONLY);
                if fd >= 0 {
                    let score = b"500\n";
                    libc::write(fd, score.as_ptr() as *const libc::c_void, score.len());
                    libc::close(fd);
                }
            }

            Ok(())
        });
    }

    let spawned = command.spawn()?;
    Ok(PtySpawnInfo {
        result: PtySpawnResult {
            pid: spawned.id(),
            leader_fd: leader,
        },
        child: spawned,
    })
}

impl Pty {
    /// Create a new pty and return a handle to interact with it.
    pub fn new(
        options: PtyOptions,
        is_crash_reporting_enabled: bool,
        ctx: &mut AppContext,
    ) -> Result<Self> {
        let size = options.size;
        let shell = options.shell_starter.shell_type();

        // Prepare signal handling before spawning child.
        let signals = Signals::new([signal_hook::consts::SIGCHLD])
            .context("error preparing signal handling")?;

        let (PtySpawnResult { pid, leader_fd }, pty_handle) = PtySpawner::handle(ctx)
            .update(ctx, |pty_spawner, ctx| {
                pty_spawner.spawn_pty(options, is_crash_reporting_enabled, ctx)
            })?;

        log::info!(
            "Successfully spawned child {} process with pid {}",
            shell.name(),
            pid
        );

        let fd = unsafe {
            // Maybe this should be done outside of this function so nonblocking
            // isn't forced upon consumers. Although maybe it should be?
            set_nonblocking(leader_fd);

            File::from_raw_fd(leader_fd)
        };

        let mut pty = Pty {
            pty_handle,
            fd,
            token: PTY_TOKEN,
            signals,
            signals_token: SIGNALS_TOKEN,
        };
        pty.on_resize(&size);
        Ok(pty)
    }

    pub fn get_pid(&self) -> u32 {
        self.pty_handle.pid()
    }

    pub fn get_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl EventedReadWrite for Pty {
    type Reader = File;
    type Writer = File;

    #[inline]
    fn register(&mut self, poll: &mio::Poll, interest: mio::Interest) -> io::Result<()> {
        poll.registry()
            .register(&mut SourceFd(&self.fd.as_raw_fd()), self.token, interest)?;

        poll.registry()
            .register(&mut self.signals, self.signals_token, Interest::READABLE)
    }

    #[inline]
    fn reregister(&mut self, poll: &mio::Poll, interest: mio::Interest) -> io::Result<()> {
        poll.registry()
            .reregister(&mut SourceFd(&self.fd.as_raw_fd()), self.token, interest)?;

        poll.registry()
            .reregister(&mut self.signals, self.signals_token, Interest::READABLE)
    }

    #[inline]
    fn deregister(&mut self, poll: &mio::Poll) -> io::Result<()> {
        poll.registry()
            .deregister(&mut SourceFd(&self.fd.as_raw_fd()))?;
        poll.registry().deregister(&mut self.signals)
    }

    #[inline]
    fn reader(&mut self) -> &mut File {
        &mut self.fd
    }

    #[inline]
    fn read_token(&self) -> mio::Token {
        self.token
    }

    #[inline]
    fn writer(&mut self) -> &mut File {
        &mut self.fd
    }

    #[inline]
    fn write_token(&self) -> mio::Token {
        self.token
    }
}

impl EventedPty for Pty {
    #[inline]
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.signals.pending().next().and_then(|signal| {
            if signal != signal_hook::consts::SIGCHLD {
                return None;
            }

            match self.pty_handle.has_process_terminated() {
                Ok(true) => Some(ChildEvent::Exited),
                Ok(false) => None,
                Err(e) => {
                    report_error!(e.context("Error checking child process termination"));
                    None
                }
            }
        })
    }

    #[inline]
    fn child_event_token(&self) -> mio::Token {
        self.signals_token
    }

    fn on_resize(&mut self, size: &SizeInfo) {
        let win = size.to_winsize();

        let res = unsafe { libc::ioctl(self.fd.as_raw_fd(), libc::TIOCSWINSZ, &win as *const _) };

        if res < 0 {
            panic!("ioctl TIOCSWINSZ failed: {}", io::Error::last_os_error());
        }
    }

    fn kill(mut self) -> Result<()> {
        // Note: on macOS if there is remaining data in the pty, the child process
        // may get stuck in an 'E' (trying to exit) state, and the wait will
        // hang. Closing the pty explicitly fixes it, though the reason is unclear;
        // it appears to be a kernel bug.
        std::mem::drop(self.fd);
        let result = self.pty_handle.kill();
        report_if_error!(result);
        result
    }
}

/// Types that can produce a `libc::winsize`.
pub trait ToWinsize {
    /// Get a `libc::winsize`.
    fn to_winsize(&self) -> winsize;
}

impl ToWinsize for &SizeInfo {
    fn to_winsize(&self) -> winsize {
        winsize {
            ws_row: self.rows as libc::c_ushort,
            ws_col: self.columns as libc::c_ushort,
            ws_xpixel: self.pane_width_px().as_f32() as libc::c_ushort,
            ws_ypixel: self.pane_height_px().as_f32() as libc::c_ushort,
        }
    }
}

unsafe fn set_nonblocking(fd: c_int) {
    use libc::{fcntl, F_GETFL, F_SETFL, O_NONBLOCK};

    let res = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL, 0) | O_NONBLOCK);
    assert_eq!(res, 0);
}

/// Spawn the PTY for a Docker sandbox session.
///
/// Performs sandbox-specific preparation (writes the init script,
/// creates per-sandbox host scratch dirs) and then delegates to the
/// shared [`spawn_command_in_pty`] helper so PTY/`pre_exec` setup stays
/// identical to the host-shell path.
fn spawn_docker_sandbox(
    options: PtyOptions,
    docker_starter: DockerSandboxShellStarter,
) -> Result<PtySpawnInfo> {
    // Prepare sandbox bootstrap assets (init script + dedicated host
    // workspace) before building the command. The sandbox container
    // itself is created + attached in a single step via `sbx run` when
    // the PTY process spawns below.
    if let Err(e) = prepare_docker_sandbox(&docker_starter) {
        report_error!(&e);
        return Err(Error::msg(format!("Docker sandbox setup failed: {e}")));
    }

    let PtyOptions {
        size,
        window_id,
        shell_starter: _,
        start_dir: _,
        env_vars,
        enable_ssh_wrapper,
        reuse_ssh_control_master,
        shell_debug_mode,
        honor_ps1,
        node_version_chip_enabled,
        close_fds,
    } = options;

    let command = build_docker_sandbox_command(
        &docker_starter,
        window_id,
        env_vars,
        enable_ssh_wrapper,
        reuse_ssh_control_master,
        shell_debug_mode,
        honor_ps1,
        node_version_chip_enabled,
    );

    spawn_command_in_pty(command, &size, close_fds)
}

/// Builds the `Command` for a Docker-sandbox PTY session: `sbx run`
/// invocation with sandbox-specific args and host-side environment
/// variables.
///
/// Does not perform any PTY-level setup; hand the returned `Command`
/// to [`spawn_command_in_pty`].
#[allow(clippy::too_many_arguments)]
fn build_docker_sandbox_command(
    docker_starter: &DockerSandboxShellStarter,
    window_id: Option<usize>,
    env_vars: HashMap<OsString, OsString>,
    enable_ssh_wrapper: bool,
    reuse_ssh_control_master: bool,
    shell_debug_mode: bool,
    honor_ps1: bool,
    node_version_chip_enabled: bool,
) -> Command {
    let pw = resolve_current_user();

    log::info!(
        "Starting Docker sandbox via {}",
        docker_starter.logical_shell_path().display()
    );

    let mut builder = Command::new(docker_starter.logical_shell_path());
    for arg in docker_sandbox_run_args(docker_starter) {
        builder.arg(arg);
    }

    let home_dir = std::env::var("HOME")
        .ok()
        .or_else(|| pw.as_ref().map(|pw| pw.dir.to_owned()))
        .unwrap_or_else(|| "/".to_owned());

    // Environment variables set on the host-side `sbx` process.
    //
    // TODO(advait): audit this list. It currently mirrors what the
    // pre-refactor host-shell `spawn` set when the starter happened to
    // be a Docker sandbox, so behaviour is unchanged from before the
    // split. Many of these (e.g. `WARP_USE_SSH_WRAPPER`,
    // `SSH_SOCKET_DIR`, `HISTFILESIZE`, `WARP_IS_LOCAL_SHELL_SESSION`)
    // are set on the *host* `sbx` process and may or may not propagate
    // into the container depending on `sbx`'s env passthrough rules.
    // Once we've validated what the container bootstrap actually needs,
    // we can trim this list down to the variables the in-container bash
    // session actually consumes.
    if let Some(user_name) = pw
        .as_ref()
        .map(|pw| pw.name.to_owned())
        .or_else(|| std::env::var("USER").ok())
        .or_else(|| std::env::var("LOGNAME").ok())
    {
        builder.env("LOGNAME", &user_name);
        builder.env("USER", &user_name);
    }
    builder.env("HOME", &home_dir);
    builder.env("TERM", "xterm-256color");
    builder.env("TERM_PROGRAM", "WarpTerminal");
    builder.env("COLORTERM", "truecolor");
    builder.env_remove("DESKTOP_STARTUP_ID");
    if let Some(version) = ChannelState::app_version() {
        builder.env("TERM_PROGRAM_VERSION", version);
        builder.env("WARP_CLIENT_VERSION", version);
    } else {
        builder.env("WARP_CLIENT_VERSION", "local");
    }
    builder.env("SHELL", docker_starter.logical_shell_path());
    if let Some(window_id) = window_id {
        builder.env("WINDOWID", format!("{window_id}"));
    }
    builder.env(
        "WARP_USE_SSH_WRAPPER",
        if enable_ssh_wrapper { "1" } else { "0" },
    );
    builder.env(
        "WARP_SSH_REUSE_CONTROL_MASTER",
        if reuse_ssh_control_master { "1" } else { "0" },
    );
    builder.env("SSH_SOCKET_DIR", ssh_socket_dir());
    builder.env("WARP_IS_LOCAL_SHELL_SESSION", "1");
    if FeatureFlag::HOANotifications.is_enabled() {
        builder.env(
            "WARP_CLI_AGENT_PROTOCOL_VERSION",
            current_protocol_version().to_string(),
        );
    }
    if shell_debug_mode {
        builder.env("WARP_SHELL_DEBUG_MODE", "1");
    }
    builder.env("WARP_HONOR_PS1", if honor_ps1 { "1" } else { "0" });
    builder.env(
        "WARP_PROMPT_NODE_VERSION_ENABLED",
        if node_version_chip_enabled { "1" } else { "0" },
    );
    let path_append = extra_path_entries()
        .map(|p| p.to_string_lossy().into_owned())
        .join(":");
    builder.env("WARP_PATH_APPEND", path_append);
    // Sandbox shell is always bash (per the container image convention),
    // matching the host-shell path's behavior for bash shells.
    builder.env("HISTFILESIZE", BASH_HISTORY_SIZE_SENTINEL);
    builder.env("HISTSIZE", BASH_HISTORY_SIZE_SENTINEL);
    builder.env("WARP_INITIAL_HISTFILESIZE", BASH_HISTORY_SIZE_SENTINEL);
    builder.env("WARP_INITIAL_HISTSIZE", BASH_HISTORY_SIZE_SENTINEL);
    // Intentionally do NOT set `WARP_INITIAL_WORKING_DIR` for sandboxes:
    // the container's init script cds into the sandbox home dir, not
    // the host's startup dir.

    // Apply any caller-provided environment overrides last, so they win.
    for (key, value) in env_vars {
        builder.env(key, value);
    }

    builder.current_dir(home_dir);

    builder
}

/// Prepare the Docker sandbox before spawning the PTY:
/// 1. Write the bash init script to the per-sandbox host init dir.
/// 2. Create a dedicated empty per-sandbox host workspace so `sbx run shell`
///    does not mount the user's current working tree or home directory into
///    the sandbox.
///
/// Both paths are derived from `starter.sandbox_id` so multiple concurrent
/// Warp panes/sandboxes don't race on or share the same host directories.
///
/// The actual sandbox creation + attachment happens via
/// `sbx run --name warp-sandbox-<id> shell WORKSPACE ... -- -c "cd /home/agent && exec bash --rcfile ..."`
/// when the PTY process is spawned.
///
/// TODO(advait): Wire up cleanup on pane close. Today, closing a Docker
/// sandbox pane leaves behind (1) the per-sandbox host init + workspace dirs
/// under the Warp cache dir, and (2) the stopped `warp-sandbox-<id>`
/// container. Both are per-sandbox so they don't clobber each other, but
/// they accumulate over repeated sessions. The right hook is likely on the
/// PTY/pane lifecycle (alongside `Pty::kill`) and should:
///   - `sbx rm --force warp-sandbox-<id>` to drop the container,
///   - `fs::remove_dir_all` on `starter.init_dir()` and
///     `starter.workspace_dir()` to reclaim host disk.
/// Tracking as a follow-up.
fn prepare_docker_sandbox(starter: &DockerSandboxShellStarter) -> Result<()> {
    // Build each per-sandbox subdirectory with mode 0700 so other local users
    // cannot traverse into them, which (combined with the parent living under
    // the per-user Warp cache dir rather than `/tmp`) prevents the init
    // script from being read or symlink-attacked by anyone other than the
    // Warp user. The file itself is left at the default mode so the
    // container's shell (which may run as a different uid than the host
    // user) can still read it via `--rcfile`.
    let mk_owner_only_dir = |path: &Path| -> Result<()> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .with_context(|| format!("create sandbox dir {}", path.display()))
    };

    // 1. Write the init script to this sandbox's dedicated host init dir.
    let init_script =
        raw_init_shell_script_for_shell(ShellType::Bash, &ASSETS, starter.session_id());
    let init_dir = starter.init_dir();
    mk_owner_only_dir(&init_dir)?;
    std::fs::write(starter.init_path(), init_script).context("write sandbox init script")?;
    // 2. Create this sandbox's dedicated empty primary workspace so the
    // sandbox does not inherit access to the user's home directory or the
    // current local repository by default.
    mk_owner_only_dir(&starter.workspace_dir())?;

    Ok(())
}

#[cfg(test)]
#[path = "unix_tests.rs"]
mod tests;

/// A set of platform helper utilities copied directly from std::sys.
///
/// See: https://github.com/rust-lang/rust/blob/master/library/std/src/sys/unix/mod.rs
mod utils {
    #[doc(hidden)]
    pub(super) trait IsMinusOne {
        fn is_minus_one(&self) -> bool;
    }

    macro_rules! impl_is_minus_one {
        ($($t:ident)*) => ($(impl IsMinusOne for $t {
            fn is_minus_one(&self) -> bool {
                *self == -1
            }
        })*)
    }

    impl_is_minus_one! { i8 i16 i32 i64 isize }

    /// Checks whether the provided value represents a platform-level error status
    /// and converts it into a [`Result`].
    pub(super) fn cvt<T: IsMinusOne>(t: T) -> std::io::Result<T> {
        if t.is_minus_one() {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(t)
        }
    }
}

#[test]
fn parse_passwd_line_extracts_matching_uid() {
    let line = "alice:x:1000:1000:Alice:/home/alice:/bin/zsh";
    assert_eq!(
        parse_passwd_line(line, 1000),
        Some(CurrentUser {
            name: "alice".to_owned(),
            dir: "/home/alice".to_owned(),
            shell: "/bin/zsh".to_owned(),
        })
    );
    // A non-matching uid or a malformed line yields None.
    assert_eq!(parse_passwd_line(line, 1001), None);
    assert_eq!(parse_passwd_line("not a passwd line", 1000), None);
}

#[test]
fn parse_passwd_line_matches_glibc_edge_cases() {
    // An empty shell field (trailing colon) is valid and yields an empty shell,
    // just as glibc allows `pw_shell` to be empty.
    assert_eq!(
        parse_passwd_line("root:x:0:0:root:/root:", 0),
        Some(CurrentUser {
            name: "root".to_owned(),
            dir: "/root".to_owned(),
            shell: String::new(),
        })
    );

    // Blank lines and `#` comment lines (even with leading blanks) are skipped.
    assert_eq!(parse_passwd_line("", 0), None);
    assert_eq!(parse_passwd_line("   ", 0), None);
    assert_eq!(
        parse_passwd_line("# alice:x:1000:1000:Alice:/home/alice:/bin/zsh", 1000),
        None
    );

    // Leading blanks before a real entry are stripped, not treated as part of
    // the name.
    assert_eq!(
        parse_passwd_line("  bob:x:1001:1001:Bob:/home/bob:/bin/bash", 1001),
        Some(CurrentUser {
            name: "bob".to_owned(),
            dir: "/home/bob".to_owned(),
            shell: "/bin/bash".to_owned(),
        })
    );

    // The shell field keeps everything after the sixth colon, so a shell value
    // containing a `:` is not truncated (matches glibc's `pw_shell = line`).
    assert_eq!(
        parse_passwd_line("carol:x:1002:1002:Carol:/home/carol:/weird/shell:arg", 1002),
        Some(CurrentUser {
            name: "carol".to_owned(),
            dir: "/home/carol".to_owned(),
            shell: "/weird/shell:arg".to_owned(),
        })
    );

    // A line missing the shell field (only six fields) is malformed → skipped.
    assert_eq!(
        parse_passwd_line("dave:x:1003:1003:Dave:/home/dave", 1003),
        None
    );

    // A non-numeric uid field is malformed → skipped.
    assert_eq!(
        parse_passwd_line("eve:x:notanumber:1004:Eve:/home/eve:/bin/sh", 1004),
        None
    );
}

#[test]
fn resolve_current_user_returns_running_user() {
    // On the test host the current uid resolves via getpwuid_r, so this should
    // succeed and report a non-empty name.
    if let Some(user) = resolve_current_user() {
        assert!(!user.name.is_empty());
    }
}
