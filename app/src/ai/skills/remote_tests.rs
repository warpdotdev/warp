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

    let mut protos = bundled_skills_snapshot_protos(&catalog);
    protos.sort_by(|a, b| a.id.cmp(&b.id));

    // `RequiresFile` is evaluated daemon-side: the missing-file skill is
    // dropped, the present-file skill ships as unconditionally active.
    let ids: Vec<&str> = protos.iter().map(|proto| proto.id.as_str()).collect();
    assert_eq!(ids, ["always-skill", "figma-skill", "file-present-skill"]);

    let figma = protos
        .iter()
        .find(|proto| proto.id == "figma-skill")
        .unwrap();
    assert_eq!(figma.requires_mcp.as_deref(), Some("figma"));
    for proto in protos.iter().filter(|proto| proto.id != "figma-skill") {
        assert_eq!(proto.requires_mcp, None);
    }

    let always = protos
        .iter()
        .find(|proto| proto.id == "always-skill")
        .unwrap();
    assert_eq!(always.path, "/daemon/bundled/skills/always-skill/SKILL.md");
    assert_eq!(always.content, "# always");
}

#[test]
fn bundled_skill_from_protos_builds_host_scoped_catalog() {
    let host_id = HostId::new("remote-host".to_string());
    let protos = vec![
        BundledSkillProto {
            id: "test-skill".to_string(),
            name: "ignored".to_string(),
            description: "ignored".to_string(),
            path: "/remote/bundled/skills/test-skill/SKILL.md".to_string(),
            content: "---\nname: test-skill\ndescription: A test skill\n---\nbody".to_string(),
            requires_mcp: None,
        },
        BundledSkillProto {
            id: "figma-skill".to_string(),
            name: "figma-skill".to_string(),
            description: "Figma helper".to_string(),
            path: "/remote/bundled/skills/figma-skill/SKILL.md".to_string(),
            content: "# figma".to_string(),
            requires_mcp: Some("figma".to_string()),
        },
        // Unknown integration (e.g. a newer daemon): the client cannot
        // evaluate the condition, so the skill is skipped.
        BundledSkillProto {
            id: "unknown-mcp-skill".to_string(),
            name: "unknown-mcp-skill".to_string(),
            description: "Unknown".to_string(),
            path: "/remote/bundled/skills/unknown-mcp-skill/SKILL.md".to_string(),
            content: "# unknown".to_string(),
            requires_mcp: Some("not-a-real-integration".to_string()),
        },
        // Invalid (relative) path: skipped.
        BundledSkillProto {
            id: "bad-path-skill".to_string(),
            name: "bad-path-skill".to_string(),
            description: "Bad path".to_string(),
            path: "relative/SKILL.md".to_string(),
            content: "# bad".to_string(),
            requires_mcp: None,
        },
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
