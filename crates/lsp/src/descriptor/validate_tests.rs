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
    assert!(s.contains("anonymous"));
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

#[test]
fn display_omits_raw_glob_pattern_and_reason() {
    let err = LspDescriptorError {
        entry_name: Some("ruby-lsp".to_string()),
        kind: LspDescriptorErrorKind::InvalidGlob {
            pattern: "se[cret-AKIA".to_string(),
            reason: "unterminated character class".to_string(),
        },
    };
    let shown = err.to_string();
    assert!(!shown.contains("se[cret-AKIA"), "leaked pattern: {shown}");
    assert!(!shown.contains("unterminated"), "leaked reason: {shown}");
    // A valid entry name is charset-constrained, so it is safe to include.
    assert!(shown.contains("ruby-lsp"));
}

#[test]
fn display_omits_raw_command() {
    let err = LspDescriptorError {
        entry_name: Some("ruby-lsp".to_string()),
        kind: LspDescriptorErrorKind::UnsafeCommandPath {
            command: "./AKIAIOSFODNN7EXAMPLE/server".to_string(),
            reason: "must be an absolute path or a bare command name (no path separators)",
        },
    };
    assert!(!err.to_string().contains("AKIA"), "leaked command");
}

#[test]
fn display_omits_raw_serde_message() {
    let err = LspDescriptorError {
        entry_name: None,
        kind: LspDescriptorErrorKind::MalformedEntry {
            reason: "invalid type: AKIAIOSFODNN7EXAMPLE".to_string(),
        },
    };
    let shown = err.to_string();
    assert!(!shown.contains("AKIA"), "leaked serde message: {shown}");
    assert!(shown.contains("anonymous"));
}

#[test]
fn display_uses_anonymous_for_invalid_name() {
    // An InvalidName entry's own name failed validation, so it may carry a
    // secret-shaped value and must not be echoed.
    let err = LspDescriptorError {
        entry_name: Some("token=AKIAIOSFODNN7EXAMPLE".to_string()),
        kind: LspDescriptorErrorKind::InvalidName {
            reason: "must contain only ASCII letters, digits, `.`, `_`, or `-`",
        },
    };
    let shown = err.to_string();
    assert!(!shown.contains("AKIA"), "leaked invalid name: {shown}");
    assert!(shown.contains("anonymous"));
}
