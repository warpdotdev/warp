use std::path::PathBuf;

use lsp_types::Uri;

use crate::config::{LanguageId, lsp_uri_to_path, path_to_lsp_uri};

const LANGUAGE_ID_GOLDEN_CASES: &[(LanguageId, &str)] = &[
    (LanguageId::Rust, "rust"),
    (LanguageId::Go, "go"),
    (LanguageId::Python, "python"),
    (LanguageId::TypeScript, "typescript"),
    (LanguageId::TypeScriptReact, "typescriptreact"),
    (LanguageId::JavaScript, "javascript"),
    (LanguageId::JavaScriptReact, "javascriptreact"),
    (LanguageId::C, "c"),
    (LanguageId::Cpp, "cpp"),
];

const PATH_GOLDEN_CASES: &[(&[&str], LanguageId)] = &[
    (&["rs"], LanguageId::Rust),
    (&["go"], LanguageId::Go),
    (&["py"], LanguageId::Python),
    (&["ts"], LanguageId::TypeScript),
    (&["tsx"], LanguageId::TypeScriptReact),
    (&["js", "mjs", "cjs"], LanguageId::JavaScript),
    (&["jsx"], LanguageId::JavaScriptReact),
    (&["c", "C"], LanguageId::C),
    (
        &["cc", "cpp", "cxx", "h", "H", "hh", "hpp", "hxx"],
        LanguageId::Cpp,
    ),
];

#[test]
fn registry_preserves_all_builtin_language_identifiers() {
    for &(language_id, expected) in LANGUAGE_ID_GOLDEN_CASES {
        assert_eq!(language_id.lsp_language_identifier(), expected);
    }
}

#[test]
fn registry_preserves_all_builtin_path_mappings() {
    for &(extensions, expected) in PATH_GOLDEN_CASES {
        for extension in extensions {
            let path = PathBuf::from(format!("fixture.{extension}"));
            assert_eq!(LanguageId::from_path(&path), Some(expected), "{extension}");
        }
    }
}

#[test]
fn filename_matches_without_builtin_facet_do_not_mask_extension_dispatch() {
    for (filename, expected) in [
        ("Dockerfile.rs", LanguageId::Rust),
        ("Containerfile.ts", LanguageId::TypeScript),
    ] {
        assert_eq!(
            LanguageId::from_path(&PathBuf::from(filename)),
            Some(expected)
        );
    }
}

#[test]
fn registry_language_ids_do_not_expand_builtin_server_dispatch() {
    for extension in ["py3", "pyw", "pyi", "cts", "mts", "h++"] {
        let path = PathBuf::from(format!("fixture.{extension}"));
        assert_eq!(LanguageId::from_path(&path), None, "{extension}");
    }
}

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
