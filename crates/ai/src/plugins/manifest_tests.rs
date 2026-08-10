use serde_json::json;

use super::*;
use crate::plugins::diagnostics::PluginDiagnosticSeverity;

/// The vendored copies of the published 1.0.0 schemas. Warp validates against the specification
/// text rather than running a generic JSON Schema validator, so these tests are what keep the
/// hand-written rules from drifting away from the canonical documents.
const VENDORED_MANIFEST_SCHEMA: &str = include_str!("schema/1.0.0/plugin.schema.json");

fn manifest_json(extra: serde_json::Value) -> String {
    let mut object = json!({
        "$schema": MANIFEST_SCHEMA_1_0_0,
        "name": "acme-tools",
    });
    if let (Some(target), Some(extra)) = (object.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    object.to_string()
}

#[test]
fn vendored_manifest_schema_matches_the_canonical_identifier() {
    let schema: serde_json::Value = serde_json::from_str(VENDORED_MANIFEST_SCHEMA).unwrap();
    assert_eq!(schema["$id"], MANIFEST_SCHEMA_1_0_0);
    assert_eq!(
        schema["properties"]["$schema"]["const"],
        MANIFEST_SCHEMA_1_0_0
    );
}

#[test]
fn permitted_top_level_fields_match_the_vendored_schema() {
    let schema: serde_json::Value = serde_json::from_str(VENDORED_MANIFEST_SCHEMA).unwrap();
    let mut from_schema: Vec<String> = schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    from_schema.sort();
    let mut permitted: Vec<String> = PERMITTED_TOP_LEVEL_FIELDS
        .iter()
        .map(|field| (*field).to_owned())
        .collect();
    permitted.sort();
    assert_eq!(permitted, from_schema);
    assert_eq!(
        schema["additionalProperties"],
        serde_json::Value::Bool(false)
    );
}

#[test]
fn permitted_author_fields_match_the_vendored_schema() {
    let schema: serde_json::Value = serde_json::from_str(VENDORED_MANIFEST_SCHEMA).unwrap();
    let mut from_schema: Vec<String> = schema["properties"]["author"]["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    from_schema.sort();
    let mut permitted: Vec<String> = PERMITTED_AUTHOR_FIELDS
        .iter()
        .map(|field| (*field).to_owned())
        .collect();
    permitted.sort();
    assert_eq!(permitted, from_schema);
}

#[test]
fn minimal_manifest_loads() {
    let parsed = parse_manifest(&manifest_json(json!({}))).unwrap();
    assert_eq!(parsed.manifest.name, "acme-tools");
    assert_eq!(parsed.manifest.agent_plugins_version, "1.0.0");
    assert!(parsed.diagnostics.is_empty());
}

#[test]
fn full_manifest_loads_every_metadata_field() {
    let parsed = parse_manifest(&manifest_json(json!({
        "version": "1.2.0",
        "description": "Brief plugin description",
        "author": { "name": "Author", "email": "a@example.com", "url": "https://example.com" },
        "homepage": "https://docs.example.com/plugin",
        "repository": "https://github.com/example/plugin",
        "license": "MIT",
        "keywords": ["one", "two"],
    })))
    .unwrap();

    let manifest = parsed.manifest;
    assert_eq!(manifest.version.as_deref(), Some("1.2.0"));
    assert_eq!(
        manifest.description.as_deref(),
        Some("Brief plugin description")
    );
    assert_eq!(manifest.license.as_deref(), Some("MIT"));
    assert_eq!(manifest.keywords, vec!["one".to_owned(), "two".to_owned()]);
    let author = manifest.author.unwrap();
    assert_eq!(author.name.as_deref(), Some("Author"));
    assert_eq!(author.email.as_deref(), Some("a@example.com"));
}

/// §5.4: metadata fields are validated only by JSON type. A non-SemVer version, a non-URL
/// homepage, and a non-SPDX license must all still load.
#[test]
fn metadata_fields_are_not_semantically_validated() {
    let parsed = parse_manifest(&manifest_json(json!({
        "version": "not-semver",
        "homepage": "not a url",
        "repository": "also not a url",
        "license": "Definitely-Not-SPDX",
        "author": { "email": "not-an-email", "url": "not a url" },
    })))
    .unwrap();
    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.manifest.version.as_deref(), Some("not-semver"));
}

/// §5.2: an unknown top-level field is reported and ignored, never fatal.
#[test]
fn unknown_top_level_field_is_reported_and_ignored() {
    let parsed = parse_manifest(&manifest_json(json!({ "components": { "hooks": [] } }))).unwrap();
    assert_eq!(parsed.manifest.name, "acme-tools");
    let diagnostic = parsed.diagnostics.first().unwrap();
    assert_eq!(diagnostic.code, PluginDiagnosticCode::ManifestUnknownField);
    assert_eq!(diagnostic.severity, PluginDiagnosticSeverity::Warning);
    assert!(diagnostic.reason.contains("components"));
}

/// §8.1: a non-object `extensions` value is reported and ignored, never fatal.
#[test]
fn non_object_extensions_is_reported_and_ignored() {
    let parsed = parse_manifest(&manifest_json(json!({ "extensions": "nope" }))).unwrap();
    assert!(parsed.manifest.extensions.is_empty());
    assert_eq!(
        parsed.diagnostics.first().unwrap().code,
        PluginDiagnosticCode::ManifestInvalidExtensions
    );
}

/// §8.1: a namespace Warp does not implement is kept without its contents being validated.
#[test]
fn unimplemented_extension_namespace_is_kept_unvalidated() {
    let parsed = parse_manifest(&manifest_json(json!({
        "extensions": {
            "com.example.client": { "anything": [1, 2, { "nested": true }] },
        },
    })))
    .unwrap();
    assert!(parsed.diagnostics.is_empty());
    assert_eq!(
        parsed.manifest.extensions["com.example.client"]["anything"][2]["nested"],
        json!(true)
    );
}

#[test]
fn fatal_manifest_violations_reject_the_plugin() {
    // (case, manifest JSON, expected code)
    let cases: Vec<(&str, String, PluginDiagnosticCode)> = vec![
        (
            "not JSON",
            "{ this is not json".to_owned(),
            PluginDiagnosticCode::ManifestInvalidJson,
        ),
        (
            "not an object",
            "[]".to_owned(),
            PluginDiagnosticCode::ManifestInvalidJson,
        ),
        (
            "missing $schema",
            json!({ "name": "acme-tools" }).to_string(),
            PluginDiagnosticCode::ManifestUnsupportedSchema,
        ),
        (
            "unsupported Agent Plugins version",
            json!({
                "$schema": "https://agent-plugins.org/schemas/2.0.0/plugin.schema.json",
                "name": "acme-tools",
            })
            .to_string(),
            PluginDiagnosticCode::ManifestUnsupportedSchema,
        ),
        (
            "unrelated $schema",
            json!({ "$schema": "https://example.com/schema.json", "name": "acme-tools" })
                .to_string(),
            PluginDiagnosticCode::ManifestUnsupportedSchema,
        ),
        (
            "missing name",
            json!({ "$schema": MANIFEST_SCHEMA_1_0_0 }).to_string(),
            PluginDiagnosticCode::ManifestInvalidField,
        ),
        (
            "name wrong type",
            json!({ "$schema": MANIFEST_SCHEMA_1_0_0, "name": 7 }).to_string(),
            PluginDiagnosticCode::ManifestInvalidField,
        ),
        (
            "version wrong type",
            manifest_json(json!({ "version": 1 })),
            PluginDiagnosticCode::ManifestInvalidField,
        ),
        (
            "keywords wrong type",
            manifest_json(json!({ "keywords": "one" })),
            PluginDiagnosticCode::ManifestInvalidField,
        ),
        (
            "keywords contains a non-string",
            manifest_json(json!({ "keywords": ["one", 2] })),
            PluginDiagnosticCode::ManifestInvalidField,
        ),
        (
            "author wrong type",
            manifest_json(json!({ "author": "Author" })),
            PluginDiagnosticCode::ManifestInvalidField,
        ),
        (
            "author has an extra field",
            manifest_json(json!({ "author": { "name": "A", "twitter": "@a" } })),
            PluginDiagnosticCode::ManifestInvalidField,
        ),
        (
            "author field wrong type",
            manifest_json(json!({ "author": { "name": 1 } })),
            PluginDiagnosticCode::ManifestInvalidField,
        ),
    ];

    for (case, content, expected) in cases {
        let diagnostic = parse_manifest(&content)
            .err()
            .unwrap_or_else(|| panic!("{case}: expected the plugin to be rejected"));
        assert_eq!(diagnostic.code, expected, "{case}");
        assert!(diagnostic.is_error(), "{case}: must be an error");
    }
}

/// §5.5, including the examples the specification calls out by name.
#[test]
fn plugin_name_constraints() {
    let valid = ["my-plugin", "acme.tools", "lint3r", "a", &"a".repeat(64)];
    for name in valid {
        assert!(
            validate_plugin_name(name).is_none(),
            "'{name}' should be a valid plugin name"
        );
    }

    let invalid = [
        "",
        "My-Plugin",
        "-start",
        "end-",
        ".start",
        "end.",
        "has--double",
        "too.many..dots",
        "has_underscore",
        "has space",
        "has/slash",
        &"a".repeat(65),
    ];
    for name in invalid {
        assert!(
            validate_plugin_name(name).is_some(),
            "'{name}' should be an invalid plugin name"
        );
    }
}

#[test]
fn invalid_name_is_reported_with_its_own_code() {
    let content = json!({ "$schema": MANIFEST_SCHEMA_1_0_0, "name": "Has-Uppercase" }).to_string();
    let diagnostic = parse_manifest(&content).unwrap_err();
    assert_eq!(diagnostic.code, PluginDiagnosticCode::ManifestInvalidName);
}

#[test]
fn schema_version_is_extracted_from_canonical_identifiers() {
    assert_eq!(
        schema_version_from_id(MANIFEST_SCHEMA_1_0_0, "plugin.schema.json").as_deref(),
        Some("1.0.0")
    );
    assert_eq!(
        schema_version_from_id(
            "https://agent-plugins.org/schemas/1.1.0/mcp.schema.json",
            "mcp.schema.json"
        )
        .as_deref(),
        Some("1.1.0")
    );
    // A different file name at the same version is not the schema the caller asked about.
    assert_eq!(
        schema_version_from_id(MANIFEST_SCHEMA_1_0_0, "mcp.schema.json"),
        None
    );
    assert_eq!(
        schema_version_from_id(
            "https://example.com/schemas/1.0.0/plugin.schema.json",
            "plugin.schema.json"
        ),
        None
    );
}
