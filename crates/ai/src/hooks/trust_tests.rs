use super::*;
use crate::hooks::test_util::{StaticTrust, test_source_id};

#[test]
fn deny_all_keeps_every_source_inert() {
    let policy = DenyAllTrustPolicy;
    for surface in [
        HookSurface::OzRun,
        HookSurface::CliRun,
        HookSurface::LocalChild,
        HookSurface::Gui,
        HookSurface::Tui,
    ] {
        for posture in [
            CommandExecutionPosture::AutoApproves,
            CommandExecutionPosture::RequiresApproval,
            CommandExecutionPosture::Unknown,
        ] {
            assert!(matches!(
                policy.may_activate(&test_source_id("s"), surface, posture),
                TrustDecision::Inert { .. }
            ));
        }
    }
}

#[test]
fn static_trust_returns_its_fixed_decision() {
    let policy = StaticTrust(TrustDecision::Activate);
    assert_eq!(
        policy.may_activate(
            &test_source_id("s"),
            HookSurface::OzRun,
            CommandExecutionPosture::RequiresApproval
        ),
        TrustDecision::Activate
    );
}
