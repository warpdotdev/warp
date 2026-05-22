use warp_multi_agent_api as api;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;

use super::{
    skill_reference_from_api_skill_ref, skill_reference_from_read_skill_ref, SkillConversionError,
    SkillPathOrigin,
};
use crate::skills::{ParsedSkill, SkillReference};

fn api_project_skill(path: &str) -> api::Skill {
    api::Skill {
        descriptor: Some(api::SkillDescriptor {
            skill_reference: Some(api::skill_descriptor::SkillReference::Path(
                path.to_string(),
            )),
            name: "deploy".to_string(),
            description: "Deploy the service".to_string(),
            scope: Some(api::skill_descriptor::Scope {
                r#type: Some(api::skill_descriptor::scope::Type::Project(())),
            }),
            provider: Some(api::skill_descriptor::Provider {
                r#type: Some(api::skill_descriptor::provider::Type::Agents(())),
            }),
        }),
        content: Some(api::FileContent {
            file_path: path.to_string(),
            content: "# Deploy".to_string(),
            line_range: None,
        }),
    }
}

#[test]
fn try_from_api_with_remote_origin_preserves_host_identity() {
    let host_id = HostId::new("remote-host".to_string());
    let parsed = ParsedSkill::try_from_api_with_origin(
        api_project_skill("/repo/.agents/skills/deploy/SKILL.md"),
        &SkillPathOrigin::Remote {
            host_id: host_id.clone(),
        },
    )
    .expect("remote project skill should convert");

    let LocalOrRemotePath::Remote(path) = parsed.path else {
        panic!("expected a remote skill path");
    };
    assert_eq!(path.host_id, host_id);
    assert_eq!(path.path.as_str(), "/repo/.agents/skills/deploy/SKILL.md");
}

#[test]
fn try_from_api_with_unavailable_origin_rejects_path_based_skills() {
    let error = ParsedSkill::try_from_api_with_origin(
        api_project_skill("/repo/.agents/skills/deploy/SKILL.md"),
        &SkillPathOrigin::Unavailable,
    )
    .expect_err("restored skills without host context should not fabricate local paths");

    assert!(matches!(error, SkillConversionError::PathOriginUnavailable));
}

#[test]
fn skill_ref_with_unavailable_origin_preserves_bundled_skills() {
    let skill_reference = skill_reference_from_api_skill_ref(
        api::SkillRef {
            skill_reference: Some(api::skill_ref::SkillReference::BundledSkillId(
                "review-comments".to_string(),
            )),
        },
        &SkillPathOrigin::Unavailable,
    );

    assert_eq!(
        skill_reference,
        Some(SkillReference::BundledSkillId(
            "review-comments".to_string()
        ))
    );
}

#[test]
fn read_skill_ref_with_remote_origin_preserves_host_identity() {
    let host_id = HostId::new("remote-host".to_string());
    let skill_reference = skill_reference_from_read_skill_ref(
        api::message::tool_call::read_skill::SkillReference::SkillPath(
            "/repo/.agents/skills/deploy/SKILL.md".to_string(),
        ),
        &SkillPathOrigin::Remote {
            host_id: host_id.clone(),
        },
    )
    .expect("remote read_skill skill references should convert");

    let SkillReference::Path(LocalOrRemotePath::Remote(path)) = skill_reference else {
        panic!("expected a remote skill path");
    };
    assert_eq!(path.host_id, host_id);
    assert_eq!(path.path.as_str(), "/repo/.agents/skills/deploy/SKILL.md");
}
