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
/// or a broken reference silently reaches every GUI, TUI, and Oz agent. Its
/// trigger has to stay anchored to a factory.yaml root: `agents/<name>/agent.md`
/// alone also describes unrelated agent-definition files.
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
    for intent in ["create", "edit", "factory.yaml", "runner", "scorer"] {
        assert!(
            description.contains(intent),
            "trigger description should mention {intent}: {description}"
        );
    }
    assert!(
        description.contains("factory mcp"),
        "trigger description should exclude Factory MCP operation: {description}"
    );
    assert!(
        description.contains("rooted at a factory.yaml"),
        "trigger description should anchor to a factory.yaml root: {description}"
    );
    assert!(
        description.contains("belongs to another tool"),
        "trigger description should exclude other tools' agent files: {description}"
    );
    assert!(
        skill.content.contains("no `factory.yaml`"),
        "SKILL.md should tell the agent to stop outside a Factory tree"
    );

    assert!(matches!(
        activation_for_bundled_skill("factory-files", &skills_dir),
        BundledSkillActivation::Always
    ));

    for reference in [
        "references/scorers.md",
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

/// The skill must not carry a copy of the Factory file format.
///
/// A bundled copy ships inside a Warp release and goes stale against the
/// warp-server it is used against. A stale copy does not fail quietly: it
/// reports fields the server accepts as unknown, and an agent clearing that
/// diagnostic deletes working configuration. An earlier revision did exactly
/// that to the Linear and Slack trigger aliases. The format is fetched from
/// the server now, so nothing here should describe it.
#[test]
fn factory_files_skill_carries_no_copy_of_the_format() {
    let skill_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../resources/bundled/skills/factory-files")
        .canonicalize()
        .expect("factory-files skill directory");

    let mut schemas = Vec::new();
    let mut pending = vec![skill_dir.clone()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("read skill directory") {
            let path = entry.expect("read skill entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.to_string_lossy().ends_with(".schema.json") {
                schemas.push(path);
            }
        }
    }
    assert!(
        schemas.is_empty(),
        "the skill has regrown bundled schemas, which go stale against the server \
         and produce false rejections; fetch the format instead: {schemas:?}"
    );

    let validator = std::fs::read_to_string(skill_dir.join("scripts/validate_factory_files.py"))
        .expect("read the validator");
    for banned in ["import yaml", "def load_yaml", "jsonschema"] {
        assert!(
            !validator.contains(banned),
            "the validator parses the format again ({banned}); it should send bytes \
             to the server and relay the verdict"
        );
    }
    assert!(
        validator.contains("/api/v1/factory-files/validate"),
        "the validator should reach the server's validation endpoint"
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
