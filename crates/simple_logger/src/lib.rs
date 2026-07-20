use std::path::PathBuf;
use std::sync::{Arc, Weak};

use async_channel::Sender;
use async_fs::OpenOptions;
use futures::AsyncWriteExt as _;
use warpui_core::r#async::executor::{Background, BackgroundTask};

pub mod manager;

/// Configuration for size-based log file rotation.
///
/// When a [`SimpleLogger`] is created with `Some(RotationConfig)`, it tracks the
/// number of bytes written to the active log file. After any write that brings
/// the cumulative byte count to `max_file_size_bytes` or above, the writer
/// closes the active file, rotates it to a `.1` suffix (shifting older `.N`
/// files up by one and discarding the file at `.{max_rotation}` before the
/// shift), and reopens a fresh active file.
///
/// The file may briefly exceed `max_file_size_bytes` by one log line — the
/// rotation happens *after* the write that crosses the threshold so log lines
/// are never split across files.
///
/// A `SimpleLogger` constructed with `rotation = None` retains the original
/// behavior: one file per logger lifetime, unbounded growth, truncate-on-create.
#[derive(Debug, Clone, Copy)]
pub struct RotationConfig {
    max_file_size_bytes: u64,
    max_rotation: usize,
}

impl RotationConfig {
    /// Build a [`RotationConfig`].
    ///
    /// Both parameters must be non-zero; passing zero for either is treated as
    /// "rotation disabled" and yields `None`. Callers that want unconditional
    /// disabling should pass `None` directly to [`SimpleLogger::new`] rather
    /// than calling this with zero — but accepting zero here keeps the
    /// `Option<RotationConfig>` API safe to thread through call sites that
    /// derive values from configuration.
    pub fn new(max_file_size_bytes: u64, max_rotation: usize) -> Option<Self> {
        if max_file_size_bytes == 0 || max_rotation == 0 {
            None
        } else {
            Some(Self {
                max_file_size_bytes,
                max_rotation,
            })
        }
    }

    pub fn max_file_size_bytes(&self) -> u64 {
        self.max_file_size_bytes
    }

    pub fn max_rotation(&self) -> usize {
        self.max_rotation
    }
}

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
    ///
    /// If `rotation` is `Some`, the active file is rotated whenever its written
    /// byte count reaches the configured threshold. If `None`, the file is
    /// truncated on creation and grows without bound for the logger's lifetime
    /// (the original behavior).
    pub(crate) fn new(
        log_path: PathBuf,
        executor: Arc<Background>,
        rotation: Option<RotationConfig>,
    ) -> Self {
        let (log_tx, log_rx) = async_channel::unbounded::<String>();

        if let Some(directory) = log_path.parent() {
            let _ = std::fs::create_dir_all(directory);
        }

        let logging_task = executor.spawn(async move {
            let mut log_file = match open_truncated(&log_path).await {
                Ok(log_file) => log_file,
                Err(e) => {
                    log::warn!("Could not open file for logging: {:?}. {:?}", &log_path, e);
                    return;
                }
            };
            let mut written_bytes: u64 = 0;
            loop {
                match log_rx.recv().await {
                    Ok(log_line) => {
                        let formatted = format!(
                            "{} | {}\n",
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                            log_line
                        );
                        let bytes = formatted.as_bytes();
                        let _ = log_file.write_all(bytes).await;
                        // Flush after each line to ensure logs are visible immediately
                        let _ = log_file.flush().await;
                        written_bytes = written_bytes.saturating_add(bytes.len() as u64);

                        if let Some(config) = rotation
                            && written_bytes >= config.max_file_size_bytes
                        {
                            // Drop the active file handle before renaming so platforms
                            // that disallow renaming an open file (notably Windows)
                            // succeed, and so the subsequent reopen receives a fresh
                            // inode.
                            drop(log_file);
                            let rotation_failed =
                                match perform_rotation(&log_path, config.max_rotation).await {
                                    Ok(()) => false,
                                    Err(e) => {
                                        // The only path through `perform_rotation` that
                                        // surfaces an error is step 3's rename of the
                                        // active file. That rename only runs when the
                                        // active file exists, so on Err the original
                                        // content is still on disk at `log_path`. Open
                                        // in append mode rather than truncating so we
                                        // don't destroy the log data rotation was meant
                                        // to preserve.
                                        log::warn!(
                                            "SimpleLogger: rotation failed for {:?}: {e}; \
                                             preserving existing log content and continuing \
                                             in append mode",
                                            &log_path,
                                        );
                                        true
                                    }
                                };
                            let reopen = if rotation_failed {
                                open_append(&log_path).await
                            } else {
                                open_truncated(&log_path).await
                            };
                            log_file = match reopen {
                                Ok(f) => f,
                                Err(e) => {
                                    log::warn!(
                                        "SimpleLogger: failed to reopen {:?} after \
                                             rotation: {e}",
                                        &log_path,
                                    );
                                    return;
                                }
                            };
                            // Seed the counter from the file's current size so the
                            // next rotation threshold check stays meaningful even
                            // after the preserve-on-failure path.
                            written_bytes = if rotation_failed {
                                async_fs::metadata(&log_path)
                                    .await
                                    .map(|m| m.len())
                                    .unwrap_or(0)
                            } else {
                                0
                            };
                        }
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

/// Open `path` for writing with truncation, ensuring the parent directory exists.
async fn open_truncated(path: &std::path::Path) -> std::io::Result<async_fs::File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .await
}

/// Open `path` for appending. Used by the rotation-failure recovery path so
/// existing log data is preserved when `perform_rotation` could not move the
/// active file to its rotated slot.
async fn open_append(path: &std::path::Path) -> std::io::Result<async_fs::File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
}

/// Rotate `base_path` and its existing `.1` … `.{max_rotation}` siblings.
///
/// After the call:
///   - the file previously at `.{max_rotation}` is gone
///   - each remaining `.N` has been renamed to `.{N+1}`
///   - the previous active file at `base_path` is now at `.1`
///   - `base_path` itself no longer exists (the caller is expected to reopen it
///     truncated)
///
/// Rename failures for intermediate `.N` files are tolerated (the file may not
/// exist yet if fewer than `max_rotation` rotations have occurred). A failure to
/// rename the current active file is reported back to the caller.
pub(crate) async fn perform_rotation(
    base_path: &std::path::Path,
    max_rotation: usize,
) -> std::io::Result<()> {
    // Step 1 — drop the file that would otherwise become `.{max_rotation + 1}`.
    // Tolerate ENOENT silently: it just means we haven't accumulated enough
    // rotations yet.
    let oldest = path_with_suffix(base_path, max_rotation);
    if let Err(e) = async_fs::remove_file(&oldest).await
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::debug!(
            "SimpleLogger: could not remove oldest rotation {:?}: {e}",
            oldest
        );
    }

    // Step 2 — shift every existing `.N` up by one, going from oldest to
    // youngest so we never overwrite a file we haven't moved yet.
    for n in (1..max_rotation).rev() {
        let src = path_with_suffix(base_path, n);
        let dst = path_with_suffix(base_path, n + 1);
        if let Err(e) = async_fs::rename(&src, &dst).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::debug!("SimpleLogger: could not rotate {:?} -> {:?}: {e}", src, dst,);
        }
    }

    // Step 3 — promote the current active file to `.1`. This is the rename
    // that matters; surface its error so the caller can decide to keep going
    // (it will reopen truncated regardless) or report it.
    if base_path.exists() {
        async_fs::rename(base_path, path_with_suffix(base_path, 1)).await?;
    }

    Ok(())
}

/// Build the rotated-suffix path for `base_path`. e.g. `mcp/srv.log` with `n=2`
/// becomes `mcp/srv.log.2`. Operating on the raw `OsString` rather than via
/// `set_extension` is intentional — we append a suffix, we don't replace one,
/// and `set_extension("log.2")` would strip a legitimate trailing `.log`.
pub(crate) fn path_with_suffix(base: &std::path::Path, n: usize) -> PathBuf {
    let mut s = base.as_os_str().to_owned();
    s.push(format!(".{n}"));
    PathBuf::from(s)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
