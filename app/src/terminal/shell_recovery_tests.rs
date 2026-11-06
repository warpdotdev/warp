use warp_terminal::event::ObservedExitStatus;

use super::recovered_command_output;

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
