use std::collections::HashSet;
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::hooks::test_util::{
    test_binding_id, test_source_id, test_source_id_of_kind, verdict_and_notice_capabilities,
};

fn binding(name: &str, index: usize) -> HookBindingId {
    test_binding_id(test_source_id(name), "tool.pre", index)
}

fn reason(text: &str) -> HookReason {
    HookReason {
        text: text.to_owned(),
        binding: None,
    }
}

fn effects(verdict: HookVerdict) -> HookEffects {
    HookEffects {
        verdict,
        ..Default::default()
    }
}

#[test]
fn reduce_picks_the_most_severe_verdict_regardless_of_order() {
    let merged = reduce_effects(vec![
        (binding("a", 0), effects(HookVerdict::Allow)),
        (binding("a", 1), effects(HookVerdict::Deny)),
        (binding("a", 2), effects(HookVerdict::Ask)),
    ]);
    assert_eq!(merged.verdict, HookVerdict::Deny);
    assert_eq!(merged.verdict_binding, Some(binding("a", 1)));

    let merged = reduce_effects(vec![
        (binding("a", 0), effects(HookVerdict::NoOpinion)),
        (binding("a", 1), effects(HookVerdict::Allow)),
    ]);
    assert_eq!(merged.verdict, HookVerdict::Allow);

    let merged = reduce_effects(vec![
        (binding("a", 0), effects(HookVerdict::Ask)),
        (binding("a", 1), effects(HookVerdict::Allow)),
    ]);
    assert_eq!(merged.verdict, HookVerdict::Ask);
    assert_eq!(merged.verdict_binding, Some(binding("a", 0)));
}

#[test]
fn reduce_keeps_the_first_binding_when_severity_ties() {
    let merged = reduce_effects(vec![
        (binding("a", 0), effects(HookVerdict::Deny)),
        (binding("a", 1), effects(HookVerdict::Deny)),
    ]);
    assert_eq!(merged.verdict_binding, Some(binding("a", 0)));
}

#[test]
fn reduce_halts_when_any_binding_halts() {
    let merged = reduce_effects(vec![
        (binding("a", 0), HookEffects::default()),
        (
            binding("a", 1),
            HookEffects {
                halt_turn: true,
                ..Default::default()
            },
        ),
        (binding("a", 2), HookEffects::default()),
    ]);
    assert!(merged.halt_turn);
}

#[test]
fn reduce_concatenates_notices_context_env_and_reasons_in_order() {
    let merged = reduce_effects(vec![
        (
            binding("a", 0),
            HookEffects {
                user_notices: vec!["first".to_owned()],
                model_context: vec!["ctx-1".to_owned()],
                session_env: vec![("A".to_owned(), "1".to_owned())],
                reasons: vec![reason("why-1")],
                ..Default::default()
            },
        ),
        (
            binding("a", 1),
            HookEffects {
                user_notices: vec!["second".to_owned(), "third".to_owned()],
                model_context: vec!["ctx-2".to_owned()],
                session_env: vec![("B".to_owned(), "2".to_owned())],
                reasons: vec![reason("why-2")],
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(merged.user_notices, vec!["first", "second", "third"]);
    assert_eq!(merged.model_context, vec!["ctx-1", "ctx-2"]);
    assert_eq!(
        merged.session_env,
        vec![
            ("A".to_owned(), "1".to_owned()),
            ("B".to_owned(), "2".to_owned())
        ]
    );
    assert_eq!(
        merged
            .reasons
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>(),
        vec!["why-1", "why-2"]
    );
    assert_eq!(merged.reasons[0].binding, Some(binding("a", 0)));
    assert_eq!(merged.reasons[1].binding, Some(binding("a", 1)));
}

#[test]
fn reduce_keeps_the_first_scalar_and_records_later_ones_as_conflicts() {
    let merged = reduce_effects(vec![
        (
            binding("a", 0),
            HookEffects {
                replacement_input: Some(json!({"command": "ls"})),
                continuation: Some(ContinuationRequest {
                    text: "keep going".to_owned(),
                    loop_limit: Some(2),
                }),
                ..Default::default()
            },
        ),
        (
            binding("b", 0),
            HookEffects {
                replacement_input: Some(json!({"command": "rm"})),
                continuation: Some(ContinuationRequest {
                    text: "stop".to_owned(),
                    loop_limit: None,
                }),
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(merged.replacement_input, Some(json!({"command": "ls"})));
    assert_eq!(
        merged.continuation.as_ref().map(|c| c.text.as_str()),
        Some("keep going")
    );
    assert_eq!(merged.continuation_binding, Some(binding("a", 0)));
    assert_eq!(merged.conflicts.len(), 2);
    assert!(merged.conflicts[0].contains("replacement input"));
    assert!(merged.conflicts[1].contains("continuation"));
}

#[test]
fn reduce_reports_defer_as_unsupported_and_does_not_merge_it() {
    let merged = reduce_effects(vec![(binding("a", 0), effects(HookVerdict::Defer))]);
    assert_eq!(merged.verdict, HookVerdict::NoOpinion);
    assert_eq!(merged.verdict_binding, None);
    assert_eq!(merged.unsupported.len(), 1);
    assert_eq!(merged.unsupported[0].effect, EffectKind::Verdict);

    let merged = reduce_effects(vec![
        (binding("a", 0), effects(HookVerdict::Allow)),
        (binding("a", 1), effects(HookVerdict::Defer)),
    ]);
    assert_eq!(merged.verdict, HookVerdict::Allow);
    assert_eq!(merged.unsupported.len(), 1);
}

#[test]
fn reduce_carries_each_bindings_unsupported_effects_in_order() {
    let unsupported = |index: usize, effect: EffectKind| UnsupportedEffect {
        binding: binding("a", index),
        effect,
        reason: format!("ignored-{index}"),
    };
    let merged = reduce_effects(vec![
        (
            binding("a", 0),
            HookEffects {
                unsupported: vec![unsupported(0, EffectKind::Verdict)],
                ..Default::default()
            },
        ),
        (binding("a", 1), effects(HookVerdict::Defer)),
        (
            binding("a", 2),
            HookEffects {
                unsupported: vec![unsupported(2, EffectKind::Continuation)],
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(merged.verdict, HookVerdict::NoOpinion);
    assert_eq!(merged.unsupported.len(), 3);
    assert_eq!(merged.unsupported[0], unsupported(0, EffectKind::Verdict));
    assert_eq!(merged.unsupported[1].binding, binding("a", 1));
    assert_eq!(merged.unsupported[1].effect, EffectKind::Verdict);
    assert_eq!(
        merged.unsupported[2],
        unsupported(2, EffectKind::Continuation)
    );
}

#[test]
fn reduce_of_nothing_is_no_opinion() {
    assert_eq!(reduce_effects(Vec::new()), MergedEffects::no_opinion());
}

#[test]
fn validate_warns_for_rewrite_capability_on_tool_pre() {
    let mut capabilities = verdict_and_notice_capabilities();
    capabilities
        .may_produce
        .insert(EffectKind::ReplacementInput);
    let warnings = validate_capabilities(HookBoundary::ToolPre, &capabilities);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].effect, EffectKind::ReplacementInput);
    assert_eq!(warnings[0].boundary, HookBoundary::ToolPre);
}

#[test]
fn validate_warns_for_result_replacement_and_model_context() {
    let capabilities = BindingCapabilities {
        may_produce: HashSet::from([EffectKind::ReplacementResult, EffectKind::ModelContext]),
        ..verdict_and_notice_capabilities()
    };
    let warnings = validate_capabilities(HookBoundary::ToolResult, &capabilities);
    let effects = warnings.iter().map(|w| w.effect).collect::<HashSet<_>>();
    assert_eq!(
        effects,
        HashSet::from([EffectKind::ReplacementResult, EffectKind::ModelContext])
    );
}

#[test]
fn validate_is_silent_for_notice_only_capabilities() {
    let capabilities = BindingCapabilities {
        may_produce: HashSet::from([EffectKind::UserNotice]),
        ..verdict_and_notice_capabilities()
    };
    assert!(validate_capabilities(HookBoundary::ToolPre, &capabilities).is_empty());
    assert!(validate_capabilities(HookBoundary::ResponseComplete, &capabilities).is_empty());
}

#[test]
fn validate_warns_for_a_verdict_on_a_boundary_that_cannot_block() {
    let capabilities = verdict_and_notice_capabilities();
    assert!(validate_capabilities(HookBoundary::ToolPre, &capabilities).is_empty());
    let warnings = validate_capabilities(HookBoundary::ToolResult, &capabilities);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].effect, EffectKind::Verdict);
}

#[test]
fn phase_1_boundaries_are_the_eight_the_loop_fires() {
    let firing = [
        HookBoundary::SessionStart,
        HookBoundary::SessionEnd,
        HookBoundary::PromptSubmit,
        HookBoundary::ToolPre,
        HookBoundary::ToolPermissionRequest,
        HookBoundary::ToolResult,
        HookBoundary::ToolFailure,
        HookBoundary::FileExposure,
        HookBoundary::SubagentStart,
        HookBoundary::SubagentStop,
        HookBoundary::TurnStop,
        HookBoundary::TurnInterrupt,
        HookBoundary::CompactionPre,
        HookBoundary::CompactionPost,
        HookBoundary::ResponseComplete,
        HookBoundary::ReasoningComplete,
    ]
    .into_iter()
    .filter(HookBoundary::fires_in_phase_1)
    .collect::<Vec<_>>();
    assert_eq!(
        firing,
        vec![
            HookBoundary::SessionStart,
            HookBoundary::PromptSubmit,
            HookBoundary::ToolPre,
            HookBoundary::ToolResult,
            HookBoundary::ToolFailure,
            HookBoundary::TurnStop,
            HookBoundary::ResponseComplete,
            HookBoundary::ReasoningComplete,
        ]
    );
}

#[test]
fn default_timeouts_follow_the_boundary_class() {
    assert_eq!(
        HookBoundary::ToolPre.default_timeout(),
        Duration::from_secs(30)
    );
    assert_eq!(
        HookBoundary::ToolResult.default_timeout(),
        Duration::from_secs(60)
    );
    assert_eq!(
        HookBoundary::SessionEnd.default_timeout(),
        Duration::from_secs(5)
    );
}

#[test]
fn runs_and_hidden_children_are_headless_and_interactive_surfaces_are_not() {
    let headless = [
        HookSurface::OzRun,
        HookSurface::CliRun,
        HookSurface::LocalChild,
        HookSurface::Gui,
        HookSurface::Tui,
    ]
    .into_iter()
    .filter(HookSurface::is_headless)
    .collect::<Vec<_>>();
    assert_eq!(
        headless,
        vec![
            HookSurface::OzRun,
            HookSurface::CliRun,
            HookSurface::LocalChild
        ]
    );
}

#[test]
fn source_kind_priority_follows_the_documented_order() {
    let mut kinds = [
        HookSourceKind::ProjectCursorFile,
        HookSourceKind::RunDelivered,
        HookSourceKind::FactoryWide,
        HookSourceKind::FactoryAgent,
    ];
    kinds.sort_by_key(HookSourceKind::priority);
    assert_eq!(
        kinds,
        [
            HookSourceKind::FactoryAgent,
            HookSourceKind::FactoryWide,
            HookSourceKind::RunDelivered,
            HookSourceKind::ProjectCursorFile,
        ]
    );
    let factory = test_source_id_of_kind("agent", HookSourceKind::FactoryAgent);
    assert_eq!(factory.to_string(), "agent");
}
