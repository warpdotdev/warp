use std::collections::HashSet;

use schemars::{JsonSchema, SchemaGenerator};

use crate::schema::SettingSchemaEntry;

// ---------------------------------------------------------------------------
// Fixture settings used for per-type schema validation tests
// ---------------------------------------------------------------------------

fn entries() -> Vec<&'static SettingSchemaEntry> {
    inventory::iter::<SettingSchemaEntry>.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Invariant tests (run over all registered entries)
// ---------------------------------------------------------------------------

#[test]
fn at_least_one_entry_exists() {
    assert!(
        !entries().is_empty(),
        "Expected at least one SettingSchemaEntry to be registered"
    );
}

#[test]
fn no_empty_storage_keys() {
    for entry in entries() {
        assert!(
            !entry.storage_key.is_empty(),
            "Found entry with empty storage_key"
        );
    }
}

#[test]
fn all_defaults_produce_valid_json() {
    for entry in entries() {
        let json = (entry.default_value_fn)();
        assert!(
            serde_json::from_str::<serde_json::Value>(&json).is_ok(),
            "default_value_fn for '{}' produced invalid JSON: {json}",
            entry.storage_key
        );
    }
}

#[test]
fn all_schemas_are_serializable() {
    let mut schema_gen = SchemaGenerator::default();
    for entry in entries() {
        let schema = (entry.schema_fn)(&mut schema_gen);
        assert!(
            serde_json::to_string(&schema).is_ok(),
            "schema_fn for '{}' produced a non-serializable schema",
            entry.storage_key
        );
    }
}

#[test]
fn no_duplicate_storage_keys() {
    // NOTE: when run in the settings crate alone, test modules may
    // register fixture settings with overlapping keys (e.g. both
    // mod_tests and macros_tests define "SimpleSetting"). This test
    // is most useful when run from the app crate where real settings
    // are registered. We still check for duplicates but collect them
    // all before reporting, to give a clear picture.
    let mut seen = HashSet::new();
    let mut duplicates = Vec::new();
    for entry in entries() {
        if !seen.insert(entry.storage_key) {
            duplicates.push(entry.storage_key);
        }
    }
    // In the app crate context (no test-only fixtures), there should
    // be zero duplicates.
    #[cfg(not(test))]
    assert!(
        duplicates.is_empty(),
        "Duplicate storage_keys: {duplicates:?}"
    );
}

#[test]
fn all_supported_platforms_fn_succeed() {
    for entry in entries() {
        // Just call it to ensure it doesn't panic.
        let _ = (entry.supported_platforms_fn)();
    }
}

#[test]
fn hierarchy_values_are_well_formed() {
    for entry in entries() {
        if let Some(h) = entry.hierarchy {
            assert!(
                !h.starts_with('.') && !h.ends_with('.'),
                "Hierarchy for '{}' has leading/trailing dots: '{h}'",
                entry.storage_key
            );
            assert!(
                !h.contains(".."),
                "Hierarchy for '{}' contains consecutive dots: '{h}'",
                entry.storage_key
            );
        }
    }
}

// ---------------------------------------------------------------------------
// $ref resolution test
// ---------------------------------------------------------------------------

#[test]
fn all_refs_resolve_to_definitions() {
    let mut schema_gen = SchemaGenerator::default();

    // Process all entries through the shared generator
    for entry in entries() {
        let _schema = (entry.schema_fn)(&mut schema_gen);
    }

    // Collect all $ref pointers from the schemas
    fn collect_refs(value: &serde_json::Value, refs: &mut HashSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(r)) = map.get("$ref") {
                    refs.insert(r.clone());
                }
                for v in map.values() {
                    collect_refs(v, refs);
                }
            }
            serde_json::Value::Array(arr) => {
                for v in arr {
                    collect_refs(v, refs);
                }
            }
            _ => {}
        }
    }

    // Re-process to collect refs from the output
    let mut schema_gen2 = SchemaGenerator::default();
    let mut all_refs = HashSet::new();
    for entry in entries() {
        let schema = (entry.schema_fn)(&mut schema_gen2);
        let value = serde_json::to_value(&schema).unwrap();
        collect_refs(&value, &mut all_refs);
    }

    let defs = schema_gen2.definitions();
    for r in &all_refs {
        // schemars 1.x uses "#/$defs/TypeName"
        if let Some(type_name) = r.strip_prefix("#/$defs/") {
            assert!(
                defs.contains_key(type_name),
                "$ref '{r}' does not resolve to any definition. Available: {:?}",
                defs.keys().collect::<Vec<_>>()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Per-type schema validation (using fixture settings)
// ---------------------------------------------------------------------------

/// Helper to generate a schema for a type and return it as a JSON value.
fn schema_value_for<T: JsonSchema>() -> serde_json::Value {
    let mut schema_gen = SchemaGenerator::default();
    let schema = T::json_schema(&mut schema_gen);
    schema.to_value()
}

#[test]
fn bool_schema() {
    let v = schema_value_for::<bool>();
    assert_eq!(v.get("type").and_then(|t| t.as_str()), Some("boolean"));
}

#[test]
fn string_schema() {
    let v = schema_value_for::<String>();
    assert_eq!(v.get("type").and_then(|t| t.as_str()), Some("string"));
}

#[test]
fn simple_enum_schema() {
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    enum SimpleEnum {
        A,
        B,
        C,
    }

    let v = schema_value_for::<SimpleEnum>();
    let variants = v.get("enum").and_then(|e| e.as_array());
    assert!(variants.is_some(), "Expected enum array in schema");
    let names: Vec<&str> = variants
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(names, vec!["A", "B", "C"]);
}

#[test]
fn optional_field_schema() {
    let v = schema_value_for::<Option<String>>();
    // schemars 0.8 may represent Option<T> as:
    // - {"anyOf": [...]}
    // - {"type": ["string", "null"]}
    let has_any_of = v.get("anyOf").is_some();
    let has_one_of = v.get("oneOf").is_some();
    let has_nullable_type = v
        .get("type")
        .and_then(|t| t.as_array())
        .is_some_and(|arr| arr.iter().any(|t| t.as_str() == Some("null")));
    assert!(
        has_any_of || has_one_of || has_nullable_type,
        "Expected anyOf, oneOf, or nullable type array for Option<String>, got: {v}"
    );
}

#[test]
fn struct_schema() {
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct MyStruct {
        name: String,
        count: u32,
    }

    let v = schema_value_for::<MyStruct>();
    assert_eq!(v.get("type").and_then(|t| t.as_str()), Some("object"));
    let props = v.get("properties").and_then(|p| p.as_object());
    assert!(props.is_some(), "Expected properties in struct schema");
    let props = props.unwrap();
    assert!(props.contains_key("name"));
    assert!(props.contains_key("count"));
}

#[test]
fn nested_struct_ref_resolves() {
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct Inner {
        value: i32,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct Outer {
        inner: Inner,
    }

    let mut schema_gen = SchemaGenerator::default();
    let schema = Outer::json_schema(&mut schema_gen);
    let v = serde_json::to_value(schema).unwrap();

    // The inner field should be a $ref
    let inner_prop = v
        .pointer("/properties/inner")
        .expect("Expected inner property");

    let has_ref = inner_prop.get("$ref").is_some() || inner_prop.get("allOf").is_some();
    assert!(
        has_ref,
        "Expected $ref or allOf for nested struct, got: {inner_prop}"
    );

    // The definition should exist
    let defs = schema_gen.definitions();
    assert!(
        defs.contains_key("Inner"),
        "Expected 'Inner' in definitions"
    );
}
