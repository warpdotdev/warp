/// Destination for log output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogDestination {
    /// Write logs to a file.
    File,
    /// Write logs to stderr.
    Stderr,
}

/// Configuration for initializing the logger.
#[derive(Debug, Clone, Copy)]
pub struct LogConfig {
    /// Whether the caller is the CLI. When true, logs are written to a separate subdirectory
    /// with a higher rotation limit so that CLI invocations don't evict GUI application logs.
    pub is_cli: bool,
    /// The destination for log output. If `None`, the destination is inferred from the environment.
    pub log_destination: Option<LogDestination>,
}

#[cfg_attr(not(target_family = "wasm"), path = "native.rs")]
#[cfg_attr(target_family = "wasm", path = "wasm.rs")]
mod imp;

pub use imp::init;
#[cfg(not(target_family = "wasm"))]
pub use imp::{create_log_bundle_zip, log_directory, log_file_path, rotate_log_files};
#[cfg(not(target_family = "wasm"))]
pub use imp::{
    init_for_crash_recovery_process, init_logging_for_unit_tests, on_crash_recovery_process_killed,
    on_parent_process_crash,
};
