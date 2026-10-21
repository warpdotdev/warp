use serde_json::json;

use super::*;
use crate::hooks::adapters::cursor::test_support::{
    CLAUDE_SHAPE_OUTPUT, EVENT_FIXTURES, binding_for, outcome,
};

const STRICT: DecodePolicy = DecodePolicy { strict: true };
const DEFAULT: DecodePolicy = DecodePolicy { strict: false };

fn decode(event: CursorEvent, fail_closed: bool, exit: ProcessExit, stdout: &str) -> HookEffects {
    decode_with(&binding_for(event, fail_closed), exit, stdout, &DEFAULT)
}

fn decode_with(
    binding: &DeclaredBinding,
    exit: ProcessExit,
    stdout: &str,
    policy: &DecodePolicy,
) -> HookEffects {
    let event = CursorEvent::from_name(&binding.vendor_event).expect("a Cursor event");
    decode_output(event.spec(), binding, &outcome(exit, stdout), policy)
}

fn reasons(effects: &HookEffects) -> Vec<&str> {
    effects.reasons.iter().map(|r| r.text.as_str()).collect()
}

const OK: ProcessExit = ProcessExit::Code(0);

/// The Phase 1 events, grouped by what a hook on them can do.
const PERMISSION_EVENTS: [CursorEvent; 3] = [
    CursorEvent::PreToolUse,
    CursorEvent::BeforeShellExecution,
    CursorEvent::BeforeMcpExecution,
];
const OBSERVER_EVENTS: [CursorEvent; 7] = [
    CursorEvent::SessionStart,
    CursorEvent::PostToolUse,
    CursorEvent::PostToolUseFailure,
    CursorEvent::AfterShellExecution,
    CursorEvent::AfterMcpExecution,
    CursorEvent::AfterAgentResponse,
    CursorEvent::AfterAgentThought,
];

fn subject(event: CursorEvent) -> &'static str {
    event.spec().subject
}

// Exit codes and process failures, for every Phase 1 event.

#[test]
fn exit_two_blocks_where_the_event_can_block_and_is_a_failure_elsewhere() {
    for event in PERMISSION_EVENTS {
        let effects = decode(event, false, ProcessExit::Code(2), "");
        assert_eq!(effects.verdict, HookVerdict::Deny, "{}", event.name());
        assert_eq!(
            reasons(&effects),
            vec!["Denied by hook ./check.sh (exit 2)."],
            "{}",
            event.name()
        );
        assert_eq!(
            effects.reasons[0].binding,
            Some(binding_for(event, false).id)
        );
    }

    let halted = decode(CursorEvent::BeforeSubmitPrompt, false, ProcessExit::Code(2), "");
    assert!(halted.halt_turn);
    assert_eq!(halted.verdict, HookVerdict::NoOpinion);
    assert_eq!(reasons(&halted), vec!["Blocked by hook ./check.sh (exit 2)."]);
    let halted = decode(
        CursorEvent::BeforeSubmitPrompt,
        false,
        ProcessExit::Code(2),
        r#"{"user_message":"No secrets in prompts."}"#,
    );
    assert!(halted.halt_turn);
    assert_eq!(reasons(&halted), vec!["No secrets in prompts."]);

    for event in OBSERVER_EVENTS.into_iter().chain([CursorEvent::Stop]) {
        assert_eq!(
            decode(event, false, ProcessExit::Code(2), ""),
            HookEffects::default(),
            "{} fails open on exit 2",
            event.name()
        );
        assert_eq!(
            decode(event, true, ProcessExit::Code(2), ""),
            HookEffects::default(),
            "{} has nothing to close on exit 2",
            event.name()
        );
    }
}

#[test]
fn exit_two_still_reads_the_messages() {
    let effects = decode(
        CursorEvent::BeforeShellExecution,
        false,
        ProcessExit::Code(2),
        r#"{"agent_message":"rm is not allowed here","user_message":"Blocked by policy"}"#,
    );
    assert_eq!(effects.verdict, HookVerdict::Deny);
    assert_eq!(reasons(&effects), vec!["rm is not allowed here"]);
    assert_eq!(effects.user_notices, vec!["Blocked by policy"]);

    let unreadable = decode(CursorEvent::PreToolUse, false, ProcessExit::Code(2), "not json");
    assert_eq!(unreadable.verdict, HookVerdict::Deny);
    assert_eq!(
        reasons(&unreadable),
        vec!["Denied by hook ./check.sh (exit 2)."]
    );
}

#[test]
fn other_failures_follow_the_failure_policy() {
    let failures = [
        (ProcessExit::Code(1), "exited with code 1"),
        (ProcessExit::Code(127), "exited with code 127"),
        (ProcessExit::Signal(9), "was killed by signal 9"),
        (ProcessExit::TimedOut, "timed out"),
        (
            ProcessExit::SpawnFailed("no such file".to_owned()),
            "could not start: no such file",
        ),
    ];
    for (exit, description) in &failures {
        for event in PERMISSION_EVENTS {
            let open = decode(event, false, exit.clone(), r#"{"permission":"deny"}"#);
            assert_eq!(open, HookEffects::default(), "{} fail open on {exit:?}", event.name());
            let closed = decode(event, true, exit.clone(), "");
            assert_eq!(closed.verdict, HookVerdict::Deny, "{} on {exit:?}", event.name());
            assert_eq!(
                reasons(&closed),
                vec![format!(
                    "Hook ./check.sh {description}, so {} was denied.",
                    subject(event)
                )]
            );
        }

        let open = decode(CursorEvent::BeforeSubmitPrompt, false, exit.clone(), "");
        assert_eq!(open, HookEffects::default());
        let closed = decode(CursorEvent::BeforeSubmitPrompt, true, exit.clone(), "");
        assert!(closed.halt_turn, "{exit:?}");
        assert_eq!(closed.verdict, HookVerdict::NoOpinion);
        assert_eq!(
            reasons(&closed),
            vec![format!(
                "Hook ./check.sh {description}, so the prompt was not sent."
            )]
        );

        for event in OBSERVER_EVENTS.into_iter().chain([CursorEvent::Stop]) {
            for fail_closed in [false, true] {
                assert_eq!(
                    decode(event, fail_closed, exit.clone(), r#"{"permission":"deny"}"#),
                    HookEffects::default(),
                    "{} has nothing to close on {exit:?}",
                    event.name()
                );
            }
        }
    }
}

#[test]
fn a_cancelled_hook_blocks_where_it_could_have_blocked_regardless_of_policy() {
    for fail_closed in [false, true] {
        for event in PERMISSION_EVENTS {
            let effects = decode(event, fail_closed, ProcessExit::Cancelled, "");
            assert_eq!(effects.verdict, HookVerdict::Deny, "{}", event.name());
            assert_eq!(
                reasons(&effects),
                vec![format!(
                    "Hook ./check.sh was cancelled before it finished, so {} was denied.",
                    subject(event)
                )]
            );
        }
        let halted = decode(
            CursorEvent::BeforeSubmitPrompt,
            fail_closed,
            ProcessExit::Cancelled,
            "",
        );
        assert!(halted.halt_turn);
        for event in OBSERVER_EVENTS.into_iter().chain([CursorEvent::Stop]) {
            assert_eq!(
                decode(event, fail_closed, ProcessExit::Cancelled, ""),
                HookEffects::default(),
                "{}",
                event.name()
            );
        }
    }
}

// Empty and malformed output.

#[test]
fn empty_output_follows_the_binding_policy() {
    for stdout in ["", "  \n"] {
        for event in PERMISSION_EVENTS.into_iter().chain([CursorEvent::BeforeSubmitPrompt]) {
            let open = decode(event, false, OK, stdout);
            assert_eq!(open, HookEffects::default(), "{} fail open", event.name());
            let closed = decode(event, true, OK, stdout);
            assert!(
                closed.verdict == HookVerdict::Deny || closed.halt_turn,
                "{} fail closed",
                event.name()
            );
            assert!(reasons(&closed)[0].starts_with("Hook ./check.sh printed no decision, so"));
        }
        for event in OBSERVER_EVENTS.into_iter().chain([CursorEvent::Stop]) {
            for fail_closed in [false, true] {
                assert_eq!(
                    decode(event, fail_closed, OK, stdout),
                    HookEffects::default(),
                    "{} has no decision to miss",
                    event.name()
                );
            }
        }
    }

    // The policies can be swapped on a binding, so each is exercised on both kinds of event.
    let mut shell = binding_for(CursorEvent::BeforeShellExecution, true);
    shell.capabilities.empty_output = EmptyOutputPolicy::NoOpinion;
    assert_eq!(
        decode_with(&shell, OK, "", &DEFAULT),
        HookEffects::default()
    );
    let mut stop = binding_for(CursorEvent::Stop, true);
    stop.capabilities.empty_output = EmptyOutputPolicy::FollowFailurePolicy;
    assert_eq!(
        decode_with(&stop, OK, "", &DEFAULT),
        HookEffects::default(),
        "a stop hook cannot be closed, so the policy changes nothing"
    );
}

#[test]
fn malformed_output_denies_on_permission_hooks_even_when_they_fail_open() {
    for stdout in [
        "not json",
        "[1, 2]",
        r#""allow""#,
        r#"{"permission": 1}"#,
        r#"{"permission": "maybe"}"#,
        r#"{"user_message": "no decision here"}"#,
        r#"{"permission": "allow", "user_message": 5}"#,
        "{}",
    ] {
        for event in PERMISSION_EVENTS {
            let effects = decode(event, false, OK, stdout);
            assert_eq!(effects.verdict, HookVerdict::Deny, "{} on {stdout:?}", event.name());
            assert_eq!(
                reasons(&effects),
                vec![format!(
                    "Hook ./check.sh printed output that is not a valid permission decision, \
                     so {} was denied.",
                    subject(event)
                )],
                "{} on {stdout:?}",
                event.name()
            );
        }
    }
}

#[test]
fn malformed_output_elsewhere_follows_the_failure_policy() {
    for stdout in ["not json", "[1]", r#"{"continue": "yes"}"#, "{}"] {
        let open = decode(CursorEvent::BeforeSubmitPrompt, false, OK, stdout);
        assert_eq!(open, HookEffects::default(), "{stdout:?}");
        let closed = decode(CursorEvent::BeforeSubmitPrompt, true, OK, stdout);
        assert!(closed.halt_turn, "{stdout:?}");
        assert_eq!(
            reasons(&closed),
            vec![
                "Hook ./check.sh printed output that is not a valid response, so the prompt was \
                 not sent."
            ]
        );
    }
    for stdout in ["not json", r#"{"additional_context": 5}"#, r#"{"env": "x"}"#] {
        for event in OBSERVER_EVENTS.into_iter().chain([CursorEvent::Stop]) {
            for fail_closed in [false, true] {
                assert_eq!(
                    decode(event, fail_closed, OK, stdout),
                    HookEffects::default(),
                    "{} on {stdout:?}",
                    event.name()
                );
            }
        }
    }

    // The policies can be swapped on a binding.
    let mut shell = binding_for(CursorEvent::BeforeShellExecution, false);
    shell.capabilities.malformed_output = MalformedOutputPolicy::FollowFailurePolicy;
    assert_eq!(
        decode_with(&shell, OK, "not json", &DEFAULT),
        HookEffects::default()
    );
    let mut prompt = binding_for(CursorEvent::BeforeSubmitPrompt, false);
    prompt.capabilities.malformed_output = MalformedOutputPolicy::Deny;
    assert!(decode_with(&prompt, OK, "not json", &DEFAULT).halt_turn);
}

#[test]
fn stdout_that_is_not_utf8_is_malformed() {
    let binding = binding_for(CursorEvent::BeforeShellExecution, false);
    let outcome = HookProcessOutcome {
        exit: ProcessExit::Code(0),
        stdout: b"{\"permission\":\"allow\xff\"}".to_vec(),
        stderr_tail: String::new(),
        duration: std::time::Duration::from_millis(1),
    };
    let effects = decode_output(
        CursorEvent::BeforeShellExecution.spec(),
        &binding,
        &outcome,
        &DEFAULT,
    );
    assert_eq!(effects.verdict, HookVerdict::Deny);
}

// Permission decisions.

#[test]
fn allow_deny_and_their_messages_decode_on_every_permission_event() {
    for event in PERMISSION_EVENTS {
        let allow = decode(event, false, OK, r#"{"permission":"allow"}"#);
        assert_eq!(allow.verdict, HookVerdict::Allow, "{}", event.name());
        assert!(allow.reasons.is_empty());

        let deny = decode(
            event,
            false,
            OK,
            r#"{"permission":"deny","agent_message":"no","user_message":"Blocked"}"#,
        );
        assert_eq!(deny.verdict, HookVerdict::Deny);
        assert_eq!(reasons(&deny), vec!["no"]);
        assert_eq!(deny.user_notices, vec!["Blocked"]);

        let silent_deny = decode(event, false, OK, r#"{"permission":"deny"}"#);
        assert_eq!(reasons(&silent_deny), vec!["Denied by hook ./check.sh."]);

        let user_only = decode(
            event,
            false,
            OK,
            r#"{"permission":"deny","user_message":"Blocked by policy"}"#,
        );
        assert_eq!(reasons(&user_only), vec!["Blocked by policy"]);
        assert!(user_only.user_notices.is_empty());

        let allow_with_messages = decode(
            event,
            false,
            OK,
            r#"{"permission":"allow","user_message":"ok","agent_message":"remember to lint"}"#,
        );
        assert_eq!(allow_with_messages.verdict, HookVerdict::Allow);
        assert_eq!(allow_with_messages.user_notices, vec!["ok"]);
        assert!(
            allow_with_messages.model_context.is_empty(),
            "agent_message has no documented meaning outside a denial"
        );
    }
}

#[test]
fn ask_means_something_different_on_each_event() {
    let honored = decode(
        CursorEvent::BeforeShellExecution,
        false,
        OK,
        r#"{"permission":"ask","user_message":"Please confirm."}"#,
    );
    assert_eq!(honored.verdict, HookVerdict::Ask);
    assert_eq!(honored.user_notices, vec!["Please confirm."]);
    assert_eq!(
        decode(CursorEvent::BeforeMcpExecution, false, OK, r#"{"permission":"ask"}"#).verdict,
        HookVerdict::Ask
    );

    let ignored = decode(CursorEvent::PreToolUse, false, OK, r#"{"permission":"ask"}"#);
    assert_eq!(
        ignored.verdict,
        HookVerdict::NoOpinion,
        "Cursor does not enforce ask on preToolUse"
    );
    let strict = decode_with(
        &binding_for(CursorEvent::PreToolUse, false),
        OK,
        r#"{"permission":"ask"}"#,
        &STRICT,
    );
    assert_eq!(strict.verdict, HookVerdict::Ask);

    let subagent = decode(CursorEvent::SubagentStart, false, OK, r#"{"permission":"ask"}"#);
    assert_eq!(subagent.verdict, HookVerdict::Deny);
    assert_eq!(
        reasons(&subagent),
        vec!["Hook ./check.sh answered ask, which subagentStart treats as deny."]
    );
    let subagent_strict = decode_with(
        &binding_for(CursorEvent::SubagentStart, false),
        OK,
        r#"{"permission":"ask"}"#,
        &STRICT,
    );
    assert_eq!(
        subagent_strict.verdict,
        HookVerdict::Deny,
        "strict mode never weakens a vendor denial"
    );

    let read = decode(CursorEvent::BeforeReadFile, false, OK, r#"{"permission":"ask"}"#);
    assert_eq!(
        read.verdict,
        HookVerdict::Deny,
        "ask is not in beforeReadFile's schema"
    );
    assert!(reasons(&read)[0].contains("not a valid permission decision"));
}

#[test]
fn updated_input_becomes_a_replacement_only_on_pre_tool_use() {
    let effects = decode(
        CursorEvent::PreToolUse,
        false,
        OK,
        r#"{"permission":"allow","updated_input":{"command":"npm ci"}}"#,
    );
    assert_eq!(effects.verdict, HookVerdict::Allow);
    assert_eq!(effects.replacement_input, Some(json!({ "command": "npm ci" })));

    let not_an_object = decode(
        CursorEvent::PreToolUse,
        false,
        OK,
        r#"{"permission":"allow","updated_input":"npm ci"}"#,
    );
    assert_eq!(not_an_object.verdict, HookVerdict::Deny);

    for event in [
        CursorEvent::BeforeShellExecution,
        CursorEvent::BeforeMcpExecution,
    ] {
        let effects = decode(
            event,
            false,
            OK,
            r#"{"permission":"allow","updated_input":{"command":"npm ci"}}"#,
        );
        assert_eq!(effects.verdict, HookVerdict::Allow, "{}", event.name());
        assert_eq!(
            effects.replacement_input, None,
            "{} does not document updated_input",
            event.name()
        );
    }
}

#[test]
fn fields_an_event_does_not_document_are_ignored() {
    let shell = decode(
        CursorEvent::AfterShellExecution,
        true,
        OK,
        r#"{"permission":"deny","followup_message":"again","additional_context":"ctx"}"#,
    );
    assert_eq!(shell, HookEffects::default());

    let stop = decode(
        CursorEvent::Stop,
        true,
        OK,
        r#"{"permission":"deny","additional_context":"ctx"}"#,
    );
    assert_eq!(stop, HookEffects::default());

    let pre_tool = decode(
        CursorEvent::PreToolUse,
        false,
        OK,
        r#"{"permission":"allow","followup_message":"again","env":{"A":"1"}}"#,
    );
    assert_eq!(pre_tool.verdict, HookVerdict::Allow);
    assert!(pre_tool.continuation.is_none());
    assert!(pre_tool.session_env.is_empty());

    let session = decode(
        CursorEvent::SessionStart,
        false,
        OK,
        r#"{"continue":false,"user_message":"ignored","permission":"deny"}"#,
    );
    assert_eq!(
        session,
        HookEffects::default(),
        "sessionStart accepts continue and user_message and ignores them"
    );
}

// Context and environment.

#[test]
fn additional_context_is_model_context_where_documented() {
    for event in [
        CursorEvent::SessionStart,
        CursorEvent::PostToolUse,
        CursorEvent::PostToolUseFailure,
    ] {
        let effects = decode(
            event,
            false,
            OK,
            r#"{"additional_context":"Tests time out on CI runners."}"#,
        );
        assert_eq!(
            effects.model_context,
            vec!["Tests time out on CI runners."],
            "{}",
            event.name()
        );
        assert_eq!(effects.verdict, HookVerdict::NoOpinion);
    }
    for event in PERMISSION_EVENTS {
        let effects = decode(
            event,
            false,
            OK,
            r#"{"permission":"allow","additional_context":"ctx"}"#,
        );
        assert!(effects.model_context.is_empty(), "{}", event.name());
    }
}

#[test]
fn session_start_env_and_context_are_read() {
    let effects = decode(
        CursorEvent::SessionStart,
        false,
        OK,
        r#"{"env":{"CI":"1","PNPM_HOME":"/opt/pnpm"},"additional_context":"Use pnpm."}"#,
    );
    assert_eq!(
        effects.session_env,
        vec![
            ("CI".to_owned(), "1".to_owned()),
            ("PNPM_HOME".to_owned(), "/opt/pnpm".to_owned()),
        ]
    );
    assert_eq!(effects.model_context, vec!["Use pnpm."]);

    let bad_value = decode(
        CursorEvent::SessionStart,
        true,
        OK,
        r#"{"env":{"CI":1}}"#,
    );
    assert_eq!(
        bad_value,
        HookEffects::default(),
        "a failed sessionStart adds no environment"
    );
}

#[test]
fn post_tool_use_reads_the_mcp_output_replacement() {
    let effects = decode(
        CursorEvent::PostToolUse,
        false,
        OK,
        r#"{"updated_mcp_tool_output":{"modified":"output"},"additional_context":"Report attached."}"#,
    );
    assert_eq!(
        effects.replacement_result,
        Some(json!({ "modified": "output" }))
    );
    assert_eq!(effects.model_context, vec!["Report attached."]);
    assert_eq!(
        decode(
            CursorEvent::PostToolUse,
            false,
            OK,
            r#"{"updated_mcp_tool_output":"text"}"#,
        ),
        HookEffects::default()
    );
}

// Prompt submission and follow-ups.

#[test]
fn before_submit_prompt_halts_on_continue_false() {
    let halted = decode(
        CursorEvent::BeforeSubmitPrompt,
        false,
        OK,
        r#"{"continue":false,"user_message":"Prompts must not contain credentials."}"#,
    );
    assert!(halted.halt_turn);
    assert_eq!(halted.verdict, HookVerdict::NoOpinion);
    assert_eq!(
        reasons(&halted),
        vec!["Prompts must not contain credentials."]
    );
    assert!(halted.user_notices.is_empty());

    let silent = decode(CursorEvent::BeforeSubmitPrompt, false, OK, r#"{"continue":false}"#);
    assert_eq!(reasons(&silent), vec!["Blocked by hook ./check.sh."]);

    let proceed = decode(
        CursorEvent::BeforeSubmitPrompt,
        false,
        OK,
        r#"{"continue":true,"user_message":"Checked."}"#,
    );
    assert!(!proceed.halt_turn);
    assert_eq!(proceed.user_notices, vec!["Checked."]);
    assert!(proceed.reasons.is_empty());
}

#[test]
fn stop_reads_the_follow_up_with_the_bindings_loop_limit() {
    let effects = decode(
        CursorEvent::Stop,
        false,
        OK,
        r#"{"followup_message":"Now run the full test suite."}"#,
    );
    assert_eq!(
        effects.continuation,
        Some(ContinuationRequest {
            text: "Now run the full test suite.".to_owned(),
            loop_limit: Some(5),
        })
    );
    assert_eq!(effects.verdict, HookVerdict::NoOpinion);
    assert!(!effects.halt_turn);

    let mut capped = binding_for(CursorEvent::Stop, false);
    capped.loop_limit = Some(2);
    let effects = decode_with(
        &capped,
        OK,
        r#"{"followup_message":"again"}"#,
        &DEFAULT,
    );
    assert_eq!(
        effects.continuation.map(|c| c.loop_limit),
        Some(Some(2))
    );

    for stdout in [r#"{"followup_message":""}"#, r#"{"followup_message":"  "}"#, "{}"] {
        assert_eq!(
            decode(CursorEvent::Stop, false, OK, stdout),
            HookEffects::default(),
            "{stdout} asks for nothing"
        );
    }
    assert_eq!(
        decode(CursorEvent::SubagentStop, false, OK, r#"{"followup_message":"more"}"#)
            .continuation
            .map(|c| c.text),
        Some("more".to_owned())
    );
}

// Claude's output shape.

#[test]
fn claude_shaped_output_is_translated_on_the_events_cursor_documents() {
    let fixture: serde_json::Value = serde_json::from_str(CLAUDE_SHAPE_OUTPUT).expect("JSON");

    let pre_tool = decode(
        CursorEvent::PreToolUse,
        false,
        OK,
        &fixture["preToolUse"].to_string(),
    );
    assert_eq!(pre_tool.verdict, HookVerdict::Deny);
    assert_eq!(reasons(&pre_tool), vec!["Blocked by policy"]);
    assert_eq!(
        pre_tool.replacement_input,
        Some(json!({ "command": "npm ci" }))
    );

    let allow = decode(
        CursorEvent::PreToolUse,
        false,
        OK,
        r#"{"hookSpecificOutput":{"permissionDecision":"allow"}}"#,
    );
    assert_eq!(allow.verdict, HookVerdict::Allow);

    let flat_wins = decode(
        CursorEvent::PreToolUse,
        false,
        OK,
        r#"{"permission":"allow","hookSpecificOutput":{"permissionDecision":"deny"}}"#,
    );
    assert_eq!(flat_wins.verdict, HookVerdict::Allow);

    for key in ["stop", "stop_flat"] {
        let stop = decode(CursorEvent::Stop, false, OK, &fixture[key].to_string());
        assert_eq!(
            stop.continuation.map(|c| c.text),
            Some("Tasks incomplete, continue working".to_owned()),
            "{key}"
        );
        assert_eq!(stop.verdict, HookVerdict::NoOpinion, "{key}");
    }
    let approve = decode(
        CursorEvent::Stop,
        false,
        OK,
        r#"{"decision":"approve","reason":"all done"}"#,
    );
    assert_eq!(approve, HookEffects::default());
    let block_without_reason = decode(CursorEvent::Stop, false, OK, r#"{"decision":"block"}"#);
    assert_eq!(block_without_reason, HookEffects::default());

    let shell = decode(
        CursorEvent::BeforeShellExecution,
        false,
        OK,
        r#"{"hookSpecificOutput":{"permissionDecision":"allow"}}"#,
    );
    assert_eq!(
        shell.verdict,
        HookVerdict::Deny,
        "beforeShellExecution does not accept the Claude shape, so there is no decision"
    );
}

// Strict mode.

#[test]
fn strict_mode_turns_every_failure_of_a_blocking_hook_into_a_block() {
    for event in PERMISSION_EVENTS {
        let binding = binding_for(event, false);
        for (exit, stdout) in [
            (ProcessExit::Code(1), ""),
            (ProcessExit::TimedOut, ""),
            (ProcessExit::Code(0), ""),
            (ProcessExit::Code(0), "not json"),
        ] {
            let effects = decode_with(&binding, exit.clone(), stdout, &STRICT);
            assert_eq!(
                effects.verdict,
                HookVerdict::Deny,
                "{} under strict on {exit:?} {stdout:?}",
                event.name()
            );
        }
        let allow = decode_with(&binding, OK, r#"{"permission":"allow"}"#, &STRICT);
        assert_eq!(allow.verdict, HookVerdict::Allow);
    }

    let prompt = binding_for(CursorEvent::BeforeSubmitPrompt, false);
    assert!(decode_with(&prompt, ProcessExit::TimedOut, "", &STRICT).halt_turn);
    assert!(decode_with(&prompt, OK, "not json", &STRICT).halt_turn);

    for event in OBSERVER_EVENTS.into_iter().chain([CursorEvent::Stop]) {
        let binding = binding_for(event, false);
        assert_eq!(
            decode_with(&binding, ProcessExit::TimedOut, "", &STRICT),
            HookEffects::default(),
            "{} cannot block, so strict mode changes nothing",
            event.name()
        );
    }
}

// Text handling and fixtures.

#[test]
fn control_characters_are_removed_from_messages() {
    let effects = decode(
        CursorEvent::BeforeShellExecution,
        false,
        ProcessExit::Code(2),
        "{\"agent_message\":\"\\u001b[31mdanger\\u001b[0m\\n second line\\t\\u0007\"}",
    );
    assert_eq!(reasons(&effects), vec!["[31mdanger[0m\n second line"]);

    let blank = decode(
        CursorEvent::BeforeShellExecution,
        false,
        ProcessExit::Code(2),
        r#"{"agent_message":"\u0007"}"#,
    );
    assert_eq!(
        reasons(&blank),
        vec!["Denied by hook ./check.sh (exit 2)."],
        "a message that is only control characters counts as absent"
    );
}

#[test]
fn every_documented_stdout_fixture_decodes_without_a_failure() {
    for fixture in &EVENT_FIXTURES {
        let effects = decode(fixture.event, true, OK, fixture.stdout);
        let failed = effects
            .reasons
            .iter()
            .any(|reason| reason.text.starts_with("Hook ./check.sh"));
        assert!(
            !failed,
            "{} stdout fixture was read as a failure: {effects:?}",
            fixture.event.name()
        );
    }
    let shell = decode(
        CursorEvent::BeforeShellExecution,
        true,
        OK,
        CursorEvent::BeforeShellExecution.spec().name,
    );
    assert_eq!(shell.verdict, HookVerdict::Deny, "the event name alone is not JSON");
}
