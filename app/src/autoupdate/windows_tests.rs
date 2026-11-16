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
