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

#[test]
fn experimental_values_remain_uninterpreted_across_versions() {
    let raw = serde_json::json!({
        "experimental": {
            "identityOnlySystemPrompt": true,
            "future": null,
            "nestedFromNewerServer": {"value": [1, false]}
        }
    });
    let snapshot: AgentConfigSnapshot = serde_json::from_value(raw.clone()).unwrap();
    assert!(!snapshot.is_empty());
    assert_eq!(serde_json::to_value(snapshot).unwrap(), raw);
    assert!(
        serde_json::from_value::<AgentConfigSnapshot>(serde_json::json!({}))
            .unwrap()
            .experimental
            .is_none()
    );
}
