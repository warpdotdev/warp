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

#[test]
fn check_name_accepts_valid_names() {
    let max_len = "a".repeat(64);
    let valid = [
        "ruby-lsp",
        "a",
        "A1",
        "dots.and_underscores-9",
        max_len.as_str(),
    ];
    for name in valid {
        assert!(
            check_name(name).is_none(),
            "expected `{name}` to be accepted"
        );
    }
}

#[test]
fn check_name_rejects_invalid_names() {
    let too_long = "a".repeat(65);
    let invalid = [
        "",                // empty
        too_long.as_str(), // > 64 chars
        ".",
        "..",
        ".hidden",       // leading `.`
        "-leading-dash", // leading `-`
        "has space",
        "slash/in/name",
        "ünïcode", // non-ASCII
    ];
    for name in invalid {
        assert!(
            matches!(
                check_name(name),
                Some(LspDescriptorErrorKind::InvalidName { .. })
            ),
            "expected `{name}` to be rejected"
        );
    }
}

#[test]
fn is_reserved_name_matches_builtin_binary_names_case_insensitively() {
    let reserved = [
        "rust-analyzer",
        "RUST-ANALYZER",
        "gopls",
        "pyright-langserver",
        "typescript-language-server",
        "clangd",
    ];
    for name in reserved {
        assert!(is_reserved_name(name), "expected `{name}` to be reserved");
    }
}

#[test]
fn is_reserved_name_allows_non_builtin_names() {
    // `pyright` is allowed — the reserved binary name is `pyright-langserver`.
    for name in ["ruby-lsp", "my-rust-analyzer", "pyright"] {
        assert!(!is_reserved_name(name), "expected `{name}` to be allowed");
    }
}

#[test]
fn check_command_accepts_absolute_bare_and_home_rooted() {
    let ok = [
        "/opt/jdtls/bin/jdtls",
        "jdtls",
        "rust-analyzer",
        "~/bin/server",
        "~",
    ];
    for cmd in ok {
        assert!(
            check_command(cmd).is_none(),
            "expected `{cmd}` to be accepted"
        );
    }
}

#[test]
fn check_command_rejects_relative_paths_with_separators() {
    let bad = [
        "./server",
        "bin/server",
        "../server",
        "..\\server",
        "a/b",
        "~someuser/bin",
    ];
    for cmd in bad {
        assert!(
            matches!(
                check_command(cmd),
                Some(LspDescriptorErrorKind::UnsafeCommandPath { .. })
            ),
            "expected `{cmd}` to be rejected"
        );
    }
}

#[test]
fn is_windows_absolute_recognizes_drive_and_unc_paths() {
    let abs = [
        "C:\\bin\\server",
        "C:/bin/server",
        "\\\\server\\share\\x",
        "//server/share/x",
    ];
    for cmd in abs {
        assert!(
            is_windows_absolute(cmd),
            "expected `{cmd}` to be Windows-absolute"
        );
    }
}

#[test]
fn is_windows_absolute_rejects_root_relative_and_bare() {
    // `\path`/`/path` (no drive) are current-drive-relative; `C:relative` is
    // drive-relative; a bare name has no root.
    for cmd in ["\\path", "/path", "C:relative", "server"] {
        assert!(
            !is_windows_absolute(cmd),
            "expected `{cmd}` not to be Windows-absolute"
        );
    }
}
