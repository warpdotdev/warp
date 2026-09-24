use std::collections::HashMap;
use std::ffi::OsString;

use warp_terminal::event::ObservedExitStatus;

use super::{recovered_command_output, sanitized_recovery_environment};

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
fn shell_recovery_output_distinguishes_unknown_status_from_observed_zero() {
    let output = recovered_command_output(
        "partial",
        ObservedExitStatus::Unavailable,
        "/home/agent",
        true,
    );

    assert!(output.contains("Observed status: exit status unavailable"));
    assert!(!output.contains("exit code 0"));
    let output = recovered_command_output("", ObservedExitStatus::Code(0), "/home/agent", false);

    assert!(output.contains("Observed status: exit code 0"));
}
