use super::*;
use crate::descriptor::{LspFiletypePattern, LspServerDescriptor};
use std::collections::BTreeMap;
use std::path::Path;

/// Builds an `LspFiletypePattern` the same way `parse::compile_pattern` does:
/// patterns with glob metacharacters compile case-insensitively, literal
/// basenames compile case-sensitively. Test patterns are all "clean" (no
/// special characters in literal basenames), so we don't bother escaping
/// here the way the real `compile_pattern` does.
fn pattern(raw: &str, language_id: Option<&str>) -> LspFiletypePattern {
    let is_glob = raw.contains(['*', '?', '[']);
    let matcher = globset::GlobBuilder::new(raw)
        .case_insensitive(is_glob)
        .literal_separator(true)
        .build()
        .expect("test pattern compiles")
        .compile_matcher();
    LspFiletypePattern::from_parts(raw.to_string(), language_id.map(str::to_string), matcher)
}

fn descriptor(name: &str, filetypes: Vec<LspFiletypePattern>) -> LspServerDescriptor {
    LspServerDescriptor {
        name: name.to_string(),
        command: format!("/usr/bin/{name}"),
        args: vec![],
        filetypes,
        env: BTreeMap::new(),
        initialization_options: None,
    }
}

#[test]
fn glob_matches_extension_case_insensitively() {
    let descs = vec![descriptor("ruby-lsp", vec![pattern("*.rb", None)])];
    let matched = match_descriptor(&descs, Path::new("/work/foo.rb")).unwrap();
    assert_eq!(matched.descriptor.name, "ruby-lsp");
    // Case-insensitive: uppercase and mixed-case also match.
    assert!(match_descriptor(&descs, Path::new("/work/FOO.RB")).is_some());
    assert!(match_descriptor(&descs, Path::new("/work/foo.RB")).is_some());
}

#[test]
fn literal_basename_is_case_sensitive() {
    let descs = vec![descriptor("ruby-lsp", vec![pattern("Gemfile", None)])];
    assert!(match_descriptor(&descs, Path::new("/work/Gemfile")).is_some());
    assert!(match_descriptor(&descs, Path::new("/work/gemfile")).is_none());
}

#[test]
fn literal_basename_is_not_extension_match() {
    // A bare token like `"rb"` is a literal basename match, not an extension
    // match. To match `.rb` files, the user has to write `"*.rb"`.
    let descs = vec![descriptor("ruby-lsp", vec![pattern("rb", None)])];
    assert!(match_descriptor(&descs, Path::new("/work/foo.rb")).is_none());
    assert!(match_descriptor(&descs, Path::new("/work/rb")).is_some());
}

#[test]
fn first_in_source_order_wins() {
    // Two descriptors both claim `.rs`. The first-declared one wins.
    let descs = vec![
        descriptor("my-rust", vec![pattern("*.rs", None)]),
        descriptor("rust-analyzer", vec![pattern("*.rs", None)]),
    ];
    let matched = match_descriptor(&descs, Path::new("/work/main.rs")).unwrap();
    assert_eq!(matched.descriptor.name, "my-rust");
}

#[test]
fn language_id_explicit_wins() {
    let descs = vec![descriptor(
        "ts-lsp",
        vec![pattern("*.tsx", Some("typescriptreact"))],
    )];
    let matched = match_descriptor(&descs, Path::new("/work/Component.tsx")).unwrap();
    assert_eq!(matched.language_id, "typescriptreact");
}

#[test]
fn language_id_defaults_to_extension_lowercase() {
    // No explicit `language_id`; matcher should derive `rb` from `foo.rb`.
    let descs = vec![descriptor("ruby-lsp", vec![pattern("*.rb", None)])];
    let matched = match_descriptor(&descs, Path::new("/work/foo.RB")).unwrap();
    assert_eq!(matched.language_id, "rb");
}

#[test]
fn language_id_defaults_to_basename_when_no_extension() {
    let descs = vec![descriptor("docker-lsp", vec![pattern("Dockerfile", None)])];
    let matched = match_descriptor(&descs, Path::new("/work/Dockerfile")).unwrap();
    assert_eq!(matched.language_id, "Dockerfile");
}

#[test]
fn no_match_returns_none() {
    let descs = vec![descriptor("ruby-lsp", vec![pattern("*.rb", None)])];
    assert!(match_descriptor(&descs, Path::new("/work/main.go")).is_none());
}
