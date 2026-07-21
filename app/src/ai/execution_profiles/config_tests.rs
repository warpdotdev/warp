use settings_value::SettingsValue as _;

use super::*;

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
