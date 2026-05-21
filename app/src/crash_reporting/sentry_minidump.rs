//! Native crash reporting adapter that uses the [`minidumper`] crate with Sentry. This allows us
//! to capture and report application crashes due to Unix signals like SIGSEGV (segfault)
//! or Windows exceptions [https://learn.microsoft.com/en-us/windows/win32/debug/structured-exception-handling].
//!
//! This is inspired by [`sentry-rust-minidump`](https://github.com/timfish/sentry-rust-minidump),
//! with a few important changes:
//! * Support for starting and stopping the crash-reporting process, since users can toggle crash reporting at runtime
//! * Startup via our command-line parsing, rather than a separate hook
//! * Use of anonymous, temporary crash dump files, to ensure they're cleaned up

use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read as _, Seek as _, Write},
    path::{Path, PathBuf},
    process,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::Context as _;
use command::blocking::Command;
use crash_handler::{CrashContext, CrashHandler};
use lazy_static::lazy_static;
use parking_lot::Mutex;
use sentry::{
    protocol::{Attachment, AttachmentType},
    Breadcrumb, Level,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_core::report_error;

use super::ToSentryTags;

lazy_static! {
    static ref GUARD: Mutex<Option<MinidumpGuard>> = Mutex::new(None);
}

/// The minidump child process will exit if it doesn't receive a message after some time. This
/// ensures that if the parent process exits without cleaning it up, the child won't linger
/// forever. We ping the child every `PING_INTERVAL` to make sure it doesn't quit while the
/// parent (this process) is running.
const PING_INTERVAL: Duration = Duration::from_secs(5);

/// Initialize the minidump reporter.
pub fn init() {
    let mut global_guard = GUARD.lock();

    match MinidumpGuard::start() {
        Ok(guard) => {
            *global_guard = Some(guard);
        }
        Err(err) => {
            report_error!(err);
        }
    }
}

/// Uninitialize the minidump reporter.
pub fn uninit() {
    let maybe_guard = { GUARD.lock().take() };
    // Ensure we drop the `MinidumpGuard` after releasing the GUARD mutex. If there's an
    // error stopping the server, we should log it as a Sentry breadcrumb in the Warp
    // process, but not forward the breadcrumb to the server process.
    std::mem::drop(maybe_guard);
}

/// Set a tag to include in minidump crash reports.
pub fn set_tag(key: String, value: String) {
    let global_guard = GUARD.lock();
    if let Some(guard) = global_guard.as_ref() {
        guard.set_tags(HashMap::from([(key, value)]));
    }
}

/// Set tags to include in minidump crash reports, using a type that implements [`ToSentryTags`].
pub fn set_tags_from<T: ToSentryTags>(tags: &T) {
    let global_guard = GUARD.lock();
    if let Some(guard) = global_guard.as_ref() {
        let tags = tags
            .to_sentry_tags()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        guard.set_tags(tags);
    }
}

/// Set the user id to include in minidump crash reports.
pub fn set_user_id(user_id: &str) {
    let global_guard = GUARD.lock();
    if let Some(guard) = global_guard.as_ref() {
        guard.set_user_id(user_id.to_owned());
    }
}

/// Forward a breadcrumb to attach to minidump crash reports.
pub fn forward_breadcrumb(breadcrumb: Breadcrumb) {
    let global_guard = GUARD.lock();
    if let Some(guard) = global_guard.as_ref() {
        guard.add_breadcrumb(breadcrumb);
    }
}

/// Send a crash report via minidump. On certain platforms, this will produce an error report
/// without actually crashing the process.
pub fn crash() {
    let global_guard = GUARD.lock();
    if let Some(guard) = global_guard.as_ref() {
        guard.crash();
    }
}

/// Handle for minidump state that must be kept in scope while crash reporting is enabled.
pub struct MinidumpGuard {
    child: process::Child,
    client: Arc<minidumper::Client>,
    crash_handler: CrashHandler,
}

/// Run the minidump server process.
pub fn run_server(socket_path: &Path) -> anyhow::Result<()> {
    // For troubleshooting, attempt to log from the minidump server. There's not much we can really
    // do if crash reporting fails, so creating the log file itself is best-effort.
    let log_dir = warp_core::paths::state_dir().join(warp_core::paths::WARP_LOGS_DIR);
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("warp-minidump.log");
    let log_target = File::create(log_path)
        .map(|file| env_logger::Target::Pipe(Box::new(file)))
        .unwrap_or_else(|_| env_logger::Target::Stdout);
    env_logger::builder()
        .parse_default_env()
        .target(log_target)
        .init();

    let _guard = sentry::init(super::sentry_client_options());

    struct Handler {
        shutdown: Arc<AtomicBool>,
        pending_crash_details: Mutex<Option<String>>,
    }

    impl minidumper::ServerHandler for Handler {
        fn create_minidump_file(&self) -> Result<(File, PathBuf), io::Error> {
            // Use an anonymous temporary file for crash dumps. The path isn't used when writing a
            // dump, so we can use an empty value.
            let file = tempfile::tempfile()?;
            Ok((file, PathBuf::new()))
        }

        fn on_minidump_created(
            &self,
            result: Result<minidumper::MinidumpBinary, minidumper::Error>,
        ) -> minidumper::LoopAction {
            if let Err(ref err) = &result {
                log::warn!("Unable to create minidump file: {err:#}");
            }

            let crash_details = self.pending_crash_details.lock().take();
            send_crash_report(crash_details, result.ok());

            minidumper::LoopAction::Exit
        }

        fn on_client_disconnected(&self, num_clients: usize) -> minidumper::LoopAction {
            if num_clients == 0 {
                log::info!("All clients disconnected, shutting down minidump server");
                minidumper::LoopAction::Exit
            } else {
                minidumper::LoopAction::Continue
            }
        }

        fn on_message(&self, _kind: u32, buffer: Vec<u8>) {
            match bincode::deserialize::<MinidumpCommand>(&buffer) {
                Ok(MinidumpCommand::Shutdown) => {
                    self.shutdown.store(true, Ordering::Relaxed);
                }
                Ok(MinidumpCommand::SetTags { tags }) => {
                    sentry::configure_scope(|scope| {
                        for (key, value) in tags {
                            scope.set_tag(&key, value);
                        }
                    });
                }
                Ok(MinidumpCommand::SetUser { user_id }) => {
                    sentry::configure_scope(|scope| {
                        scope.set_user(Some(sentry::User {
                            id: Some(user_id),
                            ..Default::default()
                        }));
                    });
                }
                Ok(MinidumpCommand::AddBreadcrumb { breadcrumb }) => {
                    sentry::add_breadcrumb(breadcrumb);
                }
                Ok(MinidumpCommand::SetCrashDetails { details }) => {
                    *self.pending_crash_details.lock() = Some(details);
                }
                Err(err) => {
                    log::warn!("Unable to deserialize minidump command: {err:#}");
                }
            }
        }
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    let handler = Box::new(Handler {
        shutdown: shutdown.clone(),
        pending_crash_details: Default::default(),
    });

    log::info!(
        "Starting minidump server listening on {}",
        socket_path.display()
    );
    let result = minidumper::Server::with_name(socket_path)
        .context("Unable to create minidump server")?
        .run(handler, &shutdown, Some(2 * PING_INTERVAL))
        .context("Error running minidump server");
    if let Err(ref err) = result {
        log::error!("Error running minidump server: {err:#}");
    }

    result
}

/// Uploads a crash report to Sentry, using the current scope.
fn send_crash_report(details: Option<String>, dump: Option<minidumper::MinidumpBinary>) {
    let message = details.as_deref().unwrap_or("Fatal exception");

    let crash_attachment = dump.and_then(|mut dump| {
        // In most cases, the minidump contents are available in memory. If not, we can read them off disk.
        let buffer = match dump.contents {
            Some(buffer) => buffer,
            None => {
                dump.file.flush().ok()?;
                dump.file.rewind().ok()?;
                let mut buffer = Vec::new();
                dump.file.read_to_end(&mut buffer).ok()?;
                buffer
            }
        };

        Some(Attachment {
            buffer,
            filename: "warp-minidump.dmp".to_string(),
            ty: Some(AttachmentType::Minidump),
            ..Default::default()
        })
    });

    sentry::with_scope(
        |scope| {
            // Do not use the crash reporting server for process info.
            scope.remove_extra("event.process");
            if let Some(attachment) = crash_attachment {
                scope.add_attachment(attachment);
            }
        },
        || sentry::capture_message(message, Level::Error),
    );
}

impl MinidumpGuard {
    // NOTE: We CANNOT use `report_error`, `report_if_error`, `log`, or similar here. Those
    // all send information to Sentry, which can deadlock.

    /// Set up minidump-backed crash reporting. This spawns a child process that reports crashes to
    /// Sentry, and a crash handler which sends crashes to that child process.
    pub fn start() -> anyhow::Result<Self> {
        let socket_name = format!("wcr-{}.sock", Uuid::new_v4().simple());
        let socket_path = if cfg!(target_os = "macos") {
            // On macOS, the maximum length of a socket path is fairly short, so use the temp directory.
            std::env::temp_dir().join(socket_name)
        } else {
            warp_core::paths::state_dir().join(socket_name)
        };

        let child =
            Command::new(std::env::current_exe().context("Unable to get current executable path")?)
                .arg("minidump-server")
                .arg(&socket_path)
                .spawn()
                .context("Unable to spawn minidump server process")?;

        let client = Arc::new(
            wait_for_server(socket_path.as_path()).context("Unable to create minidump client")?,
        );
        spawn_keepalive_thread(client.clone());

        let client2 = client.clone();

        let crash_handler = CrashHandler::attach(unsafe {
            crash_handler::make_crash_event(move |crash_context: &CrashContext| {
                if let Some(details) = format_crash_details(crash_context) {
                    let _ = send_command(
                        client.as_ref(),
                        MinidumpCommand::SetCrashDetails { details },
                    );
                }

                // Send a ping to the minidump server, ensuring that any messages sent before the
                // crash event are flushed and processed. This mostly only matters on macOS.
                let _ = client.ping();

                let dump_result = client.request_dump(crash_context);
                crash_handler::CrashEventResult::Handled(dump_result.is_ok())
            })
        })
        .context("Failed to attach crash signal handler")?;

        // Ensure that the crash server process can ptrace Warp.
        #[cfg(target_os = "linux")]
        crash_handler.set_ptracer(Some(child.id()));

        let guard = MinidumpGuard {
            child,
            client: client2,
            crash_handler,
        };

        // Forward any existing tags to the minidump server.
        guard.set_tags(super::TAGS.read().clone());

        Ok(guard)
    }

    /// Send the user id for the minidump server to attach to Sentry events.
    fn set_user_id(&self, user_id: String) {
        let _ = send_command(self.client.as_ref(), MinidumpCommand::SetUser { user_id });
    }

    /// Send tags for the minidump server to attach to Sentry events.
    fn set_tags(&self, tags: HashMap<String, String>) {
        let _ = send_command(self.client.as_ref(), MinidumpCommand::SetTags { tags });
    }

    /// Add a breadcrumb to crash reports produced by the minidump server.
    fn add_breadcrumb(&self, breadcrumb: Breadcrumb) {
        let _ = send_command(
            self.client.as_ref(),
            MinidumpCommand::AddBreadcrumb { breadcrumb },
        );
    }

    /// Simulate a crash.
    pub fn crash(&self) {
        #[cfg(target_os = "linux")]
        self.crash_handler.simulate_signal(libc::SIGSEGV as _);
        #[cfg(not(target_os = "linux"))]
        self.crash_handler.simulate_exception(None);
    }
}

impl Drop for MinidumpGuard {
    fn drop(&mut self) {
        // Dropping the crash handler will detach it.
        // We can report errors here, as the minidump handler is no longer active.

        // Send a graceful shutdown command before killing the child process.
        if let Err(err) = send_command(&self.client, MinidumpCommand::Shutdown) {
            log::warn!("Unable to send shutdown command to minidump child process: {err:#}");
        }

        if let Err(err) = self.child.kill() {
            log::warn!("Unable to kill minidump child process: {err:#}");
        }
    }
}

/// Wait for the minidump server to start and return a client handle.
///
/// Creating a [`minidumper::Client`] will fail unless the server has started.
fn wait_for_server(socket_path: &Path) -> anyhow::Result<minidumper::Client> {
    let start = instant::Instant::now();

    let mut last_error = None;
    while start.elapsed() < Duration::from_secs(1) {
        match minidumper::Client::with_name(socket_path) {
            Ok(client) => {
                return Ok(client);
            }
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    match last_error {
        Some(err) => Err(err.into()),
        None => Err(anyhow::anyhow!("Unable to connect to minidump server")),
    }
}

/// Spawn a thread that periodically pings the minidump server to prevent it from idling out.
fn spawn_keepalive_thread(client: Arc<minidumper::Client>) {
    let _ = std::thread::Builder::new()
        .name("minidump-keepalive".to_string())
        .spawn(move || loop {
            // Assume that if a ping fails, the server was shut down - the only purpose of this thread
            // is to prevent an idle timeout.
            if client.ping().is_err() {
                return;
            }
            std::thread::sleep(PING_INTERVAL);
        });
}

/// Use `client` to send a command to the minidump server.
fn send_command(client: &minidumper::Client, command: MinidumpCommand) -> anyhow::Result<()> {
    let message = bincode::serialize(&command).context("Failed to serialize minidump command")?;
    client
        .send_message(0, message)
        .context("Failed to send minidump command")?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
enum MinidumpCommand {
    Shutdown,
    SetTags { tags: HashMap<String, String> },
    SetUser { user_id: String },
    AddBreadcrumb { breadcrumb: Breadcrumb },
    SetCrashDetails { details: String },
}

/// Format details from a [`CrashContext`] into a Sentry error message. This information should
/// already be in the minidump, but it's useful to surface prominently in Sentry.
fn format_crash_details(crash_context: &CrashContext) -> Option<String> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "linux")] {
            Some(format!("Killed by signal {} / {}", crash_context.siginfo.ssi_signo, crash_context.siginfo.ssi_code))
        } else if #[cfg(target_os = "windows")] {
            Some(format!("Exception {}", crash_context.exception_code))
        } else if #[cfg(target_os = "macos")] {
            crash_context.exception.as_ref().map(|exception| {
                format!("Exception {} ({} / {:?})", exception.kind, exception.code, exception.subcode)
            })
        } else {
            None
        }
    }
}
