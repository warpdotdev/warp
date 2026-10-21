use std::collections::BTreeSet;
use std::time::Duration;

use super::*;
use crate::hooks::adapters::cursor::test_support::{
    FULL_CONFIG, INVALID_MATCHER, PROMPT_HOOK, UNKNOWN_EVENT,
};
use crate::hooks::model::{POST_AND_OTHER_TIMEOUT, PRE_EXECUTION_TIMEOUT};

fn parse(text: &str) -> Result<ParsedCursorFile, HookConfigError> {
    parse_cursor_file(text.as_bytes()).map(|(_, parsed)| parsed)
}

fn invalid_message(text: &str) -> String {
    match parse(text).expect_err("rejects") {
        HookConfigError::Invalid(message) => message,
        other => panic!("expected an invalid-config error, got {other:?}"),
    }
}

fn shell_file(definition: &str) -> String {
    format!(r#"{{ "version": 1, "hooks": {{ "beforeShellExecution": [ {definition} ] }} }}"#)
}

#[test]
fn a_minimal_file_yields_one_definition_with_defaults() {
    let parsed = parse(&shell_file(r#"{ "command": "./check.sh" }"#)).expect("parses");
    assert_eq!(
        parsed,
        ParsedCursorFile {
            definitions: vec![ParsedDefinition {
                event: CursorEvent::BeforeShellExecution,
                index: 0,
                command: "./check.sh".to_owned(),
                matcher: None,
                timeout: PRE_EXECUTION_TIMEOUT,
                fail_closed: false,
                loop_limit: None,
            }],
            warnings: Vec::new(),
        }
    );
}

#[test]
fn the_full_config_parses_every_event_with_its_options() {
    let parsed = parse(FULL_CONFIG).expect("parses");
    let events = parsed
        .definitions
        .iter()
        .map(|definition| definition.event)
        .collect::<Vec<_>>();
    assert_eq!(
        events,
        CursorEvent::ALL.to_vec(),
        "one command definition per event, in table order"
    );

    let by_event = |event: CursorEvent| {
        parsed
            .definitions
            .iter()
            .find(|definition| definition.event == event)
            .expect("parsed")
    };
    let pre_tool = by_event(CursorEvent::PreToolUse);
    assert_eq!(pre_tool.matcher.as_deref(), Some("Shell|Write"));
    assert_eq!(pre_tool.timeout, Duration::from_secs(20));
    assert!(pre_tool.fail_closed);
    assert_eq!(
        by_event(CursorEvent::PostToolUseFailure).matcher.as_deref(),
        Some("")
    );
    assert_eq!(
        by_event(CursorEvent::SubagentStop).matcher.as_deref(),
        Some("*")
    );
    assert_eq!(by_event(CursorEvent::SubagentStop).loop_limit, Some(3));
    assert_eq!(
        by_event(CursorEvent::Stop).loop_limit,
        Some(LOOP_LIMIT_HOST_CAP)
    );
    assert_eq!(
        by_event(CursorEvent::SessionStart).timeout,
        Duration::from_secs(10)
    );
    assert_eq!(
        by_event(CursorEvent::AfterMcpExecution).timeout,
        POST_AND_OTHER_TIMEOUT
    );
    assert_eq!(
        by_event(CursorEvent::WorkspaceOpen).timeout,
        POST_AND_OTHER_TIMEOUT
    );

    assert_eq!(
        parsed.warnings,
        vec![
            "sessionEnd[0]: Warp does not fire sessionEnd yet; this hook will never run",
            "subagentStart[0]: Warp does not fire subagentStart yet; this hook will never run",
            "subagentStop[0]: Warp does not fire subagentStop yet; this hook will never run",
            "beforeShellExecution[1]: prompt hooks are not supported yet; it was skipped",
            "beforeReadFile[0]: Warp does not fire beforeReadFile yet; this hook will never run",
            "afterFileEdit[0]: Warp does not fire afterFileEdit yet; this hook will never run",
            "preCompact[0]: Warp does not fire preCompact yet; this hook will never run",
            "stop[0]: loop_limit null asks for no limit; Warp caps follow-ups at 8",
            "beforeTabFileRead[0]: Warp has no beforeTabFileRead moment; this hook will never run",
            "afterTabFileEdit[0]: Warp has no afterTabFileEdit moment; this hook will never run",
            "workspaceOpen[0]: Warp has no workspaceOpen moment; this hook will never run",
        ]
    );
}

#[test]
fn only_version_1_is_accepted() {
    assert!(parse(r#"{ "version": 1, "hooks": {} }"#).is_ok());
    assert_eq!(invalid_message(r#"{ "hooks": {} }"#), "version is required");
    assert_eq!(
        invalid_message(r#"{ "version": 0, "hooks": {} }"#),
        "version must be a positive integer, got 0"
    );
    assert_eq!(
        invalid_message(r#"{ "version": "1", "hooks": {} }"#),
        "version must be a positive integer, got \"1\""
    );
    assert_eq!(
        invalid_message(r#"{ "version": 1.5, "hooks": {} }"#),
        "version must be a positive integer, got 1.5"
    );
    assert_eq!(
        invalid_message(r#"{ "version": 2, "hooks": {} }"#),
        "version 2 is not supported; only version 1 is"
    );
}

#[test]
fn a_file_with_no_hooks_object_has_no_definitions() {
    let parsed = parse(r#"{ "version": 1 }"#).expect("parses");
    assert!(parsed.definitions.is_empty());
    assert!(parsed.warnings.is_empty());
}

#[test]
fn invalid_json_is_a_parse_error() {
    assert!(matches!(
        parse("{ not json"),
        Err(HookConfigError::Parse(_))
    ));
}

#[test]
fn an_unknown_event_name_is_reported_and_the_known_ones_still_load() {
    let parsed = parse(UNKNOWN_EVENT).expect("parses");
    assert_eq!(parsed.definitions.len(), 1);
    assert_eq!(
        parsed.definitions[0].event,
        CursorEvent::BeforeShellExecution
    );
    assert_eq!(
        parsed.warnings,
        vec!["beforeToolUse is not a hook event Cursor documents; its definitions were skipped"]
    );
}

#[test]
fn an_unknown_field_in_a_definition_is_reported_and_prompt_fields_are_not() {
    let parsed = parse(&shell_file(
        r#"{ "command": "./a.sh", "retries": 3, "prompt": "ignored", "model": "fast" }"#,
    ))
    .expect("parses");
    assert_eq!(parsed.definitions.len(), 1);
    assert_eq!(
        parsed.warnings,
        vec!["beforeShellExecution[0]: unknown field retries was ignored"]
    );
}

#[test]
fn a_prompt_hook_is_skipped_and_the_command_hook_after_it_keeps_its_position() {
    let parsed = parse(PROMPT_HOOK).expect("parses");
    assert_eq!(parsed.definitions.len(), 1);
    assert_eq!(parsed.definitions[0].index, 1);
    assert_eq!(parsed.definitions[0].command, "./hooks/check.sh");
    assert_eq!(
        parsed.warnings,
        vec!["beforeShellExecution[0]: prompt hooks are not supported yet; it was skipped"]
    );
}

#[test]
fn an_unknown_hook_type_is_invalid() {
    let message = invalid_message(&shell_file(r#"{ "type": "http", "command": "x" }"#));
    assert!(message.contains("unknown hook type"), "{message}");
}

#[test]
fn a_definition_without_a_command_is_invalid() {
    let message = invalid_message(&shell_file(r#"{ "timeout": 5 }"#));
    assert_eq!(message, "beforeShellExecution[0]: command is required");
    assert!(matches!(
        parse(&shell_file(r#"{ "command": "  " }"#)),
        Err(HookConfigError::Invalid(_))
    ));
}

#[test]
fn a_definition_with_a_wrongly_typed_field_is_invalid() {
    for definition in [
        r#"{ "command": 5 }"#,
        r#"{ "command": "x", "matcher": 5 }"#,
        r#"{ "command": "x", "failClosed": "yes" }"#,
        r#"{ "command": "x", "timeout": "5" }"#,
    ] {
        assert!(
            matches!(
                parse(&shell_file(definition)),
                Err(HookConfigError::Invalid(_))
            ),
            "{definition} should be rejected"
        );
    }
}

#[test]
fn a_bad_timeout_is_invalid() {
    for timeout in ["0", "-1", "1e20", "99999999999999999999999", "1e-30"] {
        let text = shell_file(&format!(r#"{{ "command": "x", "timeout": {timeout} }}"#));
        assert!(
            matches!(parse(&text), Err(HookConfigError::Invalid(_))),
            "timeout {timeout} should be rejected"
        );
    }
    let parsed = parse(&shell_file(r#"{ "command": "x", "timeout": 2.5 }"#)).expect("parses");
    assert_eq!(parsed.definitions[0].timeout, Duration::from_millis(2500));
}

#[test]
fn a_matcher_that_does_not_compile_skips_only_its_definition() {
    let parsed = parse(INVALID_MATCHER).expect("the file still loads");
    assert_eq!(parsed.definitions.len(), 1);
    assert_eq!(parsed.definitions[0].index, 1);
    assert_eq!(parsed.definitions[0].command, "./hooks/second.sh");
    assert_eq!(parsed.definitions[0].matcher.as_deref(), Some("^rm "));
    assert_eq!(parsed.warnings.len(), 1);
    assert!(
        parsed.warnings[0].starts_with("beforeShellExecution[0]: matcher is not a valid regex"),
        "{}",
        parsed.warnings[0]
    );
    assert!(parsed.warnings[0].ends_with("this hook was skipped"));
}

#[test]
fn match_all_matchers_are_kept_without_compiling() {
    for matcher in ["", "*"] {
        let text = shell_file(&format!(r#"{{ "command": "x", "matcher": "{matcher}" }}"#));
        let parsed = parse(&text).expect("match-all matchers are accepted");
        assert_eq!(parsed.definitions[0].matcher.as_deref(), Some(matcher));
        assert!(parsed.warnings.is_empty());
    }
}

#[test]
fn a_matcher_on_an_event_without_a_target_is_ignored_with_a_warning() {
    let parsed = parse(
        r#"{ "version": 1, "hooks": { "sessionStart": [
            { "command": "./a.sh", "matcher": "foo" },
            { "command": "./b.sh", "matcher": "*" }
        ] } }"#,
    )
    .expect("parses");
    assert_eq!(parsed.definitions.len(), 2);
    assert_eq!(parsed.definitions[0].matcher, None);
    assert_eq!(parsed.definitions[1].matcher.as_deref(), Some("*"));
    assert_eq!(
        parsed.warnings,
        vec!["sessionStart[0]: sessionStart has no matcher target, so the matcher was ignored"]
    );
}

#[test]
fn loop_limit_is_read_for_stop_hooks_and_capped() {
    let stop_file = |definition: &str| {
        format!(r#"{{ "version": 1, "hooks": {{ "stop": [ {definition} ] }} }}"#)
    };
    let parsed = parse(&stop_file(r#"{ "command": "x" }"#)).expect("parses");
    assert_eq!(parsed.definitions[0].loop_limit, Some(DEFAULT_LOOP_LIMIT));
    assert!(parsed.warnings.is_empty());

    let parsed = parse(&stop_file(r#"{ "command": "x", "loop_limit": 0 }"#)).expect("parses");
    assert_eq!(parsed.definitions[0].loop_limit, Some(0));

    let parsed = parse(&stop_file(r#"{ "command": "x", "loop_limit": 3 }"#)).expect("parses");
    assert_eq!(parsed.definitions[0].loop_limit, Some(3));
    assert!(parsed.warnings.is_empty());

    let parsed = parse(&stop_file(r#"{ "command": "x", "loop_limit": 20 }"#)).expect("parses");
    assert_eq!(parsed.definitions[0].loop_limit, Some(LOOP_LIMIT_HOST_CAP));
    assert_eq!(
        parsed.warnings,
        vec!["stop[0]: loop_limit 20 is above Warp's cap; 8 is used"]
    );

    let parsed = parse(&stop_file(r#"{ "command": "x", "loop_limit": null }"#)).expect("parses");
    assert_eq!(parsed.definitions[0].loop_limit, Some(LOOP_LIMIT_HOST_CAP));
    assert_eq!(
        parsed.warnings,
        vec!["stop[0]: loop_limit null asks for no limit; Warp caps follow-ups at 8"]
    );

    for bad in ["-1", "1.5", "\"3\""] {
        let message = invalid_message(&stop_file(&format!(
            r#"{{ "command": "x", "loop_limit": {bad} }}"#
        )));
        assert!(message.contains("loop_limit must be"), "{message}");
    }
}

#[test]
fn loop_limit_on_another_event_is_ignored_with_a_warning() {
    let parsed = parse(&shell_file(r#"{ "command": "x", "loop_limit": 3 }"#)).expect("parses");
    assert_eq!(parsed.definitions[0].loop_limit, None);
    assert_eq!(
        parsed.warnings,
        vec![
            "beforeShellExecution[0]: loop_limit applies only to stop and subagentStop; it was \
             ignored"
        ]
    );
}

#[test]
fn the_event_value_must_be_an_array() {
    assert!(matches!(
        parse(r#"{ "version": 1, "hooks": { "beforeShellExecution": { "command": "x" } } }"#),
        Err(HookConfigError::Invalid(_))
    ));
}

#[test]
fn events_warp_does_not_fire_are_parsed_with_a_warning_each() {
    let parsed = parse(
        r#"{ "version": 1, "hooks": {
            "afterFileEdit": [ { "command": "./fmt.sh" }, { "command": "./lint.sh" } ],
            "workspaceOpen": [ { "command": "./open.sh" } ]
        } }"#,
    )
    .expect("parses");
    assert_eq!(parsed.definitions.len(), 3);
    assert_eq!(
        parsed.warnings.iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            &"afterFileEdit[0]: Warp does not fire afterFileEdit yet; this hook will never run"
                .to_owned(),
            &"afterFileEdit[1]: Warp does not fire afterFileEdit yet; this hook will never run"
                .to_owned(),
            &"workspaceOpen[0]: Warp has no workspaceOpen moment; this hook will never run"
                .to_owned(),
        ])
    );
}

#[test]
fn definitions_come_out_in_table_order_regardless_of_file_order() {
    let parsed = parse(
        r#"{ "version": 1, "hooks": {
            "stop": [ { "command": "./stop.sh" } ],
            "beforeShellExecution": [ { "command": "./shell.sh" } ],
            "preToolUse": [ { "command": "./pre.sh" } ]
        } }"#,
    )
    .expect("parses");
    let events = parsed
        .definitions
        .iter()
        .map(|definition| definition.event)
        .collect::<Vec<_>>();
    assert_eq!(
        events,
        vec![
            CursorEvent::PreToolUse,
            CursorEvent::BeforeShellExecution,
            CursorEvent::Stop,
        ]
    );
}
