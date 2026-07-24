// Allow disallowed types here. We actively want to use `std::process::Command` since this is the
// wrapper implementation that allows us to not import the above type elsewhere in this workspace.
#![allow(clippy::disallowed_types)]

use std::ffi::OsStr;
#[cfg(windows)]
use std::ffi::OsString;
use std::future::Future;
use std::path::Path;
use std::process::{ExitStatus, Output, Stdio};
use std::{fmt, io};

use async_process::Child;

/// Wrapper around a [`async_process::Command`] that ensures any new Command is set with the windows
/// `CREATE_NO_WINDOW` flag to avoid a console window temporarily popping up.
pub struct Command {
    pub(super) inner: async_process::Command,
    // The stdio configuration is stored rather than forwarded to `inner` immediately so it can be
    // re-applied after a WSL UNC rewrite (see `apply_wsl_unc_translation`) replaces `inner`. Each
    // stream keeps a pending value plus a "was explicitly set" flag so the pre-deferral re-run
    // semantics are preserved: a pending value is applied once (and then persists on `inner`); an
    // unset stream falls back to the method-specific default; a stream that was explicitly set on a
    // previous run is left untouched so `inner` keeps it. See [`resolve_stdio`].
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
    stdin_set: bool,
    stdout_set: bool,
    stderr_set: bool,
    // Tracked so they survive an `inner` replacement performed by the WSL UNC rewrite. The
    // defaults match `async_process::Command` (`kill_on_drop` off, `reap_on_drop` on).
    kill_on_drop: bool,
    reap_on_drop: bool,
    // Whether `env_clear` was called; replayed onto the rebuilt `inner`. Windows-only because the
    // rewrite itself is.
    #[cfg(windows)]
    env_cleared: bool,
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

/// Resolves the [`Stdio`] to apply for one stream at execution time,
/// preserving the pre-deferral semantics. Returns `Some` when `inner`
/// should be reconfigured (an explicitly configured value, applied once,
/// or a method-specific default for an unset stream) and `None` when the
/// stream was already configured on a previous run and `inner` should be
/// left untouched.
fn resolve_stdio(
    pending: &mut Option<Stdio>,
    was_set: bool,
    default: fn() -> Stdio,
) -> Option<Stdio> {
    match (pending.take(), was_set) {
        (Some(cfg), _) => Some(cfg),
        (None, false) => Some(default()),
        (None, true) => None,
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
            stdin: None,
            stdout: None,
            stderr: None,
            stdin_set: false,
            stdout_set: false,
            stderr_set: false,
            kill_on_drop: false,
            reap_on_drop: true,
            #[cfg(windows)]
            env_cleared: false,
        }
    }

    /// Rewrites a bare `git` command whose working directory is a WSL UNC path into an equivalent
    /// `wsl.exe` invocation, so it runs against the Linux-side git inside the distribution rather
    /// than the Windows `git.exe`. No-op when the command does not qualify.
    ///
    /// This replaces `inner` because the program name and argument vector cannot be mutated in
    /// place. State carried onto the replacement: the argument vector, the explicit environment
    /// variables and their cleared-inheritance flag, `WSLENV`, the `kill_on_drop` / `reap_on_drop`
    /// settings, and the mandatory creation flags. Deliberately not carried: the working directory
    /// (`--cd` supplies it inside the distribution) and `PATH` (a Linux-form `PATH` must not be
    /// handed to `wsl.exe`; see `crate::wsl_windows::build_wslenv`). Stdio is applied after this
    /// call by the execution methods.
    #[cfg(windows)]
    fn apply_wsl_unc_translation(&mut self) {
        let program = self.inner.get_program().to_owned();
        let args: Vec<OsString> = self.inner.get_args().map(|arg| arg.to_owned()).collect();
        let cwd = self.inner.get_current_dir().map(|dir| dir.to_owned());
        let env: Vec<(OsString, OsString)> = self
            .inner
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value.to_owned())))
            .collect();

        let Some(translated) =
            crate::wsl_windows::translate_for_wsl_unc_cwd(&program, &args, cwd.as_deref(), &env)
        else {
            return;
        };

        let mut replacement = async_process::Command::new(&translated.program);
        replacement.args(&translated.args);
        if self.env_cleared {
            replacement.env_clear();
        }
        for (key, value) in self.inner.get_envs() {
            match value {
                Some(value) => {
                    // An explicitly-set `PATH` rides through the argument
                    // vector instead (see `translate_for_wsl_unc_cwd`); an
                    // explicit removal below must still be replayed so the
                    // replacement does not inherit the parent's `PATH`.
                    if crate::wsl_windows::is_path_env_key(key) {
                        continue;
                    }
                    replacement.env(key, value);
                }
                None => {
                    replacement.env_remove(key);
                }
            }
        }
        if let Some(wslenv) = &translated.wslenv {
            replacement.env("WSLENV", wslenv);
        }
        #[cfg(not(feature = "test-util"))]
        {
            use async_process::windows::CommandExt;
            let flags = windows::Win32::System::Threading::CREATE_NO_WINDOW.0
                | windows::Win32::System::Threading::CREATE_BREAKAWAY_FROM_JOB.0;
            replacement.creation_flags(flags);
        }
        replacement.kill_on_drop(self.kill_on_drop);
        replacement.reap_on_drop(self.reap_on_drop);
        self.inner = replacement;
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
        // Remembered so a WSL UNC rewrite can re-clear inheritance on the rebuilt `inner`; without
        // this the replacement would silently inherit the parent environment.
        #[cfg(windows)]
        {
            self.env_cleared = true;
        }
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
        self.stdin = Some(cfg.into());
        self.stdin_set = true;
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
        self.stdout = Some(cfg.into());
        self.stdout_set = true;
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
        self.stderr = Some(cfg.into());
        self.stderr_set = true;
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
        self.reap_on_drop = reap_on_drop;
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
        self.kill_on_drop = kill_on_drop;
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
        #[cfg(windows)]
        self.apply_wsl_unc_translation();

        if let Some(cfg) = resolve_stdio(&mut self.stdin, self.stdin_set, Stdio::null) {
            self.inner.stdin(cfg);
        }
        if let Some(cfg) = resolve_stdio(&mut self.stdout, self.stdout_set, Stdio::null) {
            self.inner.stdout(cfg);
        }
        if let Some(cfg) = resolve_stdio(&mut self.stderr, self.stderr_set, Stdio::null) {
            self.inner.stderr(cfg);
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
        #[cfg(windows)]
        self.apply_wsl_unc_translation();

        if let Some(cfg) = resolve_stdio(&mut self.stdin, self.stdin_set, Stdio::null) {
            self.inner.stdin(cfg);
        }
        if let Some(cfg) = resolve_stdio(&mut self.stdout, self.stdout_set, Stdio::null) {
            self.inner.stdout(cfg);
        }
        if let Some(cfg) = resolve_stdio(&mut self.stderr, self.stderr_set, Stdio::null) {
            self.inner.stderr(cfg);
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
        #[cfg(windows)]
        self.apply_wsl_unc_translation();

        if let Some(cfg) = resolve_stdio(&mut self.stdin, self.stdin_set, Stdio::null) {
            self.inner.stdin(cfg);
        }
        if let Some(cfg) = resolve_stdio(&mut self.stdout, self.stdout_set, Stdio::piped) {
            self.inner.stdout(cfg);
        }
        if let Some(cfg) = resolve_stdio(&mut self.stderr, self.stderr_set, Stdio::piped) {
            self.inner.stderr(cfg);
        }

        self.inner.output()
    }
}

#[cfg(all(test, windows))]
#[path = "async_tests.rs"]
mod tests;
