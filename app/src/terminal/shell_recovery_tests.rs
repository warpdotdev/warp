use std::collections::HashMap;
use std::ffi::OsString;

use warp_terminal::event::ObservedExitStatus;

use super::{
    CloudShellRecoveryDecision, CloudShellRecoveryEligibility, MAX_CLOUD_SHELL_RECOVERIES,
    recovered_command_output, sanitized_recovery_environment,
};

fn eligible_recovery(recovery_count: u8) -> CloudShellRecoveryEligibility {
    CloudShellRecoveryEligibility {
        feature_enabled: true,
        recovery_count,
        login_shell_bootstrapped: true,
        shared_ambient_session: true,
        active_sharer: true,
        ..Default::default()
    }
}

#[test]
fn shell_recovery_allows_three_incidents_then_caps_the_fourth() {
    assert_eq!(
        eligible_recovery(0).decision(),
        CloudShellRecoveryDecision::Attempt(1)
    );
    assert_eq!(
        eligible_recovery(MAX_CLOUD_SHELL_RECOVERIES - 1).decision(),
        CloudShellRecoveryDecision::Attempt(MAX_CLOUD_SHELL_RECOVERIES)
    );
    assert_eq!(
        eligible_recovery(MAX_CLOUD_SHELL_RECOVERIES).decision(),
        CloudShellRecoveryDecision::Capped
    );
}

#[test]
fn shell_recovery_rejects_non_cloud_and_unsafe_lifecycle_states() {
    let eligible = eligible_recovery(0);
    let ineligible = [
        CloudShellRecoveryEligibility {
            feature_enabled: false,
            ..eligible
        },
        CloudShellRecoveryEligibility {
            manual_shutdown_requested: true,
            ..eligible
        },
        CloudShellRecoveryEligibility {
            recovery_in_progress: true,
            ..eligible
        },
        CloudShellRecoveryEligibility {
            terminal_failure: true,
            ..eligible
        },
        CloudShellRecoveryEligibility {
            login_shell_bootstrapped: false,
            ..eligible
        },
        CloudShellRecoveryEligibility {
            third_party_harness: true,
            ..eligible
        },
        CloudShellRecoveryEligibility {
            shared_ambient_session: false,
            ..eligible
        },
        CloudShellRecoveryEligibility {
            active_sharer: false,
            ..eligible
        },
        CloudShellRecoveryEligibility {
            running_environment_setup: true,
            ..eligible
        },
    ];

    assert!(
        ineligible
            .into_iter()
            .all(|state| state.decision() == CloudShellRecoveryDecision::Ineligible)
    );
}

#[test]
fn shell_recovery_sanitizes_integration_state_and_prefers_dynamic_values() {
    let original = HashMap::from([
        (OsString::from("PATH"), OsString::from("/original")),
        (OsString::from("LANG"), OsString::from("en_US.UTF-8")),
        (OsString::from("HOME"), OsString::from("/host/home")),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("WARP_SESSION_ID"), OsString::from("stale")),
        (OsString::from("PWD"), OsString::from("/stale")),
    ]);
    let dynamic = HashMap::from([
        ("PATH".to_owned(), "/dynamic".to_owned()),
        ("BASH_FUNC_helper%%".to_owned(), "() { true; }".to_owned()),
        ("EXPORTED".to_owned(), "yes".to_owned()),
    ]);

    let restored = sanitized_recovery_environment(&original, Some(dynamic));

    assert_eq!(
        restored.get(&OsString::from("PATH")),
        Some(&OsString::from("/dynamic"))
    );
    assert_eq!(
        restored.get(&OsString::from("EXPORTED")),
        Some(&OsString::from("yes"))
    );
    assert_eq!(
        restored.get(&OsString::from("LANG")),
        Some(&OsString::from("en_US.UTF-8"))
    );
    assert!(!restored.contains_key(&OsString::from("HOME")));
    assert!(!restored.contains_key(&OsString::from("TERM")));
    assert!(!restored.contains_key(&OsString::from("WARP_SESSION_ID")));
    assert!(!restored.contains_key(&OsString::from("BASH_FUNC_helper%%")));
    assert!(!restored.contains_key(&OsString::from("PWD")));
}

#[test]
fn shell_recovery_output_reports_unknown_status_without_success_code() {
    let output = recovered_command_output(
        "partial",
        ObservedExitStatus::Unavailable,
        "/home/agent",
        true,
    );

    assert!(output.starts_with("This command terminated the persistent cloud shell."));
    assert!(output.contains("Observed status: exit status unavailable"));
    assert!(output.contains("/home/agent (fallback directory)"));
    assert!(output.ends_with("partial"));
    assert!(!output.contains("exit code 0"));
}
