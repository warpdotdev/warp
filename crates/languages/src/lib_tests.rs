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

/// Both `.html` and the legacy three-character `.htm` extension should resolve to
/// the same HTML language entry. `.htm` is widely produced by static-site generators
/// and historical web tooling (DOS 8.3 filename limits) and is already treated as
/// an HTML/text file elsewhere in the codebase
/// (see `is_development_text_extension` in `crates/warp_util/src/file_type.rs`).
#[test]
fn html_extensions_resolve_to_html() {
    for filename in ["index.html", "index.htm"] {
        let language = language_by_filename(Path::new(filename))
            .unwrap_or_else(|| panic!("expected {filename} to resolve to a language"));
        assert_eq!(
            language.display_name(),
            "HTML",
            "{filename} should resolve to HTML",
        );
    }
}

#[test]
fn cpp_header_extensions_resolve_to_cpp_language() {
    // Cover the common modern C++ header extensions (`.hpp`, `.hxx`),
    // the older uppercase `.H` convention, and the rarer `.h++` form.
    for filename in ["header.hpp", "header.hxx", "header.H", "header.h++"] {
        let language = language_by_filename(Path::new(filename))
            .unwrap_or_else(|| panic!("expected {filename} to resolve to C++"));

        assert_eq!(language.display_name(), "C++");
    }
}
