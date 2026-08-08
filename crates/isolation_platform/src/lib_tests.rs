use std::io::ErrorKind;
use std::sync::Mutex;

use super::*;

/// `WARP_WORKLOAD_TOKEN` is process-global, so tests that set it must not overlap.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn set_generic_token(value: Option<&str>) {
    unsafe {
        match value {
            Some(value) => std::env::set_var(WARP_WORKLOAD_TOKEN_ENV, value),
            None => std::env::remove_var(WARP_WORKLOAD_TOKEN_ENV),
        }
    }
}

fn nsc_missing() -> IsolationPlatformError {
    IsolationPlatformError::CommandUnavailable {
        command: "nsc".to_owned(),
        source: std::io::Error::new(ErrorKind::NotFound, "no such file"),
    }
}

#[test]
fn falls_back_to_generic_token_when_platform_token_fails() {
    let _lock = ENV_LOCK.lock();
    set_generic_token(Some("server-supplied-token"));

    let token =
        fall_back_to_generic_workload_token(IsolationPlatformType::Namespace, nsc_missing())
            .expect("should fall back to the server-supplied token");

    assert_eq!(token.token, "server-supplied-token");
    assert!(token.expires_at.is_none());
    set_generic_token(None);
}

/// The reported failure: `nsc` is present in an Oz agent sandbox but exits non-zero, and the
/// server-supplied token is the usable credential.
#[cfg(unix)]
#[test]
fn falls_back_to_generic_token_when_platform_command_exits_non_zero() {
    use std::os::unix::process::ExitStatusExt;

    let _lock = ENV_LOCK.lock();
    set_generic_token(Some("server-supplied-token"));

    let platform_error = IsolationPlatformError::CommandFailed {
        command: "nsc".to_owned(),
        status: ExitStatus::from_raw(1 << 8),
    };
    let token =
        fall_back_to_generic_workload_token(IsolationPlatformType::Namespace, platform_error)
            .expect("should fall back to the server-supplied token");

    assert_eq!(token.token, "server-supplied-token");
    set_generic_token(None);
}

#[test]
fn reports_the_platform_error_when_no_generic_token_exists() {
    let _lock = ENV_LOCK.lock();
    set_generic_token(None);

    let error =
        fall_back_to_generic_workload_token(IsolationPlatformType::Namespace, nsc_missing())
            .expect_err("no fallback token means the platform failure stands");

    assert!(
        matches!(error, IsolationPlatformError::CommandUnavailable { .. }),
        "expected the original platform error, got {error:?}"
    );
}

#[test]
fn treats_an_empty_generic_token_as_absent() {
    let _lock = ENV_LOCK.lock();
    set_generic_token(Some(""));

    let error =
        fall_back_to_generic_workload_token(IsolationPlatformType::Namespace, nsc_missing())
            .expect_err("an empty token is not a usable credential");

    assert!(
        matches!(error, IsolationPlatformError::CommandUnavailable { .. }),
        "expected the original platform error, got {error:?}"
    );
    set_generic_token(None);
}
