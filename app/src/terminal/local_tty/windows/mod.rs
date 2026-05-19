mod child;
mod conpty_api;
mod environment;
mod pipes;
mod proc_thread_attribute_list;

use super::event_loop::{PTY_TOKEN, SIGNALS_TOKEN};
use super::shell::{DirectShellStarter, ShellStarter, WslShellStarter};
use super::spawner::PtyHandle;
use super::{mio_channel, EventedPty, EventedReadWrite, PtyOptions, SizeInfo};
use crate::terminal::local_tty::spawner::{PtySpawnInfo, PtySpawner};
use crate::terminal::writeable_pty;
use child::ChildExitWatcher;
use conpty_api::ConptyApiError;
use environment::get_shell_environment_variables;
pub use environment::get_user_and_system_env_variable;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle as _;
use std::path::PathBuf;
use thiserror::Error;
use warpui::{AppContext, SingletonEntity};
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Console::{COORD, HPCON};
use windows::Win32::System::Threading::{
    CreateProcessW, WaitForSingleObject, CREATE_BREAKAWAY_FROM_JOB, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW,
};

use crate::terminal::local_tty::windows::proc_thread_attribute_list::ProcThreadAttributeList;
pub use conpty_api::ConptyApi;

trait ToCoord {
    fn to_coord(&self) -> COORD;
}

impl ToCoord for SizeInfo {
    fn to_coord(&self) -> COORD {
        COORD {
            X: self.columns as i16,
            Y: self.rows as i16,
        }
    }
}

/// A wrapper around [`HANDLE`] that is marked as being safe to share between
/// threads.
#[derive(Clone, Copy)]
struct ShareableHandle(pub HANDLE);

// Mark `ShareableHandle` as being safe to share between threads,
// even though `HANDLE` holds a `*mut c_void`, which isn't inherently
// safe to share.
unsafe impl Send for ShareableHandle {}
unsafe impl Sync for ShareableHandle {}

pub(super) struct PtySpawnResult {
    pub pty_handle: HPCON,
    pub pipe: mio::windows::NamedPipe,
    pub conpty_api: ConptyApi,
    child_exit_watcher: ChildExitWatcher,
}

pub struct PseudoConsoleChild {
    process_info: PROCESS_INFORMATION,
}

// Mark `ChildExitWatcher` as being safe to share between threads,
// even though `PROCESS_INFORMATION` holds `HANDLE`s, which each hold
// a `*mut c_void`, which isn't inherently safe to share.
unsafe impl Send for PseudoConsoleChild {}
unsafe impl Sync for PseudoConsoleChild {}

impl PseudoConsoleChild {
    pub fn id(&self) -> u32 {
        self.process_info.dwProcessId
    }

    pub fn is_terminated(&self) -> bool {
        let wait_event = unsafe { WaitForSingleObject(self.process_info.hProcess, 0) };
        wait_event == WAIT_OBJECT_0
    }
}

#[derive(Error, Debug)]
pub enum PtySpawnError {
    #[error("Could not create anonymous pipes for PTY input/output: {0:#}")]
    CreateAnonymousPipesFailed(#[source] std::io::Error),
    #[error("Could not create pipe for PTY input/output: {0:#}")]
    CreatePipeFailed(#[source] pipes::CreatePipeError),
    #[error("Could not create a pseudoconsole: {0:#}")]
    CreatePseudoConsoleFailed(#[source] windows::core::Error),
    #[error("Failed to initialize the thread attribute list: {0:#}")]
    InitializeThreadAttributeListFailed(#[source] windows::core::Error),
    #[error("Failed to set psuedoconsole attribute to thread attribute list: {0:#}")]
    SetThreadAttributeListFailed(#[source] windows::core::Error),
    #[error("Failed to encode shell command: {0:#}")]
    CreateShellCommandFailed(#[from] EncodingError),
    #[error("Failed to create shell process {detail}: {error:#}")]
    CreateShellProcessFailed {
        detail: String,
        #[source]
        error: windows::core::Error,
    },
    #[error("Failed to load ConPTY functions: {0:#}")]
    LoadConPtyApiFailed(#[from] ConptyApiError),
    #[error("Failed to add Child Exit watcher: {0:#}")]
    ChildExitWatcherFailed(#[source] windows::core::Error),
    #[error("Failed to write shell command bytes to pty: {0:#}")]
    FailedToWriteCommandBytes(#[source] std::io::Error),
    #[error("Shell starter is not supported on this platform: {0}")]
    UnsupportedShellStarter(String),
}

pub(super) fn spawn(
    options: PtyOptions,
    event_loop_tx: mio_channel::Sender<writeable_pty::Message>,
) -> Result<PtySpawnInfo, PtySpawnError> {
    let conpty_api = unsafe { ConptyApi::load() }?;
    let environment_block = get_shell_environment_variables(&options);

    let PtyOptions {
        size,
        shell_starter,
        ..
    } = options;

    let pipes::DuplexPipe { client, server } =
        pipes::create_async_anonymous_pipe().map_err(PtySpawnError::CreatePipeFailed)?;

    // Create the pseudoconsole, giving it the handle to the client side of the pipe.
    let pty_handle = match unsafe { conpty_api.create(size.to_coord(), client, 0) } {
        Ok(pty_handle) => pty_handle,
        Err(err) => return Err(PtySpawnError::CreatePseudoConsoleFailed(err)),
    };

    // Tell the pseudoconsole that it is already visible.
    let _ = unsafe { conpty_api.show_hide(pty_handle, true) };

    // Spawn the child process, and tell it to communicate via the pseudoconsole.
    // The default zeros the memory.
    let mut startup_info = STARTUPINFOEXW::default();
    startup_info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup_info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;

    let mut attrs = unsafe {
        ProcThreadAttributeList::new()
            .map_err(PtySpawnError::InitializeThreadAttributeListFailed)?
    };
    attrs
        .set_pty_connection(pty_handle)
        .map_err(PtySpawnError::SetThreadAttributeListFailed)?;
    startup_info.lpAttributeList = attrs.as_mut_ptr();

    // Create the hosted shell process.
    let shell_command = match &shell_starter {
        ShellStarter::Direct(shell_starter) | ShellStarter::MSYS2(shell_starter) => {
            shell_command(shell_starter)?
        }
        ShellStarter::Wsl(wsl_shell_starter) => wsl_shell_command(wsl_shell_starter)?,
        ShellStarter::DockerSandbox(_) => {
            // Docker sandbox shells are only supported on Unix; they should
            // never reach the Windows PTY spawn path. Surface as an error
            // rather than panicking so a rogue persisted/round-tripped
            // sandbox starter degrades gracefully on Windows.
            log::error!("Docker sandbox shell starter reached the Windows PTY spawn path");
            return Err(PtySpawnError::UnsupportedShellStarter(
                "Docker sandbox shells are not supported on Windows".to_owned(),
            ));
        }
    };
    let mut process_information = PROCESS_INFORMATION::default();

    let start_directory_path = options
        .start_dir
        .filter(|start_dir| start_dir.is_dir())
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .filter(|path| path.is_dir())
        });
    let start_directory_wide = start_directory_path
        .as_ref()
        .map(|path| path.as_os_str().encode_wide().collect::<Vec<u16>>());
    let start_directory = start_directory_path
        .as_ref()
        .map(|path| HSTRING::from(path.as_os_str()));

    let creation_flags = PROCESS_CREATION_FLAGS(0)
        | EXTENDED_STARTUPINFO_PRESENT
        | CREATE_UNICODE_ENVIRONMENT
        | CREATE_BREAKAWAY_FROM_JOB;

    unsafe {
        CreateProcessW(
            PCWSTR::null(), /* lpApplicationName */
            Some(PWSTR::from_raw(shell_command.raw.as_ptr().cast_mut())),
            None,  /* lpProcessAttributes */
            None,  /* lpThreadAttributes */
            false, /* bInheritHandles */
            creation_flags,
            Some(environment_block.as_ptr() as *const std::ffi::c_void),
            start_directory
                .as_ref()
                .map(|hstring| PCWSTR::from_raw(hstring.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            &startup_info.StartupInfo as *const STARTUPINFOW,
            &mut process_information,
        )
        .map_err(|error| {
            let detail = shell_starter.shell_detail();
            log_create_process_failure(
                &error,
                &detail,
                &shell_command,
                &environment_block,
                start_directory_wide.as_deref(),
                creation_flags,
                &startup_info,
                size.columns,
                size.rows,
            );
            PtySpawnError::CreateShellProcessFailed { detail, error }
        })?;
    }

    let _ = unsafe { conpty_api.release(pty_handle) };

    let pipe = unsafe { mio::windows::NamedPipe::from_raw_handle(server.0 as *mut _) };
    let child_exit_watcher = ChildExitWatcher::new(process_information.hProcess, event_loop_tx)
        .map_err(PtySpawnError::ChildExitWatcherFailed)?;
    let child = PseudoConsoleChild {
        process_info: process_information,
    };
    let result = PtySpawnResult {
        pty_handle,
        pipe,
        conpty_api,
        child_exit_watcher,
    };
    Ok(PtySpawnInfo { result, child })
}
#[derive(Error, Debug)]
pub enum EncodingError {
    #[error("Could not encode argument {arg:?}")]
    InvalidArgumentEncoding { arg: OsString },
    #[error(transparent)]
    Other(#[from] windows::core::Error),
}

struct EncodedShellCommand {
    raw: HSTRING,
    wide_units: Vec<u16>,
}

impl EncodedShellCommand {
    fn new(wide_units: Vec<u16>) -> Self {
        Self {
            raw: HSTRING::from_wide(&wide_units),
            wide_units,
        }
    }
}

fn wsl_shell_command(
    wsl_shell_starter: &WslShellStarter,
) -> Result<EncodedShellCommand, EncodingError> {
    let mut encoded_shell_command = Vec::<u16>::new();

    log::info!(
        "Starting WSL shell process: {:?}",
        wsl_shell_starter.distribution()
    );
    append_quoted(&WslShellStarter::wsl_command(), &mut encoded_shell_command);
    for arg in wsl_shell_starter.args() {
        encoded_shell_command.push(' ' as u16);
        if arg.encode_wide().any(|c| c == 0) {
            return Err(EncodingError::InvalidArgumentEncoding { arg: arg.clone() });
        }
        append_quoted(arg, &mut encoded_shell_command);
    }
    Ok(EncodedShellCommand::new(encoded_shell_command))
}

/// Constructs the shell command in a Windows-native encoding using 16-bit values.
///
/// Windows expects a single string for its command which includes all arguments, so we need to
/// string them together and escape them appropriately.
fn shell_command(shell_starter: &DirectShellStarter) -> Result<EncodedShellCommand, EncodingError> {
    let mut encoded_shell_command = Vec::<u16>::new();

    log::info!(
        "Starting direct shell process: {:?}",
        shell_starter.logical_shell_path()
    );
    append_quoted(
        shell_starter.logical_shell_path().as_os_str(),
        &mut encoded_shell_command,
    );

    for arg in shell_starter.args() {
        encoded_shell_command.push(' ' as u16);
        if arg.encode_wide().any(|c| c == 0) {
            return Err(EncodingError::InvalidArgumentEncoding { arg: arg.clone() });
        }
        append_quoted(arg, &mut encoded_shell_command);
    }
    Ok(EncodedShellCommand::new(encoded_shell_command))
}

fn log_create_process_failure(
    error: &windows::core::Error,
    detail: &str,
    shell_command: &EncodedShellCommand,
    environment_block: &[u16],
    start_directory: Option<&[u16]>,
    creation_flags: PROCESS_CREATION_FLAGS,
    startup_info: &STARTUPINFOEXW,
    columns: usize,
    rows: usize,
) {
    let shell_command_bytes = wide_units_as_le_bytes_with_nul(&shell_command.wide_units, 1);
    let environment_block_bytes = wide_units_as_le_bytes(environment_block);
    let start_directory_bytes = start_directory
        .map(|wide| wide_units_as_le_bytes_with_nul(wide, 1))
        .unwrap_or_default();

    log::error!(
        "CreateProcessW failed for shell process {detail}: {error:#}. \
        debug lpApplicationName_is_null=true \
        lpCommandLine_utf16_units_len={} lpCommandLine_bytes={:?} \
        lpEnvironment_utf16_units_len={} lpEnvironment_bytes={:?} \
        lpCurrentDirectory_is_null={} lpCurrentDirectory_utf16_units_len={} lpCurrentDirectory_bytes={:?} \
        dwCreationFlags={:#010x} bInheritHandles=false \
        startup_cb={} startup_dwFlags={:#010x} startup_lpAttributeList_is_null={} \
        conpty_size_columns={} conpty_size_rows={}",
        shell_command.wide_units.len() + 1,
        shell_command_bytes,
        environment_block.len(),
        environment_block_bytes,
        start_directory.is_none(),
        start_directory.map_or(0, |wide| wide.len() + 1),
        start_directory_bytes,
        creation_flags.0,
        startup_info.StartupInfo.cb,
        startup_info.StartupInfo.dwFlags.0,
        startup_info.lpAttributeList.is_null(),
        columns,
        rows,
    );
}

fn wide_units_as_le_bytes(wide_units: &[u16]) -> Vec<u8> {
    wide_units
        .iter()
        .flat_map(|unit| unit.to_le_bytes())
        .collect()
}

fn wide_units_as_le_bytes_with_nul(wide_units: &[u16], trailing_nuls: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((wide_units.len() + trailing_nuls) * 2);
    bytes.extend(wide_units_as_le_bytes(wide_units));
    bytes.resize(bytes.len() + trailing_nuls * 2, 0);
    bytes
}

/// Appends an argument and properly quotes it.
///
/// See: https://learn.microsoft.com/en-us/archive/blogs/twistylittlepassagesallalike/everyone-quotes-command-line-arguments-the-wrong-way
fn append_quoted(arg: &OsStr, cmdline: &mut Vec<u16>) {
    if !arg.is_empty()
        && !arg.encode_wide().any(|c| {
            c == ' ' as u16
                || c == '\t' as u16
                || c == '\n' as u16
                || c == '\x0b' as u16
                || c == '\"' as u16
        })
    {
        cmdline.extend(arg.encode_wide());
        return;
    }
    cmdline.push('"' as u16);

    let arg: Vec<_> = arg.encode_wide().collect();
    let mut i = 0;
    while i < arg.len() {
        let mut num_backslashes = 0;
        while i < arg.len() && arg[i] == '\\' as u16 {
            i += 1;
            num_backslashes += 1;
        }

        if i == arg.len() {
            for _ in 0..num_backslashes * 2 {
                cmdline.push('\\' as u16);
            }
            break;
        } else if arg[i] == b'"' as u16 {
            for _ in 0..num_backslashes * 2 + 1 {
                cmdline.push('\\' as u16);
            }
            cmdline.push(arg[i]);
        } else {
            for _ in 0..num_backslashes {
                cmdline.push('\\' as u16);
            }
            cmdline.push(arg[i]);
        }
        i += 1;
    }
    cmdline.push('"' as u16);
}

pub struct Pty {
    handle: Box<dyn PtyHandle>,
    /// An arbitrary type on Windows used to interact with the psuedoconsole.
    pty_handle: HPCON,
    pipe: mio::windows::NamedPipe,
    token: mio::Token,
    conpty_api: ConptyApi,
    child_exit_watcher: ChildExitWatcher,
}

impl Pty {
    pub fn new(
        options: PtyOptions,
        is_crash_reporting_enabled: bool,
        event_loop_tx: mio_channel::Sender<writeable_pty::Message>,
        ctx: &mut AppContext,
    ) -> anyhow::Result<Self> {
        let size = options.size;
        PtySpawner::handle(ctx)
            .update(ctx, |pty_spawner, ctx| {
                pty_spawner.spawn_pty(options, is_crash_reporting_enabled, event_loop_tx, ctx)
            })
            .map(
                |(
                    PtySpawnResult {
                        pty_handle,
                        pipe,
                        conpty_api,
                        child_exit_watcher,
                    },
                    handle,
                )| {
                    let mut pty = Self {
                        handle,
                        pty_handle,
                        pipe,
                        token: PTY_TOKEN,
                        conpty_api,
                        child_exit_watcher,
                    };
                    pty.on_resize(&size);
                    pty
                },
            )
    }

    pub fn get_pid(&self) -> u32 {
        self.handle.pid()
    }
}

impl EventedReadWrite for Pty {
    type Reader = mio::windows::NamedPipe;
    type Writer = mio::windows::NamedPipe;

    fn register(&mut self, poll: &mio::Poll, interest: mio::Interest) -> std::io::Result<()> {
        poll.registry()
            .register(&mut self.pipe, self.token, interest)?;
        poll.registry().register(
            &mut self.child_exit_watcher,
            SIGNALS_TOKEN,
            mio::Interest::READABLE,
        )
    }

    fn reregister(&mut self, poll: &mio::Poll, interest: mio::Interest) -> std::io::Result<()> {
        poll.registry()
            .reregister(&mut self.pipe, self.token, interest)?;
        poll.registry().reregister(
            &mut self.child_exit_watcher,
            SIGNALS_TOKEN,
            mio::Interest::READABLE,
        )
    }

    fn deregister(&mut self, poll: &mio::Poll) -> std::io::Result<()> {
        poll.registry().deregister(&mut self.pipe)?;
        poll.registry().deregister(&mut self.child_exit_watcher)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.pipe
    }

    fn read_token(&self) -> mio::Token {
        self.token
    }

    fn writer(&mut self) -> &mut Self::Writer {
        &mut self.pipe
    }

    fn write_token(&self) -> mio::Token {
        self.token
    }
}

impl EventedPty for Pty {
    fn child_event_token(&self) -> mio::Token {
        SIGNALS_TOKEN
    }

    fn next_child_event(&mut self) -> Option<super::ChildEvent> {
        None
    }

    fn on_resize(&mut self, size: &crate::terminal::SizeInfo) {
        if let Err(err) = unsafe { self.conpty_api.resize(self.pty_handle, size.to_coord()) } {
            log::error!("Failed to resize pseudoconsole: {err:?}");
        }
    }

    fn kill(self) -> anyhow::Result<()> {
        Ok(())
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        use std::io::Read as _;

        // Ask the pseudoconsole to close.
        unsafe {
            self.conpty_api.close(self.pty_handle);
        }

        // Drain all data in the pipe.
        let mut buffer = [0; 1000];
        while let Ok(num_byes_read) = self.pipe.read(&mut buffer) {
            if num_byes_read == 0 {
                break;
            }
        }

        // Finally, disconnect from the console host, which will ultimately
        // get it to terminate.  We don't care if there is an error here.
        let _ = self.pipe.disconnect();
    }
}
