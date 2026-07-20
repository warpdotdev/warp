use std::collections::HashSet;
use std::path::Path;

use crate::{BuiltInLspLanguage, LANGUAGE_ENTRIES, resolve, resolve_built_in_lsp, resolve_parts};

#[test]
fn selectors_are_unique_within_each_precedence_tier() {
    let mut filenames = HashSet::new();
    let mut filename_prefixes = HashSet::new();
    let mut extensions = HashSet::new();

    for entry in LANGUAGE_ENTRIES {
        for filename in entry.filenames {
            assert!(
                filenames.insert(filename),
                "duplicate filename selector: {filename}"
            );
        }
        for prefix in entry.filename_prefixes {
            assert!(
                filename_prefixes.insert(prefix),
                "duplicate filename prefix selector: {prefix}"
            );
        }
        for extension in entry.extensions {
            assert!(
                extensions.insert(extension),
                "duplicate extension selector: {extension}"
            );
        }
    }
}

#[test]
fn entries_have_an_identity_selector_and_consumer() {
    for entry in LANGUAGE_ENTRIES {
        assert_ne!(entry.id, "", "language identity must not be empty");
        assert!(
            !entry.extensions.is_empty()
                || !entry.filenames.is_empty()
                || !entry.filename_prefixes.is_empty(),
            "{} must have at least one selector",
            entry.id
        );
        assert!(
            entry.grammar.is_some() || entry.language_id.is_some(),
            "{} must have at least one consumer facet",
            entry.id
        );
    }
}

#[test]
fn exact_filename_and_legacy_prefix_precede_extension() {
    let exact = resolve(Path::new("Dockerfile.rs"), None).expect("Dockerfile prefix should match");
    assert_eq!(exact.id, "dockerfile");
    assert_eq!(exact.grammar, Some("dockerfile"));

    let ordinary = resolve(Path::new("main.rs"), None).expect("Rust extension should match");
    assert_eq!(ordinary.id, "rust");
}

#[test]
fn filename_grammar_precedence_does_not_mask_builtin_lsp_extension() {
    for (filename, expected_lsp) in [
        ("Dockerfile.rs", BuiltInLspLanguage::Rust),
        ("Containerfile.ts", BuiltInLspLanguage::TypeScript),
    ] {
        let editor = resolve(Path::new(filename), None).expect("editor grammar should resolve");
        assert_eq!(editor.grammar, Some("dockerfile"), "{filename}");
        assert_eq!(
            resolve_built_in_lsp(Path::new(filename)),
            Some(expected_lsp),
            "{filename}"
        );
    }
}

#[test]
fn consumer_specific_selectors_do_not_expand_existing_behavior() {
    let py3 = resolve(Path::new("script.py3"), None).expect("Python grammar should match");
    assert_eq!(py3.grammar, Some("python"));
    assert_eq!(py3.language_id, None);
    assert_eq!(py3.built_in_lsp, None);

    let cts = resolve(Path::new("module.cts"), None).expect("TypeScript grammar should match");
    assert_eq!(cts.grammar, Some("typescript"));
    assert_eq!(cts.language_id, Some("typescript"));
    assert_eq!(cts.built_in_lsp, None);

    let uppercase_c = resolve(Path::new("source.C"), None).expect("C LSP mapping should match");
    assert_eq!(uppercase_c.grammar, None);
    assert_eq!(uppercase_c.language_id, Some("c"));
    assert_eq!(uppercase_c.built_in_lsp, Some(BuiltInLspLanguage::C));
}

#[test]
fn built_in_lsp_facets_have_consistent_language_ids() {
    for entry in LANGUAGE_ENTRIES {
        if let Some(built_in_lsp) = entry.built_in_lsp {
            assert_eq!(
                entry.language_id,
                Some(built_in_lsp.language_id()),
                "{} has inconsistent built-in LSP facets",
                entry.id
            );
        }
    }
}

#[test]
fn generated_registry_matches_the_pinned_seed_fixture() {
    let fixture = include_str!("../fixtures/registry_seed.tsv");
    let mut fixture_selector_count = 0;

    for line in fixture.lines().filter(|line| !line.starts_with('#')) {
        let columns = line.split('\t').collect::<Vec<_>>();
        let [
            kind,
            selector,
            id,
            grammar,
            language_id,
            built_in_lsp,
            _source,
        ] = columns.as_slice()
        else {
            panic!("invalid registry seed row: {line}");
        };
        let entry = match *kind {
            "filename" => resolve_parts(Some(selector), None, None),
            "filename_prefix" => {
                let filename = format!("{selector}fixture");
                resolve_parts(Some(&filename), None, None)
            }
            "extension" => resolve_parts(None, Some(selector), None),
            other => panic!("unknown selector kind: {other}"),
        }
        .unwrap_or_else(|| panic!("seed selector did not resolve: {kind} {selector}"));

        assert_eq!(entry.id, *id, "{kind} {selector}");
        assert_eq!(entry.grammar, fixture_option(grammar), "{kind} {selector}");
        assert_eq!(
            entry.language_id,
            fixture_option(language_id),
            "{kind} {selector}"
        );
        assert_eq!(
            entry.built_in_lsp,
            fixture_built_in_lsp(built_in_lsp),
            "{kind} {selector}"
        );
        fixture_selector_count += 1;
    }

    let registry_selector_count = LANGUAGE_ENTRIES
        .iter()
        .map(|entry| entry.extensions.len() + entry.filenames.len() + entry.filename_prefixes.len())
        .sum::<usize>();
    assert_eq!(registry_selector_count, fixture_selector_count);
}

fn fixture_option(value: &str) -> Option<&str> {
    match value {
        "-" => None,
        value => Some(value),
    }
}

fn fixture_built_in_lsp(value: &str) -> Option<BuiltInLspLanguage> {
    match value {
        "-" => None,
        "Rust" => Some(BuiltInLspLanguage::Rust),
        "Go" => Some(BuiltInLspLanguage::Go),
        "Python" => Some(BuiltInLspLanguage::Python),
        "TypeScript" => Some(BuiltInLspLanguage::TypeScript),
        "TypeScriptReact" => Some(BuiltInLspLanguage::TypeScriptReact),
        "JavaScript" => Some(BuiltInLspLanguage::JavaScript),
        "JavaScriptReact" => Some(BuiltInLspLanguage::JavaScriptReact),
        "C" => Some(BuiltInLspLanguage::C),
        "Cpp" => Some(BuiltInLspLanguage::Cpp),
        other => panic!("unknown built-in LSP language: {other}"),
    }
}

#[test]
fn first_line_detection_is_reserved_and_inert() {
    assert_eq!(
        resolve_parts(Some("script"), None, Some("#!/usr/bin/env python")),
        None
    );
}
