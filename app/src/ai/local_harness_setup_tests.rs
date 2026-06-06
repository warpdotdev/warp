use super::*;

#[test]
fn claude_is_product_enabled_when_cli_is_installed() {
    assert_eq!(
        local_harness_setup_state_with_cli_resolver(Harness::Claude, |_| true),
        LocalHarnessSetupState::Ready
    );
}

#[test]
fn claude_is_disabled_for_missing_cli() {
    assert_eq!(
        local_harness_setup_state_with_cli_resolver(Harness::Claude, |_| false),
        LocalHarnessSetupState::MissingHarness {
            tooltip: LOCAL_HARNESS_INSTALLATION_REQUIRED_TOOLTIP,
        }
    );
}

#[test]
fn codex_remains_product_disabled() {
    assert_eq!(
        local_harness_setup_state_with_cli_resolver(Harness::Codex, |_| true),
        LocalHarnessSetupState::ProductDisabled {
            message: "Local Codex child agents are temporarily disabled.",
        }
    );
}
