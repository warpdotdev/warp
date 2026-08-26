use super::*;
use crate::cloud_environment::CodeForge;

#[test]
fn additional_source_repos_round_trip_and_is_optional() {
    let snapshot = AgentConfigSnapshot {
        additional_source_repos: Some(vec![SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp".to_string(),
        )]),
        ..Default::default()
    };
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(
        json["additional_source_repos"][0],
        serde_json::json!({
            "code_forge": "GITHUB",
            "owner": "warpdotdev",
            "repo": "warp"
        })
    );

    let decoded: AgentConfigSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(decoded, snapshot);

    let legacy: AgentConfigSnapshot = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(legacy.additional_source_repos.is_none());
    assert!(legacy.is_empty());
}

#[test]
fn computer_use_model_id_round_trips_under_the_public_api_field_name() {
    // The key must stay `computer_use_model_id`: that is the field name on the
    // server's `AmbientAgentConfig`, and the run-create body is this snapshot.
    let snapshot = AgentConfigSnapshot {
        computer_use_model_id: Some("claude-4-5-sonnet".to_string()),
        ..Default::default()
    };
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(json["computer_use_model_id"], "claude-4-5-sonnet");
    assert!(!snapshot.is_empty());

    let decoded: AgentConfigSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(decoded, snapshot);

    // Omitted for runs that don't configure one, so older servers see no change.
    let default_json = serde_json::to_value(AgentConfigSnapshot::default()).unwrap();
    assert!(default_json.get("computer_use_model_id").is_none());
}

#[test]
fn additional_source_repos_make_snapshot_non_empty() {
    let snapshot = AgentConfigSnapshot {
        additional_source_repos: Some(vec![SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp".to_string(),
        )]),
        ..Default::default()
    };
    assert!(!snapshot.is_empty());
}
