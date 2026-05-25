// Allow disallowed types here. We actively want to use `std::process::Command` since this is the
// wrapper implementation that allows us to not import the above type elsewhere in this workspace.
#![allow(clippy::disallowed_types)]

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Child, CommandArgs, CommandEnvs, ExitStatus, Output, Stdio};

#[cfg(windows)]
use {super::windows::JobObject, std::os::windows::io::AsRawHandle};

/// Wrapper around a [`std::process::Command`] that ensures any new Command is set with the windows
/// `CREATE_NO_WINDOW` flag to avoid a console window temporarily popping up.
#[derive(Debug)]
pub struct Command {
    pub(super) inner: std::process::Command,
    #[cfg(windows)]
    kill_on_parent_process_close: bool,
    stdin_is_default: bool,
    stdout_is_default: bool,
    stderr_is_default: bool,
}

impl Command {
    /// Constructs a new `Command` for launching the program at
    /// path `program`, with the following default configuration:
    ///
    /// * No arguments to the program
    /// * Inherit the current process's environment
    /// * Inherit the current process's working directory
    /// * Inherit stdin/stdout/stderr for [`spawn`] or [`status`], but create pipes for [`output`]
    ///
    /// [`spawn`]: Self::spawn
    /// [`status`]: Self::status
    /// [`output`]: Self::output
    ///
    /// Builder methods are provided to change these defaults and
    /// otherwise configure the process.
    ///
    /// If `program` is not an absolute path, the `PATH` will be searched in
    /// an OS-defined way.
    ///
    /// The search path to be used may be controlled by setting the
    /// `PATH` environment variable on the Command,
    /// but this has some implementation limitations on Windows
    /// (see issue #37519).
    ///
    /// # Platform-specific behavior
    ///
    /// Note on Windows: For executable files with the .exe extension,
    /// it can be omitted when specifying the program for this Command.
    /// However, if the file has a different extension,
    /// a filename including the extension needs to be provided,
    /// otherwise the file won't be found.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::Command;
    ///
    /// Command::new("sh")
    ///     .spawn()
    ///     .expect("sh command failed to start");
    /// ```
    pub fn new<S: AsRef<OsStr>>(program: S) -> Command {
        let program = crate::wsl::translate_program_for_spawn(program.as_ref());
        #[cfg_attr(not(windows), expect(unused_mut))]
        let mut inner = std::process::Command::new(program);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // We need to set the `CREATE_BREAKAWAY_FROM_JOB` flag to avoid assigning
            // the process to the same Job Object as the Warp process, otherwise the
            // process will be killed when the Warp process is killed.
            let flags = windows::Win32::System::Threading::CREATE_NO_WINDOW.0
                | windows::Win32::System::Threading::CREATE_BREAKAWAY_FROM_JOB.0;
            inner.creation_flags(flags);
        }
        Self {
            inner,
            #[cfg(windows)]
            kill_on_parent_process_close: false,
            stdin_is_default: true,
            stdout_is_default: true,
            stderr_is_default: true,
        }
    }

    #[cfg(windows)]
    /// Sets the [process creation flags][1] to be passed to `CreateProcess`.
    ///
    /// These will always be ORed with `CREATE_UNICODE_ENVIRONMENT` and `CREATE_NO_WINDOW`.
    /// The latter is needed to avoid a console window temporarily popping up in Warp.
    ///
    /// [1]: https://msdn.microsoft.com/en-us/library/windows/desktop/ms684863(v=vs.85).aspx
    pub fn creation_flags(&mut self, flags: u32) -> &mut Self {
        use std::os::windows::process::CommandExt;
        let flags = windows::Win32::System::Threading::CREATE_NO_WINDOW.0 | flags;
        self.inner.creation_flags(flags);
        self
    }

    /// Adds an argument to pass to the program.
    ///
    /// Only one argument can be passed per use. So instead of:
    ///
    /// ```no_run
    /// # std::process::Command::new("sh")
    /// .arg("-C /path/to/repo")
    /// # ;
    /// ```
    ///
    /// usage would be:
    ///
    /// ```no_run
    /// # std::process::Command::new("sh")
    /// .arg("-C")
    /// .arg("/path/to/repo")
    /// # ;
    /// ```
    ///
    /// To pass multiple arguments see [`args`].
    ///
    /// [`args`]: std::process::Command::args
    ///
    /// Note that the argument is not passed through a shell, but given
    /// literally to the program. This means that shell syntax like quotes,
    /// escaped characters, word splitting, glob patterns, variable substitution,
    /// etc. have no effect.
    ///
    /// <div class="warning">
    ///
    /// On Windows, use caution with untrusted inputs. Most applications use the
    /// standard convention for decoding arguments passed to them. These are safe to
    /// use with `arg`. However, some applications such as `cmd.exe` and `.bat` files
    /// use a non-standard way of decoding arguments. They are therefore vulnerable
    /// to malicious input.
    ///
    /// In the case of `cmd.exe` this is especially important because a malicious
    /// argument can potentially run arbitrary shell commands.
    ///
    /// See [Windows argument splitting][windows-args] for more details
    /// or [`raw_arg`] for manually implementing non-standard argument encoding.
    ///
    /// [`raw_arg`]: crate::os::windows::process::CommandExt::raw_arg
    /// [windows-args]: crate::process#windows-argument-splitting
    ///
    /// </div>
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::Command;
    ///
    /// Command::new("ls")
    ///     .arg("-l")
    ///     .arg("-a")
    ///     .spawn()
    ///     .expect("ls command failed to start");
    /// ```
    pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.inner.arg(arg);
        self
    }

    /// Adds multiple arguments to pass to the program.
    ///
    /// To pass a single argument see [`arg`].
    ///
    /// [`arg`]: std::process::Command::arg
    ///
    /// Note that the arguments are not passed through a shell, but given
    /// literally to the program. This means that shell syntax like quotes,
    /// escaped characters, word splitting, glob patterns, variable substitution, etc.
    /// have no effect.
    ///
    /// <div class="warning">
    ///
    /// On Windows, use caution with untrusted inputs. Most applications use the
    /// standard convention for decoding arguments passed to them. These are safe to
    /// use with `arg`. However, some applications such as `cmd.exe` and `.bat` files
    /// use a non-standard way of decoding arguments. They are therefore vulnerable
    /// to malicious input.
    ///
    /// In the case of `cmd.exe` this is especially important because a malicious
    /// argument can potentially run arbitrary shell commands.
    ///
    /// See [Windows argument splitting][windows-args] for more details
    /// or [`raw_arg`] for manually implementing non-standard argument encoding.
    ///
    /// [`raw_arg`]: crate::os::windows::process::CommandExt::raw_arg
    /// [windows-args]: crate::process#windows-argument-splitting
    ///
    /// </div>
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::Command;
    ///
    /// Command::new("ls")
    ///     .args(["-l", "-a"])
    ///     .spawn()
    ///     .expect("ls command failed to start");
    /// ```
    pub fn args<I, S>(&mut self, args: I) -> &mut Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.inner.args(args);
        self
    }

    /// Inserts or updates an explicit environment variable mapping.
    ///
    /// This method allows you to add an environment variable mapping to the spawned process or
    /// overwrite a previously set value. You can use [`std::process::Command::envs`] to set multiple environment
    /// variables simultaneously.
    ///
    /// Child processes will inherit environment variables from their parent process by default.
    /// Environment variables explicitly set using [`std::process::Command::env`] take precedence over inherited
    /// variables. You can disable environment variable inheritance entirely using
    /// [`std::process::Command::env_clear`] or for a single key using [`std::process::Command::env_remove`].
    ///
    /// Note that environment variable names are case-insensitive (but
    /// case-preserving) on Windows and case-sensitive on all other platforms.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::Command;
    ///
    /// Command::new("ls")
    ///     .env("PATH", "/bin")
    ///     .spawn()
    ///     .expect("ls command failed to start");
    /// ```
    pub fn env<K, V>(&mut self, key: K, val: V) -> &mut Command
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.inner.env(key, val);
        self
    }

    /// Inserts or updates multiple explicit environment variable mappings.
    ///
    /// This method allows you to add multiple environment variable mappings to the spawned process
    /// or overwrite previously set values. You can use [`std::process::Command::env`] to set a single environment
    /// variable.
    ///
    /// Child processes will inherit environment variables from their parent process by default.
    /// Environment variables explicitly set using [`std::process::Command::envs`] take precedence over inherited
    /// variables. You can disable environment variable inheritance entirely using
    /// [`std::process::Command::env_clear`] or for a single key using [`std::process::Command::env_remove`].
    ///
    /// Note that environment variable names are case-insensitive (but case-preserving) on Windows
    /// and case-sensitive on all other platforms.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::{Command, Stdio};
    /// use std::env;
    /// use std::collections::HashMap;
    ///
    /// let filtered_env : HashMap<String, String> =
    ///     env::vars().filter(|&(ref k, _)|
    ///         k == "TERM" || k == "TZ" || k == "LANG" || k == "PATH"
    ///     ).collect();
    ///
    /// Command::new("printenv")
    ///     .stdin(Stdio::null())
    ///     .stdout(Stdio::inherit())
    ///     .env_clear()
    ///     .envs(&filtered_env)
    ///     .spawn()
    ///     .expect("printenv failed to start");
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

    /// Removes an explicitly set environment variable and prevents inheriting it from a parent
    /// process.
    ///
    /// This method will remove the explicit value of an environment variable set via
    /// [`std::process::Command::env`] or [`std::process::Command::envs`]. In addition, it will prevent the spawned child
    /// process from inheriting that environment variable from its parent process.
    ///
    /// After calling [`std::process::Command::env_remove`], the value associated with its key from
    /// [`std::process::Command::get_envs`] will be [`None`].
    ///
    /// To clear all explicitly set environment variables and disable all environment variable
    /// inheritance, you can use [`std::process::Command::env_clear`].
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::Command;
    ///
    /// Command::new("ls")
    ///     .env_remove("PATH")
    ///     .spawn()
    ///     .expect("ls command failed to start");
    /// ```
    pub fn env_remove<K: AsRef<OsStr>>(&mut self, key: K) -> &mut Command {
        self.inner.env_remove(key);
        self
    }

    /// Clears all explicitly set environment variables and prevents inheriting any parent process
    /// environment variables.
    ///
    /// This method will remove all explicitly added environment variables set via [`std::process::Command::env`]
    /// or [`std::process::Command::envs`]. In addition, it will prevent the spawned child process from inheriting
    /// any environment variable from its parent process.
    ///
    /// After calling [`std::process::Command::env_clear`], the iterator from [`std::process::Command::get_envs`] will be
    /// empty.
    ///
    /// You can use [`std::process::Command::env_remove`] to clear a single mapping.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::Command;
    ///
    /// Command::new("ls")
    ///     .env_clear()
    ///     .spawn()
    ///     .expect("ls command failed to start");
    /// ```
    pub fn env_clear(&mut self) -> &mut Command {
        self.inner.env_clear();
        self
    }

    /// Sets the working directory for the child process.
    ///
    /// # Platform-specific behavior
    ///
    /// If the program path is relative (e.g., `"./script.sh"`), it's ambiguous
    /// whether it should be interpreted relative to the parent's working
    /// directory or relative to `current_dir`. The behavior in this case is
    /// platform specific and unstable, and it's recommended to use
    /// [`canonicalize`] to get an absolute program path instead.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::Command;
    ///
    /// Command::new("ls")
    ///     .current_dir("/bin")
    ///     .spawn()
    ///     .expect("ls command failed to start");
    /// ```
    ///
    /// [`canonicalize`]: crate::fs::canonicalize
    pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut Command {
        self.inner.current_dir(dir);
        self
    }

    /// Configuration for the child process's standard input (stdin) handle.
    ///
    /// Defaults to [`inherit`] when used with [`spawn`] or [`status`], and
    /// defaults to [`piped`] when used with [`output`].
    ///
    /// [`inherit`]: Stdio::inherit
    /// [`piped`]: Stdio::piped
    /// [`spawn`]: Self::spawn
    /// [`status`]: Self::status
    /// [`output`]: Self::output
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::{Command, Stdio};
    ///
    /// Command::new("ls")
    ///     .stdin(Stdio::null())
    ///     .spawn()
    ///     .expect("ls command failed to start");
    /// ```
    pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Command {
        self.inner.stdin(cfg);
        self.stdin_is_default = false;
        self
    }

    /// Configuration for the child process's standard output (stdout) handle.
    ///
    /// Defaults to [`inherit`] when used with [`spawn`] or [`status`], and
    /// defaults to [`piped`] when used with [`output`].
    ///
    /// [`inherit`]: Stdio::inherit
    /// [`piped`]: Stdio::piped
    /// [`spawn`]: Self::spawn
    /// [`status`]: Self::status
    /// [`output`]: Self::output
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::{Command, Stdio};
    ///
    /// Command::new("ls")
    ///     .stdout(Stdio::null())
    ///     .spawn()
    ///     .expect("ls command failed to start");
    /// ```
    pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Command {
        self.inner.stdout(cfg);
        self.stdout_is_default = false;
        self
    }

    /// Configuration for the child process's standard error (stderr) handle.
    ///
    /// Defaults to [`inherit`] when used with [`spawn`] or [`status`], and
    /// defaults to [`piped`] when used with [`output`].
    ///
    /// [`inherit`]: Stdio::inherit
    /// [`piped`]: Stdio::piped
    /// [`spawn`]: Self::spawn
    /// [`status`]: Self::status
    /// [`output`]: Self::output
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::{Command, Stdio};
    ///
    /// Command::new("ls")
    ///     .stderr(Stdio::null())
    ///     .spawn()
    ///     .expect("ls command failed to start");
    /// ```
    pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Command {
        self.inner.stderr(cfg);
        self.stderr_is_default = false;
        self
    }

    /// Executes the command as a child process, returning a handle to it.
    ///
    /// By default, stdin, stdout and stderr are set to null (diverging from
    /// the std library behavior of defaulting to inheriting from the parent).
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// use std::process::Command;
    ///
    /// Command::new("ls")
    ///     .spawn()
    ///     .expect("ls command failed to start");
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

        let child = self.inner.spawn();

        #[cfg(windows)]
        if self.kill_on_parent_process_close
            && let Ok(child) = child.as_ref()
        {
            let proc_handle = child.as_raw_handle() as isize;
            if let Err(e) = JobObject::new().assign_process(proc_handle).create() {
                log::error!(
                    "Failed to create job object for command {:?}: {:#}",
                    self.inner.get_program(),
                    e
                );
            }
        }

        child
    }

    /// Executes the command as a child process, waiting for it to finish and
    /// collecting all of its output.
    ///
    /// By default, stdout and stderr are captured (and used to provide the
    /// resulting output). Stdin is not inherited from the parent and any
    /// attempt by the child process to read from the stdin stream will result
    /// in the stream immediately closing.
    ///
    /// # Examples
    ///
    /// ```should_panic
    /// use std::process::Command;
    /// use std::io::{self, Write};
    /// let output = Command::new("/bin/cat")
    ///     .arg("file.txt")
    ///     .output()
    ///     .expect("failed to execute process");
    ///
    /// println!("status: {}", output.status);
    /// io::stdout().write_all(&output.stdout).unwrap();
    /// io::stderr().write_all(&output.stderr).unwrap();
    ///
    /// assert!(output.status.success());
    /// ```
    pub fn output(&mut self) -> io::Result<Output> {
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

    /// Executes a command as a child process, waiting for it to finish and
    /// collecting its status.
    ///
    /// By default, stdin, stdout and stderr are set to null (diverging from
    /// the std library behavior of defaulting to inheriting from the parent).
    ///
    /// # Examples
    ///
    /// ```should_panic
    /// use std::process::Command;
    ///
    /// let status = Command::new("/bin/cat")
    ///     .arg("file.txt")
    ///     .status()
    ///     .expect("failed to execute process");
    ///
    /// println!("process finished with: {status}");
    ///
    /// assert!(status.success());
    /// ```
    pub fn status(&mut self) -> io::Result<ExitStatus> {
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

    /// Returns the path to the program that was given to [`std::process::Command::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use std::process::Command;
    ///
    /// let cmd = Command::new("echo");
    /// assert_eq!(cmd.get_program(), "echo");
    /// ```
    #[must_use]
    pub fn get_program(&self) -> &OsStr {
        self.inner.get_program()
    }

    /// Returns an iterator of the arguments that will be passed to the program.
    ///
    /// This does not include the path to the program as the first argument;
    /// it only includes the arguments specified with [`std::process::Command::arg`] and
    /// [`std::process::Command::args`].
    ///
    /// # Examples
    ///
    /// ```
    /// use std::ffi::OsStr;
    /// use std::process::Command;
    ///
    /// let mut cmd = Command::new("echo");
    /// cmd.arg("first").arg("second");
    /// let args: Vec<&OsStr> = cmd.get_args().collect();
    /// assert_eq!(args, &["first", "second"]);
    /// ```
    pub fn get_args(&self) -> CommandArgs<'_> {
        self.inner.get_args()
    }

    /// Returns an iterator of the environment variables explicitly set for the child process.
    ///
    /// Environment variables explicitly set using [`std::process::Command::env`], [`std::process::Command::envs`], and
    /// [`std::process::Command::env_remove`] can be retrieved with this method.
    ///
    /// Note that this output does not include environment variables inherited from the parent
    /// process.
    ///
    /// Each element is a tuple key/value pair `(&OsStr, Option<&OsStr>)`. A [`None`] value
    /// indicates its key was explicitly removed via [`std::process::Command::env_remove`]. The associated key for
    /// the [`None`] value will no longer inherit from its parent process.
    ///
    /// An empty iterator can indicate that no explicit mappings were added or that
    /// [`std::process::Command::env_clear`] was called. After calling [`std::process::Command::env_clear`], the child process
    /// will not inherit any environment variables from its parent process.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::ffi::OsStr;
    /// use std::process::Command;
    ///
    /// let mut cmd = Command::new("ls");
    /// cmd.env("TERM", "dumb").env_remove("TZ");
    /// let envs: Vec<(&OsStr, Option<&OsStr>)> = cmd.get_envs().collect();
    /// assert_eq!(envs, &[
    ///     (OsStr::new("TERM"), Some(OsStr::new("dumb"))),
    ///     (OsStr::new("TZ"), None)
    /// ]);
    /// ```
    pub fn get_envs(&self) -> CommandEnvs<'_> {
        self.inner.get_envs()
    }

    /// Returns the working directory for the child process.
    ///
    /// This returns [`None`] if the working directory will not be changed.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    /// use std::process::Command;
    ///
    /// let mut cmd = Command::new("ls");
    /// assert_eq!(cmd.get_current_dir(), None);
    /// cmd.current_dir("/bin");
    /// assert_eq!(cmd.get_current_dir(), Some(Path::new("/bin")));
    /// ```
    #[must_use]
    pub fn get_current_dir(&self) -> Option<&Path> {
        self.inner.get_current_dir()
    }

    /// Schedules a closure to be run just before the `exec` function is
    /// invoked.
    ///
    /// The closure is allowed to return an I/O error whose OS error code will
    /// be communicated back to the parent and returned as an error from when
    /// the spawn was requested.
    ///
    /// Multiple closures can be registered and they will be called in order of
    /// their registration. If a closure returns `Err` then no further closures
    /// will be called and the spawn operation will immediately return with a
    /// failure.
    ///
    /// # Safety
    ///
    /// This closure will be run in the context of the child process after a
    /// `fork`. This primarily means that any modifications made to memory on
    /// behalf of this closure will **not** be visible to the parent process.
    /// This is often a very constrained environment where normal operations
    /// like `malloc`, accessing environment variables through [`std::env`]
    /// or acquiring a mutex are not guaranteed to work (due to
    /// other threads perhaps still running when the `fork` was run).
    ///
    /// For further details refer to the [POSIX fork() specification]
    /// and the equivalent documentation for any targeted
    /// platform, especially the requirements around *async-signal-safety*.
    ///
    /// This also means that all resources such as file descriptors and
    /// memory-mapped regions got duplicated. It is your responsibility to make
    /// sure that the closure does not violate library invariants by making
    /// invalid use of these duplicates.
    ///
    /// Panicking in the closure is safe only if all the format arguments for the
    /// panic message can be safely formatted; this is because although
    /// `Command` calls [`std::panic::always_abort`](crate::panic::always_abort)
    /// before calling the pre_exec hook, panic will still try to format the
    /// panic message.
    ///
    /// When this closure is run, aspects such as the stdio file descriptors and
    /// working directory have successfully been changed, so output to these
    /// locations might not appear where intended.
    ///
    /// [POSIX fork() specification]:
    ///     https://pubs.opengroup.org/onlinepubs/9699919799/functions/fork.html
    /// [`std::env`]: mod@crate::env
    #[cfg(unix)]
    pub unsafe fn pre_exec<F>(&mut self, f: F) -> &mut Self
    where
        F: FnMut() -> io::Result<()> + Send + Sync + 'static,
    {
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut self.inner, f);
        }
        self
    }

    /// Configures the spawned child process to be killed when the parent
    /// process is closed.
    #[cfg(windows)]
    pub fn kill_on_parent_process_close(&mut self) -> &mut Self {
        self.kill_on_parent_process_close = true;
        self
    }
}
