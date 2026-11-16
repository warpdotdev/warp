use serial_test::serial;

use super::*;

#[test]
fn workload_token_available_for_platforms_with_their_own_issuance() {
    assert!(is_workload_token_available_for(Some(
        IsolationPlatformType::DockerSandbox
    )));
    assert!(is_workload_token_available_for(Some(
        IsolationPlatformType::Namespace
    )));
}

#[test]
#[serial(warp_workload_token_env)]
fn workload_token_unavailable_without_platform_or_generic_token() {
    // SAFETY: serialized via `#[serial]`, so no other test in this crate
    // reads/writes `WARP_WORKLOAD_TOKEN` concurrently.
    unsafe { std::env::remove_var(WARP_WORKLOAD_TOKEN_ENV) };

    assert!(!is_workload_token_available_for(None));
    assert!(!is_workload_token_available_for(Some(
        IsolationPlatformType::Docker
    )));
}

#[test]
#[serial(warp_workload_token_env)]
fn workload_token_available_via_generic_token_without_detected_platform() {
    // A self-hosted worker running directly on a host (no Docker/Kubernetes/
    // Namespace signals) can still authenticate via a manually configured
    // generic token; this must not be treated as unavailable.
    // SAFETY: see above.
    unsafe { std::env::set_var(WARP_WORKLOAD_TOKEN_ENV, "example-token") };

    assert!(is_workload_token_available_for(None));

    // SAFETY: see above.
    unsafe { std::env::remove_var(WARP_WORKLOAD_TOKEN_ENV) };
}

#[test]
#[serial(warp_workload_token_env)]
fn workload_token_unavailable_with_empty_generic_token() {
    // SAFETY: see above.
    unsafe { std::env::set_var(WARP_WORKLOAD_TOKEN_ENV, "") };

    assert!(!is_workload_token_available_for(None));

    // SAFETY: see above.
    unsafe { std::env::remove_var(WARP_WORKLOAD_TOKEN_ENV) };
}
