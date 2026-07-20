use std::path::Path;
use std::sync::Arc;

use warp_util::standardized_path::StandardizedPath;

use crate::{
    SUPPORTED_LANGUAGES, language_by_filename, language_by_local_filename, language_by_name,
    load_language,
};

const FILENAME_GOLDEN_CASES: &[(&str, &str)] = &[
    (".bashrc", "shell"),
    (".bash_profile", "shell"),
    (".zshrc", "shell"),
    (".zsh_profile", "shell"),
    (".zprofile", "shell"),
    ("BUILD", "starlark"),
    ("WORKSPACE", "starlark"),
    ("Dockerfile", "dockerfile"),
    ("Containerfile", "dockerfile"),
    ("dockerfile", "dockerfile"),
    ("containerfile", "dockerfile"),
    ("Dockerfile.dev", "dockerfile"),
    ("Containerfile.release", "dockerfile"),
    ("Dockerfile.rs", "dockerfile"),
    ("Containerfile.ts", "dockerfile"),
];

const EXTENSION_GOLDEN_CASES: &[(&[&str], &str)] = &[
    (&["rs"], "rust"),
    (&["go"], "golang"),
    (&["yml", "yaml"], "yaml"),
    (&["py", "py3", "pyw", "pyi"], "python"),
    (&["js", "cjs", "mjs"], "javascript"),
    (&["jsx"], "jsx"),
    (&["tsx"], "tsx"),
    (&["ts", "cts", "mts"], "typescript"),
    (&["java", "groovy", "gvy", "gy", "gsh"], "java"),
    (
        &["cpp", "cxx", "cc", "h", "hh", "hpp", "hxx", "H", "h++"],
        "cpp",
    ),
    (&["sh", "zsh", "bash", "command"], "shell"),
    (&["cs"], "csharp"),
    (&["html", "htm"], "html"),
    (&["css"], "css"),
    (&["c"], "c"),
    (&["json"], "json"),
    (&["jq"], "jq"),
    (&["tf", "hcl", "tfvars"], "hcl"),
    (&["lua"], "lua"),
    (&["nix"], "nix"),
    (&["rb"], "ruby"),
    (&["php", "phtml"], "php"),
    (&["toml"], "toml"),
    (&["swift"], "swift"),
    (&["kt", "kts"], "kotlin"),
    (&["scala", "sbt", "sc"], "scala"),
    (&["ps1", "pwsh"], "powershell"),
    (&["ex", "exs"], "elixir"),
    (&["sql"], "sql"),
    (&["bzl", "bazel"], "starlark"),
    (&["m", "mm"], "objective-c"),
    (&["xml"], "xml"),
    (&["vue"], "vue"),
    (&["dockerfile"], "dockerfile"),
    (&["md", "markdown"], "markdown"),
];

#[test]
fn filename_and_extension_registry_preserves_all_existing_mappings() {
    for &(filename, expected_grammar) in FILENAME_GOLDEN_CASES {
        assert_filename_uses_grammar(filename, expected_grammar);
    }

    for &(extensions, expected_grammar) in EXTENSION_GOLDEN_CASES {
        for extension in extensions {
            assert_filename_uses_grammar(&format!("fixture.{extension}"), expected_grammar);
        }
    }
}

#[test]
fn lsp_only_extension_does_not_expand_editor_support() {
    assert!(language_by_local_filename(Path::new("fixture.C")).is_none());
}

fn assert_filename_uses_grammar(filename: &str, expected_grammar: &str) {
    let actual = language_by_local_filename(Path::new(filename))
        .unwrap_or_else(|| panic!("expected {filename} to resolve to {expected_grammar}"));
    let expected = language_by_name(expected_grammar)
        .unwrap_or_else(|| panic!("expected {expected_grammar} grammar to load"));

    assert!(
        Arc::ptr_eq(&actual, &expected),
        "{filename} resolved to {} instead of {}",
        actual.display_name(),
        expected.display_name()
    );
}

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
