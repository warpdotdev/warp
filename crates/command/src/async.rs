// Allow disallowed types here. We actively want to use `std::process::Command` since this is the
// wrapper implementation that allows us to not import the above type elsewhere in this workspace.
#![allow(clippy::disallowed_types)]

use std::ffi::OsStr;
use std::future::Future;
use std::path::Path;
use std::process::{ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fmt, io};

use async_process::Child;

/// Wrapper around a [`async_process::Command`] that ensures any new Command is set with the windows
/// `CREATE_NO_WINDOW` flag to avoid a console window temporarily popping up.
pub struct Command {
    pub(super) inner: async_process::Command,
    stdin_is_default: bool,
    stdout_is_default: bool,
    stderr_is_default: bool,
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl Command {
    /// Constructs a new [`Command`] for launching `program`.
    ///
    /// The initial configuration (the working directory and environment variables) is inherited
    /// from the current process.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::Command;
    ///
    /// let mut cmd = Command::new("ls");
    /// ```
    pub fn new<S: AsRef<OsStr>>(program: S) -> Command {
        let program = crate::wsl::translate_program_for_spawn(program.as_ref());
        let inner = async_process::Command::new(program);
        Self::new_internal(inner)
    }

    /// Same as new, but makes this process the leader of a new session with
    /// the same ID as the process ID.
    ///
    /// This ensures the process does not inherit the controlling terminal.
    ///
    /// See [`setsid(2)`](https://man7.org/linux/man-pages/man2/setsid.2.html).
    #[cfg(unix)]
    pub fn new_with_session<S: AsRef<OsStr>>(program: S) -> Command {
        let program = crate::wsl::translate_program_for_spawn(program.as_ref());
        let mut command = std::process::Command::new(program);

        // SAFETY: `pre_exec` requires the closure to be async-signal-safe.
        // `setsid` is async-signal-safe per POSIX; see the signal-safety(7) man page:
        // https://man7.org/linux/man-pages/man7/signal-safety.7.html
        unsafe {
            use std::os::unix::process::CommandExt as _;
            command.pre_exec(|| {
                // TODO: Use `CommandExt::setsid` once it stabilizes (https://github.com/rust-lang/rust/issues/105376).
                // That enables the `posix_spawn` fast path rather than falling back to `fork`/`exec`.
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let inner: async_process::Command = command.into();
        Self::new_internal(inner)
    }

    /// Same as new, but makes this process the leader of a process group with same ID
    /// as the process ID.
    /// This allows for killing any other processes spawned by this process
    /// when we kill this process.
    pub fn new_with_process_group<S: AsRef<OsStr>>(program: S) -> Command {
        let program = crate::wsl::translate_program_for_spawn(program.as_ref());
        #[allow(unused_mut)]
        let mut command = std::process::Command::new(program);

        // Configures the new process to be the leader of a process group with its
        // process ID as the group ID. This allows for killing any other processes
        // spawned by this process when we kill this process.
        //
        // TODO(roland): handle for windows
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);

        let inner: async_process::Command = command.into();
        Self::new_internal(inner)
    }

    /// Same as new, but for a child whose whole process tree must be stoppable as a unit through
    /// the [`JobGuard`] that [`Command::spawn_in_job`] returns. On Unix the child leads a new
    /// process group, as with [`Command::new_with_process_group`]. On Windows the child is not
    /// created with `CREATE_BREAKAWAY_FROM_JOB`, so it stays inside Warp's own job as well as the
    /// fresh one it is assigned to on spawn.
    pub fn new_in_job<S: AsRef<OsStr>>(program: S) -> Command {
        #[cfg(windows)]
        {
            let program = crate::wsl::translate_program_for_spawn(program.as_ref());
            #[allow(unused_mut)]
            let mut inner = async_process::Command::new(program);
            #[cfg(not(feature = "test-util"))]
            {
                use async_process::windows::CommandExt;
                inner.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
            }
            Self {
                inner,
                stdin_is_default: true,
                stdout_is_default: true,
                stderr_is_default: true,
            }
        }
        #[cfg(not(windows))]
        {
            Self::new_with_process_group(program)
        }
    }

    #[allow(unused_mut)]
    fn new_internal(mut inner: async_process::Command) -> Command {
        #[cfg(all(windows, not(feature = "test-util")))]
        {
            use async_process::windows::CommandExt;
            // We need to set the `CREATE_BREAKAWAY_FROM_JOB` flag to avoid assigning
            // the process to the same Job Object as the Warp process, otherwise the
            // process will be killed when the Warp process is killed.
            let flags = windows::Win32::System::Threading::CREATE_NO_WINDOW.0
                | windows::Win32::System::Threading::CREATE_BREAKAWAY_FROM_JOB.0;
            inner.creation_flags(flags);
        }
        Self {
            inner,
            stdin_is_default: true,
            stdout_is_default: true,
            stderr_is_default: true,
        }
    }

    /// Adds a single argument to pass to the program.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::Command;
    ///
    /// let mut cmd = Command::new("echo");
    /// cmd.arg("hello");
    /// cmd.arg("world");
    /// ```
    pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Command {
        self.inner.arg(arg);
        self
    }

    /// Adds multiple arguments to pass to the program.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::Command;
    ///
    /// let mut cmd = Command::new("echo");
    /// cmd.args(&["hello", "world"]);
    /// ```
    pub fn args<I, S>(&mut self, args: I) -> &mut Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.inner.args(args);
        self
    }

    /// Configures an environment variable for the new process.
    ///
    /// Note that environment variable names are case-insensitive (but case-preserving) on Windows,
    /// and case-sensitive on all other platforms.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::Command;
    ///
    /// let mut cmd = Command::new("ls");
    /// cmd.env("PATH", "/bin");
    /// ```
    pub fn env<K, V>(&mut self, key: K, val: V) -> &mut Command
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.inner.env(key, val);
        self
    }

    /// Configures multiple environment variables for the new process.
    ///
    /// Note that environment variable names are case-insensitive (but case-preserving) on Windows,
    /// and case-sensitive on all other platforms.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::Command;
    ///
    /// let mut cmd = Command::new("ls");
    /// cmd.envs(vec![("PATH", "/bin"), ("TERM", "xterm-256color")]);
    /// ```
    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Command
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.inner.envs(vars);
        self
    }

    /// Removes an environment variable mapping.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::Command;
    ///
    /// let mut cmd = Command::new("ls");
    /// cmd.env_remove("PATH");
    /// ```
    pub fn env_remove<K: AsRef<OsStr>>(&mut self, key: K) -> &mut Command {
        self.inner.env_remove(key);
        self
    }

    /// Removes all environment variable mappings.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::Command;
    ///
    /// let mut cmd = Command::new("ls");
    /// cmd.env_clear();
    /// ```
    pub fn env_clear(&mut self) -> &mut Command {
        self.inner.env_clear();
        self
    }

    /// Configures the working directory for the new process.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::Command;
    ///
    /// let mut cmd = Command::new("ls");
    /// cmd.current_dir("/");
    /// ```
    pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut Command {
        self.inner.current_dir(dir);
        self
    }

    /// Returns the path to the program configured for this command.
    #[must_use]
    pub fn get_program(&self) -> &OsStr {
        self.inner.get_program()
    }

    /// Returns the arguments configured for this command.
    pub fn get_args(&self) -> impl Iterator<Item = &OsStr> {
        self.inner.get_args()
    }

    /// Returns the working directory configured for this command.
    #[must_use]
    pub fn get_current_dir(&self) -> Option<&Path> {
        self.inner.get_current_dir()
    }

    /// Configures the standard input (stdin) for the new process.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::{Command, Stdio};
    ///
    /// let mut cmd = Command::new("cat");
    /// cmd.stdin(Stdio::null());
    /// ```
    pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Command {
        self.inner.stdin(cfg);
        self.stdin_is_default = false;
        self
    }

    /// Configures the standard output (stdout) for the new process.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::{Command, Stdio};
    ///
    /// let mut cmd = Command::new("ls");
    /// cmd.stdout(Stdio::piped());
    /// ```
    pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Command {
        self.inner.stdout(cfg);
        self.stdout_is_default = false;
        self
    }

    /// Configures the standard error (stderr) for the new process.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::{Command, Stdio};
    ///
    /// let mut cmd = Command::new("ls");
    /// cmd.stderr(Stdio::piped());
    /// ```
    pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Command {
        self.inner.stderr(cfg);
        self.stderr_is_default = false;
        self
    }

    /// Configures whether to reap the zombie process when [`Child`] is dropped.
    ///
    /// When the process finishes, it becomes a "zombie" and some resources associated with it
    /// remain until [`Child::try_status()`], [`Child::status()`], or [`Child::output()`] collects
    /// its exit code.
    ///
    /// If its exit code is never collected, the resources may leak forever. This crate has a
    /// background thread named "async-process" that collects such "zombie" processes and then
    /// "reaps" them, thus preventing the resource leaks.
    ///
    /// The default value of this option is `true`.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::{Command, Stdio};
    ///
    /// let mut cmd = Command::new("cat");
    /// cmd.reap_on_drop(false);
    /// ```
    pub fn reap_on_drop(&mut self, reap_on_drop: bool) -> &mut Command {
        self.inner.reap_on_drop(reap_on_drop);
        self
    }

    /// Configures whether to kill the process when [`Child`] is dropped.
    ///
    /// The default value of this option is `false`.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_process::{Command, Stdio};
    ///
    /// let mut cmd = Command::new("cat");
    /// cmd.kill_on_drop(true);
    /// ```
    pub fn kill_on_drop(&mut self, kill_on_drop: bool) -> &mut Command {
        self.inner.kill_on_drop(kill_on_drop);
        self
    }

    /// Executes the command and returns the [`Child`] handle to it.
    ///
    /// If not configured, stdin, stdout and stderr will be set to [`Stdio::null()`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # futures_lite::future::block_on(async {
    /// use async_process::Command;
    ///
    /// let child = Command::new("ls").spawn()?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn spawn(&mut self) -> io::Result<Child> {
        if self.stdin_is_default {
            self.inner.stdin(Stdio::null());
        }
        if self.stdout_is_default {
            self.inner.stdout(Stdio::null());
        }
        if self.stderr_is_default {
            self.inner.stderr(Stdio::null());
        }

        self.inner.spawn()
    }

    /// Spawns a command built with [`Command::new_in_job`] and returns, with the child, the guard
    /// that controls its process tree. On Windows the child is assigned to a fresh kill-on-close
    /// Job Object; if that fails the child is killed and the error returned, so a hook never runs
    /// outside a job. On Unix the guard addresses the child's process group.
    pub fn spawn_in_job(&mut self) -> io::Result<(Child, JobGuard)> {
        let mut child = self.spawn()?;
        match JobGuard::for_child(&child) {
            Ok(guard) => Ok((child, guard)),
            Err(error) => {
                let _ = child.kill();
                Err(error)
            }
        }
    }

    /// Executes the command, waits for it to exit, and returns the exit status.
    ///
    /// If not configured, stdin, stdout and stderr will be set to [`Stdio::null()`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # futures_lite::future::block_on(async {
    /// use async_process::Command;
    ///
    /// let status = Command::new("cp")
    ///     .arg("a.txt")
    ///     .arg("b.txt")
    ///     .status()
    ///     .await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn status(&mut self) -> impl Future<Output = io::Result<ExitStatus>> {
        if self.stdin_is_default {
            self.inner.stdin(Stdio::null());
        }
        if self.stdout_is_default {
            self.inner.stdout(Stdio::null());
        }
        if self.stderr_is_default {
            self.inner.stderr(Stdio::null());
        }

        self.inner.status()
    }

    /// Executes the command and collects its output.
    ///
    /// If not configured, stdin will be set to [`Stdio::null()`], and stdout and stderr will be
    /// set to [`Stdio::piped()`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # futures_lite::future::block_on(async {
    /// use async_process::Command;
    ///
    /// let output = Command::new("cat")
    ///     .arg("a.txt")
    ///     .output()
    ///     .await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn output(&mut self) -> impl Future<Output = io::Result<Output>> {
        if self.stdin_is_default {
            self.inner.stdin(Stdio::null());
        }
        if self.stdout_is_default {
            self.inner.stdout(Stdio::piped());
        }
        if self.stderr_is_default {
            self.inner.stderr(Stdio::piped());
        }

        self.inner.output()
    }
}

/// Control over the process tree of a child spawned with [`Command::spawn_in_job`]: the process
/// group on Unix, the Job Object on Windows. Dropping the guard ends whatever is still running
/// in the tree.
///
/// Once the tree has been seen to have ended, the guard never signals its id again: on Unix a
/// process group id becomes reusable the moment its last member is gone, and a signal sent after
/// that could reach an unrelated process group that took the id.
#[derive(Debug)]
pub struct JobGuard {
    #[cfg(unix)]
    process_group: libc::pid_t,
    #[cfg(windows)]
    job: crate::windows::ProcessJob,
    ended: AtomicBool,
}

impl JobGuard {
    fn for_child(child: &Child) -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                process_group: child.id() as libc::pid_t,
                ended: AtomicBool::new(false),
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                job: crate::windows::ProcessJob::create_for(child.id())?,
                ended: AtomicBool::new(false),
            })
        }
    }

    /// Asks the tree to stop. On Unix this is `SIGTERM` to the process group, which a process may
    /// handle or ignore; on Windows it ends the job outright, since there is no gentler signal.
    /// Returns whether anything was sent, which is false once the tree is known to have ended.
    pub fn request_stop(&self) -> bool {
        if self.has_ended() {
            return false;
        }
        #[cfg(unix)]
        {
            self.signal_group(libc::SIGTERM)
        }
        #[cfg(windows)]
        {
            self.terminate_job()
        }
    }

    /// Ends everything still running in the tree now. Returns whether anything was sent, which is
    /// false once the tree is known to have ended.
    pub fn terminate(&self) -> bool {
        if self.has_ended() {
            return false;
        }
        #[cfg(unix)]
        {
            self.signal_group(libc::SIGKILL)
        }
        #[cfg(windows)]
        {
            self.terminate_job()
        }
    }

    /// Whether any process in the tree still exists. A finished-but-unreaped process counts, so
    /// callers should keep collecting the child's exit status while polling this.
    pub fn is_alive(&self) -> bool {
        if self.has_ended() {
            return false;
        }
        #[cfg(unix)]
        // Signal 0 only checks that the group exists.
        // SAFETY: `killpg` has no memory-safety preconditions.
        let alive = unsafe { libc::killpg(self.process_group, 0) == 0 };
        #[cfg(windows)]
        let alive = self.active_processes().is_ok_and(|count| count > 0);
        if !alive {
            self.ended.store(true, Ordering::Release);
        }
        alive
    }

    /// How many processes in the job are still running.
    #[cfg(windows)]
    pub fn active_processes(&self) -> io::Result<u32> {
        self.job.active_processes()
    }

    fn has_ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    /// Drops the guard without an explicit terminate, so a test can observe that closing the job
    /// handle alone ends the tree.
    #[cfg(all(test, windows))]
    pub(crate) fn close_without_terminating(self) {
        self.ended.store(true, Ordering::Release);
    }

    /// Sends `signal` to the group. Returns whether it was delivered; a group that no longer
    /// exists marks the tree as ended.
    #[cfg(unix)]
    fn signal_group(&self, signal: libc::c_int) -> bool {
        // SAFETY: `killpg` has no memory-safety preconditions.
        if unsafe { libc::killpg(self.process_group, signal) } == 0 {
            return true;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            self.ended.store(true, Ordering::Release);
        } else {
            log::debug!(
                "could not send signal {signal} to process group {}: {error}",
                self.process_group
            );
        }
        false
    }

    #[cfg(windows)]
    fn terminate_job(&self) -> bool {
        match self.job.terminate() {
            Ok(()) => true,
            Err(error) => {
                log::debug!("could not terminate hook job: {error}");
                false
            }
        }
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        // Checked first so a tree that already ended is never signalled by an id that may have
        // been reused since. On Windows closing the job handle would end the tree anyway;
        // terminating first makes the order explicit.
        if self.is_alive() {
            self.terminate();
        }
    }
}

#[cfg(test)]
#[path = "async_tests.rs"]
mod tests;
