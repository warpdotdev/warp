use std::path::Path;

use arborium::tree_sitter::Parser;
use warp_util::standardized_path::StandardizedPath;

use crate::{SUPPORTED_LANGUAGES, language_by_filename, language_by_local_filename, load_language};

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

#[test]
fn dart_extension_resolves_to_dart() {
    let standardized_path =
        StandardizedPath::try_new("/tmp/main.dart").expect("test path should be absolute");
    let language = language_by_filename(&standardized_path)
        .expect("`.dart` files should resolve to a language");
    assert_eq!(language.display_name(), "Dart");

    let language = language_by_local_filename(Path::new("main.dart"))
        .expect("local `.dart` files should resolve to a language");
    assert_eq!(language.display_name(), "Dart");
}

#[test]
fn dart_grammar_parses_modern_flutter_code() {
    let language = language_by_local_filename(Path::new("main.dart"))
        .expect("`.dart` files should resolve to a language");
    let source = r#"
import 'package:flutter/material.dart';

sealed class LoadState {}
class Loaded<T> extends LoadState {
  Loaded(this.value);
  final T value;
}

class Probe extends StatelessWidget {
  const Probe({super.key});

  @override
  Widget build(BuildContext context) => switch (<String, List<int>>{}) {
    final values when values.isEmpty => const SizedBox.shrink(),
    _ => const Text('Loaded'),
  };
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&language.grammar)
        .expect("Dart grammar should be compatible with tree-sitter");
    let tree = parser
        .parse(source, None)
        .expect("Dart parser should produce a syntax tree");

    assert!(
        !tree.root_node().has_error(),
        "modern Dart and Flutter syntax should parse without errors",
    );
}
