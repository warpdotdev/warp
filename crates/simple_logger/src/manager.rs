use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use thiserror::Error;
use warpui_core::r#async::executor::Background;
use warpui_core::{Entity, SingletonEntity};

use crate::{LogFileWriter, SimpleLogger};

#[derive(Error, Debug)]
pub enum LogManagerError {
    #[error("A logger is already active for this path: {}", path.display())]
    LoggerAlreadyActive { path: PathBuf },
    #[error("Unknown log namespace: {namespace}")]
    UnknownNamespace { namespace: String },
}

impl LogManagerError {
    /// Returns a description of the error suitable for use in release-channel error reporting.
    /// User-specific data (e.g. file paths) is omitted; non-sensitive details are preserved.
    pub fn safe_message(&self) -> String {
        match self {
            Self::LoggerAlreadyActive { .. } => "logger already active for path".to_string(),
            Self::UnknownNamespace { .. } => format!("{self:?}"),
        }
    }
}

/// Computes the log file path for a given namespace and relative path,
/// without requiring access to [`LogManager`].
///
/// This is useful for read-only path resolution (e.g. reading log files for display).
pub fn resolve_log_path(namespace: &str, relative_path: impl AsRef<Path>) -> PathBuf {
    log_directory_path(namespace).join(relative_path)
}

/// Returns the base log directory for a given namespace name.
fn log_directory_path(namespace: &str) -> PathBuf {
    let base_dir = warp_core::paths::secure_state_dir().unwrap_or_else(warp_core::paths::state_dir);
    if cfg!(windows) {
        base_dir
            .join(warp_core::paths::WARP_LOGS_DIR)
            .join(namespace)
    } else {
        base_dir.join(namespace)
    }
}

/// Singleton that owns all file-based loggers in the app.
///
/// Enforces that at most one active [`SimpleLogger`] exists per log file path.
/// Stale registrations are reclaimed automatically on the next
/// [`register`](LogManager::register) call for that path.
///
/// A registration is considered stale in two cases:
/// - all [`SimpleLogger`] clones have been dropped, so the [`Weak`] entry can no
///   longer be upgraded
/// - the underlying channel has already been explicitly closed via
///   [`SimpleLogger::close`], which means the writer is logically dead even if
///   some [`Arc`] handles still exist briefly
///
/// Supporting both cases lets callers opt into eager, explicit shutdown without
/// tying path reuse strictly to the last clone being dropped.
pub struct LogManager {
    namespaces: HashSet<String>,
    loggers: HashMap<PathBuf, Weak<LogFileWriter>>,
}

impl Default for LogManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LogManager {
    pub fn new() -> Self {
        Self {
            namespaces: HashSet::new(),
            loggers: HashMap::new(),
        }
    }

    /// Registers a log namespace with the given cleanup policy.
    ///
    /// On first call for a given name, stores the namespace and purges its
    /// directory if `purge_on_startup` is true. Subsequent calls for the same
    /// name are no-ops.
    pub fn register_namespace(&mut self, name: &str, purge_on_startup: bool) {
        if self.namespaces.contains(name) {
            return;
        }

        if purge_on_startup {
            let dir = log_directory_path(name);
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                if e.kind() != ErrorKind::NotFound {
                    log::warn!("Failed to purge log directory {}: {e}", dir.display());
                }
            }
        }

        self.namespaces.insert(name.to_string());
    }

    /// Registers a logger for a path relative to the namespace's log directory.
    ///
    /// Returns an error if the namespace has not been registered, or if a logger
    /// is already alive for that path. Stale registrations (where all clones have
    /// been dropped or the channel has been explicitly closed) are reclaimed silently.
    pub fn register(
        &mut self,
        namespace: &str,
        relative_path: impl AsRef<Path>,
        executor: Arc<Background>,
    ) -> Result<SimpleLogger, LogManagerError> {
        self.register_with_rotation(namespace, relative_path, executor, None)
    }

    /// Registers a logger with optional size-based rotation.
    ///
    /// Identical to [`register`](Self::register) when `rotation` is `None`. When
    /// `Some(config)`, the resulting logger rotates the active log file once
    /// it accumulates `config.max_file_size_bytes` of writes, keeping up to
    /// `config.max_rotation` rotated copies on disk and discarding older ones.
    ///
    /// This is the entry point used by callers that produce high-volume logs
    /// over long-lived sessions — primarily MCP server stderr/stdout capture,
    /// where a single chatty server could otherwise grow its log file
    /// unboundedly across a multi-day session (warpdotdev/warp#7723).
    pub fn register_with_rotation(
        &mut self,
        namespace: &str,
        relative_path: impl AsRef<Path>,
        executor: Arc<Background>,
        rotation: Option<crate::RotationConfig>,
    ) -> Result<SimpleLogger, LogManagerError> {
        if !self.namespaces.contains(namespace) {
            return Err(LogManagerError::UnknownNamespace {
                namespace: namespace.to_string(),
            });
        }
        let path = resolve_log_path(namespace, relative_path);
        self.register_resolved_path(path, executor, rotation)
    }

    fn register_resolved_path(
        &mut self,
        path: PathBuf,
        executor: Arc<Background>,
        rotation: Option<crate::RotationConfig>,
    ) -> Result<SimpleLogger, LogManagerError> {
        if let Some(existing) = self.loggers.get(&path) {
            if let Some(writer) = existing.upgrade() {
                // A live `Arc` alone is not enough to keep the path reserved.
                // Callers may explicitly close a logger to mark the stream as
                // finished before every clone has been dropped.
                if !writer.is_closed() {
                    return Err(LogManagerError::LoggerAlreadyActive { path });
                }
            }
        }

        // In the absence of an active logger at this path, initialize and return a new logger,
        // which truncates any existing log file on creation.
        let logger = SimpleLogger::new(path.clone(), executor, rotation);
        self.loggers.insert(path, logger.downgrade());
        Ok(logger)
    }
}

pub enum LogManagerEvent {}

impl Entity for LogManager {
    type Event = LogManagerEvent;
}

impl SingletonEntity for LogManager {}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
