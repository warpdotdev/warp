//! Regression tests for the `warp_cli_wasm` spike's best-effort argv parser
//! and API-key redaction. These cover the two review findings whose behavior is
//! meaningfully testable on a native target:
//!
//! - `--flag=value` long-option forms must parse (finding: argv parsing
//!   correctness) — the parser previously only matched the separated
//!   `--flag value` form, so `--api-key=$WARP_API_KEY` silently dropped the
//!   key and produced misleading feasibility evidence.
//! - The `--api-key` value must be redacted from the returned argv (finding:
//!   API key exposure) — `ParsedAgentRun.argv` previously echoed the raw key,
//!   contradicting the `has_api_key` field's claim that the key value is never
//!   returned.

use super::*;

fn s(argv: &[&str]) -> Vec<String> {
    argv.iter().map(|x| x.to_string()).collect()
}

#[test]
fn config_from_argv_parses_flag_equals_value_forms() {
    // Regression for the `--flag=value` finding: before the fix, the parser
    // only matched the separated `--flag value` form, so all of these inline
    // values were dropped (`api_key` stayed `None`, `prompt` stayed empty).
    let argv = s(&[
        "oz",
        "agent",
        "run",
        "--prompt=hello",
        "--api-key=wk-1.secret",
        "--harness=oz",
        "--output-format=json",
    ]);
    let config = config_from_argv(&argv);
    assert_eq!(config.prompt, "hello");
    assert_eq!(config.api_key.as_deref(), Some("wk-1.secret"));
    assert_eq!(config.harness, "oz");
    assert_eq!(config.output_format, "json");
}

#[test]
fn config_from_argv_parses_separated_flag_forms() {
    // The separated form must still work after the parser was extended.
    let argv = s(&[
        "oz",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--api-key",
        "wk-1.secret",
        "--harness",
        "oz",
        "--output-format",
        "json",
    ]);
    let config = config_from_argv(&argv);
    assert_eq!(config.prompt, "hello");
    assert_eq!(config.api_key.as_deref(), Some("wk-1.secret"));
    assert_eq!(config.harness, "oz");
    assert_eq!(config.output_format, "json");
}

#[test]
fn config_from_argv_supports_short_prompt_flag() {
    let argv = s(&["oz", "agent", "run", "-p", "hi"]);
    let config = config_from_argv(&argv);
    assert_eq!(config.prompt, "hi");
}

#[test]
fn config_from_argv_mixed_equals_and_separated_forms() {
    // clap accepts a mix; the best-effort parser should too.
    let argv = s(&[
        "oz",
        "agent",
        "run",
        "--prompt=hello",
        "--api-key",
        "wk-1.secret",
        "--output-format=json",
    ]);
    let config = config_from_argv(&argv);
    assert_eq!(config.prompt, "hello");
    assert_eq!(config.api_key.as_deref(), Some("wk-1.secret"));
    assert_eq!(config.output_format, "json");
}

#[test]
fn config_from_argv_unknown_flag_does_not_swallow_neighbor() {
    // An unknown long flag (with or without `=value`) must not consume the
    // following argv element, so a later known flag still parses.
    let argv = s(&[
        "oz",
        "agent",
        "run",
        "--verbose",
        "--prompt",
        "hello",
        "--unknown=ignored",
        "--harness",
        "oz",
    ]);
    let config = config_from_argv(&argv);
    assert_eq!(config.prompt, "hello");
    assert_eq!(config.harness, "oz");
}

#[test]
fn redact_api_key_in_argv_redacts_separated_form() {
    let argv = s(&["oz", "--api-key", "wk-1.secret", "--prompt", "hi"]);
    let redacted = redact_api_key_in_argv(&argv);
    assert_eq!(
        redacted,
        s(&["oz", "--api-key", "<redacted>", "--prompt", "hi"])
    );
    assert!(!redacted.iter().any(|s| s.contains("wk-1.secret")));
}

#[test]
fn redact_api_key_in_argv_redacts_equals_form() {
    let argv = s(&["oz", "--api-key=wk-1.secret", "--prompt=hi"]);
    let redacted = redact_api_key_in_argv(&argv);
    assert_eq!(redacted, s(&["oz", "--api-key=<redacted>", "--prompt=hi"]));
    assert!(!redacted.iter().any(|s| s.contains("wk-1.secret")));
}

#[test]
fn redact_api_key_in_argv_leaves_argv_without_key_unchanged() {
    let argv = s(&["oz", "agent", "run", "--prompt", "hi"]);
    let redacted = redact_api_key_in_argv(&argv);
    assert_eq!(redacted, argv);
}

#[test]
fn redact_api_key_in_argv_handles_terminal_api_key_flag() {
    // `--api-key` with no following value: keep the flag, redact nothing.
    let argv = s(&["oz", "agent", "run", "--api-key"]);
    let redacted = redact_api_key_in_argv(&argv);
    assert_eq!(redacted, s(&["oz", "agent", "run", "--api-key"]));
}

#[test]
fn agent_run_from_argv_redacts_api_key_in_result() {
    // End-to-end regression for the "API key exposure" finding: the returned
    // JSON's `argv` must not contain the raw key, and `has_api_key` must still
    // be true. Before the fix, `argv.to_vec()` echoed the raw key verbatim.
    let argv = s(&[
        "oz",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--api-key",
        "wk-1.secret",
        "--harness",
        "oz",
        "--output-format",
        "json",
    ]);
    let json = agent_run_from_argv(&serde_json::to_string(&argv).unwrap())
        .expect("agent_run_from_argv should parse a valid agent run argv");
    assert!(
        !json.contains("wk-1.secret"),
        "raw api key leaked into result: {json}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("result is valid JSON");
    let result_argv = parsed["argv"]
        .as_array()
        .expect("argv field is an array in the result");
    assert!(
        result_argv.iter().any(|v| v.as_str() == Some("<redacted>")),
        "expected a redacted api-key marker in argv, got: {result_argv:?}"
    );
    assert_eq!(parsed["has_api_key"].as_bool(), Some(true));
    assert_eq!(parsed["prompt"].as_str(), Some("hello"));
}
