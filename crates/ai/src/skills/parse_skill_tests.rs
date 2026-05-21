use std::path::PathBuf;
use tempfile::TempDir;

use super::*;

/// Creates a temporary skill file in a .agents/skills directory
fn create_temp_skill_file(content: &str) -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().unwrap();
    let skill_dir = temp_dir.path().join(".agents/skills/test-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let skill_file = skill_dir.join("SKILL.md");
    std::fs::write(&skill_file, content).unwrap();
    (temp_dir, skill_file)
}

#[test]
fn test_parse_with_front_matter() {
    let content = r#"---
name: your-skill-name
description: Brief description of what this Skill does and when to use it
---

# Your Skill Name

## Instructions
Provide clear, step-by-step guidance for Claude.

## Examples
Show concrete examples of using this Skill.
"#;

    let (_temp_dir, skill_file) = create_temp_skill_file(content);
    let result = parse_skill(&skill_file).unwrap();

    assert_eq!(result.name, "your-skill-name");
    assert_eq!(
        result.description,
        "Brief description of what this Skill does and when to use it"
    );
    // Content should include both front matter and markdown
    assert!(result.content.contains("# Your Skill Name"));
    assert!(result.content.contains("## Instructions"));
    assert!(result.content.contains("## Examples"));
    assert!(result.content.contains("---"));
    assert!(result.content.contains("name: your-skill-name"));
    // Verify line_range is set (1-indexed)
    // Front matter is lines 1-4, markdown content starts at line 5
    // Total of 12 lines, so line_range is 5..13
    assert_eq!(result.line_range, Some(5..13));
    // Verify path is the full file path
    assert_eq!(result.path, skill_file);
}

#[test]
fn test_parse_missing_name_falls_back_to_directory_name() {
    let content = r#"---
description: Some description
---

# Content

This paragraph is for body parsing.
"#;

    let (_temp_dir, skill_file) = create_temp_skill_file(content);
    let result = parse_skill(&skill_file).unwrap();

    assert_eq!(result.name, "test-skill");
    assert_eq!(result.description, "Some description");
}

#[test]
fn test_parse_missing_description_falls_back_to_first_paragraph() {
    let content = r#"---
name: some-skill
---

# Heading

This is the first paragraph.
Still first paragraph.

Second paragraph.
"#;

    let (_temp_dir, skill_file) = create_temp_skill_file(content);
    let result = parse_skill(&skill_file).unwrap();

    assert_eq!(result.name, "some-skill");
    assert_eq!(
        result.description,
        "This is the first paragraph. Still first paragraph."
    );
}

#[test]
fn test_parse_missing_both_name_and_description_falls_back() {
    let content = "---\n\n---\n\n# Heading\n\nThis is the first paragraph.\nStill first paragraph.\n\nSecond paragraph.\n";

    let (_temp_dir, skill_file) = create_temp_skill_file(content);
    let result = parse_skill(&skill_file).unwrap();

    assert_eq!(result.name, "test-skill");
    assert_eq!(
        result.description,
        "This is the first paragraph. Still first paragraph."
    );
}

#[test]
fn test_parse_no_front_matter_falls_back_to_derived_values() {
    let content = r#"# Just Content

This is just markdown content without any front matter.
"#;

    let (_temp_dir, skill_file) = create_temp_skill_file(content);
    let result = parse_skill(&skill_file).unwrap();

    assert_eq!(result.name, "test-skill");
    assert_eq!(
        result.description,
        "This is just markdown content without any front matter."
    );
    assert!(result.line_range.is_none());
}

#[test]
fn test_parse_no_skill_provider_defaults_to_agents() {
    let content = r#"---
name: some-skill
description: Some description
---

# Content
"#;

    // Create a temp file without a skill provider directory structure
    let temp_dir = TempDir::new().unwrap();
    let invalid_file = temp_dir.path().join("invalid.md");
    std::fs::write(&invalid_file, content).unwrap();

    let result = parse_skill(&invalid_file).unwrap();

    assert_eq!(result.name, "some-skill");
    assert_eq!(result.description, "Some description");
    assert_eq!(result.provider, SkillProvider::Agents);
}

#[test]
fn test_parse_truncates_long_fallback_description_at_sentence_boundary() {
    let first_sentence = format!("{}.", "a".repeat(450));
    let content = format!(
        r#"---
name: some-skill
---

{} {}
"#,
        first_sentence,
        "b".repeat(200)
    );

    let (_temp_dir, skill_file) = create_temp_skill_file(&content);
    let result = parse_skill(&skill_file).unwrap();

    assert_eq!(result.description, first_sentence);
}

#[test]
fn test_parse_truncates_long_fallback_description_at_word_boundary() {
    let content = format!(
        r#"---
name: some-skill
---

{}
"#,
        "word ".repeat(200)
    );

    let (_temp_dir, skill_file) = create_temp_skill_file(&content);
    let result = parse_skill(&skill_file).unwrap();

    assert!(result.description.chars().count() <= MAX_SKILL_DESCRIPTION_CHARS);
    assert!(!result.description.ends_with(' '));
}

#[test]
fn test_parse_truncates_fallback_description_with_hard_cut() {
    let content = format!(
        "---\nname: some-skill\n---\n\n{}",
        "x".repeat(MAX_SKILL_DESCRIPTION_CHARS + 100)
    );

    let (_temp_dir, skill_file) = create_temp_skill_file(&content);
    let result = parse_skill(&skill_file).unwrap();

    assert_eq!(
        result.description.chars().count(),
        MAX_SKILL_DESCRIPTION_CHARS
    );
}

#[test]
fn test_parse_does_not_truncate_user_provided_description() {
    let description = format!("{} {}", "a".repeat(450), "b".repeat(200));
    let content = format!(
        r#"---
name: some-skill
description: "{}"
---

# Content
"#,
        description
    );

    let (_temp_dir, skill_file) = create_temp_skill_file(&content);
    let result = parse_skill(&skill_file).unwrap();

    assert_eq!(result.description, description);
}

#[test]
fn test_truncation_does_not_cut_mid_word_like_filename() {
    // "abc.def" has no sentence boundary (no whitespace after punctuation),
    // so truncation should fall back to word boundary or hard cut.
    let long_word = "abc.def".repeat(100);
    let content = format!("---\nname: some-skill\n---\n\n{long_word}");

    let (_temp_dir, skill_file) = create_temp_skill_file(&content);
    let result = parse_skill(&skill_file).unwrap();

    assert!(result.description.chars().count() <= MAX_SKILL_DESCRIPTION_CHARS);
}

#[test]
fn test_truncation_cuts_at_sentence_boundary() {
    let first_sentence = "This is a sentence.";
    let second_sentence_start = " ".to_string() + &"b".repeat(600);
    let content = format!("---\nname: some-skill\n---\n\n{first_sentence}{second_sentence_start}");

    let (_temp_dir, skill_file) = create_temp_skill_file(&content);
    let result = parse_skill(&skill_file).unwrap();

    assert_eq!(result.description, "This is a sentence.");
}
