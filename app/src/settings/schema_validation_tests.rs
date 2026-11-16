use schemars::SchemaGenerator;
use settings::schema::SettingSchemaEntry;

fn entries() -> Vec<&'static SettingSchemaEntry> {
    inventory::iter::<SettingSchemaEntry>.into_iter().collect()
}

/// Validates that every registered setting's file default value conforms to
/// its generated JSON schema.
///
/// This catches mismatches where `SettingsValue::to_file_value` produces
/// a shape that differs from what `file_schema` declares (e.g. Duration
/// serialized as integer seconds vs. the schemars-derived `{secs, nanos}`
/// object).
///
/// Because this test lives in the app crate, all real settings are linked
/// via `inventory`, giving full coverage of every setting in the application.
#[test]
fn file_defaults_validate_against_schema() {
    let mut failures = Vec::new();

    for entry in entries() {
        // Skip private settings — they have no toml_path and aren't in the
        // user-visible schema.
        if entry.is_private {
            continue;
        }

        // Generate the type's schema with a fresh generator so $defs accumulate.
        let mut schema_gen = SchemaGenerator::default();
        let schema = (entry.schema_fn)(&mut schema_gen);
        let schema_value = schema.to_value();

        // Build a root schema document with $defs for $ref resolution.
        let mut root = serde_json::Map::new();
        root.insert(
            "$schema".to_string(),
            serde_json::Value::String("https://json-schema.org/draft/2020-12/schema".to_string()),
        );
        if let serde_json::Value::Object(obj) = schema_value {
            for (k, v) in obj {
                root.insert(k, v);
            }
        }
        let defs = schema_gen.take_definitions(true);
        if !defs.is_empty() {
            root.insert("$defs".to_string(), serde_json::Value::Object(defs));
        }
        let root_value = serde_json::Value::Object(root);

        // Parse the file default value.
        let default_json = (entry.file_default_value_fn)();
        let default_value: serde_json::Value =
            serde_json::from_str(&default_json).unwrap_or_else(|e| {
                panic!(
                    "file_default_value_fn for '{}' produced invalid JSON: {e}",
                    entry.storage_key
                )
            });

        // Validate.
        if let Err(err) = jsonschema::draft202012::validate(&root_value, &default_value) {
            failures.push(format!(
                "  '{}': default {default_json} — {err}",
                entry.storage_key,
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "File default values that do not match their schema:\n{}",
        failures.join("\n")
    );
}
