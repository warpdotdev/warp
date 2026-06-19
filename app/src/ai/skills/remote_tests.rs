use ai::skills::{ParsedSkill, SkillProvider, SkillScope};
use tempfile::TempDir;

use super::*;

fn daemon_skill(id: &str, content: &str) -> ParsedSkill {
    ParsedSkill {
        name: id.to_string(),
        description: format!("{id} description"),
        path: LocalOrRemotePath::Local(format!("/daemon/bundled/skills/{id}/SKILL.md").into()),
        content: content.to_string(),
        line_range: None,
        provider: SkillProvider::Warp,
        scope: SkillScope::Bundled,
    }
}

fn bundled_metadata(proto: &RemoteSkillProto) -> &BundledSkillMetadata {
    let Some(remote_skill_proto::Source::Bundled(metadata)) = proto.source.as_ref() else {
        panic!("expected bundled skill metadata");
    };
    metadata
}

fn bundled_skill_proto(
    id: &str,
    path: &str,
    content: &str,
    requires_mcp: Option<&str>,
) -> RemoteSkillProto {
    RemoteSkillProto {
        path: path.to_string(),
        content: content.to_string(),
        source: Some(remote_skill_proto::Source::Bundled(BundledSkillMetadata {
            id: id.to_string(),
            requires_mcp: requires_mcp.map(str::to_string),
        })),
    }
}
#[test]
fn snapshot_protos_serialize_activation_conditions() {
    let temp_dir = TempDir::new().unwrap();
    let present_file = temp_dir.path().join("settings_schema.json");
    std::fs::write(&present_file, "{}").unwrap();
    let missing_file = temp_dir.path().join("missing.json");

    let catalog = BundledSkill::from_definitions([
        (
            "always-skill".to_string(),
            daemon_skill("always-skill", "# always"),
            BundledSkillActivation::Always,
        ),
        (
            "figma-skill".to_string(),
            daemon_skill("figma-skill", "# figma"),
            BundledSkillActivation::RequiresMcp(McpIntegration::Figma),
        ),
        (
            "file-present-skill".to_string(),
            daemon_skill("file-present-skill", "# file"),
            BundledSkillActivation::RequiresFile(present_file),
        ),
        (
            "file-missing-skill".to_string(),
            daemon_skill("file-missing-skill", "# missing"),
            BundledSkillActivation::RequiresFile(missing_file),
        ),
    ]);

    let protos = bundled_skill_snapshot_protos(&catalog);

    // `RequiresFile` is evaluated daemon-side: the missing-file skill is
    // dropped, the present-file skill ships as unconditionally active.
    let mut ids: Vec<&str> = protos
        .iter()
        .map(|proto| bundled_metadata(proto).id.as_str())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, ["always-skill", "figma-skill", "file-present-skill"]);

    let figma = protos
        .iter()
        .find(|proto| bundled_metadata(proto).id == "figma-skill")
        .unwrap();
    assert_eq!(
        bundled_metadata(figma).requires_mcp.as_deref(),
        Some("figma")
    );
    for proto in protos
        .iter()
        .filter(|proto| bundled_metadata(proto).id != "figma-skill")
    {
        assert_eq!(bundled_metadata(proto).requires_mcp, None);
    }

    let always = protos
        .iter()
        .find(|proto| bundled_metadata(proto).id == "always-skill")
        .unwrap();
    assert_eq!(always.path, "/daemon/bundled/skills/always-skill/SKILL.md");
    assert_eq!(always.content, "# always");
}

#[test]
fn bundled_skill_from_protos_builds_host_scoped_catalog() {
    let host_id = HostId::new("remote-host".to_string());
    let protos = vec![
        bundled_skill_proto(
            "test-skill",
            "/remote/bundled/skills/test-skill/SKILL.md",
            "---\nname: test-skill\ndescription: A test skill\n---\nbody",
            None,
        ),
        bundled_skill_proto(
            "figma-skill",
            "/remote/bundled/skills/figma-skill/SKILL.md",
            "# figma",
            Some("figma"),
        ),
        // Unknown integration (e.g. a newer daemon): the client cannot
        // evaluate the condition, so the skill is skipped.
        bundled_skill_proto(
            "unknown-mcp-skill",
            "/remote/bundled/skills/unknown-mcp-skill/SKILL.md",
            "# unknown",
            Some("not-a-real-integration"),
        ),
        // Invalid (relative) path: skipped.
        bundled_skill_proto("bad-path-skill", "relative/SKILL.md", "# bad", None),
    ];

    let catalog = bundled_skill_from_protos(&host_id, &protos);

    let skill = catalog.skill("test-skill").expect("test-skill present");
    // The skill's identity is re-parsed from the daemon-rendered content.
    assert_eq!(skill.name, "test-skill");
    assert_eq!(skill.description, "A test skill");
    assert_eq!(
        skill.path,
        LocalOrRemotePath::Remote(RemotePath::new(
            host_id.clone(),
            StandardizedPath::try_new("/remote/bundled/skills/test-skill/SKILL.md").unwrap(),
        ))
    );
    assert_eq!(skill.scope, SkillScope::Bundled);
    assert_eq!(skill.provider, SkillProvider::Warp);

    assert!(catalog.skill("figma-skill").is_some());
    assert!(catalog.skill("unknown-mcp-skill").is_none());
    assert!(catalog.skill("bad-path-skill").is_none());
}
