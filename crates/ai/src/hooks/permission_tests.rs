use super::*;
use crate::hooks::test_util::{test_binding_id, test_source_id};

const VERDICTS: [HookVerdict; 5] = [
    HookVerdict::NoOpinion,
    HookVerdict::Allow,
    HookVerdict::Deny,
    HookVerdict::Ask,
    HookVerdict::Defer,
];

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

/// Every combination of the four floors.
fn all_floors() -> Vec<PermissionFloors> {
    (0..16u8)
        .map(|bits| PermissionFloors {
            command_denylist_hit: bits & 1 != 0,
            protected_write_path: bits & 2 != 0,
            mcp_server_denylisted: bits & 4 != 0,
            always_ask_setting: bits & 8 != 0,
        })
        .collect()
}

fn all_runs() -> Vec<RunFacts> {
    let mut runs = Vec::new();
    for is_headless in [false, true] {
        for is_strict in [false, true] {
            runs.push(RunFacts {
                is_headless,
                is_strict,
            });
        }
    }
    runs
}

fn warp_alone(warp: WarpPermissionResult) -> PermissionDecision {
    if warp.is_user_initiated || warp.can_auto_execute || warp.is_autonomous_shell_exception {
        PermissionDecision::Execute
    } else {
        PermissionDecision::RequireConfirmation { reason: None }
    }
}

fn interactive() -> RunFacts {
    RunFacts::default()
}

fn headless() -> RunFacts {
    RunFacts {
        is_headless: true,
        is_strict: false,
    }
}

fn warp(can_auto_execute: bool, is_user_initiated: bool) -> WarpPermissionResult {
    WarpPermissionResult {
        can_auto_execute,
        is_user_initiated,
        is_autonomous_shell_exception: false,
    }
}

/// The whole matrix: verdict x Warp's three facts x headless x strict x floors. The invariants
/// hold in every cell; the hand-written tests below pin the interesting corners.
#[test]
fn the_policy_only_ever_tightens_warps_decision() {
    for verdict in VERDICTS {
        for warp in all_warp_results() {
            for run in all_runs() {
                for floors in all_floors() {
                    let effects = merged(verdict, vec!["hook says so"]);
                    let decision = HookPermissionPolicy::compose(&effects, warp, &floors, run);
                    let baseline = warp_alone(warp);
                    let cell = format!("{verdict:?} {warp:?} {run:?} {floors:?}");

                    // Floors are never crossed: no cell is looser than Warp alone.
                    assert!(
                        decision.strictness() >= baseline.strictness(),
                        "loosened: {cell} -> {decision:?}"
                    );
                    // Allow and no opinion never change the outcome.
                    if matches!(verdict, HookVerdict::Allow | HookVerdict::NoOpinion) {
                        assert_eq!(decision, baseline, "{cell}");
                    }
                    // Deny always denies, whoever approved what.
                    if verdict == HookVerdict::Deny {
                        assert!(
                            matches!(
                                decision,
                                PermissionDecision::Deny {
                                    cause: DenyCause::Hook,
                                    ..
                                }
                            ),
                            "{cell} -> {decision:?}"
                        );
                    }
                    // Ask: a confirmation step where someone can answer and the user has not
                    // already approved; a denial where nobody can.
                    if verdict == HookVerdict::Ask {
                        match (run.is_headless, warp.is_user_initiated) {
                            (true, _) => assert!(
                                matches!(
                                    &decision,
                                    PermissionDecision::Deny {
                                        cause: DenyCause::ApprovalUnavailable,
                                        reason,
                                    } if reason.text.starts_with(APPROVAL_UNAVAILABLE_PREFIX)
                                ),
                                "{cell} -> {decision:?}"
                            ),
                            (false, true) => assert_eq!(decision, baseline, "{cell}"),
                            (false, false) => assert!(
                                matches!(
                                    decision,
                                    PermissionDecision::RequireConfirmation { reason: Some(_) }
                                ),
                                "{cell} -> {decision:?}"
                            ),
                        }
                    }
                    // Defer: denied where nobody can resume it or under the strict policy,
                    // ignored otherwise.
                    if verdict == HookVerdict::Defer {
                        if run.is_headless || run.is_strict {
                            assert!(
                                matches!(
                                    decision,
                                    PermissionDecision::Deny {
                                        cause: DenyCause::DeferralUnsupported,
                                        ..
                                    }
                                ),
                                "{cell} -> {decision:?}"
                            );
                        } else {
                            assert_eq!(decision, baseline, "{cell}");
                        }
                    }
                }
            }
        }
    }
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

#[test]
fn allow_cannot_make_warp_execute_what_it_would_ask_about_or_deny() {
    for floors in all_floors() {
        for run in all_runs() {
            let decision = HookPermissionPolicy::compose(
                &merged(HookVerdict::Allow, vec!["looks safe"]),
                warp(false, false),
                &floors,
                run,
            );
            assert_eq!(
                decision,
                PermissionDecision::RequireConfirmation { reason: None }
            );
        }
    }
}

#[test]
fn deny_denies_a_user_initiated_action_with_the_denying_bindings_reason() {
    let allowing = test_binding_id(test_source_id("s"), "tool.pre", 0);
    let denying = test_binding_id(test_source_id("s"), "tool.pre", 1);
    let effects = MergedEffects {
        verdict: HookVerdict::Deny,
        verdict_binding: Some(denying.clone()),
        reasons: vec![
            HookReason {
                text: "looks fine to me".to_owned(),
                binding: Some(allowing),
            },
            HookReason {
                text: "not in this repository".to_owned(),
                binding: Some(denying.clone()),
            },
        ],
        ..Default::default()
    };
    let PermissionDecision::Deny { reason, cause } = HookPermissionPolicy::compose(
        &effects,
        warp(true, true),
        &PermissionFloors::default(),
        interactive(),
    ) else {
        panic!("expected deny");
    };
    assert_eq!(reason.text, "not in this repository");
    assert_eq!(reason.binding, Some(denying));
    assert_eq!(cause, DenyCause::Hook);
}

#[test]
fn deny_without_a_reason_uses_the_default_text() {
    let PermissionDecision::Deny { reason, .. } = HookPermissionPolicy::compose(
        &merged(HookVerdict::Deny, vec![]),
        warp(true, false),
        &PermissionFloors::default(),
        interactive(),
    ) else {
        panic!("expected deny");
    };
    assert_eq!(reason.text, DEFAULT_DENY_REASON);
    assert_eq!(
        reason.binding,
        Some(test_binding_id(test_source_id("s"), "tool.pre", 0))
    );
}

#[test]
fn ask_adds_a_confirmation_step_with_the_hooks_reason_where_someone_can_answer() {
    let decision = HookPermissionPolicy::compose(
        &merged(HookVerdict::Ask, vec!["this touches production"]),
        warp(true, false),
        &PermissionFloors::default(),
        interactive(),
    );
    let PermissionDecision::RequireConfirmation {
        reason: Some(reason),
    } = decision
    else {
        panic!("expected a confirmation step, got {decision:?}");
    };
    assert_eq!(reason.text, "this touches production");

    let decision = HookPermissionPolicy::compose(
        &merged(HookVerdict::Ask, vec![]),
        warp(false, false),
        &PermissionFloors::default(),
        interactive(),
    );
    let PermissionDecision::RequireConfirmation {
        reason: Some(reason),
    } = decision
    else {
        panic!("expected a confirmation step, got {decision:?}");
    };
    assert_eq!(reason.text, DEFAULT_ASK_REASON);
}

#[test]
fn ask_is_satisfied_by_the_users_approval_of_this_revision() {
    let decision = HookPermissionPolicy::compose(
        &merged(HookVerdict::Ask, vec!["please check"]),
        warp(false, true),
        &PermissionFloors::default(),
        interactive(),
    );
    assert_eq!(decision, PermissionDecision::Execute);
}

#[test]
fn ask_denies_where_nobody_can_answer_and_names_the_hooks_reason() {
    let decision = HookPermissionPolicy::compose(
        &merged(HookVerdict::Ask, vec!["this touches production"]),
        warp(true, false),
        &PermissionFloors::default(),
        headless(),
    );
    let PermissionDecision::Deny { reason, cause } = decision else {
        panic!("expected deny, got {decision:?}");
    };
    assert_eq!(cause, DenyCause::ApprovalUnavailable);
    assert_eq!(
        reason.text,
        "Approval unavailable: an agent hook asked for confirmation (this touches production) \
         and this run has no one to ask."
    );
    assert_eq!(
        reason.binding,
        Some(test_binding_id(test_source_id("s"), "tool.pre", 0))
    );

    let PermissionDecision::Deny { reason, .. } = HookPermissionPolicy::compose(
        &merged(HookVerdict::Ask, vec![]),
        warp(true, false),
        &PermissionFloors::default(),
        headless(),
    ) else {
        panic!("expected deny");
    };
    assert_eq!(
        reason.text,
        "Approval unavailable: an agent hook asked for confirmation and this run has no one to \
         ask."
    );
}

#[test]
fn ask_denies_on_a_headless_surface_even_for_a_user_initiated_revision() {
    let decision = HookPermissionPolicy::compose(
        &merged(HookVerdict::Ask, vec!["please check"]),
        WarpPermissionResult {
            can_auto_execute: true,
            is_user_initiated: true,
            is_autonomous_shell_exception: true,
        },
        &PermissionFloors::default(),
        headless(),
    );
    assert!(matches!(
        decision,
        PermissionDecision::Deny {
            cause: DenyCause::ApprovalUnavailable,
            ..
        }
    ));
}

#[test]
fn defer_is_ignored_interactively_and_denied_headless_or_under_the_strict_policy() {
    let effects = merged(HookVerdict::Defer, vec!["later"]);
    assert_eq!(
        HookPermissionPolicy::compose(
            &effects,
            warp(true, false),
            &PermissionFloors::default(),
            interactive()
        ),
        PermissionDecision::Execute
    );
    for run in [
        headless(),
        RunFacts {
            is_headless: false,
            is_strict: true,
        },
    ] {
        let PermissionDecision::Deny { reason, cause } = HookPermissionPolicy::compose(
            &effects,
            warp(true, false),
            &PermissionFloors::default(),
            run,
        ) else {
            panic!("expected deny for {run:?}");
        };
        assert_eq!(cause, DenyCause::DeferralUnsupported);
        assert_eq!(reason.text, DEFERRAL_UNSUPPORTED_DENY_REASON);
        assert_eq!(
            reason.binding,
            Some(test_binding_id(test_source_id("s"), "tool.pre", 0))
        );
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
            interactive(),
        );
        let PermissionDecision::Deny { reason, cause } = decision else {
            panic!("expected deny for {verdict:?}");
        };
        assert_eq!(reason.text, REWRITE_UNSUPPORTED_DENY_REASON);
        assert_eq!(reason.binding, None, "Warp wrote this reason, not a hook");
        assert_eq!(cause, DenyCause::RewriteUnsupported);
    }

    // A hook that both rewrites and denies is reported with its own reason.
    let mut effects = merged(HookVerdict::Deny, vec!["no"]);
    effects.replacement_input = Some(serde_json::json!({}));
    let PermissionDecision::Deny { reason, .. } = HookPermissionPolicy::compose(
        &effects,
        warp(true, false),
        &PermissionFloors::default(),
        interactive(),
    ) else {
        panic!("expected deny");
    };
    assert_eq!(reason.text, "no");
}

#[test]
fn floors_report_whether_any_is_set() {
    assert!(!PermissionFloors::default().any());
    for floors in all_floors().into_iter().skip(1) {
        assert!(floors.any(), "{floors:?}");
    }
}

#[test]
fn decisions_order_by_strictness() {
    let reason = HookReason {
        text: "no".to_owned(),
        binding: None,
    };
    assert!(
        PermissionDecision::Execute.strictness()
            < PermissionDecision::RequireConfirmation { reason: None }.strictness()
    );
    assert!(
        PermissionDecision::RequireConfirmation {
            reason: Some(reason.clone())
        }
        .strictness()
            < PermissionDecision::Deny {
                reason,
                cause: DenyCause::Hook
            }
            .strictness()
    );
}
