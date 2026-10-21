use std::collections::HashSet;
use std::time::Duration;

use strum::EnumCount as _;

use super::*;
use crate::hooks::adapters::cursor::test_support::{FULL_CONFIG, bindings_from};
use crate::hooks::model::{PRE_EXECUTION_TIMEOUT, SESSION_END_TIMEOUT};

fn every_event() -> impl Iterator<Item = CursorEvent> {
    CURSOR_EVENTS.iter().map(|spec| spec.event)
}

/// Cursor's 21 event names, as the headings of research/01 §3 spell them.
const CURSOR_NAMES: [&str; 21] = [
    "sessionStart",
    "sessionEnd",
    "preToolUse",
    "postToolUse",
    "postToolUseFailure",
    "subagentStart",
    "subagentStop",
    "beforeShellExecution",
    "afterShellExecution",
    "beforeMCPExecution",
    "afterMCPExecution",
    "beforeReadFile",
    "afterFileEdit",
    "beforeSubmitPrompt",
    "preCompact",
    "stop",
    "afterAgentResponse",
    "afterAgentThought",
    "beforeTabFileRead",
    "afterTabFileEdit",
    "workspaceOpen",
];

const PERMISSION_HOOKS: [CursorEvent; 6] = [
    CursorEvent::BeforeShellExecution,
    CursorEvent::BeforeMcpExecution,
    CursorEvent::BeforeReadFile,
    CursorEvent::BeforeTabFileRead,
    CursorEvent::SubagentStart,
    CursorEvent::PreToolUse,
];

#[test]
fn the_table_lists_every_event_once_in_order_under_its_cursor_name() {
    assert_eq!(CURSOR_EVENTS.len(), CursorEvent::COUNT);
    assert_eq!(CURSOR_EVENTS.len(), CURSOR_NAMES.len());
    assert_eq!(
        every_event().collect::<HashSet<_>>().len(),
        CursorEvent::COUNT,
        "no event has two table entries"
    );
    for (spec, name) in CURSOR_EVENTS.iter().zip(CURSOR_NAMES) {
        assert_eq!(spec.name, name);
        assert_eq!(CursorEvent::from_name(spec.name), Some(spec.event));
        assert_eq!(spec.event.name(), spec.name);
        assert!(std::ptr::eq(spec.event.spec(), spec));
    }
    assert_eq!(CursorEvent::from_name("postToolUseFailed"), None);
    assert_eq!(CursorEvent::from_name("PreToolUse"), None);
}

#[test]
fn boundaries_follow_the_architecture_mapping() {
    let expected = [
        (CursorEvent::SessionStart, Some(HookBoundary::SessionStart)),
        (CursorEvent::SessionEnd, Some(HookBoundary::SessionEnd)),
        (CursorEvent::PreToolUse, Some(HookBoundary::ToolPre)),
        (CursorEvent::PostToolUse, Some(HookBoundary::ToolResult)),
        (
            CursorEvent::PostToolUseFailure,
            Some(HookBoundary::ToolFailure),
        ),
        (
            CursorEvent::SubagentStart,
            Some(HookBoundary::SubagentStart),
        ),
        (CursorEvent::SubagentStop, Some(HookBoundary::SubagentStop)),
        (
            CursorEvent::BeforeShellExecution,
            Some(HookBoundary::ToolPre),
        ),
        (
            CursorEvent::AfterShellExecution,
            Some(HookBoundary::ToolResult),
        ),
        (CursorEvent::BeforeMcpExecution, Some(HookBoundary::ToolPre)),
        (
            CursorEvent::AfterMcpExecution,
            Some(HookBoundary::ToolResult),
        ),
        (
            CursorEvent::BeforeReadFile,
            Some(HookBoundary::FileExposure),
        ),
        (CursorEvent::AfterFileEdit, Some(HookBoundary::ToolResult)),
        (
            CursorEvent::BeforeSubmitPrompt,
            Some(HookBoundary::PromptSubmit),
        ),
        (CursorEvent::PreCompact, Some(HookBoundary::CompactionPre)),
        (CursorEvent::Stop, Some(HookBoundary::TurnStop)),
        (
            CursorEvent::AfterAgentResponse,
            Some(HookBoundary::ResponseComplete),
        ),
        (
            CursorEvent::AfterAgentThought,
            Some(HookBoundary::ReasoningComplete),
        ),
        (CursorEvent::BeforeTabFileRead, None),
        (CursorEvent::AfterTabFileEdit, None),
        (CursorEvent::WorkspaceOpen, None),
    ];
    for (event, boundary) in expected {
        assert_eq!(event.spec().boundary, boundary, "{}", event.name());
    }
}

#[test]
fn twelve_events_fire_in_phase_1_and_the_rest_never_match() {
    let fires = every_event()
        .filter(|event| event.spec().fires_in_phase_1)
        .collect::<Vec<_>>();
    assert_eq!(
        fires,
        vec![
            CursorEvent::SessionStart,
            CursorEvent::PreToolUse,
            CursorEvent::PostToolUse,
            CursorEvent::PostToolUseFailure,
            CursorEvent::BeforeShellExecution,
            CursorEvent::AfterShellExecution,
            CursorEvent::BeforeMcpExecution,
            CursorEvent::AfterMcpExecution,
            CursorEvent::BeforeSubmitPrompt,
            CursorEvent::Stop,
            CursorEvent::AfterAgentResponse,
            CursorEvent::AfterAgentThought,
        ]
    );
    // A boundary that fires does not make its Phase 2 facet fire.
    assert_eq!(
        CursorEvent::AfterFileEdit.spec().boundary,
        Some(HookBoundary::ToolResult)
    );
    assert!(!CursorEvent::AfterFileEdit.spec().fires_in_phase_1);
    for spec in &CURSOR_EVENTS {
        if let Some(boundary) = spec.boundary {
            assert!(
                boundary.fires_in_phase_1() || !spec.fires_in_phase_1,
                "{} cannot fire on a boundary that does not",
                spec.name
            );
        } else {
            assert!(!spec.fires_in_phase_1, "{}", spec.name);
        }
    }
}

#[test]
fn only_the_six_permission_hooks_deny_on_malformed_output() {
    let permission_hooks = HashSet::from(PERMISSION_HOOKS);
    for spec in &CURSOR_EVENTS {
        let expected = if permission_hooks.contains(&spec.event) {
            MalformedOutputPolicy::Deny
        } else {
            MalformedOutputPolicy::FollowFailurePolicy
        };
        assert_eq!(spec.malformed_output, expected, "{}", spec.name);
    }
    assert_eq!(
        CursorEvent::BeforeSubmitPrompt.spec().malformed_output,
        MalformedOutputPolicy::FollowFailurePolicy,
        "beforeSubmitPrompt follows failClosed because Cursor's rule is unstated"
    );
}

#[test]
fn empty_output_is_a_failure_only_where_the_hook_decides_something() {
    for spec in &CURSOR_EVENTS {
        let expected = match spec.control() {
            ControlEffect::Verdict | ControlEffect::HaltTurn => {
                EmptyOutputPolicy::FollowFailurePolicy
            }
            ControlEffect::ObserveOnly => EmptyOutputPolicy::NoOpinion,
        };
        assert_eq!(spec.empty_output, expected, "{}", spec.name);
    }
}

#[test]
fn control_effects_follow_the_documented_output() {
    for event in PERMISSION_HOOKS {
        assert_eq!(
            event.spec().control(),
            ControlEffect::Verdict,
            "{}",
            event.name()
        );
    }
    assert_eq!(
        CursorEvent::BeforeSubmitPrompt.spec().control(),
        ControlEffect::HaltTurn
    );
    for event in [
        CursorEvent::SessionStart,
        CursorEvent::SessionEnd,
        CursorEvent::PostToolUse,
        CursorEvent::PostToolUseFailure,
        CursorEvent::SubagentStop,
        CursorEvent::AfterShellExecution,
        CursorEvent::AfterMcpExecution,
        CursorEvent::AfterFileEdit,
        CursorEvent::PreCompact,
        CursorEvent::Stop,
        CursorEvent::AfterAgentResponse,
        CursorEvent::AfterAgentThought,
        CursorEvent::AfterTabFileEdit,
        CursorEvent::WorkspaceOpen,
    ] {
        assert_eq!(
            event.spec().control(),
            ControlEffect::ObserveOnly,
            "{}",
            event.name()
        );
    }
    assert!(CursorEvent::Stop.spec().may_continue());
    assert!(CursorEvent::SubagentStop.spec().may_continue());
    assert!(!CursorEvent::PreToolUse.spec().may_continue());
}

#[test]
fn matcher_targets_follow_the_cursor_table() {
    let expected = [
        (CursorEvent::PreToolUse, MatchedValue::ToolName),
        (CursorEvent::PostToolUse, MatchedValue::ToolName),
        (CursorEvent::PostToolUseFailure, MatchedValue::ToolName),
        (CursorEvent::SubagentStart, MatchedValue::SubagentType),
        (CursorEvent::SubagentStop, MatchedValue::SubagentType),
        (CursorEvent::BeforeShellExecution, MatchedValue::CommandText),
        (CursorEvent::AfterShellExecution, MatchedValue::CommandText),
        (CursorEvent::BeforeMcpExecution, MatchedValue::McpToolName),
        (CursorEvent::AfterMcpExecution, MatchedValue::McpToolName),
        (CursorEvent::BeforeReadFile, MatchedValue::Literal("Read")),
        (CursorEvent::AfterFileEdit, MatchedValue::Literal("Write")),
        (
            CursorEvent::BeforeTabFileRead,
            MatchedValue::Literal("TabRead"),
        ),
        (
            CursorEvent::AfterTabFileEdit,
            MatchedValue::Literal("TabWrite"),
        ),
        (
            CursorEvent::BeforeSubmitPrompt,
            MatchedValue::Literal("UserPromptSubmit"),
        ),
        (CursorEvent::Stop, MatchedValue::Literal("Stop")),
        (
            CursorEvent::AfterAgentResponse,
            MatchedValue::Literal("AgentResponse"),
        ),
        (
            CursorEvent::AfterAgentThought,
            MatchedValue::Literal("AgentThought"),
        ),
        (CursorEvent::SessionStart, MatchedValue::Everything),
        (CursorEvent::SessionEnd, MatchedValue::Everything),
        (CursorEvent::PreCompact, MatchedValue::Everything),
        (CursorEvent::WorkspaceOpen, MatchedValue::Everything),
    ];
    assert_eq!(expected.len(), 21);
    for (event, value) in expected {
        assert_eq!(event.spec().matched_value, value, "{}", event.name());
    }
    assert_eq!(
        CursorEvent::BeforeMcpExecution.spec().matcher_target(),
        MatcherTarget::ToolName
    );
    assert_eq!(
        CursorEvent::Stop.spec().matcher_target(),
        MatcherTarget::Literal("Stop".to_owned())
    );
    assert_eq!(
        CursorEvent::SessionStart.spec().matcher_target(),
        MatcherTarget::None
    );
    assert!(!CursorEvent::SessionStart.spec().has_matcher_target());
    assert!(CursorEvent::Stop.spec().has_matcher_target());
}

#[test]
fn capabilities_carry_the_definitions_fail_closed_choice() {
    let open = CursorEvent::BeforeShellExecution.spec().capabilities(false);
    assert_eq!(open.failure, FailurePolicy::FailOpen);
    assert_eq!(open.malformed_output, MalformedOutputPolicy::Deny);
    assert_eq!(open.empty_output, EmptyOutputPolicy::FollowFailurePolicy);
    assert_eq!(open.matcher_target, MatcherTarget::CommandText);
    assert_eq!(open.handler, HandlerKind::Command);
    assert_eq!(
        open.may_produce,
        HashSet::from([EffectKind::Verdict, EffectKind::UserNotice])
    );

    let closed = CursorEvent::BeforeShellExecution.spec().capabilities(true);
    assert_eq!(closed.failure, FailurePolicy::FailClosed);

    let pre_tool = CursorEvent::PreToolUse.spec().capabilities(false);
    assert!(pre_tool.may_produce.contains(&EffectKind::ReplacementInput));
    let post_tool = CursorEvent::PostToolUse.spec().capabilities(false);
    assert_eq!(
        post_tool.may_produce,
        HashSet::from([EffectKind::ReplacementResult, EffectKind::ModelContext])
    );
    let session_start = CursorEvent::SessionStart.spec().capabilities(false);
    assert_eq!(
        session_start.may_produce,
        HashSet::from([EffectKind::SessionEnv, EffectKind::ModelContext])
    );
    assert!(
        CursorEvent::AfterShellExecution
            .spec()
            .capabilities(false)
            .may_produce
            .is_empty()
    );
}

#[test]
fn default_timeouts_come_from_the_boundary() {
    assert_eq!(
        CursorEvent::BeforeShellExecution.spec().default_timeout(),
        PRE_EXECUTION_TIMEOUT
    );
    assert_eq!(
        CursorEvent::SessionEnd.spec().default_timeout(),
        SESSION_END_TIMEOUT
    );
    assert_eq!(
        CursorEvent::AfterShellExecution.spec().default_timeout(),
        POST_AND_OTHER_TIMEOUT
    );
    assert_eq!(
        CursorEvent::WorkspaceOpen.spec().default_timeout(),
        POST_AND_OTHER_TIMEOUT
    );
}

#[test]
fn the_full_config_declares_a_binding_for_every_event_with_a_boundary() {
    let bindings = bindings_from(FULL_CONFIG);
    let events = bindings
        .iter()
        .map(|binding| binding.vendor_event.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        events,
        CURSOR_NAMES[..18].to_vec(),
        "table order, without the three events that have no boundary"
    );
    for binding in &bindings {
        let spec = CursorEvent::from_name(&binding.vendor_event)
            .expect("declared under a known name")
            .spec();
        assert_eq!(binding.id.event, binding.vendor_event);
        assert_eq!(Some(binding.boundary), spec.boundary);
        assert_eq!(binding.capabilities.matcher_target, spec.matcher_target());
    }

    let by_event = |event: CursorEvent| {
        bindings
            .iter()
            .find(|binding| binding.vendor_event == event.name())
            .expect("declared")
    };
    let pre_tool = by_event(CursorEvent::PreToolUse);
    assert_eq!(pre_tool.timeout, Duration::from_secs(20));
    assert_eq!(pre_tool.capabilities.failure, FailurePolicy::FailClosed);
    assert_eq!(pre_tool.matcher.as_deref(), Some("Shell|Write"));
    assert_eq!(pre_tool.loop_limit, None);
    assert_eq!(pre_tool.command, "./hooks/pre-tool.sh");

    let after_shell = by_event(CursorEvent::AfterShellExecution);
    assert_eq!(after_shell.timeout, POST_AND_OTHER_TIMEOUT);
    assert_eq!(after_shell.capabilities.failure, FailurePolicy::FailOpen);

    assert_eq!(by_event(CursorEvent::SubagentStop).loop_limit, Some(3));
    assert_eq!(
        by_event(CursorEvent::Stop).loop_limit,
        Some(LOOP_LIMIT_HOST_CAP),
        "loop_limit null is capped"
    );
    assert_eq!(by_event(CursorEvent::Stop).timeout, Duration::from_secs(15));
    assert_eq!(
        by_event(CursorEvent::SessionEnd).timeout,
        Duration::from_secs(3)
    );

    let shell = by_event(CursorEvent::BeforeShellExecution);
    assert_eq!(shell.id.index, 0, "the prompt hook after it is skipped");
    assert_eq!(
        bindings
            .iter()
            .filter(|binding| binding.vendor_event == "beforeShellExecution")
            .count(),
        1
    );
}

#[test]
fn declared_bindings_keep_array_positions() {
    let bindings = bindings_from(
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [
            { "command": "./a.sh", "matcher": "rm", "timeout": 3, "failClosed": true },
            { "command": "./b.sh" }
        ] } }"#,
    );
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0].id.index, 0);
    assert_eq!(bindings[0].command, "./a.sh");
    assert_eq!(bindings[0].matcher.as_deref(), Some("rm"));
    assert_eq!(bindings[0].timeout, Duration::from_secs(3));
    assert_eq!(
        bindings[0].capabilities,
        CursorEvent::BeforeShellExecution.spec().capabilities(true)
    );
    assert_eq!(bindings[1].id.index, 1);
    assert_eq!(bindings[1].command, "./b.sh");
    assert_eq!(
        bindings[1].capabilities,
        CursorEvent::BeforeShellExecution.spec().capabilities(false)
    );
}
