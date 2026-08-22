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
fn source_repos_to_clone_absent_empty_and_populated_round_trip() {
    let absent: AgentConfigSnapshot = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(absent.source_repos_to_clone.is_none());
    assert!(absent.deferred_source_repos.is_empty());
    assert!(absent.is_empty());

    let empty = AgentConfigSnapshot {
        source_repos_to_clone: Some(vec![]),
        ..Default::default()
    };
    let json = serde_json::to_value(&empty).unwrap();
    assert_eq!(json["source_repos_to_clone"], serde_json::json!([]));
    let decoded: AgentConfigSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(decoded, empty);
    // `Some(empty)` is authoritative, not the same thing as an absent field.
    assert!(!empty.is_empty());

    let populated = AgentConfigSnapshot {
        source_repos_to_clone: Some(vec![SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp".to_string(),
        )]),
        deferred_source_repos: vec![SourceRepo::new(
            CodeForge::GitLab,
            "platform/backend".to_string(),
            "api".to_string(),
        )],
        ..Default::default()
    };
    let json = serde_json::to_value(&populated).unwrap();
    assert_eq!(
        json["source_repos_to_clone"][0],
        serde_json::json!({"code_forge": "GITHUB", "owner": "warpdotdev", "repo": "warp"})
    );
    assert_eq!(
        json["deferred_source_repos"][0],
        serde_json::json!({"code_forge": "GITLAB", "owner": "platform/backend", "repo": "api"})
    );
    let decoded: AgentConfigSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(decoded, populated);
}
