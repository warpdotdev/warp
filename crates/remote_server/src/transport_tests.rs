use super::*;

#[test]
fn script_failed_permission_denied_produces_targeted_message() {
    let err = Error::ScriptFailed {
        exit_code: 1,
        stderr: "mkdir: cannot create directory '/home/user/.warp': Permission denied".into(),
    };
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    assert_eq!(ufe.body, "Failed to install SSH extension");
    let detail = ufe.detail.unwrap();
    assert!(
        detail.contains("check write permissions"),
        "expected permission-denied message, got: {detail}"
    );
    assert!(
        detail.contains("exit code 1"),
        "expected exit code in detail, got: {detail}"
    );
}

#[test]
fn script_failed_read_only_fs_produces_targeted_message() {
    let err = Error::ScriptFailed {
        exit_code: 1,
        stderr: "cp: cannot create regular file: Read-only file system".into(),
    };
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    let detail = ufe.detail.unwrap();
    assert!(
        detail.contains("check write permissions"),
        "expected read-only FS message, got: {detail}"
    );
}

#[test]
fn script_failed_disk_full_produces_targeted_message() {
    let err = Error::ScriptFailed {
        exit_code: 2,
        stderr: "No space left on device".into(),
    };
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    assert_eq!(ufe.body, "Failed to install SSH extension");
    let detail = ufe.detail.unwrap();
    assert!(
        detail.contains("free up space"),
        "expected disk-full message, got: {detail}"
    );
    assert!(
        detail.contains("exit code 2"),
        "expected exit code in detail, got: {detail}"
    );
}

#[test]
fn script_failed_curl_write_failure_produces_disk_full_message() {
    // curl reports "Failure writing output to destination" when the
    // download destination runs out of space (discovered via Docker test
    // with a tiny tmpfs mount).
    let err = Error::ScriptFailed {
        exit_code: 23,
        stderr: "curl: (23) Failure writing output to destination".into(),
    };
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    let detail = ufe.detail.unwrap();
    assert!(
        detail.contains("free up space"),
        "curl write-failure should produce disk-full message, got: {detail}"
    );
}

#[test]
fn script_failed_ssh_disconnect_produces_targeted_message() {
    let err = Error::ScriptFailed {
        exit_code: 255,
        stderr: "".into(),
    };
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    assert_eq!(ufe.body, "Failed to install SSH extension");
    let detail = ufe.detail.unwrap();
    assert!(
        detail.contains("SSH connection was lost"),
        "expected SSH disconnect message, got: {detail}"
    );
    assert!(
        detail.contains("reconnect"),
        "expected reconnect advice, got: {detail}"
    );
}

#[test]
fn script_failed_ssh_disconnect_only_whitespace_stderr() {
    let err = Error::ScriptFailed {
        exit_code: 255,
        stderr: "   \n  ".into(),
    };
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    let detail = ufe.detail.unwrap();
    assert!(
        detail.contains("SSH connection was lost"),
        "whitespace-only stderr with code 255 should trigger SSH disconnect, got: {detail}"
    );
}

#[test]
fn script_failed_ssh_255_with_stderr_uses_default() {
    // Exit code 255 but with actual stderr content should NOT match
    // the SSH disconnect pattern — it's a real script error.
    let err = Error::ScriptFailed {
        exit_code: 255,
        stderr: "bash: some-command: not found".into(),
    };
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    let detail = ufe.detail.unwrap();
    assert!(
        detail.contains("Script exited with code 255"),
        "non-empty stderr with code 255 should use default format, got: {detail}"
    );
}

#[test]
fn script_failed_unsupported_arch_produces_targeted_message() {
    let err = Error::ScriptFailed {
        exit_code: 2,
        stderr: "unsupported arch: armv7l".into(),
    };
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    assert_eq!(ufe.body, "Failed to install SSH extension");
    let detail = ufe.detail.unwrap();
    assert!(
        detail.contains("armv7l"),
        "expected arch name in message, got: {detail}"
    );
    assert!(
        detail.contains("x86_64 or aarch64"),
        "expected supported arch list, got: {detail}"
    );
}

#[test]
fn script_failed_default_format_for_unrecognised_error() {
    let err = Error::ScriptFailed {
        exit_code: 6,
        stderr: "curl: (6) Could not resolve host".into(),
    };
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    let detail = ufe.detail.unwrap();
    assert!(
        detail.starts_with("Script exited with code 6:"),
        "unrecognised error should use default format, got: {detail}"
    );
    assert!(
        detail.contains("Could not resolve host"),
        "default format should include stderr, got: {detail}"
    );
}

#[test]
fn script_failed_truncates_long_stderr_in_default_format() {
    let long_stderr = "x".repeat(600);
    let err = Error::ScriptFailed {
        exit_code: 1,
        stderr: long_stderr,
    };
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    let detail = ufe.detail.unwrap();
    assert!(
        detail.contains('…'),
        "long stderr should be truncated, got length: {}",
        detail.len()
    );
    // MAX_STDERR_DISPLAY_CHARS is 512, plus "Script exited with code N: " prefix + "…"
    assert!(
        detail.len() < 600,
        "truncated detail should be shorter than full stderr, got: {}",
        detail.len()
    );
}

#[test]
fn script_failed_body_reflects_stage() {
    let err = Error::ScriptFailed {
        exit_code: 1,
        stderr: "some error".into(),
    };
    let launch = err.user_facing_error(SetupStage::Launch);
    assert_eq!(launch.body, "Failed to start SSH extension");

    let check = err.user_facing_error(SetupStage::CheckBinary);
    assert_eq!(check.body, "Failed to verify SSH extension");
}

#[test]
fn timed_out_error_message() {
    let err = Error::TimedOut;
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    assert_eq!(ufe.body, "Failed to install SSH extension");
    let detail = ufe.detail.unwrap();
    assert!(detail.contains("timed out"));
}

#[test]
fn other_error_has_no_detail() {
    let err = Error::Other(anyhow::anyhow!("something unexpected"));
    let ufe = err.user_facing_error(SetupStage::InstallBinary);
    assert_eq!(ufe.body, "Failed to install SSH extension");
    assert!(ufe.detail.is_none());
}
