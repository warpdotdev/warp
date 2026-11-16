use super::*;

#[test]
fn test_parse_without_front_matter() {
    let content = r#"# Hello World

This is just markdown content without front matter.
"#;

    let result = parse_markdown_content(content).unwrap();

    assert!(result.front_matter.is_empty());
    assert_eq!(result.content, content);
    // When there's no front matter, line_range should be None
    assert_eq!(result.line_range, None);
}

#[test]
fn test_parse_empty_front_matter() {
    let content = r#"---
---

# Content
"#;

    let result = parse_markdown_content(content).unwrap();

    // Empty front matter (---\n---) doesn't match the regex, so it's treated as no front matter
    assert!(result.front_matter.is_empty());
    assert_eq!(result.content, content);
}

#[test]
fn test_parse_with_crlf_line_endings() {
    let content =
        "---\r\nname: test-skill\r\ndescription: Test description\r\n---\r\n\r\n# Content\r\n";

    let result = parse_markdown_content(content).unwrap();

    assert_eq!(result.front_matter.len(), 2);
    assert_eq!(result.front_matter.get("name").unwrap(), "test-skill");
    assert_eq!(
        result.front_matter.get("description").unwrap(),
        "Test description"
    );
    // Content should include the entire file (including front matter)
    assert_eq!(result.content, content);
    assert!(result.content.contains("# Content"));
    assert!(result.content.contains("name: test-skill"));
    // Line range should represent the markdown content after front matter (1-indexed)
    // Front matter is 4 lines (1-4), markdown content starts at line 5
    assert_eq!(result.line_range, Some(5..7));
}

#[test]
fn test_parse_with_trailing_spaces_after_delimiters() {
    let content =
        "---   \nname: test-skill\ndescription: Test description\n---  \t \n\n# Content\n";

    let result = parse_markdown_content(content).unwrap();

    assert_eq!(result.front_matter.len(), 2);
    assert_eq!(result.front_matter.get("name").unwrap(), "test-skill");
    assert_eq!(
        result.front_matter.get("description").unwrap(),
        "Test description"
    );
}

#[test]
fn test_parse_with_extra_blank_lines_in_front_matter() {
    let content = r#"---

name: test-skill
description: Test description

---

# Content
"#;

    let result = parse_markdown_content(content).unwrap();

    assert_eq!(result.front_matter.len(), 2);
    assert_eq!(result.front_matter.get("name").unwrap(), "test-skill");
    assert_eq!(
        result.front_matter.get("description").unwrap(),
        "Test description"
    );
    assert!(result.content.contains("# Content"));
}

#[test]
fn test_parse_with_mixed_crlf_and_extra_whitespace() {
    let content = "---  \r\n\r\nname: test-skill\r\ndescription: Test description\r\n\r\n---\t\r\n\r\n# Content\r\n";

    let result = parse_markdown_content(content).unwrap();

    assert_eq!(result.front_matter.len(), 2);
    assert_eq!(result.front_matter.get("name").unwrap(), "test-skill");
    assert_eq!(
        result.front_matter.get("description").unwrap(),
        "Test description"
    );
}

#[test]
fn test_parse_with_tabs_and_spaces() {
    let content =
        "---\t  \t\nname: test-skill\ndescription: Test description\n--- \t\n\n# Content\n";

    let result = parse_markdown_content(content).unwrap();

    assert_eq!(result.front_matter.len(), 2);
    assert_eq!(result.front_matter.get("name").unwrap(), "test-skill");
}

#[test]
fn test_crlf_without_proper_front_matter() {
    let content = "# Hello World\r\n\r\nThis is just markdown content.\r\n";

    let result = parse_markdown_content(content).unwrap();

    assert!(result.front_matter.is_empty());
    assert_eq!(result.content, content);
}

#[test]
fn test_parse_with_leading_whitespace_before_front_matter() {
    let content = r#"

---
name: test-skill
description: Test description
---

# Content
"#;

    let result = parse_markdown_content(content).unwrap();

    assert_eq!(result.front_matter.len(), 2);
    assert_eq!(result.front_matter.get("name").unwrap(), "test-skill");
    assert_eq!(
        result.front_matter.get("description").unwrap(),
        "Test description"
    );

    // Line numbers (1-indexed):
    // Line 1: (empty from raw string start)
    // Line 2: (empty)
    // Line 3: ---
    // Line 4: name: test-skill
    // Line 5: description: Test description
    // Line 6: ---
    // Line 7: (empty)
    // Line 8: # Content
    assert_eq!(result.line_range, Some(7..9));
}

#[test]
fn test_parse_with_spaces_and_newlines_before_front_matter() {
    let content =
        "  \n\t\n---\nname: test-skill\ndescription: Test description\n---\n\n# Content\n";

    let result = parse_markdown_content(content).unwrap();

    assert_eq!(result.front_matter.len(), 2);
    assert_eq!(result.front_matter.get("name").unwrap(), "test-skill");
}

#[test]
fn test_content_includes_front_matter_and_line_range() {
    let content = r#"---
name: my-skill
description: My skill description
---

# My Skill

This is the skill content.
"#;

    let result = parse_markdown_content(content).unwrap();

    // Verify front matter is parsed correctly
    assert_eq!(result.front_matter.len(), 2);
    assert_eq!(result.front_matter.get("name").unwrap(), "my-skill");
    assert_eq!(
        result.front_matter.get("description").unwrap(),
        "My skill description"
    );

    // Verify content includes the entire file (front matter + markdown)
    assert_eq!(result.content, content);
    assert!(result.content.contains("---"));
    assert!(result.content.contains("name: my-skill"));
    assert!(result.content.contains("# My Skill"));

    // Verify line_range points to just the markdown content (after front matter, 1-indexed)
    // Front matter is lines 1-4, markdown content starts at line 5
    assert_eq!(result.line_range, Some(5..9));
}
