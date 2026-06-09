use std::path::PathBuf;

use ai::skills::{ParsedSkill, SkillProvider, SkillScope};
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;

use super::*;

fn remote_location(path: &str) -> LocalOrRemotePath {
    LocalOrRemotePath::Remote(RemotePath::new(
        HostId::new("remote-host".to_string()),
        StandardizedPath::try_new(path).unwrap(),
    ))
}

#[test]
fn skill_path_from_unix_encoded_remote_location() {
    let location = remote_location("/repo/.agents/skills/deploy/scripts/run.sh");

    assert_eq!(
        skill_path_from_location(&location),
        Some(remote_location("/repo/.agents/skills/deploy/SKILL.md"))
    );
}

#[test]
fn skill_path_from_windows_encoded_remote_location() {
    let location = remote_location(r"C:\repo\.agents\skills\deploy\scripts\run.ps1");

    assert_eq!(
        skill_path_from_location(&location),
        Some(remote_location(r"C:\repo\.agents\skills\deploy\SKILL.md"))
    );
}
#[test]
fn test_unique_skills_dedupes_identical_skills_same_dir() {
    let shared_skill_dir = PathBuf::from("/home/user");
    let skill_path1 = shared_skill_dir.join(".agents/skills/my-skill/SKILL.md");
    let skill_path2 = shared_skill_dir.join(".claude/skills/my-skill/SKILL.md");

    let content = "---\nname: test-skill\ndescription: A test skill\n---\nContent here";
    let skill = ParsedSkill {
        path: LocalOrRemotePath::Local(skill_path1.clone()),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Agents,
        scope: SkillScope::Home,
    };

    let skill2 = ParsedSkill {
        path: LocalOrRemotePath::Local(skill_path2.clone()),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Claude,
        scope: SkillScope::Home,
    };

    let mut skills_by_path = HashMap::new();
    skills_by_path.insert(LocalOrRemotePath::Local(skill_path1.clone()), skill);
    skills_by_path.insert(LocalOrRemotePath::Local(skill_path2.clone()), skill2);

    let skill_paths = vec![
        (
            LocalOrRemotePath::Local(shared_skill_dir.clone()),
            LocalOrRemotePath::Local(skill_path1),
        ),
        (
            LocalOrRemotePath::Local(shared_skill_dir),
            LocalOrRemotePath::Local(skill_path2),
        ),
    ];

    let result = unique_skills(&skill_paths, &skills_by_path);
    assert_eq!(result.len(), 1);
    // Agents has higher priority (index 0) than Claude, so it should be preferred
    assert_eq!(result[0].provider, SkillProvider::Agents);
}

#[test]
fn test_unique_skills_does_not_dedupe_different_dirs() {
    let home_dir = PathBuf::from("/home/user");
    let project_dir = PathBuf::from("/home/user/projects/repo");
    let home_path = home_dir.join(".agents/skills/my-skill/SKILL.md");
    let project_path = project_dir.join(".agents/skills/my-skill/SKILL.md");

    let content = "---\nname: test-skill\ndescription: A test skill\n---\nContent here";
    let home_skill = ParsedSkill {
        path: LocalOrRemotePath::Local(home_path.clone()),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Agents,
        scope: SkillScope::Home,
    };

    let project_skill = ParsedSkill {
        path: LocalOrRemotePath::Local(project_path.clone()),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    };

    let mut skills_by_path = HashMap::new();
    skills_by_path.insert(LocalOrRemotePath::Local(home_path.clone()), home_skill);
    skills_by_path.insert(
        LocalOrRemotePath::Local(project_path.clone()),
        project_skill,
    );

    let skill_paths = vec![
        (
            LocalOrRemotePath::Local(home_dir),
            LocalOrRemotePath::Local(home_path),
        ),
        (
            LocalOrRemotePath::Local(project_dir),
            LocalOrRemotePath::Local(project_path),
        ),
    ];

    let result = unique_skills(&skill_paths, &skills_by_path);
    assert_eq!(
        result.len(),
        2,
        "Skills with same content but different directories should not be deduped"
    );
}

#[test]
fn test_unique_skills_does_not_dedupe_different_content() {
    let shared_skill_dir = PathBuf::from("/home/user");
    let skill_path1 = shared_skill_dir.join(".agents/skills/my-skill/SKILL.md");
    let skill_path2 = shared_skill_dir.join(".claude/skills/my-skill/SKILL.md");

    let content1 = "---\nname: test-skill\ndescription: A test skill\n---\nContent here";
    let content2 = "---\nname: test-skill\ndescription: A test skill\n---\nDifferent content";

    let skill1 = ParsedSkill {
        path: LocalOrRemotePath::Local(skill_path1.clone()),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content1.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Agents,
        scope: SkillScope::Home,
    };

    let skill2 = ParsedSkill {
        path: LocalOrRemotePath::Local(skill_path2.clone()),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content2.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Claude,
        scope: SkillScope::Home,
    };

    let mut skills_by_path = HashMap::new();
    skills_by_path.insert(LocalOrRemotePath::Local(skill_path1.clone()), skill1);
    skills_by_path.insert(LocalOrRemotePath::Local(skill_path2.clone()), skill2);

    let skill_paths = vec![
        (
            LocalOrRemotePath::Local(shared_skill_dir.clone()),
            LocalOrRemotePath::Local(skill_path1),
        ),
        (
            LocalOrRemotePath::Local(shared_skill_dir),
            LocalOrRemotePath::Local(skill_path2),
        ),
    ];

    let result = unique_skills(&skill_paths, &skills_by_path);
    assert_eq!(
        result.len(),
        2,
        "Skills with different content should not be deduped even if same directory and name"
    );
}
