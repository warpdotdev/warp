use super::*;
use serde_json::json;

#[test]
fn parses_minimal_entry() {
    let entries = vec![json!({
        "name": "ruby-lsp",
        "command": "ruby-lsp",
        "filetypes": [{ "pattern": "*.rb" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    assert_eq!(result.descriptors.len(), 1);
    let desc = &result.descriptors[0];
    assert_eq!(desc.name, "ruby-lsp");
    assert_eq!(desc.command, "ruby-lsp");
    assert!(desc.args.is_empty());
    assert_eq!(desc.filetypes.len(), 1);
    // Bare string filetype has no explicit language_id.
    assert!(desc.filetypes[0].language_id.is_none());
}

#[test]
fn parses_inline_filetype_with_language_id() {
    let entries = vec![json!({
        "name": "ts-lsp",
        "command": "typescript-language-server",
        "args": ["--stdio"],
        "filetypes": [
            { "pattern": "*.ts", "language_id": "typescript" },
            { "pattern": "*.tsx", "language_id": "typescriptreact" },
        ],
    })];
    let result = parse_entries(&entries);
    assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    let desc = &result.descriptors[0];
    assert_eq!(desc.filetypes.len(), 2);
    assert_eq!(desc.filetypes[0].language_id.as_deref(), Some("typescript"),);
    assert_eq!(
        desc.filetypes[1].language_id.as_deref(),
        Some("typescriptreact"),
    );
}

#[test]
fn parses_literal_basename_pattern() {
    // No glob metacharacters → matches the basename exactly, case-sensitively.
    let entries = vec![json!({
        "name": "ruby-lsp",
        "command": "ruby-lsp",
        "filetypes": [{ "pattern": "Gemfile" }],
    })];
    let result = parse_entries(&entries);
    let desc = &result.descriptors[0];
    assert_eq!(desc.filetypes[0].pattern, "Gemfile");
    assert!(desc.filetypes[0].is_match("Gemfile"));
    assert!(!desc.filetypes[0].is_match("gemfile"));
    assert!(!desc.filetypes[0].is_match("Gemfile.lock"));
}

#[test]
fn parses_glob_pattern() {
    // Has `*` → matches case-insensitively across extensions.
    let entries = vec![json!({
        "name": "ruby-lsp",
        "command": "ruby-lsp",
        "filetypes": [{ "pattern": "*.rb" }],
    })];
    let result = parse_entries(&entries);
    let desc = &result.descriptors[0];
    assert_eq!(desc.filetypes[0].pattern, "*.rb");
    assert!(desc.filetypes[0].is_match("foo.rb"));
    assert!(desc.filetypes[0].is_match("FOO.RB"));
    assert!(!desc.filetypes[0].is_match("foo.py"));
}

#[test]
fn missing_name_drops_entry() {
    let entries = vec![json!({
        "command": "ruby-lsp",
        "filetypes": [{ "pattern": "*.rb" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.descriptors.is_empty());
    assert_eq!(result.errors.len(), 1);
    assert!(matches!(
        result.errors[0].kind,
        LspDescriptorErrorKind::MissingName,
    ));
}

#[test]
fn wrong_type_for_name_field_reports_malformed() {
    // `name = 42` is a type error, not "missing name." The error should
    // attribute the failure with a meaningful description rather than
    // claim `name` is missing.
    let entries = vec![json!({
        "name": 42,
        "command": "ruby-lsp",
        "filetypes": [{ "pattern": "*.rb" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.descriptors.is_empty());
    assert_eq!(result.errors.len(), 1);
    assert!(matches!(
        result.errors[0].kind,
        LspDescriptorErrorKind::MalformedEntry { .. },
    ));
}

#[test]
fn missing_command_drops_entry() {
    let entries = vec![json!({
        "name": "ruby-lsp",
        "filetypes": [{ "pattern": "*.rb" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.descriptors.is_empty());
    assert_eq!(result.errors.len(), 1);
    assert!(matches!(
        result.errors[0].kind,
        LspDescriptorErrorKind::MissingCommand,
    ));
}

#[test]
fn empty_filetypes_drops_entry() {
    let entries = vec![json!({
        "name": "ruby-lsp",
        "command": "ruby-lsp",
        "filetypes": [],
    })];
    let result = parse_entries(&entries);
    assert!(result.descriptors.is_empty());
    assert!(result
        .errors
        .iter()
        .any(|e| matches!(e.kind, LspDescriptorErrorKind::EmptyFiletypes)));
}

#[test]
fn filetype_missing_pattern_drops_entry() {
    // `pattern` is required on each filetype entry; serde rejects entries
    // without it and the failure surfaces as `MalformedEntry`.
    let entries = vec![json!({
        "name": "ruby-lsp",
        "command": "ruby-lsp",
        "filetypes": [{ "language_id": "ruby" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.descriptors.is_empty());
    assert!(result
        .errors
        .iter()
        .any(|e| matches!(e.kind, LspDescriptorErrorKind::MalformedEntry { .. })));
}

#[test]
fn invalid_glob_drops_entry() {
    // Unclosed `[` is a glob compile error.
    let entries = vec![json!({
        "name": "ruby-lsp",
        "command": "ruby-lsp",
        "filetypes": [{ "pattern": "[unclosed" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.descriptors.is_empty());
    assert!(result
        .errors
        .iter()
        .any(|e| matches!(e.kind, LspDescriptorErrorKind::InvalidGlob { .. })));
}

#[test]
fn unsupported_glob_double_star_rejected() {
    let entries = vec![json!({
        "name": "ruby-lsp",
        "command": "ruby-lsp",
        "filetypes": [{ "pattern": "**/*.rb" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.descriptors.is_empty());
    assert!(result.errors.iter().any(|e| matches!(
        e.kind,
        LspDescriptorErrorKind::UnsupportedGlobFeature { feature: "**", .. }
    )));
}

#[test]
fn unsupported_glob_brace_alternation_rejected() {
    // The pattern must contain at least one glob metacharacter (`*`, `?`, `[`)
    // to be classified as a glob in the first place. A pattern that uses
    // brace alternation alongside a glob metacharacter triggers the
    // "unsupported feature" path.
    let entries = vec![json!({
        "name": "ruby-lsp",
        "command": "ruby-lsp",
        "filetypes": [{ "pattern": "{foo,bar}.*" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.descriptors.is_empty());
    assert!(result.errors.iter().any(|e| matches!(
        e.kind,
        LspDescriptorErrorKind::UnsupportedGlobFeature {
            feature: "{a,b}",
            ..
        }
    )));
}

#[test]
fn brace_alternation_without_glob_metacharacter_is_literal() {
    // A pattern containing none of `*`, `?`, `[` is a literal basename — even
    // if it looks like it has brace alternation, those braces are treated as
    // part of the literal filename.
    let entries = vec![json!({
        "name": "weird",
        "command": "weird",
        "filetypes": [{ "pattern": "{foo,bar}.rb" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    assert_eq!(result.descriptors.len(), 1);
    let desc = &result.descriptors[0];
    assert_eq!(desc.filetypes[0].pattern, "{foo,bar}.rb");
    // No glob metacharacters → exact-basename match.
    assert!(desc.filetypes[0].is_match("{foo,bar}.rb"));
    assert!(!desc.filetypes[0].is_match("foo.rb"));
}

#[test]
fn duplicate_name_first_wins() {
    let entries = vec![
        json!({
            "name": "ruby-lsp",
            "command": "first",
            "filetypes": [{ "pattern": "*.rb" }],
        }),
        json!({
            "name": "ruby-lsp",
            "command": "second",
            "filetypes": [{ "pattern": "*.rb" }],
        }),
    ];
    let result = parse_entries(&entries);
    assert_eq!(result.descriptors.len(), 1);
    assert_eq!(result.descriptors[0].command, "first");
    assert!(result.errors.iter().any(|e| {
        matches!(e.kind, LspDescriptorErrorKind::DuplicateName)
            && e.entry_name.as_deref() == Some("ruby-lsp")
    }));
}

#[test]
fn unknown_fields_are_ignored() {
    // Unknown fields produce no error; they're silently dropped. This keeps
    // forward-compatibility when new fields are added in later versions.
    let entries = vec![json!({
        "name": "ruby-lsp",
        "command": "ruby-lsp",
        "filetypes": [{ "pattern": "*.rb" }],
        "totally_made_up": 42,
        "future_field": { "nested": true },
    })];
    let result = parse_entries(&entries);
    assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    assert_eq!(result.descriptors.len(), 1);
}

#[test]
fn raw_descriptor_captures_unknown_fields() {
    let raw: RawDescriptor = serde_json::from_value(json!({
        "name": "ruby-lsp",
        "command": "ruby-lsp",
        "totally_made_up": 42,
        "future_field": true,
    }))
    .expect("known fields still deserialize");
    let mut keys: Vec<&str> = raw.unknown_fields.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["future_field", "totally_made_up"]);
}

#[test]
fn parses_initialization_options_verbatim() {
    let entries = vec![json!({
        "name": "jdtls",
        "command": "java",
        "filetypes": [{ "pattern": "*.java" }],
        "initialization_options": {
            "settings": {
                "java": { "import": { "gradle": { "enabled": true } } }
            }
        },
    })];
    let result = parse_entries(&entries);
    assert!(result.errors.is_empty());
    let desc = &result.descriptors[0];
    let opts = desc.initialization_options.as_ref().unwrap();
    assert_eq!(
        opts.pointer("/settings/java/import/gradle/enabled"),
        Some(&serde_json::Value::Bool(true)),
    );
}

#[test]
fn parses_env_and_args() {
    let entries = vec![json!({
        "name": "my-lsp",
        "command": "/opt/my-lsp/bin/server",
        "args": ["--stdio", "--workspace={{workspace_root}}"],
        "filetypes": [{ "pattern": "*.foo" }],
        "env": { "FOO": "1", "BAR": "{{env_HOME}}" },
    })];
    let result = parse_entries(&entries);
    assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    let desc = &result.descriptors[0];
    assert_eq!(desc.args.len(), 2);
    assert_eq!(desc.args[1], "--workspace={{workspace_root}}");
    assert_eq!(desc.env.get("FOO").map(String::as_str), Some("1"));
    assert_eq!(
        desc.env.get("BAR").map(String::as_str),
        Some("{{env_HOME}}")
    );
}

#[test]
fn one_bad_entry_does_not_drop_others() {
    // When any entry is invalid, that entry is dropped (other valid entries
    // continue to load).
    let entries = vec![
        json!({
            "name": "good",
            "command": "good",
            "filetypes": [{ "pattern": "*.good" }],
        }),
        json!({
            "name": "bad",
            "command": "bad",
            "filetypes": [],
        }),
        json!({
            "name": "also-good",
            "command": "also-good",
            "filetypes": [{ "pattern": "*.also" }],
        }),
    ];
    let result = parse_entries(&entries);
    assert_eq!(result.descriptors.len(), 2);
    let names: Vec<&str> = result.descriptors.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, vec!["good", "also-good"]);
    assert_eq!(result.errors.len(), 1);
}

#[test]
fn invalid_name_rejects_entry() {
    let entries = vec![json!({
        "name": "has space",
        "command": "ruby-lsp",
        "filetypes": [{ "pattern": "*.rb" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.descriptors.is_empty());
    assert!(result
        .errors
        .iter()
        .any(|e| matches!(e.kind, LspDescriptorErrorKind::InvalidName { .. })));
}

#[test]
fn reserved_name_rejects_entry_case_insensitively() {
    let entries = vec![json!({
        "name": "Rust-Analyzer",
        "command": "rust-analyzer",
        "filetypes": [{ "pattern": "*.rs" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.descriptors.is_empty());
    assert!(result
        .errors
        .iter()
        .any(|e| matches!(e.kind, LspDescriptorErrorKind::ReservedName)));
}

#[test]
fn unsafe_command_rejects_entry() {
    let entries = vec![json!({
        "name": "ruby-lsp",
        "command": "./server",
        "filetypes": [{ "pattern": "*.rb" }],
    })];
    let result = parse_entries(&entries);
    assert!(result.descriptors.is_empty());
    assert!(result
        .errors
        .iter()
        .any(|e| matches!(e.kind, LspDescriptorErrorKind::UnsafeCommandPath { .. })));
}

#[test]
fn duplicate_name_is_case_insensitive() {
    let entries = vec![
        json!({
            "name": "ruby-lsp",
            "command": "ruby-lsp",
            "filetypes": [{ "pattern": "*.rb" }],
        }),
        json!({
            "name": "Ruby-LSP",
            "command": "ruby-lsp",
            "filetypes": [{ "pattern": "*.rake" }],
        }),
    ];
    let result = parse_entries(&entries);
    // First in source order wins; the case-variant is a duplicate.
    assert_eq!(result.descriptors.len(), 1);
    assert_eq!(result.descriptors[0].name, "ruby-lsp");
    assert!(result
        .errors
        .iter()
        .any(|e| matches!(e.kind, LspDescriptorErrorKind::DuplicateName)));
}
