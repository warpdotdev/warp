use std::{
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use async_channel::Sender;
use async_fs::OpenOptions;
use chrono::Utc;
use futures::AsyncWriteExt as _;
use warpui::r#async::executor::{Background, BackgroundTask};

pub mod manager;
pub mod rotation_events;

pub use rotation_events::{
    MockSummarizer, PipelineStep, RotationEvent, RotationSummarizer, RotationSummary,
    SummarizerError,
};

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
    /// Creates a new logger that, in addition to size-based rotation, optionally
    /// invokes a [`RotationSummarizer`] on each rotation to produce a structured
    /// summary record of the soon-to-be-discarded content.
    ///
    /// Two sidecar files are emitted alongside the active log:
    ///
    /// - `<path>.rotations.jsonl` — written whenever rotation is configured,
    ///   regardless of `summarizer`. Records timestamp, bytes rotated, and the
    ///   path of the discarded file (if any).
    /// - `<path>.summaries.jsonl` — written only when `summarizer` is `Some`
    ///   and returns `Ok(Some(_))`. Records the summary content + per-step
    ///   pipeline traces.
    ///
    /// Summarizer errors are logged and swallowed; the rotation itself always
    /// completes regardless of summarization outcome, so a failing model call
    /// or unavailable backend never blocks logging.
    pub(crate) fn new_with_summarizer(
        log_path: PathBuf,
        executor: Arc<Background>,
        rotation: Option<RotationConfig>,
        summarizer: Option<Arc<dyn RotationSummarizer>>,
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

                        if let Some(config) = rotation {
                            if written_bytes >= config.max_file_size_bytes {
                                let pre_rotation_bytes = written_bytes;
                                // Snapshot the soon-to-be-discarded file's content
                                // *before* `perform_rotation` deletes it, so the
                                // summarizer (if any) has something to work with.
                                let oldest_path = path_with_suffix(&log_path, config.max_rotation);
                                let discarded_snapshot =
                                    if summarizer.is_some() && oldest_path.exists() {
                                        async_fs::read_to_string(&oldest_path).await.ok()
                                    } else {
                                        None
                                    };
                                let discarded_path_for_event = if oldest_path.exists() {
                                    Some(oldest_path.clone())
                                } else {
                                    None
                                };

                                // Drop the active file handle before renaming so platforms
                                // that disallow renaming an open file (notably Windows)
                                // succeed, and so the subsequent reopen receives a fresh
                                // inode.
                                drop(log_file);
                                if let Err(e) =
                                    perform_rotation(&log_path, config.max_rotation).await
                                {
                                    log::warn!(
                                        "SimpleLogger: rotation failed for {:?}: {e}; \
                                         continuing with truncated active file",
                                        &log_path,
                                    );
                                }

                                // Layer A — emit a rotation event. Always-on when
                                // rotation is configured. Failure here is logged but
                                // never bubbles up: a missing sidecar must not
                                // prevent the logger from continuing to write logs.
                                if let Err(e) = emit_rotation_event(
                                    &log_path,
                                    pre_rotation_bytes,
                                    discarded_path_for_event,
                                )
                                .await
                                {
                                    log::warn!(
                                        "SimpleLogger: failed to write rotation event for \
                                         {:?}: {e}",
                                        &log_path,
                                    );
                                }

                                // Layer B — optional summarization of the discarded
                                // file. Skipped silently when no summarizer is
                                // configured or when the discarded file was empty.
                                if let (Some(summarizer), Some(content)) =
                                    (summarizer.as_ref(), discarded_snapshot)
                                {
                                    let snapshot_path =
                                        path_with_suffix(&log_path, config.max_rotation);
                                    match summarizer.summarize(&snapshot_path, &content).await {
                                        Ok(Some(summary)) => {
                                            if let Err(e) =
                                                append_summary(&log_path, &summary).await
                                            {
                                                log::warn!(
                                                    "SimpleLogger: failed to append rotation \
                                                     summary for {:?}: {e}",
                                                    &log_path,
                                                );
                                            }
                                        }
                                        Ok(None) => {
                                            // Summarizer chose to emit nothing — this
                                            // is a valid response (e.g. trivially-small
                                            // content). Silent.
                                        }
                                        Err(e) => {
                                            log::warn!(
                                                "SimpleLogger: summarizer errored for {:?}: \
                                                 {e}; rotation proceeded without summary",
                                                &log_path,
                                            );
                                        }
                                    }
                                }

                                log_file = match open_truncated(&log_path).await {
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
                                written_bytes = 0;
                            }
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
async fn open_truncated(path: &Path) -> std::io::Result<async_fs::File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
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
pub(crate) async fn perform_rotation(base_path: &Path, max_rotation: usize) -> std::io::Result<()> {
    // Step 1 — drop the file that would otherwise become `.{max_rotation + 1}`.
    // Tolerate ENOENT silently: it just means we haven't accumulated enough
    // rotations yet.
    let oldest = path_with_suffix(base_path, max_rotation);
    if let Err(e) = async_fs::remove_file(&oldest).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            log::debug!(
                "SimpleLogger: could not remove oldest rotation {:?}: {e}",
                oldest
            );
        }
    }

    // Step 2 — shift every existing `.N` up by one, going from oldest to
    // youngest so we never overwrite a file we haven't moved yet.
    for n in (1..max_rotation).rev() {
        let src = path_with_suffix(base_path, n);
        let dst = path_with_suffix(base_path, n + 1);
        if let Err(e) = async_fs::rename(&src, &dst).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::debug!("SimpleLogger: could not rotate {:?} -> {:?}: {e}", src, dst,);
            }
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
pub(crate) fn path_with_suffix(base: &Path, n: usize) -> PathBuf {
    let mut s = base.as_os_str().to_owned();
    s.push(format!(".{n}"));
    PathBuf::from(s)
}

/// Sidecar path: `<base>.rotations.jsonl` — the rotation event log.
pub(crate) fn rotations_sidecar_path(base: &Path) -> PathBuf {
    let mut s = base.as_os_str().to_owned();
    s.push(".rotations.jsonl");
    PathBuf::from(s)
}

/// Sidecar path: `<base>.summaries.jsonl` — model-generated summary records.
pub(crate) fn summaries_sidecar_path(base: &Path) -> PathBuf {
    let mut s = base.as_os_str().to_owned();
    s.push(".summaries.jsonl");
    PathBuf::from(s)
}

/// Append a single `RotationEvent` JSON line to the rotations sidecar. Creates
/// the file if it doesn't exist. Errors propagate so the caller can decide
/// whether to log the failure; the framework specifically swallows them so a
/// missing sidecar never blocks logging.
async fn emit_rotation_event(
    base_path: &Path,
    bytes_rotated: u64,
    discarded_path: Option<PathBuf>,
) -> std::io::Result<()> {
    let event = RotationEvent {
        timestamp: Utc::now(),
        active_log: base_path.to_path_buf(),
        bytes_rotated,
        discarded_path,
    };
    let line = event
        .to_jsonl_line()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    append_line(&rotations_sidecar_path(base_path), &line).await
}

/// Append a single `RotationSummary` JSON line to the summaries sidecar.
async fn append_summary(base_path: &Path, summary: &RotationSummary) -> std::io::Result<()> {
    let line = summary
        .to_jsonl_line()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    append_line(&summaries_sidecar_path(base_path), &line).await
}

/// Append `line` to `path`, creating the file if needed. The line is expected
/// to already include its terminating newline; this helper is a thin wrapper
/// around `OpenOptions::append` so the sidecar callers don't each repeat the
/// file-open dance.
async fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .await?;
    file.write_all(line.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
