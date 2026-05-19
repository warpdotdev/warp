use super::*;

// Helper: classify via the timeout-aware overload.
fn classify(stderr: &str, exit_code: Option<i32>) -> InstallFailureCategory {
    classify_install_failure(stderr, exit_code)
}

fn classify_timeout(stderr: &str, exit_code: Option<i32>) -> InstallFailureCategory {
    classify_install_failure_with_timeout(stderr, exit_code, true)
}

// ═══════════════════════════════════════════════════════════════════════
// § 1  Classifier coverage for every CSV failure family
// ═══════════════════════════════════════════════════════════════════════

// ── Timeout ─────────────────────────────────────────────────────────

#[test]
fn classify_timeout_from_flag() {
    assert_eq!(classify_timeout("", None), InstallFailureCategory::Timeout);
}

#[test]
fn classify_timeout_overrides_stderr() {
    assert_eq!(
        classify_timeout("Permission denied", Some(1)),
        InstallFailureCategory::Timeout
    );
}

// ── MissingHttpClient (no curl, no wget — exit 3) ───────────────────

#[test]
fn classify_missing_http_client_exit_code() {
    assert_eq!(
        classify("error: neither curl nor wget is available", Some(3)),
        InstallFailureCategory::MissingHttpClient
    );
}

#[test]
fn classify_missing_http_client_exit_code_only() {
    assert_eq!(
        classify("", Some(3)),
        InstallFailureCategory::MissingHttpClient
    );
}

#[test]
fn classify_missing_http_client_when_both_absent() {
    assert_eq!(
        classify("error: neither curl nor wget is available\n", Some(3)),
        InstallFailureCategory::MissingHttpClient
    );
}

// ── MissingTar ──────────────────────────────────────────────────────

#[test]
fn classify_missing_tar_not_found() {
    assert_eq!(
        classify("tar: not found", Some(127)),
        InstallFailureCategory::MissingTar
    );
}

#[test]
fn classify_missing_tar_command_not_found() {
    assert_eq!(
        classify("bash: tar: command not found", Some(127)),
        InstallFailureCategory::MissingTar
    );
}

#[test]
fn classify_missing_tar_no_such_file() {
    assert_eq!(
        classify("/usr/bin/tar: No such file or directory", Some(127)),
        InstallFailureCategory::MissingTar
    );
}

// ── MissingBash ─────────────────────────────────────────────────────

#[test]
fn classify_missing_bash_not_found() {
    assert_eq!(
        classify("bash: not found", Some(127)),
        InstallFailureCategory::MissingBash
    );
}

#[test]
fn classify_missing_bash_command_not_found() {
    assert_eq!(
        classify("bash: command not found", Some(127)),
        InstallFailureCategory::MissingBash
    );
}

#[test]
fn classify_missing_bash_no_such_file() {
    assert_eq!(
        classify("No such file or directory: bash", Some(127)),
        InstallFailureCategory::MissingBash
    );
}

// ── UnsupportedArchitecture ─────────────────────────────────────────

#[test]
fn classify_unsupported_arch_mips() {
    assert_eq!(
        classify("unsupported arch: mips\n", Some(2)),
        InstallFailureCategory::UnsupportedArchitecture
    );
}

#[test]
fn classify_unsupported_arch_ppc64le() {
    assert_eq!(
        classify("unsupported arch: ppc64le\n", Some(2)),
        InstallFailureCategory::UnsupportedArchitecture
    );
}

#[test]
fn classify_unsupported_arch_exit2_no_message() {
    assert_eq!(
        classify("", Some(2)),
        InstallFailureCategory::UnsupportedArchitecture
    );
}

// ── UnsupportedOs ───────────────────────────────────────────────────

#[test]
fn classify_unsupported_os_freebsd() {
    assert_eq!(
        classify("unsupported OS: FreeBSD\n", Some(2)),
        InstallFailureCategory::UnsupportedOs
    );
}

// ── DnsFailure ──────────────────────────────────────────────────────

#[test]
fn classify_dns_could_not_resolve() {
    assert_eq!(
        classify("curl: (6) Could not resolve host: app.warp.dev", Some(6)),
        InstallFailureCategory::DnsFailure
    );
}

#[test]
fn classify_dns_name_or_service() {
    assert_eq!(
        classify(
            "wget: unable to resolve host address 'app.warp.dev': Name or service not known",
            Some(6)
        ),
        InstallFailureCategory::DnsFailure
    );
}

#[test]
fn classify_dns_temporary_failure() {
    assert_eq!(
        classify("Temporary failure in name resolution", Some(6)),
        InstallFailureCategory::DnsFailure
    );
}

// ── ConnectionRefused ───────────────────────────────────────────────

#[test]
fn classify_connection_refused() {
    assert_eq!(
        classify(
            "curl: (7) Failed to connect to app.warp.dev port 443: Connection refused",
            Some(7)
        ),
        InstallFailureCategory::ConnectionRefused
    );
}

// ── ConnectionUnreachable ───────────────────────────────────────────

#[test]
fn classify_no_route_to_host() {
    assert_eq!(
        classify("curl: (7) Failed to connect: No route to host", Some(7)),
        InstallFailureCategory::ConnectionUnreachable
    );
}

#[test]
fn classify_network_unreachable() {
    assert_eq!(
        classify("curl: (7) Network is unreachable", Some(7)),
        InstallFailureCategory::ConnectionUnreachable
    );
}

// ── TlsCaFailure ────────────────────────────────────────────────────

#[test]
fn classify_tls_ssl_connect_error() {
    assert_eq!(
        classify("curl: (35) SSL connect error", Some(35)),
        InstallFailureCategory::TlsCaFailure
    );
}

#[test]
fn classify_tls_certificate_verify() {
    assert_eq!(
        classify(
            "curl: (60) SSL certificate problem: unable to get local issuer certificate",
            Some(60)
        ),
        InstallFailureCategory::TlsCaFailure
    );
}

#[test]
fn classify_tls_ca_bundle() {
    assert_eq!(
        classify(
            "curl: (77) error setting certificate verify locations: CA-bundle",
            Some(77)
        ),
        InstallFailureCategory::TlsCaFailure
    );
}

// ── HttpForbidden ───────────────────────────────────────────────────

#[test]
fn classify_http_403_forbidden_stderr() {
    assert_eq!(
        classify("The requested URL returned error: 403 Forbidden", Some(22)),
        InstallFailureCategory::HttpForbidden
    );
}

#[test]
fn classify_http_403_curl_exit_22() {
    assert_eq!(
        classify("403", Some(22)),
        InstallFailureCategory::HttpForbidden
    );
}

// ── HttpBadGateway ──────────────────────────────────────────────────

#[test]
fn classify_http_502_bad_gateway() {
    assert_eq!(
        classify("502 Bad Gateway", Some(22)),
        InstallFailureCategory::HttpBadGateway
    );
}

// ── HttpError ───────────────────────────────────────────────────────

#[test]
fn classify_http_503_service_unavailable() {
    assert_eq!(
        classify("503 Service Unavailable", Some(1)),
        InstallFailureCategory::HttpError
    );
}

#[test]
fn classify_curl_exit_22_generic() {
    assert_eq!(
        classify("The requested URL returned error: 500", Some(22)),
        InstallFailureCategory::HttpError
    );
}

// ── PartialDownload ─────────────────────────────────────────────────

#[test]
fn classify_partial_download_curl_exit_18() {
    assert_eq!(
        classify(
            "curl: (18) transfer closed with 12345 bytes remaining",
            Some(18)
        ),
        InstallFailureCategory::PartialDownload
    );
}

#[test]
fn classify_partial_download_stderr_partial_file() {
    assert_eq!(
        classify("Partial file received", Some(1)),
        InstallFailureCategory::PartialDownload
    );
}

#[test]
fn classify_partial_download_unexpected_end_gz() {
    let stderr =
        "gzip: stdin: unexpected end of file\ntar: Child returned status 1\ntar: Error: oz.tar.gz";
    assert_eq!(
        classify(stderr, Some(1)),
        InstallFailureCategory::PartialDownload
    );
}

// ── DownloadWriteFailure ────────────────────────────────────────────

#[test]
fn classify_download_write_failure_curl_exit_23() {
    assert_eq!(
        classify("Failed writing body (0 != 1234)", Some(23)),
        InstallFailureCategory::DownloadWriteFailure
    );
}

#[test]
fn classify_download_write_failure_failed_writing_body() {
    assert_eq!(
        classify("curl: Failed writing body", Some(1)),
        InstallFailureCategory::DownloadWriteFailure
    );
}

// ── InstallDirPermissionDenied ──────────────────────────────────────

#[test]
fn classify_install_dir_permission_denied() {
    assert_eq!(
        classify(
            "mkdir: cannot create directory '/root/.warp/remote-server': Permission denied",
            Some(1)
        ),
        InstallFailureCategory::InstallDirPermissionDenied
    );
}

// ── NoSpaceLeft ─────────────────────────────────────────────────────

#[test]
fn classify_no_space_left() {
    assert_eq!(
        classify("write error: No space left on device", Some(1)),
        InstallFailureCategory::NoSpaceLeft
    );
}

#[test]
fn classify_disk_quota_exceeded() {
    assert_eq!(
        classify("Disk quota exceeded", Some(1)),
        InstallFailureCategory::NoSpaceLeft
    );
}

// ── ReadOnlyFilesystem ──────────────────────────────────────────────

#[test]
fn classify_read_only_filesystem() {
    assert_eq!(
        classify(
            "mkdir: cannot create directory: Read-only file system",
            Some(1)
        ),
        InstallFailureCategory::ReadOnlyFilesystem
    );
}

#[test]
fn classify_erofs() {
    assert_eq!(
        classify(
            "mv: cannot move file: EROFS: read-only file system",
            Some(1)
        ),
        InstallFailureCategory::ReadOnlyFilesystem
    );
}

// ── TarPermissionFailure ────────────────────────────────────────────

#[test]
fn classify_tar_cannot_change_ownership() {
    assert_eq!(
        classify(
            "tar: oz: Cannot change ownership to uid 1000, gid 1000: Operation not permitted",
            Some(1)
        ),
        InstallFailureCategory::TarPermissionFailure
    );
}

#[test]
fn classify_tar_cannot_open_non_sentinel_exit() {
    assert_eq!(
        classify(
            "tar: oz.tar.gz: Cannot open: Permission denied\ntar: Error is not recoverable",
            Some(1)
        ),
        InstallFailureCategory::TarPermissionFailure
    );
}

// ── TarExtractionFailure ────────────────────────────────────────────

#[test]
fn classify_tar_not_gzip() {
    assert_eq!(
        classify(
            "tar: This does not look like a tar archive\ngzip: stdin: not in gzip format",
            Some(1)
        ),
        InstallFailureCategory::TarExtractionFailure
    );
}

#[test]
fn classify_tar_corrupted() {
    assert_eq!(
        classify("tar: Archive is corrupted", Some(1)),
        InstallFailureCategory::TarExtractionFailure
    );
}

// ── ExpiredPassword ─────────────────────────────────────────────────

#[test]
fn classify_expired_password() {
    assert_eq!(
        classify(
            "WARNING: Your password has expired.\nPassword change required but no TTY available.",
            Some(1)
        ),
        InstallFailureCategory::ExpiredPassword
    );
}

#[test]
fn classify_no_tty_present() {
    assert_eq!(
        classify(
            "sudo: no tty present and no askpass program specified",
            Some(1)
        ),
        InstallFailureCategory::ExpiredPassword
    );
}

// ── StartupFilePermissionDenied ─────────────────────────────────────

#[test]
fn classify_startup_file_bashrc_permission_denied() {
    assert_eq!(
        classify("bash: /home/user/.bashrc: Permission denied", Some(1)),
        InstallFailureCategory::StartupFilePermissionDenied
    );
}

#[test]
fn classify_startup_file_zshrc_permission_denied() {
    assert_eq!(
        classify("zsh: permission denied: /home/user/.zshrc", Some(1)),
        InstallFailureCategory::StartupFilePermissionDenied
    );
}

// ── SshDisconnect ───────────────────────────────────────────────────

#[test]
fn classify_ssh_disconnect_exit_255() {
    assert_eq!(
        classify(
            "ssh: connect to host example.com: Connection reset by peer",
            Some(255)
        ),
        InstallFailureCategory::SshDisconnect
    );
}

#[test]
fn classify_ssh_disconnect_exit_255_no_stderr() {
    assert_eq!(
        classify("", Some(255)),
        InstallFailureCategory::SshDisconnect
    );
}

// ── ScriptError ─────────────────────────────────────────────────────

#[test]
fn classify_script_error_unrecognized_nonzero() {
    assert_eq!(
        classify("some totally unexpected error output", Some(42)),
        InstallFailureCategory::ScriptError
    );
}

#[test]
fn classify_script_error_empty_stderr_nonzero() {
    assert_eq!(classify("", Some(1)), InstallFailureCategory::ScriptError);
}

// ── Unknown ─────────────────────────────────────────────────────────

#[test]
fn classify_unknown_for_exit_zero() {
    // exit 0 should not be classified as a failure; if the caller asks
    // anyway the fallback is Unknown.
    assert_eq!(classify("", Some(0)), InstallFailureCategory::Unknown);
}

#[test]
fn classify_unknown_for_signal_kill() {
    assert_eq!(classify("", None), InstallFailureCategory::Unknown);
}

// ═══════════════════════════════════════════════════════════════════════
// § 2  is_retriable – no blind retry for permanent conditions
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn retriable_categories() {
    let retriable = [
        InstallFailureCategory::Timeout,
        InstallFailureCategory::DnsFailure,
        InstallFailureCategory::ConnectionRefused,
        InstallFailureCategory::ConnectionUnreachable,
        InstallFailureCategory::TlsCaFailure,
        InstallFailureCategory::HttpBadGateway,
        InstallFailureCategory::HttpError,
        InstallFailureCategory::PartialDownload,
    ];
    for cat in &retriable {
        assert!(cat.is_retriable(), "{cat:?} should be retriable");
    }
}

#[test]
fn non_retriable_permanent_conditions() {
    let non_retriable = [
        InstallFailureCategory::MissingCurl,
        InstallFailureCategory::MissingWget,
        InstallFailureCategory::MissingHttpClient,
        InstallFailureCategory::MissingTar,
        InstallFailureCategory::MissingBash,
        InstallFailureCategory::UnsupportedArchitecture,
        InstallFailureCategory::UnsupportedOs,
        InstallFailureCategory::HttpForbidden,
        InstallFailureCategory::DownloadWriteFailure,
        InstallFailureCategory::InstallDirPermissionDenied,
        InstallFailureCategory::NoSpaceLeft,
        InstallFailureCategory::ReadOnlyFilesystem,
        InstallFailureCategory::TarExtractionFailure,
        InstallFailureCategory::TarPermissionFailure,
        InstallFailureCategory::ExpiredPassword,
        InstallFailureCategory::StartupFilePermissionDenied,
        InstallFailureCategory::SshDisconnect,
        InstallFailureCategory::ScriptError,
        InstallFailureCategory::Unknown,
    ];
    for cat in &non_retriable {
        assert!(!cat.is_retriable(), "{cat:?} should NOT be retriable");
    }
}

// ═══════════════════════════════════════════════════════════════════════
// § 3  as_str / title / description stability
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn as_str_tags_are_unique_and_non_empty() {
    let all = all_categories();
    let mut seen = std::collections::HashSet::new();
    for cat in &all {
        let tag = cat.as_str();
        assert!(!tag.is_empty(), "{cat:?} has empty as_str tag");
        assert!(seen.insert(tag), "duplicate as_str tag {tag:?} for {cat:?}");
    }
}

#[test]
fn title_is_non_empty() {
    for cat in &all_categories() {
        assert!(!cat.title().is_empty(), "{cat:?} has empty title");
    }
}

#[test]
fn description_is_non_empty() {
    for cat in &all_categories() {
        assert!(
            !cat.description().is_empty(),
            "{cat:?} has empty description"
        );
    }
}

#[test]
fn display_matches_description() {
    for cat in &all_categories() {
        assert_eq!(format!("{cat}"), cat.description());
    }
}

fn all_categories() -> Vec<InstallFailureCategory> {
    vec![
        InstallFailureCategory::Timeout,
        InstallFailureCategory::MissingCurl,
        InstallFailureCategory::MissingWget,
        InstallFailureCategory::MissingHttpClient,
        InstallFailureCategory::MissingTar,
        InstallFailureCategory::MissingBash,
        InstallFailureCategory::UnsupportedArchitecture,
        InstallFailureCategory::UnsupportedOs,
        InstallFailureCategory::DnsFailure,
        InstallFailureCategory::ConnectionRefused,
        InstallFailureCategory::ConnectionUnreachable,
        InstallFailureCategory::TlsCaFailure,
        InstallFailureCategory::HttpForbidden,
        InstallFailureCategory::HttpBadGateway,
        InstallFailureCategory::HttpError,
        InstallFailureCategory::PartialDownload,
        InstallFailureCategory::DownloadWriteFailure,
        InstallFailureCategory::InstallDirPermissionDenied,
        InstallFailureCategory::NoSpaceLeft,
        InstallFailureCategory::ReadOnlyFilesystem,
        InstallFailureCategory::TarExtractionFailure,
        InstallFailureCategory::TarPermissionFailure,
        InstallFailureCategory::ExpiredPassword,
        InstallFailureCategory::StartupFilePermissionDenied,
        InstallFailureCategory::SshDisconnect,
        InstallFailureCategory::ScriptError,
        InstallFailureCategory::Unknown,
    ]
}

// ═══════════════════════════════════════════════════════════════════════
// § 4  Priority ordering
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn timeout_takes_priority_over_exit_code() {
    assert_eq!(
        classify_install_failure_with_timeout("connection reset", Some(255), true),
        InstallFailureCategory::Timeout
    );
}

#[test]
fn exit_255_takes_priority_over_stderr() {
    assert_eq!(
        classify("Permission denied", Some(255)),
        InstallFailureCategory::SshDisconnect
    );
}

#[test]
fn exit_3_takes_priority_over_stderr() {
    assert_eq!(
        classify("Permission denied", Some(3)),
        InstallFailureCategory::MissingHttpClient
    );
}

// ═══════════════════════════════════════════════════════════════════════
// § 5  Script structural probes
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn install_script_tries_curl_before_wget() {
    let template = super::super::INSTALL_SCRIPT_TEMPLATE;
    let curl_pos = template
        .find("command -v curl")
        .expect("script must check for curl");
    let wget_pos = template
        .find("command -v wget")
        .expect("script must check for wget");
    assert!(curl_pos < wget_pos);
}

#[test]
fn install_script_no_http_client_sentinel_after_checks() {
    let template = super::super::INSTALL_SCRIPT_TEMPLATE;
    let wget_pos = template.find("command -v wget").expect("must check wget");
    let sentinel_pos = template
        .find("exit {no_http_client_exit_code}")
        .expect("must have sentinel");
    assert!(sentinel_pos > wget_pos);
}

#[test]
fn install_script_arch_case_covers_known_architectures() {
    let template = super::super::INSTALL_SCRIPT_TEMPLATE;
    assert!(template.contains("x86_64)"));
    assert!(template.contains("aarch64|arm64)"));
    assert!(template.contains("*) echo \"unsupported arch:"));
}

#[test]
fn install_script_os_case_covers_known_os() {
    let template = super::super::INSTALL_SCRIPT_TEMPLATE;
    assert!(template.contains("Darwin) os_name=macos"));
    assert!(
        template.contains("Linux)  os_name=linux") || template.contains("Linux) os_name=linux")
    );
}

#[test]
fn install_script_has_set_e() {
    assert!(super::super::INSTALL_SCRIPT_TEMPLATE.contains("set -e"));
}

#[test]
fn install_script_staging_tarball_skips_download() {
    assert!(super::super::INSTALL_SCRIPT_TEMPLATE.contains("if [ -n \"$staging_tarball_path\" ]"));
}

// ═══════════════════════════════════════════════════════════════════════
// § 6  SCP fallback trigger alignment
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn scp_fallback_triggered_only_by_missing_http_client() {
    assert_eq!(super::super::NO_HTTP_CLIENT_EXIT_CODE, 3);
    assert_eq!(
        classify("", Some(3)),
        InstallFailureCategory::MissingHttpClient
    );

    for code in [0, 1, 2, 4, 18, 22, 23, 42, 127, 255] {
        assert_ne!(
            classify("", Some(code)),
            InstallFailureCategory::MissingHttpClient,
            "exit code {code} should not classify as MissingHttpClient"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// § 7  Architecture mapping consistency
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn rust_arch_mapping_consistent_with_script() {
    use super::super::{parse_uname_output, RemoteArch};

    let cases = [
        ("Linux x86_64", RemoteArch::X86_64),
        ("Linux aarch64", RemoteArch::Aarch64),
        ("Darwin arm64", RemoteArch::Aarch64),
        ("Linux armv8l", RemoteArch::Aarch64),
        ("Darwin x86_64", RemoteArch::X86_64),
    ];

    for (input, expected_arch) in &cases {
        let platform = parse_uname_output(input).expect(input);
        assert_eq!(&platform.arch, expected_arch, "arch mismatch for {input}");
    }

    for bad in [
        "Linux mips",
        "Linux ppc64le",
        "Linux s390x",
        "Linux riscv64",
    ] {
        assert!(
            parse_uname_output(bad).is_err(),
            "{bad} should not be supported"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// § 8  Production stderr edge cases
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn classify_dns_failure_with_motd_noise() {
    let stderr =
        "Welcome to Ubuntu 22.04 LTS\nLast login: Mon Apr 7 10:00:00 2025\ncurl: (6) Could not resolve host: app.warp.dev\n";
    assert_eq!(
        classify(stderr, Some(6)),
        InstallFailureCategory::DnsFailure
    );
}

#[test]
fn classify_partial_download_gzip_unexpected_eof() {
    let stderr =
        "gzip: stdin: unexpected end of file\ntar: Child returned status 1\ntar: Error is not recoverable: exiting now\n";
    assert_eq!(
        classify(stderr, Some(1)),
        InstallFailureCategory::PartialDownload
    );
}

#[test]
fn classify_exit2_tar_error_is_unsupported_sentinel() {
    // Exit 2 is the script's sentinel and takes priority over stderr.
    assert_eq!(
        classify(
            "tar: oz.tar.gz: Cannot open: Permission denied\ntar: Error is not recoverable",
            Some(2)
        ),
        InstallFailureCategory::UnsupportedArchitecture
    );
}
