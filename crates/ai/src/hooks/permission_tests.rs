use super::*;
use crate::hooks::test_util::{test_binding_id, test_source_id};

fn merged(verdict: HookVerdict, reasons: Vec<&str>) -> MergedEffects {
    let binding = test_binding_id(test_source_id("s"), "tool.pre", 0);
    MergedEffects {
        verdict,
        verdict_binding: (verdict != HookVerdict::NoOpinion).then(|| binding.clone()),
        reasons: reasons
            .into_iter()
            .map(|text| HookReason {
                text: text.to_owned(),
                binding: Some(binding.clone()),
            })
            .collect(),
        ..Default::default()
    }
}

fn all_warp_results() -> Vec<WarpPermissionResult> {
    let mut results = Vec::new();
    for can_auto_execute in [false, true] {
        for is_user_initiated in [false, true] {
            for is_autonomous_shell_exception in [false, true] {
                results.push(WarpPermissionResult {
                    can_auto_execute,
                    is_user_initiated,
                    is_autonomous_shell_exception,
                });
            }
        }
    }
    results
}

fn warp_alone(warp: WarpPermissionResult) -> PermissionDecision {
    if warp.is_user_initiated || warp.can_auto_execute || warp.is_autonomous_shell_exception {
        PermissionDecision::Execute
    } else {
        PermissionDecision::RequireConfirmation { reason: None }
    }
}

#[test]
fn deny_denies_for_every_warp_result_including_user_initiated() {
    for warp in all_warp_results() {
        let decision = HookPermissionPolicy::compose(
            &merged(HookVerdict::Deny, vec!["policy says no", "second"]),
            warp,
            &PermissionFloors::default(),
        );
        let PermissionDecision::Deny { reason } = decision else {
            panic!("expected deny for {warp:?}, got {decision:?}");
        };
        assert_eq!(reason.text, "policy says no");
    }
}

#[test]
fn deny_without_a_reason_uses_the_default_text() {
    let decision = HookPermissionPolicy::compose(
        &merged(HookVerdict::Deny, vec![]),
        WarpPermissionResult {
            can_auto_execute: true,
            is_user_initiated: false,
            is_autonomous_shell_exception: false,
        },
        &PermissionFloors::default(),
    );
    let PermissionDecision::Deny { reason } = decision else {
        panic!("expected deny");
    };
    assert_eq!(reason.text, DEFAULT_DENY_REASON);
    assert_eq!(
        reason.binding,
        Some(test_binding_id(test_source_id("s"), "tool.pre", 0))
    );
}

#[test]
fn every_other_verdict_leaves_warps_decision_in_place() {
    for verdict in [
        HookVerdict::NoOpinion,
        HookVerdict::Allow,
        HookVerdict::Ask,
        HookVerdict::Defer,
    ] {
        for warp in all_warp_results() {
            let decision = HookPermissionPolicy::compose(
                &merged(verdict, vec!["ignored"]),
                warp,
                &PermissionFloors::default(),
            );
            assert_eq!(decision, warp_alone(warp), "{verdict:?} with {warp:?}");
        }
    }
}

#[test]
fn a_rewrite_request_denies_because_rewrites_cannot_be_honored_yet() {
    for verdict in [HookVerdict::NoOpinion, HookVerdict::Allow, HookVerdict::Ask] {
        let mut effects = merged(verdict, vec![]);
        effects.replacement_input = Some(serde_json::json!({"command": "ls -la"}));
        let decision = HookPermissionPolicy::compose(
            &effects,
            WarpPermissionResult {
                can_auto_execute: true,
                is_user_initiated: true,
                is_autonomous_shell_exception: true,
            },
            &PermissionFloors::default(),
        );
        let PermissionDecision::Deny { reason } = decision else {
            panic!("expected deny for {verdict:?}");
        };
        assert_eq!(reason.text, REWRITE_UNSUPPORTED_DENY_REASON);
        assert_eq!(reason.binding, None, "Warp wrote this reason, not a hook");
    }

    // A hook that both rewrites and denies is reported with its own reason.
    let mut effects = merged(HookVerdict::Deny, vec!["no"]);
    effects.replacement_input = Some(serde_json::json!({}));
    let PermissionDecision::Deny { reason } = HookPermissionPolicy::compose(
        &effects,
        WarpPermissionResult {
            can_auto_execute: true,
            is_user_initiated: false,
            is_autonomous_shell_exception: false,
        },
        &PermissionFloors::default(),
    ) else {
        panic!("expected deny");
    };
    assert_eq!(reason.text, "no");
}

#[test]
fn warp_alone_requires_confirmation_only_when_every_fact_is_false() {
    let needs_confirmation = all_warp_results()
        .into_iter()
        .filter(|warp| {
            warp_alone(*warp) == PermissionDecision::RequireConfirmation { reason: None }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        needs_confirmation,
        vec![WarpPermissionResult {
            can_auto_execute: false,
            is_user_initiated: false,
            is_autonomous_shell_exception: false,
        }]
    );
}
