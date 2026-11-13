use super::*;
use crate::hooks::test_util::{StaticTrust, test_source_id, test_source_id_of_kind};

const SURFACES: [HookSurface; 5] = [
    HookSurface::OzRun,
    HookSurface::CliRun,
    HookSurface::LocalChild,
    HookSurface::Gui,
    HookSurface::Tui,
];

const KINDS: [HookSourceKind; 4] = [
    HookSourceKind::ProjectCursorFile,
    HookSourceKind::RunDelivered,
    HookSourceKind::FactoryAgent,
    HookSourceKind::FactoryWide,
];

const POSTURES: [CommandExecutionPosture; 3] = [
    CommandExecutionPosture::AutoApproves,
    CommandExecutionPosture::RequiresApproval,
    CommandExecutionPosture::Unknown,
];

#[test]
fn strictest_prefers_requires_approval_then_unknown_then_auto_approves() {
    use CommandExecutionPosture::*;
    assert_eq!(AutoApproves.strictest(AutoApproves), AutoApproves);
    assert_eq!(AutoApproves.strictest(Unknown), Unknown);
    assert_eq!(Unknown.strictest(RequiresApproval), RequiresApproval);
    assert_eq!(RequiresApproval.strictest(AutoApproves), RequiresApproval);
}

#[test]
fn strictest_of_folds_a_sequence_and_is_unknown_for_none() {
    use CommandExecutionPosture::*;
    assert_eq!(
        CommandExecutionPosture::strictest_of([AutoApproves, AutoApproves]),
        AutoApproves
    );
    assert_eq!(
        CommandExecutionPosture::strictest_of([AutoApproves, Unknown]),
        Unknown
    );
    assert_eq!(
        CommandExecutionPosture::strictest_of([Unknown, AutoApproves, RequiresApproval]),
        RequiresApproval
    );
    assert_eq!(
        CommandExecutionPosture::strictest_of(Vec::new()),
        Unknown,
        "no posture to read means the posture cannot be known"
    );
}

#[test]
fn deny_all_keeps_every_source_inert() {
    let policy = DenyAllTrustPolicy;
    for surface in SURFACES {
        for posture in POSTURES {
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

#[test]
fn static_policy_activates_exactly_the_expected_cells() {
    let policy = StaticTrustPolicy;
    let mut activated = Vec::new();
    for surface in SURFACES {
        for kind in KINDS {
            for posture in POSTURES {
                let source = test_source_id_of_kind("s", kind);
                match policy.may_activate(&source, surface, posture) {
                    TrustDecision::Activate => activated.push((surface, kind, posture)),
                    TrustDecision::Inert { reason } => {
                        assert!(!reason.is_empty(), "{surface:?} {kind:?} {posture:?}")
                    }
                }
            }
        }
    }

    let mut expected = Vec::new();
    for surface in SURFACES {
        for kind in [
            HookSourceKind::RunDelivered,
            HookSourceKind::FactoryAgent,
            HookSourceKind::FactoryWide,
        ] {
            for posture in POSTURES {
                expected.push((surface, kind, posture));
            }
        }
    }
    for surface in [HookSurface::OzRun, HookSurface::CliRun] {
        expected.push((
            surface,
            HookSourceKind::ProjectCursorFile,
            CommandExecutionPosture::AutoApproves,
        ));
    }
    activated.sort_by_key(|cell| format!("{cell:?}"));
    expected.sort_by_key(|cell| format!("{cell:?}"));
    assert_eq!(activated, expected);
}

#[test]
fn project_files_stay_inert_on_interactive_surfaces_even_when_commands_auto_approve() {
    let policy = StaticTrustPolicy;
    let source = test_source_id_of_kind("project", HookSourceKind::ProjectCursorFile);
    for surface in [HookSurface::Gui, HookSurface::Tui, HookSurface::LocalChild] {
        let decision = policy.may_activate(&source, surface, CommandExecutionPosture::AutoApproves);
        assert!(
            matches!(decision, TrustDecision::Inert { .. }),
            "{surface:?}"
        );
    }
}

#[test]
fn an_unknown_posture_keeps_project_files_inert_on_headless_surfaces() {
    let policy = StaticTrustPolicy;
    let source = test_source_id_of_kind("project", HookSourceKind::ProjectCursorFile);
    for surface in [HookSurface::OzRun, HookSurface::CliRun] {
        for posture in [
            CommandExecutionPosture::RequiresApproval,
            CommandExecutionPosture::Unknown,
        ] {
            let decision = policy.may_activate(&source, surface, posture);
            assert!(
                matches!(decision, TrustDecision::Inert { .. }),
                "{surface:?} {posture:?}"
            );
        }
    }
}
