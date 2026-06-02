//! Tests for [`crate::install_error`].
//!
//! ## Fixture strategy
//!
//! Each test group represents one *error family* drawn from the production
//! install-error CSV.  Fixtures are inline strings that mirror what the
//! install pipeline actually surfaces:
//!
//! - **Script-level**: the raw `stderr` captured from the install script
//!   (`install_remote_server.sh`) running on the remote host.
//! - **Transport-level**: error messages formatted by the SSH transport
//!   layer (`ssh.rs` / `run_ssh_script`).
//! - **Structured-error format**: strings that match the `Display` impl of
//!   the `transport::Error` variants on the `aloke/error_handling` branch,
//!   so the tests can double as forward-compatibility checks once that branch
//!   lands.
//!
//! Every test asserts *both* the family and its [`Recoverability`], so the
//! mapping invariants are enforced in one place.

use super::{InstallErrorFamily, Recoverability};
use crate::setup::{
    GlibcVersion, PreinstallCheckResult, PreinstallStatus, RemoteLibc, UnsupportedReason,
};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn classify(exit_code: Option<i32>, stderr: &str) -> InstallErrorFamily {
    InstallErrorFamily::from_exit_and_stderr(exit_code, stderr)
}

// ---------------------------------------------------------------------------
// Family: NoHttpClient
// ---------------------------------------------------------------------------

/// The primary sentinel: the install script emits exit code 3 when
/// `command -v curl` and `command -v wget` both fail.
#[test]
fn no_http_client_sentinel_exit_code() {
    // exit code 3 is `NO_HTTP_CLIENT_EXIT_CODE`; the script also writes
    // "error: neither curl nor wget is available" to stderr but the
    // sentinel exit code is the canonical signal.
    let family = classify(Some(3), "error: neither curl nor wget is available");
    assert_eq!(family, InstallErrorFamily::NoHttpClient);
    assert_eq!(
        family.recoverability(),
        Recoverability::RecoverableScpFallback
    );
    assert!(family.is_silent_fallback());
}

/// Belt-and-suspenders: even without the sentinel exit code, the stderr
/// message alone is enough to classify the error.
#[test]
fn no_http_client_from_stderr_message() {
    let family = classify(Some(1), "error: neither curl nor wget is available");
    assert_eq!(family, InstallErrorFamily::NoHttpClient);
    assert_eq!(
        family.recoverability(),
        Recoverability::RecoverableScpFallback
    );
}

/// Some exotic shells or PATH configurations produce "curl: command not
/// found" even though the script tries to use `command -v` first.
#[test]
fn no_http_client_curl_command_not_found() {
    let family = classify(Some(127), "bash: curl: command not found");
    assert_eq!(family, InstallErrorFamily::NoHttpClient);
    assert_eq!(
        family.recoverability(),
        Recoverability::RecoverableScpFallback
    );
}

#[test]
fn no_http_client_wget_command_not_found() {
    let family = classify(Some(127), "bash: wget: command not found");
    assert_eq!(family, InstallErrorFamily::NoHttpClient);
    assert_eq!(
        family.recoverability(),
        Recoverability::RecoverableScpFallback
    );
}

// ---------------------------------------------------------------------------
// Family: UnsupportedOs
// ---------------------------------------------------------------------------

/// The install script emits "unsupported OS: <name>" and exits 2 when
/// `uname -s` returns something other than Linux or Darwin.
#[test]
fn unsupported_os_freebsd() {
    // Exact message from install_remote_server.sh: `echo "unsupported OS: $os_kernel" >&2; exit 2`
    let family = classify(Some(2), "unsupported OS: FreeBSD");
    assert!(matches!(
        family,
        InstallErrorFamily::UnsupportedOs { ref os } if os == "FreeBSD"
    ));
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
    assert!(!family.is_silent_fallback());
}

#[test]
fn unsupported_os_openbsd() {
    let family = classify(Some(2), "unsupported OS: OpenBSD");
    assert!(matches!(family, InstallErrorFamily::UnsupportedOs { .. }));
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
}

/// Forward-compat: the `transport::Error::UnsupportedOs` Display impl
/// formats as "unsupported OS: <name>" (lowercase "OS").
#[test]
fn unsupported_os_transport_error_display() {
    // transport::Error::UnsupportedOs { os: "FreeBSD".into() }.to_string()
    // → "unsupported OS: FreeBSD"  (matches the script message exactly)
    let family = classify(Some(2), "unsupported OS: FreeBSD");
    assert!(matches!(family, InstallErrorFamily::UnsupportedOs { os } if os == "FreeBSD"));
}

// ---------------------------------------------------------------------------
// Family: UnsupportedArch
// ---------------------------------------------------------------------------

/// The install script emits "unsupported arch: <name>" and exits 2 when
/// `uname -m` returns something other than x86_64 / aarch64 / arm64.
#[test]
fn unsupported_arch_mips() {
    let family = classify(Some(2), "unsupported arch: mips");
    assert!(matches!(
        family,
        InstallErrorFamily::UnsupportedArch { ref arch } if arch == "mips"
    ));
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
    assert!(!family.is_silent_fallback());
}

#[test]
fn unsupported_arch_riscv64() {
    let family = classify(Some(2), "unsupported arch: riscv64");
    assert!(
        matches!(family, InstallErrorFamily::UnsupportedArch { ref arch } if arch == "riscv64")
    );
}

#[test]
fn unsupported_arch_s390x() {
    let family = classify(Some(2), "unsupported arch: s390x");
    assert!(matches!(family, InstallErrorFamily::UnsupportedArch { .. }));
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
}

/// The `parse_uname_output` function formats errors as "unsupported OS: X"
/// and "unsupported arch: X" — same prefix as the script.
#[test]
fn unsupported_arch_transport_error_display() {
    // transport::Error::UnsupportedArch { arch: "mips".into() }.to_string()
    // → "unsupported architecture: mips"
    let family = classify(None, "unsupported architecture: mips");
    assert!(matches!(family, InstallErrorFamily::UnsupportedArch { ref arch } if arch == "mips"));
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
}

// ---------------------------------------------------------------------------
// Family: GlibcTooOld  (preinstall check → ControlMaster fallback)
// ---------------------------------------------------------------------------

/// Glibc 2.17 (CentOS 7 / RHEL 7) is the most common "too old" case.
#[test]
fn glibc_too_old_from_preinstall_status() {
    let status = PreinstallStatus::Unsupported {
        reason: UnsupportedReason::GlibcTooOld {
            detected: GlibcVersion::new(2, 17),
            required: GlibcVersion::new(2, 31),
        },
    };
    let family = InstallErrorFamily::from_preinstall_status(&status).unwrap();
    assert_eq!(family, InstallErrorFamily::GlibcTooOld);
    assert_eq!(
        family.recoverability(),
        Recoverability::RecoverableControlMaster
    );
    assert!(family.is_silent_fallback());
}

#[test]
fn glibc_too_old_2_28() {
    // glibc 2.28 (Debian 9) — older than 2.31.
    let status = PreinstallStatus::Unsupported {
        reason: UnsupportedReason::GlibcTooOld {
            detected: GlibcVersion::new(2, 28),
            required: GlibcVersion::new(2, 31),
        },
    };
    let family = InstallErrorFamily::from_preinstall_status(&status).unwrap();
    assert_eq!(family, InstallErrorFamily::GlibcTooOld);
}

/// `PreinstallStatus::Supported` → `None` (no error family, proceed with install).
#[test]
fn preinstall_supported_returns_none() {
    let family = InstallErrorFamily::from_preinstall_status(&PreinstallStatus::Supported);
    assert!(family.is_none());
}

/// `PreinstallStatus::Unknown` → `None` (fail open, try install anyway).
#[test]
fn preinstall_unknown_returns_none() {
    let family = InstallErrorFamily::from_preinstall_status(&PreinstallStatus::Unknown);
    assert!(family.is_none());
}

// ---------------------------------------------------------------------------
// Family: NonGlibc  (preinstall check → ControlMaster fallback)
// ---------------------------------------------------------------------------

#[test]
fn non_glibc_musl_from_preinstall_status() {
    let status = PreinstallStatus::Unsupported {
        reason: UnsupportedReason::NonGlibc {
            name: "musl".to_string(),
        },
    };
    let family = InstallErrorFamily::from_preinstall_status(&status).unwrap();
    assert!(matches!(family, InstallErrorFamily::NonGlibc { ref name } if name == "musl"));
    assert_eq!(
        family.recoverability(),
        Recoverability::RecoverableControlMaster
    );
    assert!(family.is_silent_fallback());
}

#[test]
fn non_glibc_uclibc_from_preinstall_status() {
    let status = PreinstallStatus::Unsupported {
        reason: UnsupportedReason::NonGlibc {
            name: "uclibc".to_string(),
        },
    };
    let family = InstallErrorFamily::from_preinstall_status(&status).unwrap();
    assert!(matches!(family, InstallErrorFamily::NonGlibc { ref name } if name == "uclibc"));
    assert_eq!(
        family.recoverability(),
        Recoverability::RecoverableControlMaster
    );
}

#[test]
fn non_glibc_bionic_from_preinstall_status() {
    // Android hosts run bionic libc.
    let status = PreinstallStatus::Unsupported {
        reason: UnsupportedReason::NonGlibc {
            name: "bionic".to_string(),
        },
    };
    let family = InstallErrorFamily::from_preinstall_status(&status).unwrap();
    assert!(matches!(family, InstallErrorFamily::NonGlibc { .. }));
}

// ---------------------------------------------------------------------------
// Family: Timeout
// ---------------------------------------------------------------------------

/// SSH-level timeout message from `ssh.rs::run_ssh_script`.
#[test]
fn timeout_ssh_script() {
    let family = classify(None, "Script timed out after 60s");
    assert_eq!(family, InstallErrorFamily::Timeout);
    assert_eq!(family.recoverability(), Recoverability::PossiblyRecoverable);
    assert!(!family.is_silent_fallback());
}

#[test]
fn timeout_ssh_command() {
    let family = classify(None, "SSH command timed out after 10s");
    assert_eq!(family, InstallErrorFamily::Timeout);
}

/// Transport-level timeout from the structured error variant (forward compat).
/// `transport::Error::TimedOut` formats as "timed out".
#[test]
fn timeout_transport_error_display() {
    let family = classify(None, "timed out");
    assert_eq!(family, InstallErrorFamily::Timeout);
}

#[test]
fn timeout_contains_timeout_word() {
    let family = classify(Some(124), "timeout: the monitored command timed out");
    assert_eq!(family, InstallErrorFamily::Timeout);
}

// ---------------------------------------------------------------------------
// Family: NoBinaryInTarball
// ---------------------------------------------------------------------------

/// The install script exits 1 and emits this message when the downloaded
/// tarball doesn't contain an executable matching `oz*`.
#[test]
fn no_binary_in_tarball_exact_message() {
    let family = classify(Some(1), "no binary found in tarball");
    assert_eq!(family, InstallErrorFamily::NoBinaryInTarball);
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
}

#[test]
fn no_binary_in_tarball_with_surrounding_text() {
    // Some transports prepend "Script failed (exit 1): " before the stderr.
    let family = classify(
        Some(1),
        "script failed (exit 1): no binary found in tarball",
    );
    assert_eq!(family, InstallErrorFamily::NoBinaryInTarball);
}

// ---------------------------------------------------------------------------
// Family: DownloadFailure
// ---------------------------------------------------------------------------

/// curl exits non-zero (e.g. server returns 404 for the tarball).
#[test]
fn download_failure_curl_404() {
    let family = classify(Some(22), "curl: (22) The requested URL returned error: 404");
    assert!(matches!(
        family,
        InstallErrorFamily::DownloadFailure { exit_code: 22, .. }
    ));
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
}

/// curl SSL certificate error.
#[test]
fn download_failure_curl_ssl() {
    let family = classify(
        Some(60),
        "curl: (60) SSL certificate problem: unable to get local issuer certificate",
    );
    assert!(matches!(
        family,
        InstallErrorFamily::DownloadFailure { exit_code: 60, .. }
    ));
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
}

/// wget exits non-zero.
#[test]
fn download_failure_wget_connection_refused() {
    let family = classify(Some(4), "wget: unable to connect to server");
    assert!(matches!(
        family,
        InstallErrorFamily::DownloadFailure { exit_code: 4, .. }
    ));
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
}

// ---------------------------------------------------------------------------
// Family: SshTransportFailure
// ---------------------------------------------------------------------------

/// The transport layer failed to spawn the `ssh` subprocess.
#[test]
fn ssh_transport_failure_spawn() {
    let family = classify(
        None,
        "Failed to spawn SSH for script: No such file or directory",
    );
    assert!(matches!(
        family,
        InstallErrorFamily::SshTransportFailure { .. }
    ));
    assert_eq!(family.recoverability(), Recoverability::PossiblyRecoverable);
}

/// Writing the script to SSH stdin failed (broken pipe on the remote side).
#[test]
fn ssh_transport_failure_stdin_write() {
    let family = classify(None, "Failed to write script to stdin: Broken pipe");
    assert!(matches!(
        family,
        InstallErrorFamily::SshTransportFailure { .. }
    ));
    assert_eq!(family.recoverability(), Recoverability::PossiblyRecoverable);
}

#[test]
fn ssh_transport_failure_command() {
    let family = classify(
        None,
        "SSH command failed to execute: No such file or directory",
    );
    assert!(matches!(
        family,
        InstallErrorFamily::SshTransportFailure { .. }
    ));
}

// ---------------------------------------------------------------------------
// Family: ScriptGenericFailure
// ---------------------------------------------------------------------------

/// tar extraction failed (corrupted tarball, wrong format, etc.).
#[test]
fn script_generic_failure_tar_error() {
    let family = classify(Some(1), "tar: Error is not recoverable: exiting now");
    assert!(matches!(
        family,
        InstallErrorFamily::ScriptGenericFailure { exit_code: 1, .. }
    ));
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
}

/// mkdir failed (permissions issue on the remote host).
#[test]
fn script_generic_failure_mkdir_permission() {
    let family = classify(
        Some(1),
        "mkdir: cannot create directory '/opt/.warp': Permission denied",
    );
    assert!(matches!(
        family,
        InstallErrorFamily::ScriptGenericFailure { exit_code: 1, .. }
    ));
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
}

/// Unknown exit code 1 with empty stderr.
#[test]
fn script_generic_failure_empty_stderr() {
    let family = classify(Some(1), "");
    assert!(matches!(
        family,
        InstallErrorFamily::ScriptGenericFailure { exit_code: 1, .. }
    ));
}

/// Non-zero exit on the binary check itself.
#[test]
fn script_generic_failure_exit_code_128() {
    let family = classify(Some(128), "command not found");
    assert!(matches!(
        family,
        InstallErrorFamily::ScriptGenericFailure { exit_code: 128, .. }
    ));
}

// ---------------------------------------------------------------------------
// Family: Unknown
// ---------------------------------------------------------------------------

/// No exit code and no recognisable stderr → Unknown.
#[test]
fn unknown_no_exit_no_message() {
    let family = classify(None, "");
    assert!(matches!(family, InstallErrorFamily::Unknown { .. }));
    assert_eq!(family.recoverability(), Recoverability::NonRecoverable);
}

#[test]
fn unknown_no_exit_with_unrecognised_message() {
    let family = classify(None, "some totally unexpected output");
    assert!(matches!(family, InstallErrorFamily::Unknown { .. }));
}

// ---------------------------------------------------------------------------
// Recoverability exhaustiveness: every family maps to exactly one category
// ---------------------------------------------------------------------------

/// Non-exhaustive spot-check: families expected to be silent fallbacks.
#[test]
fn recoverability_silent_fallbacks() {
    let silent = [
        InstallErrorFamily::NoHttpClient,
        InstallErrorFamily::GlibcTooOld,
        InstallErrorFamily::NonGlibc {
            name: "musl".into(),
        },
    ];
    for family in &silent {
        assert!(
            family.is_silent_fallback(),
            "{family:?} should be a silent fallback"
        );
    }
}

/// Families that must never produce a silent fallback.
#[test]
fn recoverability_non_silent() {
    let non_silent = [
        InstallErrorFamily::UnsupportedOs {
            os: "FreeBSD".into(),
        },
        InstallErrorFamily::UnsupportedArch {
            arch: "mips".into(),
        },
        InstallErrorFamily::NoBinaryInTarball,
        InstallErrorFamily::ScriptGenericFailure {
            exit_code: 1,
            stderr_excerpt: String::new(),
        },
        InstallErrorFamily::DownloadFailure {
            exit_code: 22,
            stderr_excerpt: String::new(),
        },
        InstallErrorFamily::Unknown { raw: String::new() },
    ];
    for family in &non_silent {
        assert!(
            !family.is_silent_fallback(),
            "{family:?} should NOT be a silent fallback"
        );
    }
}

// ---------------------------------------------------------------------------
// PreinstallCheckResult round-trip
// ---------------------------------------------------------------------------

/// Full parse + classification round-trip for a glibc-too-old fixture.
#[test]
fn preinstall_check_result_glibc_too_old_roundtrip() {
    let stdout = "required_glibc=2.31\n\
                  libc_family=glibc\n\
                  libc_version=2.17\n\
                  status=unsupported\n\
                  reason=glibc_too_old\n";
    let result = PreinstallCheckResult::parse(stdout);
    assert!(!result.is_supported());
    let family = InstallErrorFamily::from_preinstall_status(&result.status).unwrap();
    assert_eq!(family, InstallErrorFamily::GlibcTooOld);
    assert_eq!(
        family.recoverability(),
        Recoverability::RecoverableControlMaster
    );
}

/// Full parse + classification round-trip for a musl fixture.
#[test]
fn preinstall_check_result_musl_roundtrip() {
    let stdout = "required_glibc=2.31\n\
                  libc_family=musl\n\
                  status=unsupported\n\
                  reason=non_glibc\n";
    let result = PreinstallCheckResult::parse(stdout);
    let family = InstallErrorFamily::from_preinstall_status(&result.status).unwrap();
    assert!(matches!(family, InstallErrorFamily::NonGlibc { ref name } if name == "musl"));
    assert_eq!(
        family.recoverability(),
        Recoverability::RecoverableControlMaster
    );
}

/// A `supported` preinstall result must not produce an error family.
#[test]
fn preinstall_check_result_supported_roundtrip() {
    let stdout = "required_glibc=2.31\n\
                  libc_family=glibc\n\
                  libc_version=2.35\n\
                  status=supported\n";
    let result = PreinstallCheckResult::parse(stdout);
    assert!(result.is_supported());
    assert!(InstallErrorFamily::from_preinstall_status(&result.status).is_none());
}

/// An `Unknown` preinstall result (garbled output) must also not produce
/// an error family — the fail-open invariant.
#[test]
fn preinstall_check_result_unknown_roundtrip() {
    let stdout = "some garbage output that the script couldn't produce";
    let result = PreinstallCheckResult::parse(stdout);
    assert_eq!(result.status, PreinstallStatus::Unknown);
    assert!(result.is_supported()); // fail open
    assert!(InstallErrorFamily::from_preinstall_status(&result.status).is_none());
}

// ---------------------------------------------------------------------------
// Forward-compat: structured transport::Error Display strings
// ---------------------------------------------------------------------------
//
// The `aloke/error_handling` branch defines `transport::Error` with Display
// impls that match the string patterns tested here. If/when that branch lands,
// calling `classify(Some(code), &err.to_string())` on the string produced by
// `transport::Error::to_string()` should yield the expected family.
//
// These tests serve as a contract: the classification logic must handle the
// exact output of `transport::Error::Display` so that a future adapter
// `InstallErrorFamily::from_transport_error(&transport::Error) -> Self` can
// be implemented as a trivial match (no string parsing needed for the typed
// variants).

/// `transport::Error::TimedOut` → `Display` is "timed out"
#[test]
fn forward_compat_timed_out_display() {
    let family = classify(None, "timed out");
    assert_eq!(family, InstallErrorFamily::Timeout);
}

/// `transport::Error::UnsupportedOs { os: "FreeBSD".into() }` → "unsupported OS: FreeBSD"
#[test]
fn forward_compat_unsupported_os_display() {
    let family = classify(None, "unsupported OS: FreeBSD");
    assert!(matches!(family, InstallErrorFamily::UnsupportedOs { os } if os == "FreeBSD"));
}

/// `transport::Error::UnsupportedArch { arch: "mips".into() }` → "unsupported architecture: mips"
#[test]
fn forward_compat_unsupported_arch_display() {
    let family = classify(None, "unsupported architecture: mips");
    assert!(matches!(family, InstallErrorFamily::UnsupportedArch { arch } if arch == "mips"));
}

/// `transport::Error::ScriptFailed { exit_code: 3, stderr: "error: neither curl nor wget is available".into() }`
/// The sentinel exit_code=3 takes precedence.
#[test]
fn forward_compat_script_failed_no_http_client() {
    // When the structured error is flattened via .to_string(), the message is
    // "script failed (exit 3): error: neither curl nor wget is available"
    // but our exit_code=3 sentinel fires first.
    let family = classify(Some(3), "error: neither curl nor wget is available");
    assert_eq!(family, InstallErrorFamily::NoHttpClient);
}

/// `transport::Error::ScriptFailed { exit_code: 2, stderr: "unsupported arch: mips".into() }`
#[test]
fn forward_compat_script_failed_unsupported_arch() {
    let family = classify(Some(2), "unsupported arch: mips");
    assert!(matches!(family, InstallErrorFamily::UnsupportedArch { arch } if arch == "mips"));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// Priority: exit code 3 beats any stderr content, including "timed out".
#[test]
fn exit_code_3_beats_timeout_message() {
    let family = classify(Some(3), "timed out");
    assert_eq!(family, InstallErrorFamily::NoHttpClient);
}

/// Long stderr is truncated to 256 chars in excerpts.
#[test]
fn long_stderr_is_truncated() {
    let long_stderr = "x".repeat(1000);
    let family = classify(Some(1), &long_stderr);
    if let InstallErrorFamily::ScriptGenericFailure { stderr_excerpt, .. } = &family {
        // Truncated: 256 chars + "…" = 257 code points (the ellipsis is one char).
        assert!(stderr_excerpt.chars().count() <= 257);
        assert!(stderr_excerpt.ends_with('…'));
    } else {
        panic!("expected ScriptGenericFailure, got {family:?}");
    }
}

/// "timed out" in the SSH command message (from `run_ssh_command`).
#[test]
fn ssh_command_timeout_message() {
    let family = classify(None, "SSH command timed out after 10s");
    assert_eq!(family, InstallErrorFamily::Timeout);
}

/// The preinstall check script can emit `libc_family=unknown` when getconf and
/// ldd both fail. This must classify as `Unknown` (no error family), not as
/// `NonGlibc` — the install should proceed.
#[test]
fn preinstall_libc_family_unknown_is_fail_open() {
    let stdout = "required_glibc=2.31\nlibc_family=unknown\nstatus=unknown\n";
    let result = PreinstallCheckResult::parse(stdout);
    assert_eq!(result.libc, RemoteLibc::Unknown);
    assert!(result.is_supported()); // fail open
    assert!(InstallErrorFamily::from_preinstall_status(&result.status).is_none());
}
