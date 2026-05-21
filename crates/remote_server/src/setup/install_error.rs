//! Typed classification of remote-server install failures.
//!
//! Production install errors arrive as raw exit codes + stderr strings
//! from the install script or SSH transport layer.  This module converts
//! them into a [`InstallFailureCategory`] enum so that:
//!
//! 1. Telemetry gets a stable, enumerated tag instead of a free-form string.
//! 2. The retry/fallback logic can match on categories instead of fragile
//!    substring tests.
//! 3. UI can render targeted user-facing messages.
//!
//! The classifier is intentionally conservative: if the stderr doesn't
//! match any known pattern, it falls through to [`InstallFailureCategory::Unknown`].

use std::fmt;

/// Exit code the install script uses when the detected architecture is
/// unsupported (e.g. `mips`, `ppc64le`).
const UNSUPPORTED_ARCH_OR_OS_EXIT_CODE: i32 = 2;

/// Typed classification of a remote-server install failure.
///
/// Each variant corresponds to one of the CSV failure families observed
/// in production.  The ordering roughly follows the install script's
/// execution flow: platform checks → download → extraction → placement.
///
/// Aligned with the coordinated API shape so that
/// `RemoteServerSetupState::Failed` can carry an
/// `Option<InstallFailureCategory>` alongside the raw stderr, and
/// telemetry / UI can switch on the category.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallFailureCategory {
    // ── Timeout ─────────────────────────────────────────────────────
    /// The install script timed out (SSH-level timeout fired before
    /// the script exited).
    Timeout,

    // ── Platform / environment ──────────────────────────────────────
    /// `curl` is specifically missing (detected from stderr). In
    /// practice the install script falls through to wget, so this is
    /// rarely seen in isolation; the script emits [`MissingHttpClient`]
    /// when *both* are absent.
    MissingCurl,
    /// `wget` is specifically missing (detected from stderr). Same
    /// caveat as [`MissingCurl`].
    MissingWget,
    /// Neither `curl` nor `wget` is available.  This is the trigger for
    /// the SCP upload fallback in the SSH transport (exit code 3).
    MissingHttpClient,
    /// `tar` is not available on the remote host.
    MissingTar,
    /// `bash` is not available (script is piped into `bash -s`).
    MissingBash,
    /// `uname -m` reported an architecture we don't ship a binary for.
    UnsupportedArchitecture,
    /// `uname -s` reported an OS we don't support (e.g. FreeBSD).
    UnsupportedOs,

    // ── Network / download ──────────────────────────────────────────
    /// DNS resolution failed.
    DnsFailure,
    /// TCP connection was refused by the remote endpoint.
    ConnectionRefused,
    /// Host or network is unreachable (no route, network down).
    ConnectionUnreachable,
    /// TLS handshake failed (certificate validation, expired cert, etc.).
    TlsCaFailure,
    /// HTTP 403 Forbidden from the download endpoint.
    HttpForbidden,
    /// HTTP 502 Bad Gateway from the CDN.
    HttpBadGateway,
    /// Any other HTTP error (e.g. 500, 503, generic curl -f exit 22).
    HttpError,
    /// The download started but was truncated (curl exit 18 / wget
    /// partial content).
    PartialDownload,

    // ── Filesystem / extraction ─────────────────────────────────────
    /// Writing the downloaded tarball to disk failed (e.g. broken pipe
    /// to the temp file, I/O error).
    DownloadWriteFailure,
    /// `mkdir -p` or `mv` on the install directory failed with EACCES.
    InstallDirPermissionDenied,
    /// No space left on device (ENOSPC) or disk quota exceeded.
    NoSpaceLeft,
    /// The filesystem (or mount) is read-only.
    ReadOnlyFilesystem,
    /// `tar -xzf` failed for a non-permission reason (corrupt archive,
    /// unsupported format, etc.).
    TarExtractionFailure,
    /// `tar -xzf` failed due to ownership or permission errors.
    TarPermissionFailure,

    // ── SSH / auth ──────────────────────────────────────────────────
    /// The remote requires a password change or has no TTY for
    /// interactive auth prompts.
    ExpiredPassword,
    /// Permission denied writing to a startup file (e.g. ~/.bashrc is
    /// read-only or owned by root).
    StartupFilePermissionDenied,
    /// SSH exited with code 255, indicating a forced disconnect, broken
    /// pipe, or connection reset.
    SshDisconnect,

    // ── Script-level ────────────────────────────────────────────────
    /// The install script exited with a non-zero code that doesn't match
    /// any sentinel, and stderr doesn't match a known pattern — but the
    /// script clearly ran (non-signal exit). Distinguished from
    /// [`Unknown`] for telemetry bucketing.
    ScriptError,

    // ── Catch-all ───────────────────────────────────────────────────
    /// The error didn't match any known pattern.
    Unknown,
}

impl InstallFailureCategory {
    /// Short human-readable title for UI banners and error summaries.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Timeout => "Install Timeout",
            Self::MissingCurl => "curl Not Found",
            Self::MissingWget => "wget Not Found",
            Self::MissingHttpClient => "No HTTP Client",
            Self::MissingTar => "tar Not Found",
            Self::MissingBash => "bash Not Found",
            Self::UnsupportedArchitecture => "Unsupported Architecture",
            Self::UnsupportedOs => "Unsupported OS",
            Self::DnsFailure => "DNS Resolution Failed",
            Self::ConnectionRefused => "Connection Refused",
            Self::ConnectionUnreachable => "Host Unreachable",
            Self::TlsCaFailure => "TLS/Certificate Error",
            Self::HttpForbidden => "HTTP 403 Forbidden",
            Self::HttpBadGateway => "HTTP 502 Bad Gateway",
            Self::HttpError => "HTTP Error",
            Self::PartialDownload => "Incomplete Download",
            Self::DownloadWriteFailure => "Download Write Error",
            Self::InstallDirPermissionDenied => "Permission Denied",
            Self::NoSpaceLeft => "No Space Left",
            Self::ReadOnlyFilesystem => "Read-Only Filesystem",
            Self::TarExtractionFailure => "Extraction Failed",
            Self::TarPermissionFailure => "Extraction Permission Error",
            Self::ExpiredPassword => "Password Expired",
            Self::StartupFilePermissionDenied => "Startup File Permission Denied",
            Self::SshDisconnect => "SSH Disconnected",
            Self::ScriptError => "Install Script Error",
            Self::Unknown => "Install Failed",
        }
    }

    /// Longer description suitable for error detail panels and telemetry.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Timeout => "The install script timed out before completing.",
            Self::MissingCurl => "curl is not installed on the remote host.",
            Self::MissingWget => "wget is not installed on the remote host.",
            Self::MissingHttpClient => "Neither curl nor wget is available on the remote host.",
            Self::MissingTar => "tar is not installed on the remote host.",
            Self::MissingBash => "bash is not available on the remote host.",
            Self::UnsupportedArchitecture => "The remote host's CPU architecture is not supported.",
            Self::UnsupportedOs => "The remote host's operating system is not supported.",
            Self::DnsFailure => "Failed to resolve the download server hostname.",
            Self::ConnectionRefused => "The download server refused the connection.",
            Self::ConnectionUnreachable => {
                "The download server is unreachable (no route or network down)."
            }
            Self::TlsCaFailure => "TLS certificate verification failed.",
            Self::HttpForbidden => "The download server returned HTTP 403 Forbidden.",
            Self::HttpBadGateway => "The download server returned HTTP 502 Bad Gateway.",
            Self::HttpError => "The download server returned an HTTP error.",
            Self::PartialDownload => "The download was truncated or incomplete.",
            Self::DownloadWriteFailure => "Failed to write the downloaded file to disk.",
            Self::InstallDirPermissionDenied => {
                "Permission denied creating or writing to the install directory."
            }
            Self::NoSpaceLeft => "No space left on device or disk quota exceeded.",
            Self::ReadOnlyFilesystem => "The filesystem is mounted read-only.",
            Self::TarExtractionFailure => "Failed to extract the downloaded archive.",
            Self::TarPermissionFailure => "Archive extraction failed due to a permission error.",
            Self::ExpiredPassword => "The remote account's password has expired or requires a TTY.",
            Self::StartupFilePermissionDenied => {
                "Permission denied writing to a shell startup file."
            }
            Self::SshDisconnect => "The SSH connection was forcibly closed (exit 255).",
            Self::ScriptError => "The install script exited with an error.",
            Self::Unknown => "An unknown error occurred during installation.",
        }
    }

    /// Short, stable string tag suitable for telemetry and serialization.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::MissingCurl => "missing_curl",
            Self::MissingWget => "missing_wget",
            Self::MissingHttpClient => "missing_http_client",
            Self::MissingTar => "missing_tar",
            Self::MissingBash => "missing_bash",
            Self::UnsupportedArchitecture => "unsupported_architecture",
            Self::UnsupportedOs => "unsupported_os",
            Self::DnsFailure => "dns_failure",
            Self::ConnectionRefused => "connection_refused",
            Self::ConnectionUnreachable => "connection_unreachable",
            Self::TlsCaFailure => "tls_ca_failure",
            Self::HttpForbidden => "http_forbidden",
            Self::HttpBadGateway => "http_bad_gateway",
            Self::HttpError => "http_error",
            Self::PartialDownload => "partial_download",
            Self::DownloadWriteFailure => "download_write_failure",
            Self::InstallDirPermissionDenied => "install_dir_permission_denied",
            Self::NoSpaceLeft => "no_space_left",
            Self::ReadOnlyFilesystem => "read_only_filesystem",
            Self::TarExtractionFailure => "tar_extraction_failure",
            Self::TarPermissionFailure => "tar_permission_failure",
            Self::ExpiredPassword => "expired_password",
            Self::StartupFilePermissionDenied => "startup_file_permission_denied",
            Self::SshDisconnect => "ssh_disconnect",
            Self::ScriptError => "script_error",
            Self::Unknown => "unknown",
        }
    }

    /// Whether this failure category is potentially retriable.
    ///
    /// Categories caused by transient conditions (network hiccups,
    /// timeouts, server errors) return `true`.  Permanent host
    /// conditions (permissions, disk, auth, architecture) return
    /// `false` to prevent wasteful blind retries.
    pub fn is_retriable(&self) -> bool {
        match self {
            // Transient / network
            Self::Timeout
            | Self::DnsFailure
            | Self::ConnectionRefused
            | Self::ConnectionUnreachable
            | Self::TlsCaFailure
            | Self::HttpBadGateway
            | Self::HttpError
            | Self::PartialDownload => true,

            // Permanent host condition — do NOT retry
            Self::MissingCurl
            | Self::MissingWget
            | Self::MissingHttpClient
            | Self::MissingTar
            | Self::MissingBash
            | Self::UnsupportedArchitecture
            | Self::UnsupportedOs
            | Self::HttpForbidden
            | Self::DownloadWriteFailure
            | Self::InstallDirPermissionDenied
            | Self::NoSpaceLeft
            | Self::ReadOnlyFilesystem
            | Self::TarExtractionFailure
            | Self::TarPermissionFailure
            | Self::ExpiredPassword
            | Self::StartupFilePermissionDenied
            | Self::SshDisconnect
            | Self::ScriptError
            | Self::Unknown => false,
        }
    }
}

impl fmt::Display for InstallFailureCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

/// Classify a raw install failure from the install script or SSH transport
/// into a typed [`InstallFailureCategory`].
///
/// # Arguments
///
/// * `stderr` — Combined stderr output from the install script / SSH
///   command.
/// * `exit_code` — The process exit code, if available. `None` when the
///   process was killed by a signal or the exit code wasn't captured.
pub fn classify_install_failure(stderr: &str, exit_code: Option<i32>) -> InstallFailureCategory {
    classify_install_failure_inner(stderr, exit_code, false)
}

/// Like [`classify_install_failure`] but accepts an explicit timeout flag.
///
/// `is_timeout` should be `true` when the failure was caused by the
/// SSH-level timeout firing before the script exited (the async runtime
/// kills the child and returns a timeout error).
pub fn classify_install_failure_with_timeout(
    stderr: &str,
    exit_code: Option<i32>,
    is_timeout: bool,
) -> InstallFailureCategory {
    classify_install_failure_inner(stderr, exit_code, is_timeout)
}

fn classify_install_failure_inner(
    stderr: &str,
    exit_code: Option<i32>,
    is_timeout: bool,
) -> InstallFailureCategory {
    // Timeout is unambiguous — the script didn't finish in time.
    if is_timeout {
        return InstallFailureCategory::Timeout;
    }

    // SSH exit 255 → forced disconnect / broken pipe.
    if exit_code == Some(255) {
        return InstallFailureCategory::SshDisconnect;
    }

    // Script exit 3 → no HTTP client (sentinel from install script).
    if exit_code == Some(super::NO_HTTP_CLIENT_EXIT_CODE) {
        return InstallFailureCategory::MissingHttpClient;
    }

    // Script exit 2 → unsupported arch or OS.
    if exit_code == Some(UNSUPPORTED_ARCH_OR_OS_EXIT_CODE) {
        if stderr_contains_unsupported_arch(stderr) {
            return InstallFailureCategory::UnsupportedArchitecture;
        }
        if stderr_contains_unsupported_os(stderr) {
            return InstallFailureCategory::UnsupportedOs;
        }
        // Exit 2 but no parseable arch/os — still treat as unsupported.
        return InstallFailureCategory::UnsupportedArchitecture;
    }

    let lower = stderr.to_lowercase();

    // ── Bash / shell availability ───────────────────────────────────
    if lower.contains("bash: not found")
        || lower.contains("bash: command not found")
        || lower.contains("no such file or directory: bash")
        || lower.contains("cannot execute binary file") && lower.contains("bash")
    {
        return InstallFailureCategory::MissingBash;
    }

    // ── tar availability ────────────────────────────────────────────
    if lower.contains("tar: not found")
        || lower.contains("tar: command not found")
        || (lower.contains("no such file") && lower.contains("tar"))
    {
        return InstallFailureCategory::MissingTar;
    }

    // ── DNS failure ─────────────────────────────────────────────────
    if lower.contains("could not resolve host")
        || lower.contains("name or service not known")
        || lower.contains("temporary failure in name resolution")
        || lower.contains("unable to resolve host")
        || lower.contains("dns_error")
    {
        return InstallFailureCategory::DnsFailure;
    }

    // ── Connection refused ──────────────────────────────────────────
    if lower.contains("connection refused") {
        return InstallFailureCategory::ConnectionRefused;
    }

    // ── Connection unreachable ──────────────────────────────────────
    if lower.contains("no route to host")
        || lower.contains("network is unreachable")
        || (lower.contains("connection timed out")
            && !lower.contains("ssl")
            && !lower.contains("tls"))
    {
        return InstallFailureCategory::ConnectionUnreachable;
    }

    // ── TLS / CA failures ───────────────────────────────────────────
    if lower.contains("ssl")
        || lower.contains("certificate")
        || lower.contains("tls")
        || lower.contains("ca-bundle")
        || lower.contains("unable to get local issuer certificate")
        || lower.contains("verify failed")
    {
        return InstallFailureCategory::TlsCaFailure;
    }

    // ── HTTP status codes ───────────────────────────────────────────
    if lower.contains("403 forbidden")
        || lower.contains("http/1.1 403")
        || lower.contains("http/2 403")
    {
        return InstallFailureCategory::HttpForbidden;
    }
    if lower.contains("502 bad gateway")
        || lower.contains("http/1.1 502")
        || lower.contains("http/2 502")
    {
        return InstallFailureCategory::HttpBadGateway;
    }
    // curl exit 22 = HTTP error ≥ 400 (with -f flag)
    if exit_code == Some(22) && lower.contains("403") {
        return InstallFailureCategory::HttpForbidden;
    }
    if exit_code == Some(22) {
        return InstallFailureCategory::HttpError;
    }
    if lower.contains("503 service unavailable") || lower.contains("http/1.1 503") {
        return InstallFailureCategory::HttpError;
    }

    // ── Partial download ────────────────────────────────────────────
    if exit_code == Some(18)
        || lower.contains("partial file")
        || lower.contains("transfer closed with outstanding read data")
        || lower.contains("incomplete download")
        || (lower.contains("unexpected end") && lower.contains("gz"))
    {
        return InstallFailureCategory::PartialDownload;
    }

    // ── Download write failure ──────────────────────────────────────
    if exit_code == Some(23)
        || (lower.contains("write error") && lower.contains("download"))
        || lower.contains("failed writing body")
    {
        return InstallFailureCategory::DownloadWriteFailure;
    }

    // ── tar extraction failures (permission) ────────────────────────
    if (lower.contains("tar") || lower.contains("extract"))
        && (lower.contains("cannot open")
            || lower.contains("operation not permitted")
            || lower.contains("cannot change ownership")
            || lower.contains("permission denied"))
    {
        return InstallFailureCategory::TarPermissionFailure;
    }

    // ── tar extraction failures (non-permission) ────────────────────
    if (lower.contains("tar") || lower.contains("extract"))
        && (lower.contains("not in gzip format")
            || lower.contains("unexpected eof")
            || lower.contains("invalid tar")
            || lower.contains("corrupted"))
    {
        return InstallFailureCategory::TarExtractionFailure;
    }

    // ── Filesystem: permission denied ───────────────────────────────
    if lower.contains("permission denied") {
        if lower.contains(".bashrc")
            || lower.contains(".bash_profile")
            || lower.contains(".profile")
            || lower.contains(".zshrc")
        {
            return InstallFailureCategory::StartupFilePermissionDenied;
        }
        return InstallFailureCategory::InstallDirPermissionDenied;
    }

    // ── Filesystem: no space / quota ────────────────────────────────
    if lower.contains("no space left on device")
        || lower.contains("disk quota exceeded")
        || lower.contains("enospc")
    {
        return InstallFailureCategory::NoSpaceLeft;
    }

    // ── Filesystem: read-only ───────────────────────────────────────
    if lower.contains("read-only file system") || lower.contains("erofs") {
        return InstallFailureCategory::ReadOnlyFilesystem;
    }

    // ── Expired password / no TTY ───────────────────────────────────
    if lower.contains("password has expired")
        || lower.contains("you must change your password")
        || lower.contains("no tty present")
        || lower.contains("password change required")
    {
        return InstallFailureCategory::ExpiredPassword;
    }

    // ── Script error: non-zero exit with no recognized pattern ──────
    if let Some(code) = exit_code {
        if code != 0 {
            return InstallFailureCategory::ScriptError;
        }
    }

    InstallFailureCategory::Unknown
}

/// Checks if stderr contains the install script's "unsupported arch:" message.
fn stderr_contains_unsupported_arch(stderr: &str) -> bool {
    stderr
        .lines()
        .any(|l| l.trim().starts_with("unsupported arch:"))
}

/// Checks if stderr contains the install script's "unsupported OS:" message.
fn stderr_contains_unsupported_os(stderr: &str) -> bool {
    stderr
        .lines()
        .any(|l| l.trim().starts_with("unsupported OS:"))
}

#[cfg(test)]
#[path = "install_error_tests.rs"]
mod tests;
