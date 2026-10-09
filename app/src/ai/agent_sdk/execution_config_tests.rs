use warp_graphql::queries::execution_config::{
    ExecutionRepositoryRef, SessionSharingAccessLevel, SessionSharingSubjectType,
};

use super::*;
use crate::workspaces::user_workspaces::TeamScope;

#[test]
fn execution_scope_uses_configuration_team_uid() {
    let team_id = ServerId::from(7);
    assert_eq!(
        team_scope(Some(&cynic::Id::new(team_id.to_string())))
            .unwrap()
            .team_uid(),
        Some(team_id)
    );
    assert_eq!(team_scope(None).unwrap().team_uid(), None);
    assert!(team_scope(Some(&cynic::Id::new("invalid-uid"))).is_err());
}

#[test]
fn execution_mcp_config_preserves_literal_environment_and_headers() {
    let json = r#"{"literal":{"url":"https://example.com/mcp","headers":{"Authorization":"Bearer literal-value"},"env":{"TOKEN":"literal-value"}}}"#;
    let specs = mcp_specs(json).unwrap();
    assert_eq!(specs.len(), 1);
    let MCPSpec::Json(spec) = &specs[0] else {
        panic!("expected inline MCP config");
    };
    let map: Map<String, Value> = serde_json::from_str(spec).unwrap();
    assert_eq!(map["literal"]["env"]["TOKEN"], "literal-value");
    assert_eq!(
        map["literal"]["headers"]["Authorization"],
        "Bearer literal-value"
    );
    assert!(mcp_specs("[]").is_err());
    assert!(mcp_specs(r#"{"invalid":{"url":4}}"#).is_err());
}

#[test]
fn execution_repositories_keep_individual_origin_policy_and_refs() {
    let resolved = repositories(vec![
        ExecutionRepository {
            forge: CodeForge::GitHub,
            owner: "owner".into(),
            name: "first".into(),
            ref_: None,
            clone_from: None,
            preserve_origin: true,
        },
        ExecutionRepository {
            forge: CodeForge::GitLab,
            owner: "owner".into(),
            name: "second".into(),
            ref_: Some(ExecutionRepositoryRef {
                type_: ExecutionRepositoryRefType::CommitSha,
                value: "abcdef".into(),
            }),
            clone_from: Some(ConfigSourceRepo {
                code_forge: CodeForge::GitLab,
                owner: "source".into(),
                repo: "second".into(),
            }),
            preserve_origin: false,
        },
    ])
    .unwrap();
    assert!(resolved[0].preserve_origin);
    assert!(!resolved[1].preserve_origin);
    assert!(matches!(
        &resolved[1].checkout,
        Some(RepositoryHeadRef::CommitSha(sha)) if sha == "abcdef"
    ));
    assert_eq!(resolved[1].clone_from.as_ref().unwrap().owner, "source");
}

#[test]
fn execution_deferred_repositories_reject_unknown_forges() {
    assert!(
        deferred_repositories(vec![ConfigSourceRepo {
            code_forge: CodeForge::Unknown,
            owner: "owner".into(),
            repo: "repo".into(),
        }])
        .is_err()
    );
}

#[test]
fn execution_acls_reject_missing_email_and_unknown_access() {
    assert!(
        sharing_acls(vec![SessionSharingAclSpec {
            subject_type: SessionSharingSubjectType::UserEmail,
            email: None,
            access: SessionSharingAccessLevel::Edit,
        }])
        .is_err()
    );
    assert!(
        sharing_acls(vec![SessionSharingAclSpec {
            subject_type: SessionSharingSubjectType::Public,
            email: None,
            access: SessionSharingAccessLevel::Unknown,
        }])
        .is_err()
    );
}

#[test]
fn execution_factory_skill_directories_are_relative_and_unambiguous() {
    assert_eq!(
        factory_skill_dirs(vec![".agents/skills".into(), "repo/skills".into()]).unwrap(),
        vec![
            PathBuf::from(".agents/skills"),
            PathBuf::from("repo/skills")
        ]
    );
    assert!(factory_skill_dirs(vec!["/absolute/skills".into()]).is_err());
    assert!(factory_skill_dirs(vec!["one,two".into()]).is_err());
    assert!(factory_skill_dirs(vec!["../other/skills".into()]).is_err());
    assert!(factory_skill_dirs(vec!["~/skills".into()]).is_err());
}
