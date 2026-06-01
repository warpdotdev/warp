/// Destination for log output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogDestination {
    /// Write logs to a file.
    File,
    /// Write logs to stderr.
    Stderr,
}

/// Configuration for initializing the logger.
#[derive(Debug, Clone, Copy, Default)]
pub struct LogConfig {
    /// Whether the caller is the CLI. When true, logs are written to a separate subdirectory
    /// with a higher rotation limit so that CLI invocations don't evict GUI application logs.
    pub is_cli: bool,
    /// The destination for log output. If `None`, the destination is inferred from the environment.
    pub log_destination: Option<LogDestination>,
    /// Optional in-session size threshold for `warp.log`. When `Some(n)` and the active
    /// file accumulates more than `n` bytes during a single execution, it is rotated to
    /// `warp.log.in_session.0` and a fresh active file is opened. Older `.in_session.N`
    /// files shift up and the oldest is discarded, matching the per-startup
    /// `rotate_log_files` behavior. `None` preserves the existing unbounded-within-session
    /// growth (warpdotdev/warp#10879).
    pub max_file_size_bytes: Option<u64>,
}

#[cfg_attr(not(target_family = "wasm"), path = "native.rs")]
#[cfg_attr(target_family = "wasm", path = "wasm.rs")]
mod imp;

#[cfg(not(target_family = "wasm"))]
mod rotation;

pub use imp::init;
#[cfg(not(target_family = "wasm"))]
pub use imp::{create_log_bundle_zip, log_directory, log_file_path, rotate_log_files};
#[cfg(not(target_family = "wasm"))]
pub use imp::{
    init_for_crash_recovery_process, init_logging_for_unit_tests, on_crash_recovery_process_killed,
    on_parent_process_crash,
};
