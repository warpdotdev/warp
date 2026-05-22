use std::path::PathBuf;

use ai::skills::{ParsedSkill, SkillProvider, SkillScope};

use super::*;

#[test]
fn test_skill_path_from_file_path_skill_md() {
    let skill = PathBuf::from("/home/user/.claude/skills/my-skill/SKILL.md");
    let result = skill_path_from_file_path(&skill);
    assert_eq!(
        result,
        Some(PathBuf::from("/home/user/.claude/skills/my-skill/SKILL.md"))
    );
}

#[test]
fn test_skill_path_from_file_path_warp_home_skill() {
    let Some(warp_home_skills_dir) = warp_core::paths::warp_home_skills_dir() else {
        eprintln!("Skipping test: Warp home skills directory not available");
        return;
    };
    let warp_home_skill = warp_home_skills_dir
        .join("my-skill")
        .join("assets")
        .join("image.png");
    let result = skill_path_from_file_path(&warp_home_skill);
    assert_eq!(
        result,
        Some(warp_home_skills_dir.join("my-skill").join("SKILL.md"))
    );
}

#[test]
fn test_skill_path_from_file_path_nested_file() {
    let skill_nested = PathBuf::from("/home/user/.agents/skills/my-skill/assets/image.png");
    let result = skill_path_from_file_path(&skill_nested);
    assert_eq!(
        result,
        Some(PathBuf::from("/home/user/.agents/skills/my-skill/SKILL.md"))
    );
}

#[test]
fn test_skill_path_from_file_path_non_skill() {
    let non_skill = PathBuf::from("/home/user/Documents/file.txt");
    let result = skill_path_from_file_path(&non_skill);
    assert_eq!(result, None);

    let similar_path = PathBuf::from("/home/user/.claude/other/file.txt");
    let result = skill_path_from_file_path(&similar_path);
    assert_eq!(result, None);

    let empty_path = PathBuf::from("");
    let result = skill_path_from_file_path(&empty_path);
    assert_eq!(result, None);
}

#[test]
fn test_unique_skills_dedupes_identical_skills_same_dir() {
    let shared_skill_dir = PathBuf::from("/home/user");
    let skill_path1 = shared_skill_dir.join(".agents/skills/my-skill/SKILL.md");
    let skill_path2 = shared_skill_dir.join(".claude/skills/my-skill/SKILL.md");

    let content = "---\nname: test-skill\ndescription: A test skill\n---\nContent here";
    let skill = ParsedSkill {
        path: skill_path1.clone(),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Agents,
        scope: SkillScope::Home,
    };

    let skill2 = ParsedSkill {
        path: skill_path2.clone(),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Claude,
        scope: SkillScope::Home,
    };

    let mut skills_by_path = HashMap::new();
    skills_by_path.insert(skill_path1.clone(), skill);
    skills_by_path.insert(skill_path2.clone(), skill2);

    let skill_paths = vec![
        (shared_skill_dir.clone(), skill_path1),
        (shared_skill_dir, skill_path2),
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
        path: home_path.clone(),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Agents,
        scope: SkillScope::Home,
    };

    let project_skill = ParsedSkill {
        path: project_path.clone(),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    };

    let mut skills_by_path = HashMap::new();
    skills_by_path.insert(home_path.clone(), home_skill);
    skills_by_path.insert(project_path.clone(), project_skill);

    let skill_paths = vec![(home_dir, home_path), (project_dir, project_path)];

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
        path: skill_path1.clone(),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content1.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Agents,
        scope: SkillScope::Home,
    };

    let skill2 = ParsedSkill {
        path: skill_path2.clone(),
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        content: content2.to_string(),
        line_range: Some(8..18),
        provider: SkillProvider::Claude,
        scope: SkillScope::Home,
    };

    let mut skills_by_path = HashMap::new();
    skills_by_path.insert(skill_path1.clone(), skill1);
    skills_by_path.insert(skill_path2.clone(), skill2);

    let skill_paths = vec![
        (shared_skill_dir.clone(), skill_path1),
        (shared_skill_dir, skill_path2),
    ];

    let result = unique_skills(&skill_paths, &skills_by_path);
    assert_eq!(
        result.len(),
        2,
        "Skills with different content should not be deduped even if same directory and name"
    );
}
