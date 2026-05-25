use std::path::{Path, PathBuf};

use lsp::supported_servers::LSPServerType;
use serde_json::json;
use settings::Setting as _;
use settings_value::SettingsValue;
use warpui::{App, SingletonEntity};

use crate::code::lsp_dispatch::{resolve_server_for_path, ResolvedLspServer};
use crate::settings::{LanguageServersSettings, LspServerDescriptors};
use crate::test_util::settings::initialize_settings_for_tests;

/// Replaces the user's `[[editor.language_servers]]` value with descriptors
/// parsed from the given JSON array (each item is a single descriptor's
/// `Value`). Uses `LspServerDescriptors::from_file_value` so the compiled
/// glob matchers on each `filetypes` entry are real, matching what the
/// settings load path produces in production.
fn set_language_servers(app: &mut App, entries: serde_json::Value) {
    let descriptors = LspServerDescriptors::from_file_value(&entries)
        .expect("test entries parse into descriptors");
    let handle = LanguageServersSettings::handle(app);
    handle.update(app, |settings, ctx| {
        settings
            .language_servers
            .set_value(descriptors, ctx)
            .expect("set_value succeeds");
    });
}

#[test]
fn resolve_returns_built_in_when_no_custom_matches() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.read(|ctx| {
            let resolved = resolve_server_for_path(Path::new("/tmp/foo.rs"), ctx);
            assert!(matches!(
                resolved,
                Some(ResolvedLspServer::BuiltIn(LSPServerType::RustAnalyzer))
            ));
        });
    });
}

#[test]
fn resolve_returns_custom_when_descriptor_matches() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        set_language_servers(
            &mut app,
            json!([
                { "name": "ruby-lsp", "command": "ruby-lsp", "filetypes": [{ "pattern": "*.rb" }] }
            ]),
        );
        app.read(|ctx| {
            let resolved = resolve_server_for_path(Path::new("/tmp/foo.rb"), ctx);
            match resolved {
                Some(ResolvedLspServer::Custom(descriptor)) => {
                    assert_eq!(descriptor.name, "ruby-lsp");
                }
                other => panic!("expected Custom(ruby-lsp), got {other:?}"),
            }
        });
    });
}

#[test]
fn resolve_custom_overrides_built_in() {
    // A custom entry whose `filetypes` matches a path takes precedence over
    // the built-in `LanguageId` mapping for that path.
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        set_language_servers(
            &mut app,
            json!([
                { "name": "my-rust-lsp", "command": "my-rust-lsp", "filetypes": [{ "pattern": "*.rs" }] }
            ]),
        );
        app.read(|ctx| {
            let resolved = resolve_server_for_path(Path::new("/tmp/foo.rs"), ctx);
            match resolved {
                Some(ResolvedLspServer::Custom(descriptor)) => {
                    assert_eq!(descriptor.name, "my-rust-lsp");
                }
                other => panic!("custom should override built-in rust-analyzer, got {other:?}"),
            }
        });
    });
}

#[test]
fn resolve_returns_none_when_nothing_matches() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.read(|ctx| {
            let resolved = resolve_server_for_path(Path::new("/tmp/foo.unknown_extension"), ctx);
            assert!(resolved.is_none());
        });
    });
}

#[test]
fn display_name_built_in_returns_binary_name() {
    let resolved = ResolvedLspServer::BuiltIn(LSPServerType::RustAnalyzer);
    assert_eq!(resolved.display_name(), "rust-analyzer");
}

#[test]
fn display_name_custom_returns_descriptor_name() {
    let descriptors = LspServerDescriptors::from_file_value(&json!([
        { "name": "ruby-lsp", "command": "ruby-lsp", "filetypes": [{ "pattern": "*.rb" }] }
    ]))
    .expect("parses");
    let descriptor = descriptors.0.into_iter().next().unwrap();
    let resolved = ResolvedLspServer::Custom(Box::new(descriptor));
    assert_eq!(resolved.display_name(), "ruby-lsp");
}

#[test]
fn log_file_path_built_in_routes_to_built_in_helper() {
    // Built-in log paths are keyed by `binary_name` per `lsp_logs::log_file_path`.
    let resolved = ResolvedLspServer::BuiltIn(LSPServerType::RustAnalyzer);
    let log_path = resolved.log_file_path(&PathBuf::from("/tmp/workspace"));
    assert!(
        log_path.to_string_lossy().contains("rust-analyzer"),
        "expected built-in log path to contain binary name, got {}",
        log_path.display()
    );
}

#[test]
fn log_file_path_custom_routes_to_custom_helper() {
    let descriptors = LspServerDescriptors::from_file_value(&json!([
        { "name": "ruby-lsp", "command": "ruby-lsp", "filetypes": [{ "pattern": "*.rb" }] }
    ]))
    .expect("parses");
    let descriptor = descriptors.0.into_iter().next().unwrap();
    let resolved = ResolvedLspServer::Custom(Box::new(descriptor));
    let log_path = resolved.log_file_path(&PathBuf::from("/tmp/workspace"));
    assert!(
        log_path.to_string_lossy().contains("ruby-lsp"),
        "expected custom log path to contain descriptor name, got {}",
        log_path.display()
    );
}
