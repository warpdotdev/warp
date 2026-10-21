use std::time::Duration;

use super::*;
use crate::hooks::model::PRE_EXECUTION_TIMEOUT;

fn parse(text: &str) -> Result<ParsedCursorFile, HookConfigError> {
    parse_cursor_file(text.as_bytes()).map(|(_, parsed)| parsed)
}

#[test]
fn a_minimal_file_yields_one_shell_hook_with_defaults() {
    let parsed = parse(
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "./check.sh" } ] } }"#,
    )
    .expect("parses");
    assert_eq!(
        parsed,
        ParsedCursorFile {
            shell_hooks: vec![ShellHookDefinition {
                command: "./check.sh".to_owned(),
                matcher: None,
                timeout: PRE_EXECUTION_TIMEOUT,
                fail_closed: false,
            }],
            unknown_events: Vec::new(),
        }
    );
}

#[test]
fn timeout_matcher_and_fail_closed_are_read() {
    let parsed = parse(
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [
            { "command": "./a.sh", "timeout": 2.5, "matcher": "rm .*", "failClosed": true },
            { "command": "./b.sh", "type": "command", "loop_limit": 3 }
        ] } }"#,
    )
    .expect("parses");
    assert_eq!(parsed.shell_hooks.len(), 2);
    assert_eq!(parsed.shell_hooks[0].timeout, Duration::from_millis(2500));
    assert_eq!(parsed.shell_hooks[0].matcher.as_deref(), Some("rm .*"));
    assert!(parsed.shell_hooks[0].fail_closed);
    assert_eq!(parsed.shell_hooks[1].command, "./b.sh");
    assert!(!parsed.shell_hooks[1].fail_closed);
}

#[test]
fn unknown_events_are_skipped_and_reported() {
    let parsed = parse(
        r#"{ "version": 1, "hooks": {
            "stop": [ { "command": "./stop.sh" } ],
            "beforeShellExecution": [ { "command": "./check.sh" } ],
            "afterFileEdit": [ { "command": "./fmt.sh" } ]
        } }"#,
    )
    .expect("parses");
    assert_eq!(parsed.shell_hooks.len(), 1);
    assert_eq!(parsed.unknown_events, vec!["afterFileEdit", "stop"]);
}

#[test]
fn a_file_with_no_hooks_object_has_no_bindings() {
    let parsed = parse(r#"{ "version": 1 }"#).expect("parses");
    assert!(parsed.shell_hooks.is_empty());
    assert!(parsed.unknown_events.is_empty());
}

#[test]
fn invalid_json_is_a_parse_error() {
    assert!(matches!(parse("{ not json"), Err(HookConfigError::Parse(_))));
}

#[test]
fn a_missing_or_zero_version_is_invalid() {
    assert!(matches!(
        parse(r#"{ "hooks": {} }"#),
        Err(HookConfigError::Invalid(_))
    ));
    assert!(matches!(
        parse(r#"{ "version": 0, "hooks": {} }"#),
        Err(HookConfigError::Invalid(_))
    ));
}

#[test]
fn a_definition_without_a_command_is_invalid() {
    let error = parse(r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "timeout": 5 } ] } }"#)
        .expect_err("rejects");
    let HookConfigError::Invalid(message) = error else {
        panic!("expected an invalid-config error, got {error:?}");
    };
    assert!(message.contains("beforeShellExecution[0]"), "{message}");
    assert!(message.contains("command is required"), "{message}");

    assert!(matches!(
        parse(r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "  " } ] } }"#),
        Err(HookConfigError::Invalid(_))
    ));
}

#[test]
fn a_prompt_hook_is_rejected() {
    let error = parse(
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [
            { "type": "prompt", "prompt": "Is this command safe?" }
        ] } }"#,
    )
    .expect_err("rejects");
    let HookConfigError::Invalid(message) = error else {
        panic!("expected an invalid-config error, got {error:?}");
    };
    assert!(message.contains("prompt hooks are not supported"), "{message}");
}

#[test]
fn a_bad_timeout_is_invalid() {
    for timeout in ["0", "-1", "\"5\""] {
        let text = format!(
            r#"{{ "version": 1, "hooks": {{ "beforeShellExecution": [ {{ "command": "x", "timeout": {timeout} }} ] }} }}"#
        );
        assert!(
            matches!(parse(&text), Err(HookConfigError::Invalid(_))),
            "timeout {timeout} should be rejected"
        );
    }
}

#[test]
fn a_matcher_that_does_not_compile_is_invalid() {
    let error = parse(
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "x", "matcher": "(" } ] } }"#,
    )
    .expect_err("rejects");
    assert!(matches!(error, HookConfigError::Invalid(_)));

    for matcher in ["", "*"] {
        let text = format!(
            r#"{{ "version": 1, "hooks": {{ "beforeShellExecution": [ {{ "command": "x", "matcher": "{matcher}" }} ] }} }}"#
        );
        let parsed = parse(&text).expect("match-all matchers are accepted");
        assert_eq!(parsed.shell_hooks[0].matcher.as_deref(), Some(matcher));
    }
}

#[test]
fn the_event_array_must_be_an_array() {
    assert!(matches!(
        parse(r#"{ "version": 1, "hooks": { "beforeShellExecution": { "command": "x" } } }"#),
        Err(HookConfigError::Invalid(_))
    ));
}
