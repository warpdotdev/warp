use std::os::unix::prelude::*;
use std::sync::{Arc, mpsc};

use parking_lot::Mutex;

use super::{api, protocol};

/// A logger which forwards log entries from the terminal server to the host
/// application process via a Unix socket.
pub(super) struct RemoteLogger {
    /// A Sender for sending log messages to the thread that interacts with
    /// the send socket.
    tx: mpsc::Sender<api::Message>,
}

impl RemoteLogger {
    pub fn new(socket_fd: Arc<Mutex<RawFd>>) -> Self {
        let tx = Self::spawn_send_thread(socket_fd);
        Self { tx }
    }
}

impl RemoteLogger {
    /// Spawns a thread that receives log messages to send to the host and
    /// sends them over the socket.
    ///
    /// We do this in a background thread to prevent deadlocks that could occur
    /// if we were asked to send a log message by some code that was already
    /// holding the socket lock.  By using a separate thread, we guarantee that
    /// main-thread code will never block during logging.
    fn spawn_send_thread(socket_fd: Arc<Mutex<RawFd>>) -> mpsc::Sender<api::Message> {
        let (tx, rx) = mpsc::channel();

        let _ = std::thread::Builder::new()
            .name("log-message-sender".to_owned())
            .spawn(move || {
                loop {
                    let Ok(mut message) = rx.recv() else {
                        eprintln!(
                            "Log message sending channel closed; terminating logging thread."
                        );
                        return;
                    };

                    let socket_fd_lock = socket_fd.lock();
                    'inner: loop {
                        if let Err(err) =
                            protocol::send_message(*socket_fd_lock, message, Option::<RawFd>::None)
                        {
                            // In failing tests, the app shuts down abruptly and this
                            // message pollutes the test output.
                            if !cfg!(feature = "integration_tests") {
                                // Use `eprintln!()` instead of `log::error!()` to avoid
                                // recursive logging.
                                eprintln!("Failed to send log record to host process: {err:#}");
                            }
                        }

                        message = match rx.try_recv() {
                            Ok(message) => message,
                            Err(_) => {
                                // Nothing left in the receive queue; drop the lock
                                // and wait until there are more messages in the
                                // receive queue.
                                drop(socket_fd_lock);
                                break 'inner;
                            }
                        }
                    }
                }
            });

        tx
    }
}

impl log::Log for RemoteLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let message = api::Message::WriteLogRequest {
            level: record.level(),
            target: record.target().to_string(),
            message: record.args().to_string(),
        };
        if self.tx.send(message).is_err() {
            eprintln!("Failed to send log message to logger thread");
        }
    }

    fn flush(&self) {}
}

/// Handles a WriteLogRequest received from the server.
pub(super) fn handle_write_log_request(level: log::Level, target: String, message: String) {
    let log_fn = || {
        // Write the log line that was forwarded from the terminal server.
        log::log!(target: target.as_str(), level, "{message}");
    };
    cfg_if::cfg_if! {
        if #[cfg(feature = "crash_reporting")] {
            // Explicitly write the log line in the context of the main
            // Sentry hub; this log receiver thread is spawned before
            // Sentry is configured, so the thread-local hub doesn't
            // have the appropriate client and scope configuration.
            sentry::Hub::run(sentry::Hub::main(), log_fn);
        } else {
            log_fn();
        }
    }
}
