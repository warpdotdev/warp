// Allow disallowed types here. We actively want to use `std::process::Command` since this is the
// wrapper implementation that allows us to not import the above type elsewhere in this workspace.
#![allow(clippy::disallowed_types)]

use async_process::Child;
use std::ffi::OsStr;
use std::fmt;
use std::future::Future;
use std::io;
use std::path::Path;
use std::process::{ExitStatus, Output, Stdio};

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
