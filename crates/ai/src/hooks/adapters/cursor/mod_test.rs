use super::*;
use crate::hooks::adapters::adapter_for;
use crate::hooks::adapters::cursor::events::CursorEvent;
use crate::hooks::adapters::cursor::test_support::{
    FULL_CONFIG, INVALID_MATCHER, event_fixture, fixture_envelope, json, outcome, shell_action,
    tool_pre,
};
use crate::hooks::model::{HookBoundary, HookVerdict, validate_capabilities};
use crate::hooks::runner::ProcessExit;
use crate::hooks::test_util::{test_binding, test_source_id};

fn parsed(text: &str) -> ParsedHookConfig {
    CursorAdapter::new()
        .parse(text.as_bytes(), &test_source_id("hooks.json"))
        .expect("parses")
}

#[test]
fn the_cursor_adapter_is_registered() {
    let adapter = adapter_for(HookFormat::Cursor).expect("registered");
    assert_eq!(adapter.format(), HookFormat::Cursor);
}

#[test]
fn parse_turns_file_warnings_into_diagnostics_on_the_source() {
    let config = parsed(FULL_CONFIG);
    assert_eq!(config.format, HookFormat::Cursor);
    assert_eq!(config.source, test_source_id("hooks.json"));
    assert_eq!(config.warnings.len(), 11);
    for warning in &config.warnings {
        assert_eq!(warning.source, config.source);
        assert_eq!(warning.severity, DiagnosticSeverity::Warning);
    }
    assert!(config.warnings.iter().any(|warning| warning.message
        == "workspaceOpen[0]: Warp has no workspaceOpen moment; this hook will never run"));
    assert_eq!(config.document, json(FULL_CONFIG));
}

#[test]
fn bindings_come_from_the_kept_document_and_carry_activation_warnings_where_due() {
    let adapter = CursorAdapter::new();
    let config = parsed(FULL_CONFIG);
    let bindings = adapter.bindings(&config);
    assert_eq!(bindings.len(), 18);
    assert!(
        bindings
            .iter()
            .all(|binding| binding.id.source == config.source)
    );

    let warned = bindings
        .iter()
        .filter(|binding| {
            !validate_capabilities(binding.boundary, &binding.capabilities).is_empty()
        })
        .map(|binding| binding.vendor_event.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        warned,
        vec![
            "sessionStart",
            "preToolUse",
            "postToolUse",
            "postToolUseFailure",
        ],
        "only events that document context or rewrites get an activation warning"
    );
}

#[test]
fn a_bad_matcher_skips_only_that_definition() {
    let adapter = CursorAdapter::new();
    let config = parsed(INVALID_MATCHER);
    assert_eq!(config.warnings.len(), 1);
    let bindings = adapter.bindings(&config);
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].id.index, 1);
    assert_eq!(bindings[0].command, "./hooks/second.sh");
}

#[test]
fn the_trait_methods_work_end_to_end_for_a_shell_hook() {
    let adapter = CursorAdapter::new();
    let fixture = event_fixture(CursorEvent::BeforeShellExecution);
    let config = parsed(fixture.hooks);
    let binding = &adapter.bindings(&config)[0];
    assert_eq!(binding.boundary, HookBoundary::ToolPre);
    assert_eq!(binding.matcher.as_deref(), Some("^git push"));

    let envelope = fixture_envelope(tool_pre(shell_action("git push --force")));
    assert!(adapter.matches(binding, &envelope));
    assert!(!adapter.matches(binding, &fixture_envelope(tool_pre(shell_action("ls")))));
    assert_eq!(
        adapter.encode_input(binding, &envelope),
        json(fixture.stdin)
    );

    let effects = adapter.decode_output(
        binding,
        &outcome(ProcessExit::Code(0), fixture.stdout),
        &DecodePolicy::default(),
    );
    assert_eq!(effects.verdict, HookVerdict::Deny);
    assert_eq!(
        effects.reasons[0].text,
        "Push without --force, or open a pull request."
    );
    assert_eq!(
        effects.user_notices,
        vec!["Force pushes are blocked in this repository."]
    );
}

#[test]
fn a_binding_declared_under_an_unknown_event_never_fires() {
    let adapter = CursorAdapter::new();
    let mut binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, None);
    binding.vendor_event = "notACursorEvent".to_owned();
    let envelope = fixture_envelope(tool_pre(shell_action("ls")));
    assert!(!adapter.matches(&binding, &envelope));
    assert_eq!(adapter.encode_input(&binding, &envelope), json("{}"));
    assert_eq!(
        adapter.decode_output(
            &binding,
            &outcome(ProcessExit::Code(2), ""),
            &DecodePolicy::default()
        ),
        HookEffects::default()
    );
}

#[test]
fn a_document_that_stopped_parsing_yields_no_bindings() {
    let adapter = CursorAdapter::new();
    let mut config = parsed(r#"{ "version": 1, "hooks": { "stop": [ { "command": "x" } ] } }"#);
    config.document = json(r#"{ "version": 2 }"#);
    assert!(adapter.bindings(&config).is_empty());
}
