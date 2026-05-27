//! Generates a JSON Schema file describing Warp's user-facing settings.
//!
//! Usage:
//! ```
//! cargo run --bin generate_settings_schema -- [--channel dev|preview|stable] [--locale en-US|zh-CN] [output_path]
//! ```

use std::collections::{HashMap, HashSet};
use std::io::Write;

use schemars::SchemaGenerator;
use serde_json::{Map, Value};
use settings::schema::SettingSchemaEntry;
use warp_core::features::{FeatureFlag, DEBUG_FLAGS, DOGFOOD_FLAGS, PREVIEW_FLAGS, RELEASE_FLAGS};
use warp_localization::{Catalog, CatalogBundle, LocaleId, TranslationSource};

const BUNDLED_EN_US: &str = include_str!("../../assets/bundled/locales/en-US.json");
const BUNDLED_ZH_CN: &str = include_str!("../../assets/bundled/locales/zh-CN.json");

/// Ensures all `inventory::submit!` registrations from the app crate's
/// dependency tree are linked into the binary.
///
/// Binary targets only link crate code that is transitively referenced.
/// Without an explicit reference to the `warp` library, the linker will
/// not include most of the app's object files and the `inventory`
/// submissions they contain.
fn ensure_settings_linked() {
    let _ = std::hint::black_box(warp::settings::RESTORE_SESSION);
}

/// Recursively strips `minimum`, `maximum`, and `format` from integer and
/// number schemas. schemars derives these from Rust type bounds (e.g. `u8`
/// → `minimum: 0, maximum: 255, format: "uint8"`), which are misleading
/// for settings whose valid domain is narrower than the type allows.
fn strip_numeric_metadata(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let is_numeric = map
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| t == "integer" || t == "number");

            if is_numeric {
                map.remove("minimum");
                map.remove("maximum");
                map.remove("format");
            }

            for val in map.values_mut() {
                strip_numeric_metadata(val);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                strip_numeric_metadata(val);
            }
        }
        _ => {}
    }
}

/// Removes `{"enum": [], "type": "string"}` entries from `oneOf` arrays.
/// schemars emits an empty enum bucket for externally-tagged enums when all
/// unit variants have individual descriptions (and are therefore promoted to
/// separate `oneOf` branches with `const`). The empty bucket is unreachable
/// and confuses schema consumers.
fn strip_empty_enum_entries(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(one_of)) = map.get_mut("oneOf") {
                one_of.retain(|entry| {
                    !matches!(entry, Value::Object(obj)
                        if obj.get("enum").is_some_and(|e| e.as_array().is_some_and(|a| a.is_empty()))
                    )
                });
            }

            for val in map.values_mut() {
                strip_empty_enum_entries(val);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                strip_empty_enum_entries(val);
            }
        }
        _ => {}
    }
}

fn active_flags_for_channel(channel: &str) -> HashSet<FeatureFlag> {
    let mut flags = HashSet::new();

    let flag_lists: &[&[FeatureFlag]] = match channel {
        "stable" => &[RELEASE_FLAGS],
        "preview" => &[RELEASE_FLAGS, PREVIEW_FLAGS],
        "dev" => &[RELEASE_FLAGS, PREVIEW_FLAGS, DOGFOOD_FLAGS, DEBUG_FLAGS],
        other => {
            eprintln!("Unknown channel '{other}', defaulting to dev");
            &[RELEASE_FLAGS, PREVIEW_FLAGS, DOGFOOD_FLAGS, DEBUG_FLAGS]
        }
    };

    for list in flag_lists {
        for flag in *list {
            flags.insert(*flag);
        }
    }

    flags
}

fn parse_locale(value: &str) -> LocaleId {
    LocaleId::from_system_locale(value).unwrap_or_else(|| {
        eprintln!("Unknown locale '{value}', defaulting to en-US");
        LocaleId::EnUs
    })
}

fn localization_catalogs() -> CatalogBundle {
    CatalogBundle::new(
        LocaleId::EnUs,
        [
            Catalog::from_json(LocaleId::EnUs, BUNDLED_EN_US)
                .expect("bundled en-US localization catalog should parse"),
            Catalog::from_json(LocaleId::ZhCn, BUNDLED_ZH_CN)
                .expect("bundled zh-CN localization catalog should parse"),
        ],
    )
    .expect("default localization catalog should be bundled")
}

fn schema_translation_key(entry: &SettingSchemaEntry) -> String {
    match entry.hierarchy {
        Some(hierarchy) => {
            format!(
                "settings.schema.{hierarchy}.{}.description",
                entry.storage_key
            )
        }
        None => format!("settings.schema.{}.description", entry.storage_key),
    }
}

fn localized_setting_description(
    catalogs: &CatalogBundle,
    locale: LocaleId,
    entry: &SettingSchemaEntry,
) -> String {
    let key = schema_translation_key(entry);
    let lookup = catalogs.lookup(locale, &key);
    if lookup.source == TranslationSource::Key {
        entry.description.to_string()
    } else {
        lookup.text.into_owned()
    }
}

fn localized_catalog_text_by_default_text(locale: LocaleId) -> HashMap<String, String> {
    if locale == LocaleId::EnUs {
        return HashMap::new();
    }

    let default_entries: HashMap<String, String> =
        serde_json::from_str(BUNDLED_EN_US).expect("bundled en-US catalog should parse");
    let locale_entries: HashMap<String, String> = match locale {
        LocaleId::EnUs => HashMap::new(),
        LocaleId::ZhCn => {
            serde_json::from_str(BUNDLED_ZH_CN).expect("bundled zh-CN catalog should parse")
        }
    };

    let mut localized_texts_by_default_text: HashMap<String, HashSet<String>> = HashMap::new();
    for (key, default_text) in &default_entries {
        if let Some(locale_text) = locale_entries
            .get(key)
            .filter(|locale_text| *locale_text != default_text)
        {
            localized_texts_by_default_text
                .entry(default_text.clone())
                .or_default()
                .insert(locale_text.clone());
        }
    }

    default_entries
        .into_iter()
        .filter_map(|(key, default_text)| {
            if localized_texts_by_default_text
                .get(&default_text)
                .is_none_or(|localized_texts| localized_texts.len() != 1)
            {
                return None;
            }

            locale_entries
                .get(&key)
                .filter(|locale_text| *locale_text != &default_text)
                .map(|locale_text| (default_text, locale_text.clone()))
        })
        .collect()
}

fn localize_schema_descriptions(
    value: &mut Value,
    translations_by_default_text: &HashMap<String, String>,
) {
    match value {
        Value::Object(map) => {
            for key in ["description", "title"] {
                if let Some(Value::String(text)) = map.get_mut(key) {
                    if let Some(localized_text) = translations_by_default_text.get(text.as_str()) {
                        *text = localized_text.clone();
                    }
                }
            }

            for val in map.values_mut() {
                localize_schema_descriptions(val, translations_by_default_text);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                localize_schema_descriptions(val, translations_by_default_text);
            }
        }
        _ => {}
    }
}

fn root_title(catalogs: &CatalogBundle, locale: LocaleId) -> String {
    catalogs
        .text(locale, "settings.schema.root.title")
        .into_owned()
}

fn root_description(
    catalogs: &CatalogBundle,
    locale: LocaleId,
    channel: &str,
    entry_count: usize,
) -> String {
    catalogs
        .text(locale, "settings.schema.root.description")
        .replace("{channel}", channel)
        .replace("{entry_count}", &entry_count.to_string())
}

/// Creates intermediate hierarchy objects so that a setting at e.g.
/// `appearance.text` is nested under `properties.appearance.properties.text.properties`.
fn ensure_hierarchy<'a>(
    root_properties: &'a mut Map<String, Value>,
    hierarchy: &str,
) -> &'a mut Map<String, Value> {
    let segments: Vec<&str> = hierarchy.split('.').collect();
    let mut current = root_properties;

    for segment in segments {
        // Ensure the segment object exists
        let entry = current.entry(segment.to_string()).or_insert_with(|| {
            Value::Object({
                let mut m = Map::new();
                m.insert("type".to_string(), Value::String("object".to_string()));
                m.insert("properties".to_string(), Value::Object(Map::new()));
                m
            })
        });

        // Navigate into its properties
        current = entry
            .as_object_mut()
            .expect("hierarchy node should be an object")
            .entry("properties")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .expect("properties should be an object");
    }

    current
}

fn main() {
    ensure_settings_linked();

    let args: Vec<String> = std::env::args().collect();

    let mut channel = "dev";
    let mut locale = LocaleId::EnUs;
    let mut output_path: Option<&str> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--channel" => {
                i += 1;
                if i < args.len() {
                    channel = &args[i];
                }
            }
            "--locale" => {
                i += 1;
                if i < args.len() {
                    locale = parse_locale(&args[i]);
                }
            }
            arg if !arg.starts_with('-') => {
                output_path = Some(arg);
            }
            other => {
                eprintln!("Unknown argument: {other}");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let active_flags = active_flags_for_channel(channel);
    let catalogs = localization_catalogs();
    let translations_by_default_text = localized_catalog_text_by_default_text(locale);
    let mut generator = SchemaGenerator::default();
    let mut root_properties = Map::new();
    let mut entry_count = 0;

    for entry in inventory::iter::<SettingSchemaEntry> {
        // Skip private settings
        if entry.is_private {
            continue;
        }

        // Skip settings whose feature flag is not active
        if let Some(flag) = entry.feature_flag {
            if !active_flags.contains(&flag) {
                continue;
            }
        }

        let type_schema = (entry.schema_fn)(&mut generator);

        let mut schema_value: Value = type_schema.to_value();

        // Compute default value — prefer file default over serde default
        let default_json = (entry.file_default_value_fn)();

        if let Ok(default_value) = serde_json::from_str::<Value>(&default_json) {
            if let Some(obj) = schema_value.as_object_mut() {
                obj.insert("default".to_string(), default_value);
            }
        }

        // Always overwrite description with the macro-provided one
        if !entry.description.is_empty() {
            if let Some(obj) = schema_value.as_object_mut() {
                obj.insert(
                    "description".to_string(),
                    Value::String(localized_setting_description(&catalogs, locale, entry)),
                );
            }
        }

        // Place the setting in the hierarchy
        let target = if let Some(hierarchy) = entry.hierarchy {
            ensure_hierarchy(&mut root_properties, hierarchy)
        } else {
            &mut root_properties
        };

        target.insert(entry.storage_key.to_string(), schema_value);
        entry_count += 1;
    }

    // Collect $defs from the generator
    let defs_map = generator.take_definitions(true);

    // Assemble the root document
    let mut root = Map::new();
    root.insert(
        "$schema".to_string(),
        Value::String("https://json-schema.org/draft/2020-12/schema".to_string()),
    );
    root.insert(
        "title".to_string(),
        Value::String(root_title(&catalogs, locale)),
    );
    root.insert(
        "description".to_string(),
        Value::String(root_description(&catalogs, locale, channel, entry_count)),
    );
    root.insert("type".to_string(), Value::String("object".to_string()));
    root.insert("properties".to_string(), Value::Object(root_properties));

    if !defs_map.is_empty() {
        root.insert("$defs".to_string(), Value::Object(defs_map));
    }

    // Strip type-derived numeric metadata (minimum, maximum, format) that
    // schemars emits from Rust primitive bounds (e.g. u8 → max 255).
    // These leak implementation details rather than semantic constraints.
    let mut root_value = Value::Object(root);
    localize_schema_descriptions(&mut root_value, &translations_by_default_text);
    strip_numeric_metadata(&mut root_value);
    strip_empty_enum_entries(&mut root_value);

    let output = serde_json::to_string_pretty(&root_value).expect("schema should serialize");

    if let Some(path) = output_path {
        let mut file = std::fs::File::create(path)
            .unwrap_or_else(|e| panic!("Failed to create output file '{path}': {e}"));
        file.write_all(output.as_bytes())
            .unwrap_or_else(|e| panic!("Failed to write to '{path}': {e}"));
        eprintln!("Wrote {entry_count} settings to {path}");
    } else {
        println!("{output}");
    }
}
