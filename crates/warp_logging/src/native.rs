use std::path::{Path, PathBuf};
use std::{
    env,
    fs::{self, File},
    io::{IsTerminal, Write, copy},
};

use anyhow::Result;
use chrono::Local;
use log::LevelFilter;
use std::sync::OnceLock;
use warp_core::features::FeatureFlag;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{LogConfig, LogDestination};
use warp_core::channel::ChannelState;

const MAX_FILES_IN_GUI_ROTATION: usize = 5;
const MAX_FILES_IN_CLI_ROTATION: usize = 10;
const CLI_LOG_SUBDIRECTORY: &str = "oz";
const TEMP_LOG_FILE_SUFFIX: &str = "old.temp";

/// Runtime logging state, computed from `LogConfig` during initialization.
#[derive(Debug)]
struct LogState {
    /// Whether or not logs should be written to a file.
    use_logfile: bool,

    /// The directory that logs should be written to. This is set even if `use_logfile` is false,
    /// as we sometimes generate other log files.
    log_directory: PathBuf,

    /// The maximum number of backup log files to keep during rotation.
    max_rotation: usize,
}

static LOG_STATE: OnceLock<LogState> = OnceLock::new();

/// Formats a log record to be output to the terminal.
fn format_for_terminal_output(
    buf: &mut env_logger::fmt::Formatter,
    record: &log::Record,
) -> std::io::Result<()> {
    let level = record.level();
    let mut level_style = buf.default_level_style(record.level());
    // Adjust colors to match what we're used to from simplelog.
    match &level {
        log::Level::Info => {
            level_style.set_color(env_logger::fmt::Color::Blue);
        }
        log::Level::Debug => {
            level_style.set_color(env_logger::fmt::Color::Green);
        }
        _ => {}
    }
    let level = level_style.value(format!("[{level}]"));

    let mut target_style = buf.style();
    let target = if cfg!(debug_assertions) {
        target_style.set_dimmed(true);
        target_style.value(format!("[{}] ", record.target()))
    } else {
        target_style.value(String::default())
    };

    let time = chrono::Local::now();
    writeln!(
        buf,
        "{} {level} {target}{}",
        time.format("%H:%M:%S%.3f"),
        record.args()
    )
}

/// Formats a log record to be output to a file.
fn format_for_file_output(
    buf: &mut env_logger::fmt::Formatter,
    record: &log::Record,
) -> std::io::Result<()> {
    let target = if cfg!(debug_assertions) {
        format!("[{}] ", record.target())
    } else {
        String::default()
    };

    writeln!(
        buf,
        "{} [{}] {}{}",
        buf.timestamp(),
        record.level(),
        target,
        record.args()
    )
}

/// Handles the crash recovery process being killed by removing the crash recovery process log file
/// (which is stored in a temp directory and only persisted if the crash recovery process actually
/// handled a crash in the parent process).
pub fn on_crash_recovery_process_killed() {
    let config = LOG_STATE.get().expect("Logging not initialized");
    if !config.use_logfile {
        return;
    }

    let _ = fs::remove_file(crash_recovery_process_log_file_path(&config.log_directory));
}

/// Handles the crash recovery process "recovering" from a parent crash by:
/// 1) Renaming the log file from the main process (which just panicked) to `warp.log.old.temp`.
/// 2) Moving the crash recovery process log (which is located at `warp.log.recovery`) to the usual
///    path warp logs are located (log_directory/warp.log).
///    The temp log file (`warp.log.old.temp`) will ultimately be rotated to `warp.log.old.0` the next
///    time [`rotate_log_files`] is called (which will get called when the event loop starts and we
///    have access to the `AppContext`)
pub fn on_parent_process_crash() {
    let config = LOG_STATE.get().expect("Logging not initialized");
    if !config.use_logfile {
        return;
    }

    let main_log_path = main_process_log_file_path(&config.log_directory);
    let temp_path = temp_log_file_path(&config.log_directory);

    let _ = fs::rename(&main_log_path, temp_path);

    let _ = fs::rename(
        crash_recovery_process_log_file_path(&config.log_directory),
        main_log_path,
    );
}

/// Rotates the log and telemetry files, such that:
/// - Each file stores the logs of a single execution.
/// - The .old files store the previous executions, with larger suffixes indicating older executions.
pub async fn rotate_log_files() {
    let config = LOG_STATE.get().expect("Logging not initialized");
    if !config.use_logfile {
        return;
    }

    let max_rotation = config.max_rotation;

    if let Err(err) = rotate_files(&ChannelState::logfile_name(), max_rotation).await {
        log::error!("Failed to rotate log files: {err:?}");
    }

    if FeatureFlag::SendTelemetryToFile.is_enabled()
        && let Err(err) = rotate_files(&ChannelState::telemetry_file_name(), max_rotation).await
    {
        log::error!("Failed to rotate telemetry files: {err:?}");
    }
}

pub async fn rotate_files(channel_file_name: &str, max_rotation: usize) -> Result<()> {
    let log_directory = match log_directory() {
        Ok(log_directory) => log_directory,
        Err(err) => {
            return Err(anyhow::anyhow!("Could not get log directory {err:?}"));
        }
    };

    // Delete the oldest log file.
    let largest_log_file_suffix = max_rotation.saturating_sub(1);
    let _ = fs::remove_file(
        log_directory.join(format!("{channel_file_name}.old.{largest_log_file_suffix}")),
    );

    // Rotate the log files.
    for file_no in (0..largest_log_file_suffix).rev() {
        let old_file_path = log_directory.join(format!("{channel_file_name}.old.{file_no}"));
        let new_file_path = log_directory.join(format!("{channel_file_name}.old.{}", file_no + 1));
        let _ = fs::rename(old_file_path, new_file_path);
    }

    // Rename `warp.log.old.temp` (the temporary file) to `warp.log.old.0`.
    let temp_file_path = temp_log_file_path(&log_directory);

    let _ = fs::rename(
        temp_file_path,
        log_directory.join(format!("{channel_file_name}.old.0")),
    );

    Ok(())
}

/// Initializes the logger for the crash recovery process.
pub fn init_for_crash_recovery_process() -> Result<()> {
    init_internal(
        true,  /* is_from_crash_recovery_process */
        false, /* is_cli */
        None,  /* log_destination */
    )
}

/// Initializes the global logger for the application.
/// If `config.log_destination` is `Some`, always use the specified destination regardless of
/// environment. If `config.is_cli` is true, logs are written to a separate "oz" subdirectory with
/// a higher rotation limit so that CLI invocations don't evict GUI application logs.
pub fn init(config: LogConfig) -> Result<()> {
    init_internal(
        false, /* is_from_crash_recovery_process */
        config.is_cli,
        config.log_destination,
    )
}

/// Return the path to the log file that is used within the crash recovery process.
/// We use a separate log file for the crash recovery process. If the crash
/// recovery process handles a crash, we'll move the crash recovery process log file to its usual
/// location at `log_directory/warp.log`.
fn crash_recovery_process_log_file_path(log_directory: impl AsRef<Path>) -> PathBuf {
    log_directory
        .as_ref()
        .join(format!("{}.recovery", ChannelState::logfile_name()))
}

/// Returns the path to the main process's log file.
fn main_process_log_file_path(log_directory: impl AsRef<Path>) -> PathBuf {
    log_directory.as_ref().join(&*ChannelState::logfile_name())
}

/// Returns the path to the current execution's main log file.
///
/// Note: logging must be initialized before calling this function, otherwise this will
/// return an error.
pub fn log_file_path() -> Result<PathBuf> {
    let dir = log_directory()?;
    Ok(main_process_log_file_path(&dir))
}

/// Collects a list of the paths to both the current warp instance's log file,
/// and any older log files (we keep up to 6 log files around at any time,
/// all of which are potentially useful for debugging).
fn current_and_rotated_log_paths() -> Result<Vec<PathBuf>> {
    let log_directory = log_directory()?;
    let current_log_path = main_process_log_file_path(&log_directory);

    let mut rotated_logs: Vec<(usize, PathBuf)> = fs::read_dir(&log_directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|path| {
            let file_name = path.file_name()?.to_str()?;
            let suffix =
                file_name.strip_prefix(&format!("{}.old.", ChannelState::logfile_name()))?;
            let index = suffix.parse::<usize>().ok()?;
            Some((index, path))
        })
        .collect();
    rotated_logs.sort_by_key(|(index, _)| *index);

    let mut files = Vec::new();
    if current_log_path.is_file() {
        files.push(current_log_path);
    }

    files.extend(
        rotated_logs
            .into_iter()
            .map(|(_, path)| path)
            .filter(|path| path.is_file()),
    );

    if files.is_empty() {
        return Err(anyhow::anyhow!(
            "No warp logs were found for {}",
            ChannelState::logfile_name()
        ));
    }

    Ok(files)
}

/// Creates a timestamped zip archive containing the current log file
/// and any older logs for the active instance.
pub fn create_log_bundle_zip() -> Result<PathBuf> {
    let log_files = current_and_rotated_log_paths()?;
    let log_directory = log_directory()?;
    let logfile_name = ChannelState::logfile_name();
    let logfile_stem = logfile_name.strip_suffix(".log").unwrap_or(&logfile_name);

    let zip_path = log_directory.join(format!(
        "{logfile_stem}-{}.zip",
        Local::now().format("%Y%m%d-%H%M%S")
    ));
    if zip_path.exists() {
        let error_message = format!(
            "New log zip path conflicts with an existing zip: {}",
            zip_path.display()
        );
        return Err(anyhow::anyhow!("{error_message}"));
    }

    let zip_file = File::create(&zip_path)?;
    let mut zip_writer = ZipWriter::new(zip_file);
    let zip_options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for log_file in log_files {
        let entry_name = log_file
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid log file name: {}", log_file.display()))?;
        zip_writer.start_file(entry_name, zip_options)?;

        let mut source = File::open(&log_file)?;
        copy(&mut source, &mut zip_writer)?;
    }

    zip_writer.finish()?;
    Ok(zip_path)
}

fn temp_log_file_path(log_directory: impl AsRef<Path>) -> PathBuf {
    let channel_logfile_name = ChannelState::logfile_name();
    log_directory
        .as_ref()
        .join(format!("{channel_logfile_name}.{TEMP_LOG_FILE_SUFFIX}"))
}

#[cfg(feature = "crash_reporting")]
fn sentry_log_filter(md: &log::Metadata) -> sentry_log::LogFilter {
    if warp_core::errors::should_ignore_log_for_sentry(md) {
        return sentry_log::LogFilter::Ignore;
    }

    match md.target() {
        // Ignore any log lines that come from the `log_panics` crate.
        "panic" => sentry_log::LogFilter::Ignore,

        // Filter out spammy INFO-level log lines from wgpu.
        t if t.starts_with("wgpu_core") || t.starts_with("wgpu_hal") => {
            sentry_log::LogFilter::Ignore
        }

        // Filter out the "redraw_frame" logging from breadcrumbs.
        "warpui::core::redraw_frame" => sentry_log::LogFilter::Ignore,

        // Filter out logs from the crash-reporting implementation, in case it logs
        // anything in the process of forwarding logs to Sentry.
        t if t.starts_with("warp::crash_reporting::") => sentry_log::LogFilter::Ignore,

        _ => sentry_log::default_filter(md),
    }
}

fn init_internal(
    is_from_crash_recovery_process: bool,
    is_cli: bool,
    log_destination: Option<LogDestination>,
) -> Result<()> {
    /// Returns an empty file named `warp.log` to log the current execution, and
    /// renames the previous execution's log to a temporary name.
    fn setup_log_files_for_current_execution(
        log_directory: &Path,
        is_from_crash_recovery_process: bool,
    ) -> Result<File> {
        fs::create_dir_all(log_directory)?;

        let main_log_path = if is_from_crash_recovery_process {
            // Use a temporary file for logs within the crash recovery process. We intentionally do
            // not rename the old main log file to `warp.log.temp` like we do below because this
            // would result in us moving the log file of the parent process.
            crash_recovery_process_log_file_path(log_directory)
        } else {
            let main_log_path = main_process_log_file_path(log_directory);

            // Rename the old main log file to `warp.log.temp`.
            // We rotate the log files later in the background to make fewer blocking calls.
            let _ = fs::rename(main_log_path.clone(), temp_log_file_path(log_directory));
            main_log_path
        };

        let main_log_file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(main_log_path)?;
        Ok(main_log_file)
    }

    let mut base_logger = env_logger::builder();

    base_logger.filter_level(LevelFilter::Info);

    // Only include `WARN` or higher logs for wgpu. By default, wgpu outputs logs at the `INFO`
    // level multiple times _per_ frame. See https://github.com/gfx-rs/wgpu/issues/3206.
    // Naga is overly noisy at `DEBUG`, so increase to `INFO`.
    base_logger
        .filter(Some("naga"), LevelFilter::Info)
        .filter(Some("wgpu_core"), LevelFilter::Warn)
        // Since we always pair an insertion with a deletion to avoid duplicate,
        // tantivy will log a lot of warnings for deleting a non-existing doc.
        .filter(Some("tantivy"), LevelFilter::Error)
        .filter(
            Some("wgpu_hal"),
            // On Windows with the DX12 backend, wgpu_hal outputs a ton of WARN-level logs.
            if cfg!(windows) {
                LevelFilter::Error
            } else {
                LevelFilter::Warn
            },
        );
    base_logger.parse_default_env();

    let stdout_is_a_tty = std::io::stdout().is_terminal();
    let in_ci = env::var("CI").is_ok();
    let integration_test = env::var("WARP_INTEGRATION").is_ok();
    let use_logfile = match log_destination {
        Some(LogDestination::File) => true,
        Some(LogDestination::Stderr) => false,
        None => !stdout_is_a_tty && !in_ci && !integration_test,
    };

    let max_rotation = if is_cli {
        MAX_FILES_IN_CLI_ROTATION
    } else {
        MAX_FILES_IN_GUI_ROTATION
    };

    let mut log_directory = init_log_directory()?;
    if is_cli {
        log_directory = log_directory.join(CLI_LOG_SUBDIRECTORY);
    }
    if use_logfile {
        base_logger.target(env_logger::Target::Pipe(Box::new(
            setup_log_files_for_current_execution(&log_directory, is_from_crash_recovery_process)?,
        )));
        base_logger.format(format_for_file_output);
    } else {
        // Agent mode eval outputs are written to stdout but redirected to a file, so we don't want terminal styling.
        if cfg!(feature = "agent_mode_evals") {
            base_logger.write_style(env_logger::WriteStyle::Never);
        } else {
            base_logger.write_style(env_logger::WriteStyle::Always);
        }
        base_logger.format(format_for_terminal_output);
    }

    #[cfg(feature = "crash_reporting")]
    {
        let base_logger = base_logger.build();
        log::set_max_level(base_logger.filter());
        let logger = sentry_log::SentryLogger::with_dest(base_logger).filter(sentry_log_filter);
        log::set_boxed_logger(Box::new(logger))
            .expect("Should not have already initialized a logger");
    }

    #[cfg(not(feature = "crash_reporting"))]
    base_logger.init();

    // If we're logging to a file, initialize the `log_panics` crate, which
    // will install a panic hook that writes out panics using `log::error`.
    if use_logfile {
        log_panics::init();
    }

    LOG_STATE
        .set(LogState {
            use_logfile,
            log_directory,
            max_rotation,
        })
        .expect("Logging already initialized");
    // We can .expect here because .init would have already panicked if we initialized logging twice.

    Ok(())
}

pub fn log_directory() -> Result<std::path::PathBuf> {
    LOG_STATE
        .get()
        .map(|config| config.log_directory.clone())
        .ok_or_else(|| anyhow::anyhow!("Logging not initialized"))
}

fn init_log_directory() -> Result<std::path::PathBuf> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            Ok(dirs::home_dir()
                .ok_or_else(|| {
                    anyhow::anyhow!("could not locate home directory in order to create a log file")
                })?
                .join("Library/Logs/"))
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            Ok(warp_core::paths::state_dir())
        } else if #[cfg(windows)] {
            Ok(warp_core::paths::state_dir().join(warp_core::paths::WARP_LOGS_DIR))
        } else {
            Err(anyhow::anyhow!("Have not configured file-based logging for the current platform!"))
        }
    }
}

/// Initializes the logger before running tests.
///
/// Additionally, we must not write anything to stdout in this function, as it
/// can interfere with test harnesses collecting the set of tests to run.  (This
/// is why we're not simply calling the init() function above.)
pub fn init_logging_for_unit_tests() {
    env_logger::builder()
        .is_test(true)
        .filter_level(LevelFilter::Info)
        .write_style(env_logger::WriteStyle::Always)
        .parse_default_env()
        .format(format_for_terminal_output)
        .init();
}
