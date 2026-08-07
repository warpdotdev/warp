use ai::skills::{ParsedSkill, SkillProvider, SkillScope};
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;

use super::*;

/// The repository's `resources/` root, which is what ships as the app
/// bundle's resources directory.
fn repo_resources_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the app crate always has a repository root")
        .join("resources")
}

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

#[test]
fn bundled_warp_intro_tour_skill_covers_every_tour_surface() {
    let resources_dir = repo_resources_dir();
    let skills_dir = resources_dir.join("bundled").join("skills");
    let skills = futures::executor::block_on(read_bundled_skills(&skills_dir, &resources_dir));

    let skill = skills
        .get("warp-intro-tour")
        .expect("the intro tour skill ships in resources/bundled/skills");
    assert_eq!(skill.name, "warp-intro-tour");
    assert!(!skill.description.is_empty());

    // Every handlebars variable the tour uses must be one the bundled context
    // supplies, otherwise the agent is handed a literal `{{...}}` to run.
    assert!(
        !skill.content.contains("{{"),
        "the intro tour skill contains an unrendered template variable"
    );
    assert!(
        skill
            .content
            .contains(ChannelState::channel().warpctrl_command_name()),
        "the intro tour skill must invoke the channel's warpctrl command"
    );

    // The tour's required stops, as named in the ticket's acceptance criteria.
    for command in [
        "surface settings open",
        "pane split --direction right",
        "surface keybindings open",
        "surface warp-drive open",
    ] {
        assert!(
            skill.content.contains(command),
            "the intro tour skill must demonstrate `{command}`"
        );
    }
}

fn remote_content<'a>(bundled_skills: &'a BundledSkills, host_id: &HostId) -> Option<&'a str> {
    bundled_skills
        .remote(host_id)?
        .skill("test-skill")
        .map(|skill| skill.content.as_str())
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
