use std::path::PathBuf;

use lsp_types::Uri;

use crate::config::{lsp_uri_to_path, path_to_lsp_uri};

// Unix-specific tests use Unix paths
#[cfg(not(windows))]
mod unix_tests {
    use super::*;

    #[test]
    fn test_lsp_uri_to_path_basic() {
        let uri: Uri = "file:///Users/test/project/src/main.rs".parse().unwrap();
        let path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(path, PathBuf::from("/Users/test/project/src/main.rs"));
    }

    #[test]
    fn test_lsp_uri_to_path_decodes_at_symbol() {
        // %40 is the URL encoding for @
        let uri: Uri = "file:///Users/test/node_modules/%40firebase/auth/dist/index.d.ts"
            .parse()
            .unwrap();
        let path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/Users/test/node_modules/@firebase/auth/dist/index.d.ts")
        );
    }

    #[test]
    fn test_lsp_uri_to_path_decodes_spaces() {
        // %20 is the URL encoding for space
        let uri: Uri = "file:///Users/test/My%20Project/src/main.rs"
            .parse()
            .unwrap();
        let path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(path, PathBuf::from("/Users/test/My Project/src/main.rs"));
    }

    #[test]
    fn test_lsp_uri_to_path_decodes_multiple_special_chars() {
        // Test multiple encoded characters: @ (%40), space (%20), # (%23)
        let uri: Uri = "file:///Users/test/%40scope/my%20package%23v1/index.ts"
            .parse()
            .unwrap();
        let path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/Users/test/@scope/my package#v1/index.ts")
        );
    }

    #[test]
    fn test_path_to_lsp_uri_basic() {
        let path = PathBuf::from("/Users/test/project/src/main.rs");
        let uri = path_to_lsp_uri(&path).unwrap();
        assert_eq!(uri.as_str(), "file:///Users/test/project/src/main.rs");
    }

    #[test]
    fn test_path_to_lsp_uri_encodes_spaces() {
        let path = PathBuf::from("/Users/test/My Project/src/main.rs");
        let uri = path_to_lsp_uri(&path).unwrap();
        assert_eq!(uri.as_str(), "file:///Users/test/My%20Project/src/main.rs");
    }

    #[test]
    fn test_path_to_lsp_uri_encodes_non_ascii() {
        let path = PathBuf::from("/Users/관리자/project/src/main.rs");
        let uri = path_to_lsp_uri(&path).unwrap();
        assert!(uri.as_str().starts_with("file:///Users/%"));
    }

    #[test]
    fn test_path_to_lsp_uri_encodes_accented_chars() {
        let path = PathBuf::from("/Users/José/project/src/main.rs");
        let uri = path_to_lsp_uri(&path).unwrap();
        assert!(uri.as_str().starts_with("file:///Users/Jos%"));
    }

    #[test]
    fn test_path_to_lsp_uri_encodes_hash() {
        let path = PathBuf::from("/Users/test/my#project/src/main.rs");
        let uri = path_to_lsp_uri(&path).unwrap();
        assert_eq!(uri.as_str(), "file:///Users/test/my%23project/src/main.rs");
    }

    #[test]
    fn test_roundtrip_path_to_uri_to_path() {
        let original_path = PathBuf::from("/Users/test/project/src/main.rs");
        let uri = path_to_lsp_uri(&original_path).unwrap();
        let roundtrip_path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(original_path, roundtrip_path);
    }

    #[test]
    fn test_roundtrip_non_ascii_path() {
        let original_path = PathBuf::from("/Users/관리자/project/src/main.rs");
        let uri = path_to_lsp_uri(&original_path).unwrap();
        let roundtrip_path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(original_path, roundtrip_path);
    }

    #[test]
    fn test_roundtrip_path_with_spaces() {
        let original_path = PathBuf::from("/Users/test/My Project/src/main.rs");
        let uri = path_to_lsp_uri(&original_path).unwrap();
        let roundtrip_path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(original_path, roundtrip_path);
    }

    #[test]
    fn test_path_to_lsp_uri_encodes_brackets() {
        let path = PathBuf::from("/Users/test/routes/blog/[slug].tsx");
        let uri = path_to_lsp_uri(&path).unwrap();
        assert_eq!(
            uri.as_str(),
            "file:///Users/test/routes/blog/%5Bslug%5D.tsx"
        );
    }

    #[test]
    fn test_roundtrip_path_with_brackets() {
        let original_path = PathBuf::from("/Users/test/routes/[id]/[slug].tsx");
        let uri = path_to_lsp_uri(&original_path).unwrap();
        let roundtrip_path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(original_path, roundtrip_path);
    }
}

// Windows-specific tests use Windows paths
#[cfg(windows)]
mod windows_tests {
    use super::*;

    #[test]
    fn test_lsp_uri_to_path_basic() {
        let uri: Uri = "file:///C:/Users/test/project/src/main.rs".parse().unwrap();
        let path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(
            path,
            PathBuf::from("C:\\Users\\test\\project\\src\\main.rs")
        );
    }

    #[test]
    fn test_lsp_uri_to_path_decodes_at_symbol() {
        // %40 is the URL encoding for @
        let uri: Uri = "file:///C:/Users/test/node_modules/%40firebase/auth/dist/index.d.ts"
            .parse()
            .unwrap();
        let path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(
            path,
            PathBuf::from("C:\\Users\\test\\node_modules\\@firebase\\auth\\dist\\index.d.ts")
        );
    }

    #[test]
    fn test_lsp_uri_to_path_decodes_spaces() {
        // %20 is the URL encoding for space
        let uri: Uri = "file:///C:/Users/test/My%20Project/src/main.rs"
            .parse()
            .unwrap();
        let path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(
            path,
            PathBuf::from("C:\\Users\\test\\My Project\\src\\main.rs")
        );
    }

    #[test]
    fn test_lsp_uri_to_path_decodes_multiple_special_chars() {
        // Test multiple encoded characters: @ (%40), space (%20), # (%23)
        let uri: Uri = "file:///C:/Users/test/%40scope/my%20package%23v1/index.ts"
            .parse()
            .unwrap();
        let path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(
            path,
            PathBuf::from("C:\\Users\\test\\@scope\\my package#v1\\index.ts")
        );
    }

    #[test]
    fn test_path_to_lsp_uri_basic() {
        let path = PathBuf::from("C:\\Users\\test\\project\\src\\main.rs");
        let uri = path_to_lsp_uri(&path).unwrap();
        assert_eq!(uri.as_str(), "file:///C:/Users/test/project/src/main.rs");
    }

    #[test]
    fn test_roundtrip_path_to_uri_to_path() {
        let original_path = PathBuf::from("C:\\Users\\test\\project\\src\\main.rs");
        let uri = path_to_lsp_uri(&original_path).unwrap();
        let roundtrip_path = lsp_uri_to_path(&uri).unwrap();
        assert_eq!(original_path, roundtrip_path);
    }
}

// Platform-independent tests
#[test]
fn test_lsp_uri_to_path_rejects_non_file_uri() {
    let uri: Uri = "https://example.com/path".parse().unwrap();
    let result = lsp_uri_to_path(&uri);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Invalid file URI"));
}

#[test]
fn test_path_to_lsp_uri_rejects_relative_path() {
    let path = PathBuf::from("relative/path/file.rs");
    let result = path_to_lsp_uri(&path);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("must be absolute"));
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(not(windows))]
mod custom_config_tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use serde_json::json;
    use warpui::r#async::block_on;

    use crate::config::CustomLspServerConfig;
    use crate::descriptor::LspServerDescriptor;
    use crate::log_redaction::NoopLogRedactor;

    fn descriptor_with(
        command: &str,
        args: &[&str],
        env: BTreeMap<String, String>,
        initialization_options: Option<serde_json::Value>,
    ) -> LspServerDescriptor {
        LspServerDescriptor {
            name: "test-lsp".to_string(),
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            filetypes: Vec::new(),
            env,
            initialization_options,
        }
    }

    fn config_with(descriptor: LspServerDescriptor) -> CustomLspServerConfig {
        CustomLspServerConfig::new(
            descriptor,
            PathBuf::from("/tmp/workspace"),
            "abc123def4567890".to_string(),
            PathBuf::from("/tmp/cache/test-lsp"),
            None,
            "warp-test".to_string(),
            Arc::new(NoopLogRedactor),
        )
    }

    #[test]
    fn workspace_folders_match_initial_workspace() {
        // InitializeParams.workspace_folders is the LSP-side identity of which
        // workspace this server is for. It must reflect `initial_workspace`,
        // not anything from the descriptor.
        let config = config_with(descriptor_with("ruby-lsp", &[], BTreeMap::new(), None));
        let resolved = block_on(config.command_and_params()).unwrap();
        let folders = resolved
            .params
            .workspace_folders
            .expect("workspace_folders set");
        assert_eq!(folders.len(), 1);
        assert!(
            folders[0].uri.as_str().ends_with("/tmp/workspace"),
            "workspace folder URI should target initial_workspace, got {}",
            folders[0].uri.as_str()
        );
    }

    #[test]
    fn initialization_options_is_none_when_descriptor_has_none() {
        let config = config_with(descriptor_with("ruby-lsp", &[], BTreeMap::new(), None));
        let resolved = block_on(config.command_and_params()).unwrap();
        assert!(resolved.params.initialization_options.is_none());
    }

    #[test]
    fn initialization_options_resolves_placeholders() {
        // The descriptor provides `{{workspace_root}}` and `{{cache_dir}}` in
        // a string leaf; expand_json must substitute them and leave other
        // value types untouched (numbers / bools / nulls pass through).
        let init_options = json!({
            "rootPath": "{{workspace_root}}",
            "cachePath": "{{cache_dir}}/index",
            "maxFiles": 1000,
            "experimental": true,
        });
        let config = config_with(descriptor_with(
            "ruby-lsp",
            &[],
            BTreeMap::new(),
            Some(init_options),
        ));
        let resolved = block_on(config.command_and_params()).unwrap();
        let options = resolved
            .params
            .initialization_options
            .expect("initialization_options set");

        assert_eq!(options["rootPath"], json!("/tmp/workspace"));
        assert_eq!(options["cachePath"], json!("/tmp/cache/test-lsp/index"));
        // Non-string leaves pass through unchanged.
        assert_eq!(options["maxFiles"], json!(1000));
        assert_eq!(options["experimental"], json!(true));
    }

    #[test]
    fn initialization_options_resolves_workspace_slug() {
        // `{{workspace_slug}}` is the third placeholder name resolved by
        // LspPlaceholderContext; the other two are exercised in
        // `initialization_options_resolves_placeholders`. This isolates
        // the slug substitution because it does not appear in any default
        // path component.
        let init_options = json!({ "id": "session-{{workspace_slug}}" });
        let config = config_with(descriptor_with(
            "ruby-lsp",
            &[],
            BTreeMap::new(),
            Some(init_options),
        ));
        let resolved = block_on(config.command_and_params()).unwrap();
        let options = resolved.params.initialization_options.unwrap();
        assert_eq!(options["id"], json!("session-abc123def4567890"));
    }

    #[test]
    fn unknown_placeholder_passes_through_in_initialization_options() {
        // Unknown placeholders are not substituted and do not error. They
        // appear verbatim in the resolved value.
        let init_options = json!({ "weird": "{{not_a_real_placeholder}}" });
        let config = config_with(descriptor_with(
            "ruby-lsp",
            &[],
            BTreeMap::new(),
            Some(init_options),
        ));
        let resolved = block_on(config.command_and_params()).unwrap();
        let options = resolved.params.initialization_options.unwrap();
        assert_eq!(options["weird"], json!("{{not_a_real_placeholder}}"));
    }

    #[test]
    fn config_kind_custom_delegates_accessors() {
        // The enum dispatch is one-line `match` per accessor; this test
        // verifies the Custom arm actually delegates instead of e.g.
        // returning a default. The BuiltIn arm is covered transitively by
        // every existing test that constructs an `LspServerModel`.
        use crate::config::LspServerConfigKind;
        let config = config_with(descriptor_with("ruby-lsp", &[], BTreeMap::new(), None));
        let kind = LspServerConfigKind::Custom(Box::new(config));
        assert_eq!(kind.server_name(), "test-lsp");
        assert_eq!(kind.initial_workspace(), PathBuf::from("/tmp/workspace"));
        assert!(kind.log_relative_path().is_none());
    }

    #[test]
    fn config_kind_custom_key_uses_descriptor_name() {
        // Custom servers identify by their descriptor `name`. Two configs
        // wrapping descriptors with the same `name` produce equal keys
        // even though every other field could differ.
        use crate::config::LspServerConfigKind;
        use crate::ServerKey;

        let kind = LspServerConfigKind::Custom(Box::new(config_with(descriptor_with(
            "ruby-lsp",
            &[],
            BTreeMap::new(),
            None,
        ))));
        assert_eq!(kind.key(), ServerKey::Custom("test-lsp".to_string()));
        assert_ne!(kind.key(), ServerKey::Custom("ruby-lsp".to_string()));
    }
}
