use std::path::Path;

use warp_util::standardized_path::StandardizedPath;

use crate::{
    SUPPORTED_LANGUAGES, language_by_filename, language_by_local_filename, language_by_name,
    load_language,
};

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
        let path = StandardizedPath::try_new(&format!("/tmp/{filename}"))
            .expect("test path should be absolute");
        let language = language_by_filename(&path)
            .unwrap_or_else(|| panic!("expected {filename} to resolve to a language"));
        assert_eq!(
            language.display_name(),
            "HTML",
            "{filename} should resolve to HTML",
        );
    }
}

#[test]
fn local_html_extensions_resolve_to_html() {
    for filename in ["index.html", "index.htm"] {
        let path = Path::new(filename);
        let language = language_by_local_filename(path)
            .unwrap_or_else(|| panic!("expected {filename} to resolve to a language"));
        assert_eq!(
            language.display_name(),
            "HTML",
            "{filename} should resolve to HTML",
        );
    }
}

/// `.command` is the macOS convention for double-clickable shell scripts.
/// Make sure `language_by_filename` recognizes it as shell so the editor
/// renders syntax highlighting instead of the
/// "Language support is unavailable for this file type" footer.
#[test]
fn command_extension_resolves_to_shell() {
    let path =
        StandardizedPath::try_new("/tmp/script.command").expect("test path should be absolute");
    let language =
        language_by_filename(&path).expect("`.command` files should resolve to a language");
    assert_eq!(language.display_name(), "Shell");
}

#[test]
fn local_command_extension_resolves_to_shell() {
    let language = language_by_local_filename(Path::new("script.command"))
        .expect("`.command` files should resolve to a language");
    assert_eq!(language.display_name(), "Shell");
}

/// `.md` and `.markdown` should resolve to the Markdown language so the editor applies
/// syntax highlighting to Markdown source files.
#[test]
fn markdown_extensions_resolve_to_markdown() {
    for filename in ["README.md", "notes.markdown"] {
        let path = StandardizedPath::try_new(&format!("/tmp/{filename}"))
            .expect("test path should be absolute");
        let language = language_by_filename(&path)
            .unwrap_or_else(|| panic!("expected {filename} to resolve to a language"));
        assert_eq!(
            language.display_name(),
            "Markdown",
            "{filename} should resolve to Markdown",
        );
    }
}

/// Pascal is the one language whose grammar does not come from arborium — it is vendored
/// in `tree_sitter_pascal` and its highlight query is loaded from
/// `grammars/pascal/highlights.scm` instead. Cover the whole spread of unit, program and
/// package extensions, since each one reaches the editor through this table.
#[test]
fn pascal_extensions_resolve_to_pascal() {
    for filename in [
        "unit1.pas",
        "unit1.pp",
        "project1.dpr",
        "package1.dpk",
        "project1.lpr",
        "package1.lpk",
    ] {
        let path = StandardizedPath::try_new(&format!("/tmp/{filename}"))
            .expect("test path should be absolute");
        let language = language_by_filename(&path)
            .unwrap_or_else(|| panic!("expected {filename} to resolve to a language"));
        assert_eq!(
            language.display_name(),
            "Pascal",
            "{filename} should resolve to Pascal",
        );
    }
}

/// Markdown fences and agent output spell Pascal several different ways.
#[test]
fn pascal_aliases_resolve_to_pascal() {
    for alias in [
        "pascal",
        "pas",
        "delphi",
        "objectpascal",
        "object-pascal",
        "objpas",
        "freepascal",
        "fpc",
    ] {
        let language = language_by_name(alias)
            .unwrap_or_else(|| panic!("expected {alias} to resolve to a language"));
        assert_eq!(
            language.display_name(),
            "Pascal",
            "{alias} should resolve to Pascal",
        );
    }
}

/// Pascal's highlight query is Warp's own rather than arborium's, so assert the captures the
/// editor actually colors are present. `convert_capture_name_to_color` keys off the segment
/// before the first `.`, and a query that silently lost these would still load.
#[test]
fn pascal_highlight_query_covers_the_colored_captures() {
    let language = language_by_name("pascal").expect("Pascal should load");
    let captures = language.highlight_query.capture_names();

    for expected in ["keyword", "string", "number", "comment", "type", "function"] {
        assert!(
            captures.contains(&expected),
            "Pascal highlights.scm should capture @{expected}, got {captures:?}",
        );
    }
}
