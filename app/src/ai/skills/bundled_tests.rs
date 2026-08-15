use ai::skills::{ParsedSkill, SkillProvider, SkillScope};
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;

use super::*;

fn bundled_skill(content: &str) -> BundledSkill {
    let mut bundled_skill = BundledSkill::default();
    bundled_skill.insert_for_testing(
        "test-skill",
        ParsedSkill {
            name: "test-skill".to_string(),
            description: "Test skill".to_string(),
            path: LocalOrRemotePath::Local("/bundled/skills/test-skill/SKILL.md".into()),
            content: content.to_string(),
            line_range: None,
            provider: SkillProvider::Warp,
            scope: SkillScope::Bundled,
        },
        BundledSkillActivation::Always,
    );
    bundled_skill
}

#[test]
fn unavailable_bundled_context_path_renders_as_empty_string() {
    assert_eq!(display_optional_path(None), "");
}

fn remote_content<'a>(bundled_skills: &'a BundledSkills, host_id: &HostId) -> Option<&'a str> {
    bundled_skills
        .remote(host_id)?
        .skill("test-skill")
        .map(|skill| skill.content.as_str())
}

#[test]
fn factory_mcp_bundled_skill_bootstraps_canonical_mcp_resource() {
    let skill = include_str!("../../../../resources/bundled/skills/factory-mcp/SKILL.md");

    assert!(skill.contains("skill://warp/factory-mcp/SKILL.md"));
    assert!(!skill.contains("references/factory-mcp-tools.md"));
}

/// The Factory files skill is always bundled, so a stale trigger description
/// or a broken reference silently reaches every GUI, TUI, and Oz agent.
#[test]
fn factory_files_bundled_skill_is_always_active_and_scoped_to_authoring() {
    let skills_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../resources/bundled/skills")
        .canonicalize()
        .expect("bundled skills directory");
    let skill_dir = skills_dir.join("factory-files");
    let skill = parse_bundled_skill(&skill_dir.join("SKILL.md")).expect("factory-files parses");

    assert_eq!(skill.name, "factory-files");
    let description = skill.description.to_lowercase();
    for intent in ["create", "edit", "factory.yaml", "runner"] {
        assert!(
            description.contains(intent),
            "trigger description should mention {intent}: {description}"
        );
    }
    assert!(
        description.contains("factory mcp"),
        "trigger description should exclude Factory MCP operation: {description}"
    );

    assert!(matches!(
        activation_for_bundled_skill("factory-files", &skills_dir),
        BundledSkillActivation::Always
    ));

    for reference in [
        "references/schema.md",
        "references/triggers.md",
        "references/examples.md",
        "references/validation.md",
        "scripts/validate_factory_files.py",
    ] {
        assert!(
            skill.content.contains(reference),
            "SKILL.md should point at {reference}"
        );
        assert!(
            skill_dir.join(reference).is_file(),
            "{reference} should exist"
        );
    }
}

/// The schemas are the contract the skill tells agents to author against, so
/// they have to stay parseable and keep the Factory document's shape.
#[test]
fn factory_files_schemas_are_parseable_and_keep_the_factory_contract() {
    let schemas_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../resources/bundled/skills/factory-files/schemas");

    for name in [
        "common.schema.json",
        "factory.schema.json",
        "agent.schema.json",
        "automation.schema.json",
        "runner.schema.json",
    ] {
        let raw = std::fs::read_to_string(schemas_dir.join(name))
            .unwrap_or_else(|error| panic!("read {name}: {error}"));
        let schema: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("{name} should be valid JSON: {error}"));
        assert!(schema.get("$id").is_some(), "{name} should declare $id");
    }

    let factory: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(schemas_dir.join("factory.schema.json")).unwrap(),
    )
    .unwrap();
    let required: Vec<&str> = factory["required"]
        .as_array()
        .expect("factory schema declares required fields")
        .iter()
        .map(|value| value.as_str().expect("required entries are strings"))
        .collect();
    assert_eq!(
        required,
        ["schemaVersion", "name", "repositories", "agentDefaults"]
    );
    assert_eq!(
        factory["additionalProperties"],
        serde_json::Value::Bool(false)
    );
}

#[test]
fn local_and_remote_catalogs_are_isolated() {
    let first_host_id = HostId::new("first-host".to_string());
    let second_host_id = HostId::new("second-host".to_string());
    let mut bundled_skills = BundledSkills::default();
    bundled_skills.set_local(bundled_skill("local"));
    bundled_skills.insert_remote(first_host_id.clone(), bundled_skill("first"));
    bundled_skills.insert_remote(second_host_id.clone(), bundled_skill("second"));

    assert_eq!(
        bundled_skills
            .local_skill("test-skill")
            .map(|skill| skill.content.as_str()),
        Some("local")
    );
    assert_eq!(
        remote_content(&bundled_skills, &first_host_id),
        Some("first")
    );
    assert_eq!(
        remote_content(&bundled_skills, &second_host_id),
        Some("second")
    );

    // A reconnect refresh replaces the host's catalog wholesale.
    bundled_skills.insert_remote(first_host_id.clone(), bundled_skill("first-refreshed"));
    assert_eq!(
        remote_content(&bundled_skills, &first_host_id),
        Some("first-refreshed")
    );

    // Disconnecting one host leaves the local and sibling-host catalogs intact.
    bundled_skills.remove_remote(&first_host_id);
    assert_eq!(
        bundled_skills
            .local_skill("test-skill")
            .map(|skill| skill.content.as_str()),
        Some("local")
    );
    assert_eq!(remote_content(&bundled_skills, &first_host_id), None);
    assert_eq!(
        remote_content(&bundled_skills, &second_host_id),
        Some("second")
    );
}
