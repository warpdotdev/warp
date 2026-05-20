use crate::server::telemetry::TelemetryEvent;
use anyhow::anyhow;
use anyhow::{bail, Result};
use channel_versions::VersionInfo;
use command::blocking::Command;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::{fs, io};
use std::{io::Write as _, time::Duration};
use tempfile::TempPath;
use warp_core::channel::{Channel, ChannelState};
use warpui::AppContext;

use super::{release_assets_directory_url, DownloadReady};
use crate::util::windows::install_dir;

lazy_static! {
    /// The path to the temporary file that stores the installer for the new update.
    static ref INSTALLER_PATH: Arc<Mutex<Option<TempPath>>> = Default::default();
}

/// Download the Inno Setup install wizard, the same one users run on the first Warp install, and
/// place it into the "data dir".
pub(super) async fn download_update_and_cleanup(
    version_info: &VersionInfo,
    _update_id: &str,
    client: &http_client::Client,
) -> Result<DownloadReady> {
    const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

    let installer_file_name = installer_file_name()?;
    let url = format!(
        "{}/{}",
        release_assets_directory_url(ChannelState::channel(), &version_info.version),
        installer_file_name
    );

    // Create a temporary file that we'll write the download into.
    let mut already_exists = false;
    let mut new_installer = tempfile::Builder::new()
        .rand_bytes(0)
        .suffix(&format!("{}-{}", version_info.version, installer_file_name))
        .make(|path| {
            already_exists = path.is_file();
            if already_exists {
                File::open(path)
            } else {
                File::create(path)
            }
        })?;

    if !already_exists {
        log::info!("Downloading {url} to {}...", new_installer.path().display());

        let response = client
            .get(&url)
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await?
            .error_for_status()?;
        new_installer
            .as_file_mut()
            .write_all(&response.bytes().await?)?;
    }

    *INSTALLER_PATH.lock() = Some(new_installer.into_temp_path());

    Ok(DownloadReady::Yes)
}

const UPDATE_LOG_FILENAME: &str = "warp_update.log";

fn autoupdate_log_file() -> Result<PathBuf> {
    warp_logging::log_directory().map(|dir| dir.join(UPDATE_LOG_FILENAME))
}

fn parse_exit_code_after_marker(contents_lowercase: &[u8], failed_marker: &[u8]) -> Option<i32> {
    const EXIT_CODE_MARKER: &[u8] = b"exit code: ";

    let failed_pos = memchr::memmem::find(contents_lowercase, failed_marker)?;
    let after_failed = &contents_lowercase[failed_pos..];
    let marker_pos = memchr::memmem::find(after_failed, EXIT_CODE_MARKER)?;
    let after_marker = &after_failed[marker_pos + EXIT_CODE_MARKER.len()..];
    let sign_len = if after_marker.first() == Some(&b'-') {
        1
    } else {
        0
    };
    let digit_len = after_marker[sign_len..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digit_len == 0 {
        return None;
    }
    std::str::from_utf8(&after_marker[..sign_len + digit_len])
        .ok()?
        .parse()
        .ok()
}

/// Parses the taskkill exit code from an Inno Setup log containing a
/// "force-kill failed for" line. Returns `None` if no such line is found or
/// the exit code cannot be parsed.
fn parse_forcekill_exit_code(contents_lowercase: &[u8]) -> Option<i32> {
    const FAILED_MARKER: &[u8] = b"force-kill failed for";
    parse_exit_code_after_marker(contents_lowercase, FAILED_MARKER)
}

/// Parses the PowerShell exit code from an Inno Setup log containing a
/// "minidump-server cleanup failed" line. Returns `None` if no such line is
/// found or the exit code cannot be parsed.
fn parse_minidump_cleanup_exit_code(contents_lowercase: &[u8]) -> Option<i32> {
    const FAILED_MARKER: &[u8] = b"minidump-server cleanup failed";
    parse_exit_code_after_marker(contents_lowercase, FAILED_MARKER)
}

/// Checks the autoupdate log file from a previous update attempt.
/// Sends telemetry for specific known issues, and sends a Sentry event if errors are found.
/// The log file is renamed after processing to avoid duplicate reports on subsequent launches.
pub(super) fn check_and_report_update_errors(ctx: &mut AppContext) {
    let log_path = match autoupdate_log_file() {
        Ok(path) => path,
        Err(e) => {
            log::warn!("Failed to determine autoupdate log file path: {e:#}");
            return;
        }
    };

    // Inno Setup logs use the system's active codepage (often Windows-1252), not UTF-8.
    // We read as raw bytes to avoid silently skipping non-UTF-8 log files.
    let contents = match fs::read(&log_path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            log::info!("No autoupdate logs found");
            return;
        }
        Err(e) => {
            log::warn!("Failed to read autoupdate log file: {e:#}");
            return;
        }
    };

    let contents_lowercase = contents.to_ascii_lowercase();

    let has_unable_to_close = memchr::memmem::find(
        &contents_lowercase,
        b"setup was unable to automatically close all applications",
    )
    .is_some();
    if has_unable_to_close {
        crate::send_telemetry_sync_from_app_ctx!(
            TelemetryEvent::AutoupdateUnableToCloseApplications,
            ctx
        );
    }

    let has_file_in_use = memchr::memmem::find(
        &contents_lowercase,
        b"the process cannot access the file because it is being used by another process",
    )
    .is_some();
    if has_file_in_use {
        crate::send_telemetry_sync_from_app_ctx!(TelemetryEvent::AutoupdateFileInUse, ctx);
    }

    // Fired when the mutex polling loop timed out and a force-kill was attempted.
    let has_mutex_timeout =
        memchr::memmem::find(&contents_lowercase, b"warp mutex still held after timeout").is_some();
    if has_mutex_timeout {
        crate::send_telemetry_sync_from_app_ctx!(TelemetryEvent::AutoupdateMutexTimeout, ctx);
    }

    // Fired when taskkill returned non-zero after the mutex timeout.
    // Exit code 128 means "no matching process found" — the process was already
    // gone when taskkill ran — so suppress that harmless race condition.
    if let Some(exit_code) = parse_forcekill_exit_code(&contents_lowercase) {
        if exit_code != 128 {
            crate::send_telemetry_sync_from_app_ctx!(
                TelemetryEvent::AutoupdateForcekillFailed { exit_code },
                ctx
            );
        }
    }

    // Fired when the PowerShell cleanup of the orphaned minidump server process
    // returned a non-zero exit code.
    if let Some(exit_code) = parse_minidump_cleanup_exit_code(&contents_lowercase) {
        crate::send_telemetry_sync_from_app_ctx!(
            TelemetryEvent::AutoupdateMinidumpCleanupFailed { exit_code },
            ctx
        );
    }

    #[cfg(feature = "crash_reporting")]
    {
        use sentry::protocol::{Attachment, AttachmentType};

        // Patterns for known benign errors that should not trigger Sentry reporting.
        const IGNOREABLE_ERRORS: &[&[u8]] = &[
            // User running out of disk space is not an error we need concern ourselves with.
            // This message occurs after "An error occurred while trying to copy a file:"
            b"there is not enough space on the disk",
            // Recent Inno Setup versions try to enable a security feature which is unavailable on
            // Windows 10 versions prior to 22H2 and this call fails. The failure is benign.
            b"setprocessmitigationpolicy failed with error code 87",
            // Bundled skill files whose names contain "error" appear in "Dest filename:" log lines
            // and produce false positives.
            b"error-codes.md",
            b"error-recovery.md",
        ];

        let mut error_count = memchr::memmem::find_iter(&contents_lowercase, b"error").count();

        for pattern in IGNOREABLE_ERRORS {
            let ignoreable_count = memchr::memmem::find_iter(&contents_lowercase, pattern).count();
            error_count = error_count.saturating_sub(ignoreable_count);
        }

        if error_count > 0 {
            log::warn!("Autoupdate log file contains errors; reporting to Sentry");

            let attachment = Attachment {
                buffer: contents,
                filename: UPDATE_LOG_FILENAME.to_string(),
                ty: Some(AttachmentType::Attachment),
                ..Default::default()
            };
            sentry::with_scope(
                |scope| {
                    scope.add_attachment(attachment);
                },
                || sentry::capture_message("Windows auto-update error", sentry::Level::Error),
            );
        }
    }

    // Rename the log file to avoid duplicate reports on subsequent launches.
    // We keep the file around so the user can still view it or attach it to a GitHub issue.
    let reported_path = log_path.with_extension("log.reported");
    if let Err(e) = fs::rename(&log_path, &reported_path) {
        log::warn!("Failed to rename autoupdate log file after reporting: {e:#}");
    }
}

pub(super) fn relaunch() -> Result<()> {
    let install_dir = install_dir()?;
    let Some(installer_path) = INSTALLER_PATH
        .lock()
        .as_ref()
        .map(|path| path.to_path_buf())
    else {
        bail!("No installer path");
    };

    let log_arg = match autoupdate_log_file() {
        Ok(dir) => format!("/LOG={}", dir.display()),
        Err(e) => {
            log::warn!("Failed to determine location for autoupdate logs: {e:#}");
            "/LOG".to_string()
        }
    };

    // The Inno Setup install wizard will run without user input. It will re-launch Warp after
    // installing the update files.
    // https://jrsoftware.org/ishelp/index.php?topic=setupcmdline
    Command::new(&installer_path)
        .args([
            // Skip asking the user to confirm.
            "/SP-",
            // Do not prompt the user for anything. Note that we do not use "VERYSILENT" so that a
            // progress bar is still shown. This is useful since the update process may take a few
            // seconds.
            "/SILENT",
            // Do not provide a cancel button on the progress bar page.
            "/NOCANCEL",
            // Indicate that restarting Windows is not necessary.
            "/NORESTART",
            &log_arg,
            "/update=1",
            // Do not forcibly kill Warp via RestartManager. The installer will wait for
            // Warp to exit naturally by polling the single-instance mutex instead.
            "/NOCLOSEAPPLICATIONS",
            &format!("/DIR={}", install_dir.display()),
        ])
        .spawn()?;

    Ok(())
}

fn installer_file_name() -> Result<String> {
    let app_name_prefix = app_name_prefix(ChannelState::channel());

    // For example, on arm64 this is WarpSetup-arm64.exe and on x64 this is
    // WarpSetup.exe.
    if cfg!(target_arch = "aarch64") {
        Ok(format!("{app_name_prefix}Setup-arm64.exe"))
    } else if cfg!(target_arch = "x86_64") {
        Ok(format!("{app_name_prefix}Setup.exe"))
    } else {
        Err(anyhow!(
            "Could not construct setup file name for unsupported architecture"
        ))
    }
}

fn app_name_prefix(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "Warp",
        Channel::Preview => "WarpPreview",
        Channel::Local => "warp",
        Channel::Integration => "integration",
        Channel::Dev => "WarpDev",
        Channel::Oss => "warp-oss",
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_forcekill_exit_code, parse_minidump_cleanup_exit_code};

    fn log(line: &str) -> Vec<u8> {
        line.to_ascii_lowercase().into_bytes()
    }

    #[test]
    fn parses_typical_failure() {
        // Typical Inno Setup log line for a real taskkill failure (e.g. access denied).
        let contents = log("force-kill failed for dev.exe (exit code: 1)");
        assert_eq!(parse_forcekill_exit_code(&contents), Some(1));
    }

    #[test]
    fn parses_exit_code_128() {
        // Exit code 128 = "no matching process" — the harmless race condition.
        let contents = log("force-kill failed for dev.exe (exit code: 128)");
        assert_eq!(parse_forcekill_exit_code(&contents), Some(128));
    }

    #[test]
    fn parses_exit_code_embedded_in_multiline_log() {
        // The pattern appears after several unrelated log lines.
        let contents = log(
            "[2024-01-01 00:00:00] Warp mutex still held after timeout; force-killing remaining processes.\n\
             [2024-01-01 00:00:01] force-kill failed for warp.exe (exit code: 5)\n\
             [2024-01-01 00:00:02] Installation complete.",
        );
        assert_eq!(parse_forcekill_exit_code(&contents), Some(5));
    }

    #[test]
    fn returns_none_when_no_forcekill_line() {
        // Log contains no force-kill attempt at all.
        let contents = log("warp mutex still held after timeout; proceeding.");
        assert_eq!(parse_forcekill_exit_code(&contents), None);
    }

    #[test]
    fn returns_none_when_forcekill_marker_present_but_no_exit_code() {
        // Malformed log line — marker present but no "exit code:" substring.
        let contents = log("force-kill failed for dev.exe");
        assert_eq!(parse_forcekill_exit_code(&contents), None);
    }
    #[test]
    fn returns_none_when_exit_code_has_no_digits() {
        // Malformed log line — marker present but no parseable integer.
        let contents = log("force-kill failed for dev.exe (exit code: -)");
        assert_eq!(parse_forcekill_exit_code(&contents), None);
    }

    #[test]
    fn parses_signed_minidump_cleanup_exit_code() {
        // PowerShell can report signed HRESULT values for cleanup failures.
        let contents = log("minidump-server cleanup failed (exit code: -2147024891)");
        assert_eq!(
            parse_minidump_cleanup_exit_code(&contents),
            Some(-2147024891)
        );
    }

    #[test]
    fn parses_unsigned_minidump_cleanup_exit_code() {
        let contents = log("minidump-server cleanup failed (exit code: 5)");
        assert_eq!(parse_minidump_cleanup_exit_code(&contents), Some(5));
    }

    #[test]
    fn returns_none_for_empty_log() {
        assert_eq!(parse_forcekill_exit_code(b""), None);
        assert_eq!(parse_minidump_cleanup_exit_code(b""), None);
    }
}
