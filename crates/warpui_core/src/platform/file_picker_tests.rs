use super::*;

#[test]
fn yaml_file_type_accepts_both_yaml_and_yml() {
    assert_eq!(FileType::Yaml.extensions(), &["yaml", "yml"]);
}

#[test]
fn markdown_file_type_accepts_md_and_markdown() {
    assert_eq!(FileType::Markdown.extensions(), &["md", "markdown"]);
}
