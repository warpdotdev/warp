use super::{classify_agent_mode_base_model_id, parse_ambient_task_id};
use crate::ai::llms::LLMId;

#[test]
fn parse_ambient_task_id_accepts_valid_ids() {
    let task_id =
        parse_ambient_task_id("550e8400-e29b-41d4-a716-446655440000", "Invalid run ID").unwrap();

    assert_eq!(task_id.to_string(), "550e8400-e29b-41d4-a716-446655440000");
}

#[test]
fn parse_ambient_task_id_preserves_error_prefix() {
    let err = parse_ambient_task_id("not-a-run-id", "Invalid run ID").unwrap_err();

    assert!(err.to_string().contains("Invalid run ID 'not-a-run-id'"));
}

// -- validate_agent_mode_base_model_id unavailable-vs-invalid error heuristics --
//
// Regression tests for the scenario where the server is unhealthy and returns
// an empty/unavailable agent-mode model list. Previously both validators blamed
// the user's model id ("Unknown model id ..." / "is not a valid agent mode
// LLM"), hiding the real server-availability issue. The fix tracks the
// fetch-failure / list-unavailable state and surfaces a distinct error.

#[test]
fn classify_returns_server_unavailable_error_when_list_unavailable() {
    // Simulates an unhealthy server: the authed model-list fetch failed, so the
    // list is unavailable. The cached/default list is still non-empty (e.g.
    // "auto"), which is exactly the case that previously produced the
    // misleading "Unknown model id" error for any id not in the list.
    let valid_ids = vec![LLMId::from("auto")];
    let err = classify_agent_mode_base_model_id("claude-sonnet-4-5", &valid_ids, true)
        .expect_err("unavailable list should error");
    let msg = format!("{err:#}");
    assert!(
        !msg.contains("Unknown model id"),
        "should not blame the model id when the list is unavailable: {msg}"
    );
    assert!(
        msg.to_lowercase().contains("unavailable"),
        "should surface a server/model-list unavailability error: {msg}"
    );
}

#[test]
fn classify_returns_unknown_id_error_when_list_available_and_id_genuinely_invalid() {
    // A non-empty, available list that does not contain the id still produces
    // the existing "Unknown model id" error (with suggestions).
    let valid_ids = vec![LLMId::from("auto"), LLMId::from("gpt-x")];
    let err = classify_agent_mode_base_model_id("claude-sonnet-4-5", &valid_ids, false)
        .expect_err("genuinely invalid id should error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("Unknown model id"),
        "should preserve the existing 'Unknown model id' error: {msg}"
    );
    assert!(
        msg.contains("auto") && msg.contains("gpt-x"),
        "should list the available model suggestions: {msg}"
    );
}

#[test]
fn classify_accepts_id_in_choices_even_when_list_unavailable() {
    // A custom-endpoint (local) id that is among the choices should still
    // validate even when the server list is unavailable, because custom
    // endpoints are independent of server health (the validator chains custom
    // choices alongside the server list).
    let valid_ids = vec![LLMId::from("custom-config-key")];
    let id = classify_agent_mode_base_model_id("custom-config-key", &valid_ids, true)
        .expect("an id present in the choices should validate");
    assert_eq!(id.as_str(), "custom-config-key");
}
