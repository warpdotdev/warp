use std::time::Duration;

use super::*;
use crate::hooks::adapters::cursor::shell_permission_capabilities;
use crate::hooks::model::HookBoundary;
use crate::hooks::test_util::{test_binding_id, test_source_id};

fn binding(fail_closed: bool) -> DeclaredBinding {
    DeclaredBinding {
        id: test_binding_id(test_source_id("hooks.json"), "beforeShellExecution", 0),
        boundary: HookBoundary::ToolPre,
        vendor_event: "beforeShellExecution".to_owned(),
        capabilities: shell_permission_capabilities(fail_closed),
        matcher: None,
        command: "./check.sh".to_owned(),
        timeout: Duration::from_secs(30),
    }
}

fn outcome(exit: ProcessExit, stdout: &str) -> HookProcessOutcome {
    HookProcessOutcome {
        exit,
        stdout: stdout.as_bytes().to_vec(),
        stderr_tail: String::new(),
        duration: Duration::from_millis(1),
    }
}

fn decode(fail_closed: bool, exit: ProcessExit, stdout: &str) -> HookEffects {
    decode_shell_permission_output(&binding(fail_closed), &outcome(exit, stdout))
}

fn reasons(effects: &HookEffects) -> Vec<&str> {
    effects.reasons.iter().map(|r| r.text.as_str()).collect()
}

#[test]
fn exit_two_denies_with_the_default_reason_when_nothing_is_printed() {
    let effects = decode(false, ProcessExit::Code(2), "");
    assert_eq!(effects.verdict, HookVerdict::Deny);
    assert_eq!(reasons(&effects), vec!["Denied by hook ./check.sh (exit 2)."]);
    assert_eq!(effects.reasons[0].binding, Some(binding(false).id));
    assert!(effects.user_notices.is_empty());
}

#[test]
fn exit_two_uses_the_printed_agent_message_as_the_reason() {
    let effects = decode(
        false,
        ProcessExit::Code(2),
        r#"{"agent_message":"rm is not allowed here","user_message":"Blocked by policy"}"#,
    );
    assert_eq!(effects.verdict, HookVerdict::Deny);
    assert_eq!(reasons(&effects), vec!["rm is not allowed here"]);
    assert_eq!(effects.user_notices, vec!["Blocked by policy"]);
}

#[test]
fn exit_two_with_unreadable_stdout_still_denies_with_the_default_reason() {
    let effects = decode(false, ProcessExit::Code(2), "not json");
    assert_eq!(effects.verdict, HookVerdict::Deny);
    assert_eq!(reasons(&effects), vec!["Denied by hook ./check.sh (exit 2)."]);
}

#[test]
fn exit_zero_with_a_permission_is_decoded() {
    let allow = decode(false, ProcessExit::Code(0), r#"{"permission":"allow"}"#);
    assert_eq!(allow.verdict, HookVerdict::Allow);
    assert!(allow.reasons.is_empty());

    let deny = decode(
        false,
        ProcessExit::Code(0),
        r#"{"permission":"deny","agent_message":"no"}"#,
    );
    assert_eq!(deny.verdict, HookVerdict::Deny);
    assert_eq!(reasons(&deny), vec!["no"]);

    let ask = decode(false, ProcessExit::Code(0), r#"{"permission":"ask"}"#);
    assert_eq!(ask.verdict, HookVerdict::Ask);
}

#[test]
fn a_deny_with_only_a_user_message_uses_it_as_the_reason_without_a_second_notice() {
    let effects = decode(
        false,
        ProcessExit::Code(0),
        r#"{"permission":"deny","user_message":"Blocked by policy"}"#,
    );
    assert_eq!(reasons(&effects), vec!["Blocked by policy"]);
    assert!(effects.user_notices.is_empty());
}

#[test]
fn messages_on_an_allow_are_observational() {
    let effects = decode(
        false,
        ProcessExit::Code(0),
        r#"{"permission":"allow","user_message":"ok","agent_message":"remember to lint"}"#,
    );
    assert_eq!(effects.verdict, HookVerdict::Allow);
    assert!(effects.reasons.is_empty());
    assert_eq!(effects.user_notices, vec!["ok"]);
    assert_eq!(effects.model_context, vec!["remember to lint"]);
}

#[test]
fn empty_output_follows_the_failure_policy() {
    for stdout in ["", "  \n"] {
        let open = decode(false, ProcessExit::Code(0), stdout);
        assert_eq!(open, HookEffects::default(), "fail open on {stdout:?}");

        let closed = decode(true, ProcessExit::Code(0), stdout);
        assert_eq!(closed.verdict, HookVerdict::Deny);
        assert_eq!(
            reasons(&closed),
            vec!["Hook ./check.sh printed no decision, so the command was denied."]
        );
    }
}

#[test]
fn malformed_output_denies_even_when_the_hook_fails_open() {
    for stdout in [
        "not json",
        "[1, 2]",
        r#"{"permission": 1}"#,
        r#"{"permission": "maybe"}"#,
        r#"{"user_message": "no decision here"}"#,
        r#"{"permission": "allow", "user_message": 5}"#,
    ] {
        let effects = decode(false, ProcessExit::Code(0), stdout);
        assert_eq!(effects.verdict, HookVerdict::Deny, "stdout {stdout:?}");
        assert_eq!(
            reasons(&effects),
            vec![
                "Hook ./check.sh printed output that is not a valid permission decision, so the \
                 command was denied."
            ],
            "stdout {stdout:?}"
        );
    }
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
    for (exit, description) in failures {
        let open = decode(false, exit.clone(), r#"{"permission":"deny"}"#);
        assert_eq!(open, HookEffects::default(), "fail open on {exit:?}");

        let closed = decode(true, exit.clone(), "");
        assert_eq!(closed.verdict, HookVerdict::Deny, "fail closed on {exit:?}");
        assert_eq!(
            reasons(&closed),
            vec![format!(
                "Hook ./check.sh {description}, so the command was denied."
            )]
        );
    }
}

#[test]
fn a_cancelled_hook_denies_under_either_policy() {
    for fail_closed in [false, true] {
        let effects = decode(fail_closed, ProcessExit::Cancelled, "");
        assert_eq!(effects.verdict, HookVerdict::Deny);
        assert_eq!(
            reasons(&effects),
            vec!["Hook ./check.sh was cancelled before it finished."]
        );
    }
}

#[test]
fn control_characters_are_removed_from_messages() {
    let effects = decode(
        false,
        ProcessExit::Code(2),
        "{\"agent_message\":\"\\u001b[31mdanger\\u001b[0m\\n second line\\t\\u0007\"}",
    );
    assert_eq!(reasons(&effects), vec!["[31mdanger[0m\n second line"]);

    let blank = decode(false, ProcessExit::Code(2), r#"{"agent_message":"\u0007"}"#);
    assert_eq!(
        reasons(&blank),
        vec!["Denied by hook ./check.sh (exit 2)."],
        "a message that is only control characters counts as absent"
    );
}
