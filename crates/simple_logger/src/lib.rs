use std::path::PathBuf;
use std::sync::{Arc, Weak};

use async_channel::Sender;
use async_fs::OpenOptions;
use futures::AsyncWriteExt as _;
use warpui::r#async::executor::{Background, BackgroundTask};

pub mod manager;

/// Shared state for a [`SimpleLogger`].
///
/// When all [`SimpleLogger`] clones are dropped, this is dropped too, which closes
/// the logging channel and lets the background writing task finish.
///
/// We also support explicit shutdown via [`SimpleLogger::close`]. That allows a
/// caller to mark a log stream as finished immediately, even if some incidental
/// clones are still alive briefly in background tasks or callback state.
pub(crate) struct LogFileWriter {
    log_tx: Sender<String>,
    _logging_task: BackgroundTask,
}

impl LogFileWriter {
    /// Returns true if the underlying channel has been closed.
    ///
    /// A closed writer is logically dead even if some [`Arc`] handles still
    /// exist, because it can no longer accept new log lines.
    pub(crate) fn is_closed(&self) -> bool {
        self.log_tx.is_closed()
    }
}

/// A simple file-based logger for server stderr output.
/// Writes timestamped log entries to a file asynchronously.
#[derive(Clone)]
pub struct SimpleLogger {
    // Cheaply cloneable reference to the log file writer.
    writer: Arc<LogFileWriter>,
}

impl SimpleLogger {
    /// Creates a new logger that writes to the specified file path.
    /// The file is truncated on creation.
    pub(crate) fn new(log_path: PathBuf, executor: Arc<Background>) -> Self {
        let (log_tx, log_rx) = async_channel::unbounded::<String>();

        if let Some(directory) = log_path.parent() {
            let _ = std::fs::create_dir_all(directory);
        }

        let logging_task = executor.spawn(async move {
            let mut log_file = match OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&log_path)
                .await
            {
                Ok(log_file) => log_file,
                Err(e) => {
                    log::warn!("Could not open file for logging: {:?}. {:?}", &log_path, e);
                    return;
                }
            };
            loop {
                match log_rx.recv().await {
                    Ok(log_line) => {
                        let _ = log_file
                            .write_all(
                                format!(
                                    "{} | {}\n",
                                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                                    log_line
                                )
                                .as_bytes(),
                            )
                            .await;
                        // Flush after each line to ensure logs are visible immediately
                        let _ = log_file.flush().await;
                    }
                    Err(e) => {
                        log::warn!("SimpleLogger: channel closed: {e}");
                        break;
                    }
                }
            }

            // Final flush
            let _ = log_file.flush().await;
        });

        Self {
            writer: Arc::new(LogFileWriter {
                log_tx,
                _logging_task: logging_task,
            }),
        }
    }

    /// Log a message to the file.
    pub fn log(&self, message: String) {
        let _ = self.writer.log_tx.try_send(message);
    }

    /// Explicitly close the logger channel before all clones are dropped.
    ///
    /// This is useful when the caller wants "this log stream is finished" to be
    /// a first-class state, rather than waiting for every clone to be dropped.
    /// For example, a failed connection attempt may want to write a final error
    /// line, close the stream immediately, and let a later retry reclaim the
    /// same log path even if some transient clones have not been dropped yet.
    ///
    /// This is a no-op if the channel is already closed. Shutdown also happens
    /// automatically when the last [`SimpleLogger`] clone is dropped.
    pub fn close(&self) {
        self.writer.log_tx.close();
    }

    /// Returns a weak reference to the shared writer, used by [`manager::LogManager`]
    /// to track liveness without preventing shutdown.
    pub(crate) fn downgrade(&self) -> Weak<LogFileWriter> {
        Arc::downgrade(&self.writer)
    }
}
