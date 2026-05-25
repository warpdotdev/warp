use super::*;

#[test]
fn check_supported_glob_features_rejects_double_star() {
    let kind = check_supported_glob_features("**/*.rs").unwrap();
    assert!(matches!(
        kind,
        LspDescriptorErrorKind::UnsupportedGlobFeature { feature: "**", .. }
    ));
}

#[test]
fn check_supported_glob_features_rejects_brace_alternation() {
    let kind = check_supported_glob_features("{foo,bar}.rs").unwrap();
    assert!(matches!(
        kind,
        LspDescriptorErrorKind::UnsupportedGlobFeature {
            feature: "{a,b}",
            ..
        }
    ));
}

#[test]
fn display_includes_entry_name() {
    let err = LspDescriptorError {
        entry_name: Some("ruby-lsp".to_string()),
        kind: LspDescriptorErrorKind::EmptyFiletypes,
    };
    let s = format!("{err}");
    assert!(s.contains("ruby-lsp"));
    assert!(s.contains("filetypes"));
}

#[test]
fn display_handles_anonymous_entry() {
    let err = LspDescriptorError {
        entry_name: None,
        kind: LspDescriptorErrorKind::MissingName,
    };
    let s = format!("{err}");
    assert!(s.contains("entry without"));
    assert!(s.contains("name"));
}
