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
fn file_profile_without_web_fetch_inherits_web_search() {
    for web_search_enabled in [false, true] {
        let value = serde_json::json!({
            "default": {"web_search_enabled": web_search_enabled},
        });

        let decoded = ExecutionProfilesConfig::from_file_value(&value).unwrap();
        let profile = decoded
            .profile(&ExecutionProfileId::default_profile())
            .unwrap();

        assert_eq!(profile.web_search_enabled, web_search_enabled);
        assert_eq!(profile.web_fetch_enabled, web_search_enabled);
    }
}

#[test]
fn file_web_tool_settings_round_trip_independently() {
    for (web_search_enabled, web_fetch_enabled) in [(true, false), (false, true)] {
        let mut config = ExecutionProfilesConfig::default();
        let profile = config
            .profile_mut(&ExecutionProfileId::default_profile())
            .unwrap();
        profile.web_search_enabled = web_search_enabled;
        profile.web_fetch_enabled = web_fetch_enabled;

        let file_value = config.to_file_value();
        assert_eq!(
            file_value["default"]["web_fetch_enabled"],
            web_fetch_enabled
        );

        let decoded = ExecutionProfilesConfig::from_file_value(&file_value).unwrap();
        assert_eq!(decoded, config);
    }
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
