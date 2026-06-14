use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use crate::config::{CustomLspServerConfig, LspServerConfig, LspServerConfigKind};
use crate::descriptor::{parse::parse_entries, LspServerDescriptor};
use crate::log_redaction::NoopLogRedactor;
use crate::model::LspServerModel;
use crate::supported_servers::LSPServerType;

/// Returns a minimal `LspServerModel` configured for the rust-analyzer
/// built-in. The HTTP client uses the in-tree test constructor so no
/// network resources are touched.
fn built_in_model(server_type: LSPServerType) -> LspServerModel {
    let config = LspServerConfig::new(
        server_type,
        PathBuf::from("/tmp/workspace"),
        None,
        "warp-test".to_string(),
        Arc::new(http_client::Client::new()),
    );
    LspServerModel::new(LspServerConfigKind::BuiltIn(config))
}

/// Returns an `LspServerModel` wrapping a custom descriptor with the
/// given name and `filetypes` patterns. The descriptor is constructed
/// through `parse_entries` so its compiled matchers are real (same path
/// the settings layer uses), not the never-matches placeholder produced
/// by direct serde deserialization.
fn custom_model(name: &str, filetypes: &[&str]) -> LspServerModel {
    let pairs: Vec<(&str, Option<&str>)> = filetypes.iter().map(|p| (*p, None)).collect();
    custom_model_with_filetypes(name, &pairs)
}

/// Like [`custom_model`] but allows an explicit `language_id` override per
/// filetype pattern (`None` exercises the default-derivation path).
fn custom_model_with_filetypes(name: &str, filetypes: &[(&str, Option<&str>)]) -> LspServerModel {
    let filetype_entries: Vec<_> = filetypes
        .iter()
        .map(|(pattern, language_id)| match language_id {
            Some(id) => json!({ "pattern": pattern, "language_id": id }),
            None => json!({ "pattern": pattern }),
        })
        .collect();
    let entries = vec![json!({
        "name": name,
        "command": name,
        "filetypes": filetype_entries,
    })];
    let parsed = parse_entries(&entries);
    let descriptor: LspServerDescriptor = parsed
        .descriptors
        .into_iter()
        .next()
        .expect("test fixture parses");
    let config = CustomLspServerConfig::new(
        descriptor,
        PathBuf::from("/tmp/workspace"),
        "abc123def4567890".to_string(),
        PathBuf::from("/tmp/cache/test-lsp"),
        None,
        "warp-test".to_string(),
        Arc::new(NoopLogRedactor),
    );
    LspServerModel::new(LspServerConfigKind::Custom(Box::new(config)))
}

#[test]
fn supports_path_built_in_matches_when_language_id_resolves() {
    let model = built_in_model(LSPServerType::RustAnalyzer);
    assert!(model.supports_path(Path::new("/tmp/workspace/src/main.rs")));
}

#[test]
fn supports_path_built_in_rejects_unrelated_extension() {
    let model = built_in_model(LSPServerType::RustAnalyzer);
    assert!(!model.supports_path(Path::new("/tmp/workspace/script.py")));
}

#[test]
fn supports_path_custom_matches_via_filetype_glob() {
    // Custom servers dispatch through `LspFiletypePattern::is_match` against
    // the basename — not through the built-in `LanguageId` map. `.rb` has
    // no `LanguageId` mapping yet `supports_path` still returns true.
    let model = custom_model("ruby-lsp", &["*.rb"]);
    assert!(model.supports_path(Path::new("/tmp/repo/foo.rb")));
}

#[test]
fn supports_path_custom_rejects_when_no_pattern_matches() {
    let model = custom_model("ruby-lsp", &["*.rb"]);
    assert!(!model.supports_path(Path::new("/tmp/repo/foo.py")));
}

#[test]
fn supports_path_custom_matches_literal_basename() {
    // A `filetypes` entry without glob metacharacters matches as a
    // literal basename.
    let model = custom_model("solargraph", &["Gemfile"]);
    assert!(model.supports_path(Path::new("/tmp/repo/Gemfile")));
    assert!(!model.supports_path(Path::new("/tmp/repo/Gemfile.lock")));
}

#[test]
fn language_id_for_path_resolution() {
    // (model, path, expected languageId, case description)
    let cases: Vec<(LspServerModel, &str, Option<&str>, &str)> = vec![
        (
            built_in_model(LSPServerType::RustAnalyzer),
            "/tmp/workspace/src/main.rs",
            Some("rust"),
            "built-in via LanguageId map",
        ),
        (
            built_in_model(LSPServerType::RustAnalyzer),
            "/tmp/workspace/foo.rb",
            None,
            "built-in: extension it doesn't claim",
        ),
        (
            custom_model("ruby-lsp", &["*.rb"]),
            "/tmp/repo/Foo.RB",
            Some("rb"),
            "custom: no built-in mapping, defaults to lowercase extension",
        ),
        (
            custom_model("dockerfile-lsp", &["Dockerfile"]),
            "/tmp/repo/Dockerfile",
            Some("Dockerfile"),
            "custom: basename when no extension",
        ),
        (
            custom_model("ruby-lsp", &["*.rb"]),
            "/tmp/repo/foo.py",
            None,
            "custom: no filetype matches",
        ),
        (
            custom_model_with_filetypes("ruby-lsp", &[("*.rb", Some("ruby"))]),
            "/tmp/repo/foo.rb",
            Some("ruby"),
            "custom: explicit language_id wins",
        ),
    ];
    for (model, path, expected, desc) in cases {
        assert_eq!(
            model.language_id_for_path(Path::new(path)).as_deref(),
            expected,
            "{desc} ({path})"
        );
    }
}

#[test]
fn language_id_for_path_custom_resolves_per_file_in_multi_filetype_descriptor() {
    let model = custom_model_with_filetypes(
        "ts-ls",
        &[
            ("*.ts", Some("typescript")),
            ("*.tsx", Some("typescriptreact")),
        ],
    );
    assert_eq!(
        model.language_id_for_path(Path::new("/tmp/repo/a.ts")),
        Some("typescript".to_string())
    );
    assert_eq!(
        model.language_id_for_path(Path::new("/tmp/repo/b.tsx")),
        Some("typescriptreact".to_string())
    );
}
