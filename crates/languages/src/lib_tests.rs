use std::path::Path;

use crate::{language_by_filename, load_language, SUPPORTED_LANGUAGES};

/// Validate that every supported language can be loaded successfully.
/// This catches invalid node types, syntax errors, and other issues in .scm query files
/// (highlights, indents, identifiers) that would otherwise only surface at runtime.
#[test]
fn all_supported_languages_load_successfully() {
    let failures: Vec<_> = SUPPORTED_LANGUAGES
        .iter()
        .filter(|lang| load_language(lang).is_none())
        .collect();

    assert!(
        failures.is_empty(),
        "The following languages failed to load:\n{}",
        failures
            .iter()
            .map(|lang| format!("  - {lang}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn cpp_header_extensions_resolve_to_cpp_language() {
    for filename in ["header.hpp", "header.hxx", "header.H"] {
        let language = language_by_filename(Path::new(filename))
            .unwrap_or_else(|| panic!("expected {filename} to resolve to C++"));

        assert_eq!(language.display_name(), "C++");
    }
}
