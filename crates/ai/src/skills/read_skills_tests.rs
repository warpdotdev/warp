use super::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_read_skills_with_valid_skills() {
    let temp_dir = tempdir().unwrap();
    // Create .agents/skills directory structure so skills can have a valid provider
    let skills_dir = temp_dir.path().join(".agents/skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create first skill directory with valid SKILL.md
    let skill1_dir = skills_dir.join("skill1");
    fs::create_dir(&skill1_dir).unwrap();
    fs::write(
        skill1_dir.join("SKILL.md"),
        r#"---
name: test-skill-1
description: First test skill
---

# Test Skill 1
This is the first test skill.
"#,
    )
    .unwrap();

    // Create second skill directory with valid SKILL.md
    let skill2_dir = skills_dir.join("skill2");
    fs::create_dir(&skill2_dir).unwrap();
    fs::write(
        skill2_dir.join("SKILL.md"),
        r#"---
name: test-skill-2
description: Second test skill
---

# Test Skill 2
This is the second test skill.
"#,
    )
    .unwrap();

    let skills = read_skills(&skills_dir);

    assert_eq!(skills.len(), 2);

    // Find each skill by name
    let skill1 = skills.iter().find(|s| s.name == "test-skill-1").unwrap();
    assert_eq!(
        skill1.path,
        skill1_dir.join("SKILL.md").to_string_lossy().to_string()
    );
    assert_eq!(skill1.description, "First test skill");
    assert!(skill1.content.contains("# Test Skill 1"));
    assert!(skill1.content.contains("---"));
    assert!(skill1.content.contains("name: test-skill-1"));
    assert_eq!(skill1.line_range, Some(5..8)); // Front matter is lines 1-4, markdown starts at line 5

    let skill2 = skills.iter().find(|s| s.name == "test-skill-2").unwrap();
    assert_eq!(
        skill2.path,
        skill2_dir.join("SKILL.md").to_string_lossy().to_string()
    );
    assert_eq!(skill2.description, "Second test skill");
    assert!(skill2.content.contains("# Test Skill 2"));
    assert!(skill2.content.contains("---"));
    assert!(skill2.content.contains("name: test-skill-2"));
    assert_eq!(skill2.line_range, Some(5..8)); // Front matter is lines 1-4, markdown starts at line 5
}

#[test]
fn test_read_skills_ignores_only_truly_invalid_files() {
    let temp_dir = tempdir().unwrap();
    // Create .agents/skills directory structure so skills can have a valid provider
    let skills_dir = temp_dir.path().join(".agents/skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create valid skill
    let valid_skill_dir = skills_dir.join("valid-skill");
    fs::create_dir(&valid_skill_dir).unwrap();
    fs::write(
        valid_skill_dir.join("SKILL.md"),
        r#"---
name: valid-skill
description: Valid skill
---

# Valid Skill
"#,
    )
    .unwrap();

    // Create skill missing name (now valid via directory-name fallback)
    let invalid_skill_dir = skills_dir.join("invalid-skill");
    fs::create_dir(&invalid_skill_dir).unwrap();
    fs::write(
        invalid_skill_dir.join("SKILL.md"),
        r#"---
description: Invalid skill missing name
---

# Invalid Skill
"#,
    )
    .unwrap();

    // Create skill with no front matter (now valid via full fallback)
    let no_frontmatter_dir = skills_dir.join("no-frontmatter-skill");
    fs::create_dir(&no_frontmatter_dir).unwrap();
    fs::write(
        no_frontmatter_dir.join("SKILL.md"),
        r#"# No Front Matter Skill

No front matter here.
"#,
    )
    .unwrap();

    let skills = read_skills(&skills_dir);

    // All three skills should be returned — none are truly invalid
    assert_eq!(skills.len(), 3);

    let valid_skill = skills.iter().find(|s| s.name == "valid-skill").unwrap();
    assert_eq!(valid_skill.description, "Valid skill");
    assert!(valid_skill.content.contains("---"));
    assert_eq!(valid_skill.line_range, Some(5..7));

    let fallback_name_skill = skills.iter().find(|s| s.name == "invalid-skill").unwrap();
    assert_eq!(
        fallback_name_skill.description,
        "Invalid skill missing name"
    );
    assert!(fallback_name_skill.content.contains("# Invalid Skill"));

    let no_fm_skill = skills
        .iter()
        .find(|s| s.name == "no-frontmatter-skill")
        .unwrap();
    assert_eq!(no_fm_skill.description, "No front matter here.");
    assert!(no_fm_skill.line_range.is_none());
}

#[test]
fn test_read_skills_empty_directory() {
    let temp_dir = tempdir().unwrap();
    let skills_dir = temp_dir.path();

    let skills = read_skills(skills_dir);

    assert_eq!(skills.len(), 0);
}

#[test]
fn test_read_skills_no_skill_files() {
    let temp_dir = tempdir().unwrap();
    let skills_dir = temp_dir.path();

    // Create directories without SKILL.md files
    let dir1 = skills_dir.join("dir1");
    fs::create_dir(&dir1).unwrap();

    let dir2 = skills_dir.join("dir2");
    fs::create_dir(&dir2).unwrap();
    fs::write(dir2.join("README.md"), "Not a skill file").unwrap();

    let skills = read_skills(skills_dir);

    assert_eq!(skills.len(), 0);
}

#[test]
fn test_read_skills_ignores_files_in_root() {
    let temp_dir = tempdir().unwrap();
    // Create .agents/skills directory structure so skills can have a valid provider
    let skills_dir = temp_dir.path().join(".agents/skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create a valid skill in a subdirectory
    let skill_dir = skills_dir.join("valid-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: valid-skill
description: Valid skill in subdirectory
---

# Valid Skill
"#,
    )
    .unwrap();

    // Create a SKILL.md file in the root directory (should be ignored)
    fs::write(
        skills_dir.join("SKILL.md"),
        r#"---
name: root-skill
description: This should be ignored
---

# Root Skill
"#,
    )
    .unwrap();

    let skills = read_skills(&skills_dir);

    // Only the skill in the subdirectory should be returned
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "valid-skill");
    assert_eq!(skills[0].line_range, Some(5..7));
}

#[test]
fn test_read_skills_nonexistent_directory() {
    let skills = read_skills(Path::new("/nonexistent/path/that/does/not/exist"));

    assert_eq!(skills.len(), 0);
}
