use schemars::JsonSchema as _;
use settings_value::SettingsValue as _;

use super::*;

#[test]
fn context_window_limit_schema_has_description() {
    let mut generator = schemars::SchemaGenerator::default();
    let schema = ExecutionProfileFile::json_schema(&mut generator);
    let value = schemars::Schema::to_value(schema);
    let props = value
        .pointer("/properties/context_window_limit")
        .expect("context_window_limit should be a property");
    let description = props
        .get("description")
        .and_then(|d| d.as_str())
        .expect("context_window_limit should have a description");
    assert!(
        description.contains("model-dependent"),
        "description should mention model-dependent range, got: {description}"
    );
    assert!(
        description.contains("server-side"),
        "description should mention server-side determination, got: {description}"
    );
}

#[test]
fn file_collection_round_trips_multiple_profiles() {
    let mut config = ExecutionProfilesConfig::default();
    let custom_id = ExecutionProfileId::parse("code-review").unwrap();
    let custom = AIExecutionProfile {
        name: "Code Review".to_string(),
        apply_code_diffs: ActionPermission::AlwaysAllow,
        command_allowlist: vec![
            AgentModeCommandExecutionPredicate::new_regex("git status").unwrap(),
        ],
        mcp_allowlist: vec![uuid::Uuid::new_v4()],
        base_model: Some(LLMId::from("model-id")),
        ..Default::default()
    };
    config.insert(custom_id.clone(), custom.clone());

    let file_value = config.to_file_value();
    assert_eq!(
        file_value["code-review"]["apply_code_diffs"],
        "always_allow"
    );
    assert_eq!(
        file_value["code-review"]["command_allowlist"][0],
        "git status"
    );

    let decoded = ExecutionProfilesConfig::from_file_value(&file_value).unwrap();
    assert_eq!(decoded.profile(&custom_id), Some(&custom));
}

#[test]
fn file_collection_rejects_invalid_values_as_a_unit() {
    for value in [
        serde_json::json!({"custom": {"name": "Missing default"}}),
        serde_json::json!({"default": {}, "invalid key": {}}),
        serde_json::json!({
            "default": {},
            "custom": {"command_allowlist": ["("]}
        }),
    ] {
        assert_eq!(ExecutionProfilesConfig::from_file_value(&value), None);
    }
}
