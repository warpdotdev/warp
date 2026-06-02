//! Classification of remote-server install failures into actionable families.
//!
//! ## Design notes
//!
//! The install pipeline produces errors as plain `String` values on the current
//! codebase (`master`).  The `aloke/error_handling` branch introduces a
//! structured `transport::Error` enum; once that lands, callers can obtain the
//! family cheaply via [`InstallErrorFamily::from_transport_error`] instead of
//! parsing strings.  Both paths are provided here so tests can cover the full
//! fixture set today and the adapter function is trivial to fill in once the
//! structured type is merged.
//!
//! ## Error families (derived from production error CSV)
//!
//! | Family                  | Recoverability                          | Trigger                                   |
//! |-------------------------|-----------------------------------------|-------------------------------------------|
//! | `NoHttpClient`          | [`Recoverability::RecoverableScpFallback`] | Script exits `NO_HTTP_CLIENT_EXIT_CODE=3`  |
//! | `UnsupportedOs`         | [`Recoverability::NonRecoverable`]      | Script/uname reports unknown OS           |
//! | `UnsupportedArch`       | [`Recoverability::NonRecoverable`]      | Script/uname reports unknown arch         |
//! | `GlibcTooOld`           | [`Recoverability::RecoverableControlMaster`] | Preinstall check → `glibc_too_old`   |
//! | `NonGlibc`              | [`Recoverability::RecoverableControlMaster`] | Preinstall check → `non_glibc`       |
//! | `Timeout`               | [`Recoverability::PossiblyRecoverable`] | SSH op timed out                          |
//! | `NoBinaryInTarball`     | [`Recoverability::NonRecoverable`]      | Script: "no binary found in tarball"      |
//! | `ScriptGenericFailure`  | [`Recoverability::NonRecoverable`]      | Script non-zero exit, known stderr        |
//! | `SshTransportFailure`   | [`Recoverability::PossiblyRecoverable`] | SSH spawn/write error                     |
//! | `DownloadFailure`       | [`Recoverability::NonRecoverable`]      | curl/wget non-zero (bad URL, 404, etc.)   |
//! | `Unknown`               | [`Recoverability::NonRecoverable`]      | Anything else                             |

use crate::setup::{PreinstallStatus, UnsupportedReason};

/// How recoverable a given install error family is.
///
/// Each variant encodes *why* recovery is possible, so the controller can
/// pick the right fallback without re-parsing the error message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recoverability {
    /// No HTTP client on the remote host; the SCP upload fallback should be
    /// triggered immediately.  This is the only family where the *current*
    /// install attempt can continue without user interaction.
    RecoverableScpFallback,
    /// The host's libc is incompatible with the prebuilt binary.  Fall back
    /// to the legacy ControlMaster-backed SSH flow silently (no error banner).
    RecoverableControlMaster,
    /// The operation may succeed on a retry (e.g. transient timeout).
    PossiblyRecoverable,
    /// The failure is deterministic for this host; no fallback is available.
    NonRecoverable,
}

/// A coarse classification of a remote-server install failure.
///
/// Variants are ordered from most specific (positively-identified) to least
/// (catch-all `Unknown`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallErrorFamily {
    /// Neither `curl` nor `wget` is available on the remote host.
    /// The install script exits with [`crate::setup::NO_HTTP_CLIENT_EXIT_CODE`].
    /// The SCP upload fallback should be triggered.
    NoHttpClient,
    /// The remote host's OS is not supported by the prebuilt binary
    /// (reported by `uname` or the install script).
    UnsupportedOs {
        /// The raw OS string reported by the remote host (e.g. `"FreeBSD"`).
        os: String,
    },
    /// The remote host's CPU architecture is not supported by the prebuilt
    /// binary.
    UnsupportedArch {
        /// The raw arch string (e.g. `"mips"`, `"riscv64"`).
        arch: String,
    },
    /// The remote host's glibc is older than the minimum required by the
    /// prebuilt binary.  Classified by the preinstall check script, not the
    /// installer.  Falls back to ControlMaster.
    GlibcTooOld,
    /// The remote host does not use glibc (musl, bionic, uClibc, …).
    /// Classified by the preinstall check script.  Falls back to
    /// ControlMaster.
    NonGlibc {
        /// The libc family name reported by the probe (e.g. `"musl"`).
        name: String,
    },
    /// The SSH operation or install script timed out.  May be transient.
    Timeout,
    /// The downloaded tarball did not contain a recognisable binary.
    /// Usually indicates a CDN 404 or a partial download.
    NoBinaryInTarball,
    /// The install script exited non-zero for a reason other than the
    /// sentinel exit codes.
    ScriptGenericFailure {
        /// The non-zero exit code from the script.
        exit_code: i32,
        /// A short extract from stderr.
        stderr_excerpt: String,
    },
    /// The SSH transport itself failed before the script could run
    /// (spawn failure, stdin write failure, broken pipe).
    SshTransportFailure { detail: String },
    /// curl or wget was found but returned a non-zero exit (e.g. 404, SSL,
    /// connection refused).
    DownloadFailure {
        exit_code: i32,
        stderr_excerpt: String,
    },
    /// Could not be classified more precisely.
    Unknown { raw: String },
}

impl InstallErrorFamily {
    /// Returns the recoverability of this error family.
    pub fn recoverability(&self) -> Recoverability {
        match self {
            Self::NoHttpClient => Recoverability::RecoverableScpFallback,
            Self::GlibcTooOld | Self::NonGlibc { .. } => Recoverability::RecoverableControlMaster,
            Self::Timeout | Self::SshTransportFailure { .. } => Recoverability::PossiblyRecoverable,
            Self::UnsupportedOs { .. }
            | Self::UnsupportedArch { .. }
            | Self::NoBinaryInTarball
            | Self::ScriptGenericFailure { .. }
            | Self::DownloadFailure { .. }
            | Self::Unknown { .. } => Recoverability::NonRecoverable,
        }
    }

    /// Returns `true` if this family has a clean fall-back path that should
    /// not surface an error banner to the user.
    pub fn is_silent_fallback(&self) -> bool {
        matches!(
            self.recoverability(),
            Recoverability::RecoverableControlMaster | Recoverability::RecoverableScpFallback
        )
    }

    /// Classify a raw install-script error given its exit code and stderr.
    ///
    /// This is the *string-based* path used on the current `master` where
    /// transport errors are plain `String`s.  Once `aloke/error_handling` is
    /// merged, prefer [`Self::from_transport_error`] for typed errors and
    /// reserve this function for legacy or test-only call sites.
    ///
    /// Matching priority (first match wins):
    ///
    /// 1. Exit code 3 → [`Self::NoHttpClient`] (sentinel from the install script).
    /// 2. "timed out" anywhere in the message → [`Self::Timeout`].
    /// 3. "neither curl nor wget" → [`Self::NoHttpClient`] (belt-and-suspenders
    ///    for transports that surface the full stderr as a string error).
    /// 4. "unsupported OS:" prefix → [`Self::UnsupportedOs`].
    /// 5. "unsupported arch:" prefix → [`Self::UnsupportedArch`].
    /// 6. "no binary found in tarball" → [`Self::NoBinaryInTarball`].
    /// 7. "curl: command not found" / "wget: command not found" → [`Self::NoHttpClient`].
    /// 8. "curl" or "wget" error followed by non-zero exit → [`Self::DownloadFailure`].
    /// 9. "Failed to spawn SSH" / "Failed to write script" → [`Self::SshTransportFailure`].
    /// 10. Any other non-zero exit → [`Self::ScriptGenericFailure`].
    /// 11. Zero exit with no message → [`Self::Unknown`].
    pub fn from_exit_and_stderr(exit_code: Option<i32>, stderr: &str) -> Self {
        use crate::setup::NO_HTTP_CLIENT_EXIT_CODE;

        // 1. Sentinel exit code for missing HTTP client.
        if exit_code == Some(NO_HTTP_CLIENT_EXIT_CODE) {
            return Self::NoHttpClient;
        }

        let stderr_lower = stderr.to_ascii_lowercase();

        // 2. Timeout (covers both SSH-level "timed out" and script-level "script timed out").
        if stderr_lower.contains("timed out") || stderr_lower.contains("timeout") {
            return Self::Timeout;
        }

        // 3. Belt-and-suspenders: explicit "neither curl nor wget" message.
        if stderr_lower.contains("neither curl nor wget") {
            return Self::NoHttpClient;
        }

        // 4–5. Unsupported OS / arch from the install script or uname parser.
        if let Some(rest) = find_after(stderr, "unsupported OS: ") {
            return Self::UnsupportedOs {
                os: rest.split_whitespace().next().unwrap_or(rest).to_string(),
            };
        }
        if let Some(rest) = find_after(stderr, "unsupported arch: ") {
            return Self::UnsupportedArch {
                arch: rest.split_whitespace().next().unwrap_or(rest).to_string(),
            };
        }
        // Also handle lower-case variants emitted by transport::Error formatting.
        if let Some(rest) = find_after_lower(&stderr_lower, "unsupported os: ") {
            return Self::UnsupportedOs {
                os: rest.to_string(),
            };
        }
        if let Some(rest) = find_after_lower(&stderr_lower, "unsupported architecture: ") {
            return Self::UnsupportedArch {
                arch: rest.to_string(),
            };
        }

        // 6. Tarball had no binary (CDN 404 / wrong package).
        if stderr_lower.contains("no binary found in tarball") {
            return Self::NoBinaryInTarball;
        }

        // 7. "curl: command not found" style messages (shell expansions that
        //    bypass our check with `command -v`).
        if stderr_lower.contains("curl: command not found")
            || stderr_lower.contains("wget: command not found")
        {
            return Self::NoHttpClient;
        }

        // 8. curl/wget invocation itself failed (network error, 404, etc.).
        if exit_code.is_some() && (stderr_lower.contains("curl:") || stderr_lower.contains("wget:"))
        {
            let excerpt = truncate(stderr, 256);
            return Self::DownloadFailure {
                exit_code: exit_code.unwrap_or(1),
                stderr_excerpt: excerpt,
            };
        }

        // 9. SSH transport failure (spawn / stdin).
        if stderr_lower.contains("failed to spawn ssh")
            || stderr_lower.contains("failed to write script")
            || stderr_lower.contains("ssh command failed to execute")
        {
            return Self::SshTransportFailure {
                detail: truncate(stderr, 256),
            };
        }

        // 10. Any other non-zero exit.
        if let Some(code) = exit_code {
            return Self::ScriptGenericFailure {
                exit_code: code,
                stderr_excerpt: truncate(stderr, 256),
            };
        }

        // 11. Catch-all.
        Self::Unknown {
            raw: truncate(stderr, 256),
        }
    }

    /// Classify from a [`crate::setup::PreinstallStatus`].
    ///
    /// Called when the preinstall check script positively identified the host
    /// as incompatible with the prebuilt binary. Returns `None` for `Supported`
    /// and `Unknown` (both treated as "pass through to install").
    pub fn from_preinstall_status(status: &PreinstallStatus) -> Option<Self> {
        match status {
            PreinstallStatus::Unsupported {
                reason: UnsupportedReason::GlibcTooOld { .. },
            } => Some(Self::GlibcTooOld),
            PreinstallStatus::Unsupported {
                reason: UnsupportedReason::NonGlibc { name },
            } => Some(Self::NonGlibc { name: name.clone() }),
            PreinstallStatus::Supported | PreinstallStatus::Unknown => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Find the substring after `needle` (case-sensitive).
fn find_after<'a>(haystack: &'a str, needle: &str) -> Option<&'a str> {
    let pos = haystack.find(needle)?;
    Some(&haystack[pos + needle.len()..])
}

/// Find the substring after `needle` in an already-lower-cased haystack.
fn find_after_lower<'a>(lower_haystack: &'a str, lower_needle: &str) -> Option<&'a str> {
    let pos = lower_haystack.find(lower_needle)?;
    Some(&lower_haystack[pos + lower_needle.len()..])
}

/// Truncate `s` to at most `max_chars` Unicode characters, appending `…`
/// when truncation occurs.
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}…", &s[..end])
    }
}

#[cfg(test)]
#[path = "install_error_tests.rs"]
mod tests;
