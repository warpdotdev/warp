//! Root `plugin.json` parsing and validation for Agent Plugins 1.0.0.
//!
//! The manifest schema is closed, but not uniformly fatal. Agent Plugins §5.2 and §8.1 carve out
//! two non-fatal exceptions — an unknown top-level field and a non-object `extensions` value are
//! reported and ignored — while every other violation rejects the whole plugin. Validation is
//! therefore written against the specification text rather than run through a generic JSON
//! Schema validator, because the failure boundary, not just the pass/fail answer, is what the
//! rest of the loader depends on. The published schemas are vendored beside this module and a
//! test keeps the canonical identifiers and closed field sets in sync with them.
use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::diagnostics::{PluginDiagnostic, PluginDiagnosticCode};

/// The fixed manifest location inside a plugin root.
pub const MANIFEST_FILE_NAME: &str = "plugin.json";

/// The canonical Agent Plugins 1.0.0 manifest schema identifier.
pub const MANIFEST_SCHEMA_1_0_0: &str =
    "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";

/// The only Agent Plugins version Warp implements.
pub const AGENT_PLUGINS_VERSION_1_0_0: &str = "1.0.0";

/// Maximum manifest name length, in characters (Agent Plugins §5.5).
const MAX_NAME_CHARS: usize = 64;

/// Every top-level field the closed 1.0.0 manifest schema permits.
const PERMITTED_TOP_LEVEL_FIELDS: &[&str] = &[
    "$schema",
    "name",
    "version",
    "description",
    "author",
    "homepage",
    "repository",
    "license",
    "keywords",
    "extensions",
];

/// Every field the closed `author` object permits.
const PERMITTED_AUTHOR_FIELDS: &[&str] = &["name", "email", "url"];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginAuthor {
    pub name: Option<String>,
    pub email: Option<String>,
    pub url: Option<String>,
}

/// A validated Agent Plugins manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    /// The Agent Plugins version the package targets, derived from `$schema`.
    pub agent_plugins_version: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub author: Option<PluginAuthor>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub license: Option<String>,
    pub keywords: Vec<String>,
    /// Extension data keyed by reverse-domain namespace.
    ///
    /// Contents are kept unvalidated on purpose: §8.1 requires ignoring namespaces the client
    /// does not implement without inspecting their values.
    pub extensions: BTreeMap<String, Value>,
}

/// A manifest that loaded successfully, plus any non-fatal problems worth reporting.
#[derive(Debug, Clone)]
pub struct ParsedManifest {
    pub manifest: PluginManifest,
    pub diagnostics: Vec<PluginDiagnostic>,
}

/// Extracts the Agent Plugins version from a canonical schema identifier.
///
/// Returns `None` for anything that is not an `agent-plugins.org` schema URL of the expected
/// shape, which the caller treats as an unsupported schema rather than a version mismatch.
pub fn schema_version_from_id(schema_id: &str, file_name: &str) -> Option<String> {
    let rest = schema_id.strip_prefix("https://agent-plugins.org/schemas/")?;
    let (version, tail) = rest.split_once('/')?;
    if tail != file_name || version.is_empty() {
        return None;
    }
    Some(version.to_owned())
}

/// Validates a manifest `name` against the Agent Plugins §5.5 constraints.
///
/// Returns the reason the name is invalid, or `None` when it is valid.
pub fn validate_plugin_name(name: &str) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    if chars.is_empty() {
        return Some("name must not be empty".to_owned());
    }
    if chars.len() > MAX_NAME_CHARS {
        return Some(format!(
            "name must be at most {MAX_NAME_CHARS} characters, found {}",
            chars.len()
        ));
    }
    if let Some(bad) = chars
        .iter()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || **c == '-' || **c == '.'))
    {
        return Some(format!(
            "name may contain only lowercase letters, digits, '-' and '.', found '{bad}'"
        ));
    }
    let is_alphanumeric = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    if !is_alphanumeric(chars[0]) || !is_alphanumeric(chars[chars.len() - 1]) {
        return Some("name must start and end with a lowercase letter or digit".to_owned());
    }
    if name.contains("--") || name.contains("..") {
        return Some("name must not contain consecutive '-' or '.' characters".to_owned());
    }
    None
}

/// Parses and validates the contents of a root `plugin.json`.
///
/// `Err` means the plugin is rejected and none of its components may be discovered or executed.
/// `Ok` may still carry diagnostics for the two non-fatal exceptions.
pub fn parse_manifest(content: &str) -> Result<ParsedManifest, PluginDiagnostic> {
    let value: Value = serde_json::from_str(content).map_err(|error| {
        PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestInvalidJson,
            format!("{MANIFEST_FILE_NAME} is not valid JSON: {error}"),
        )
    })?;
    let Value::Object(object) = value else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestInvalidJson,
            format!("{MANIFEST_FILE_NAME} must contain a top-level JSON object"),
        ));
    };

    let mut diagnostics = Vec::new();
    for field in object.keys() {
        if !PERMITTED_TOP_LEVEL_FIELDS.contains(&field.as_str()) {
            diagnostics.push(PluginDiagnostic::new(
                PluginDiagnosticCode::ManifestUnknownField,
                format!("ignoring unknown top-level manifest field '{field}'"),
            ));
        }
    }

    let agent_plugins_version = parse_schema(&object)?;
    let name = parse_name(&object)?;
    let author = parse_author(&object)?;
    let keywords = parse_keywords(&object)?;
    let extensions = parse_extensions(&object, &mut diagnostics);

    Ok(ParsedManifest {
        manifest: PluginManifest {
            agent_plugins_version,
            name,
            version: optional_string(&object, "version")?,
            description: optional_string(&object, "description")?,
            author,
            homepage: optional_string(&object, "homepage")?,
            repository: optional_string(&object, "repository")?,
            license: optional_string(&object, "license")?,
            keywords,
            extensions,
        },
        diagnostics,
    })
}

fn parse_schema(object: &Map<String, Value>) -> Result<String, PluginDiagnostic> {
    let Some(value) = object.get("$schema") else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestUnsupportedSchema,
            format!("{MANIFEST_FILE_NAME} is missing the required '$schema' field"),
        ));
    };
    let Some(schema_id) = value.as_str() else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestUnsupportedSchema,
            "manifest '$schema' must be a string".to_owned(),
        ));
    };
    if schema_id != MANIFEST_SCHEMA_1_0_0 {
        let reported = match schema_version_from_id(schema_id, "plugin.schema.json") {
            Some(version) => {
                format!(
                    "Agent Plugins {version} is not supported; Warp implements {AGENT_PLUGINS_VERSION_1_0_0}"
                )
            }
            None => format!("manifest '$schema' must be '{MANIFEST_SCHEMA_1_0_0}'"),
        };
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestUnsupportedSchema,
            reported,
        ));
    }
    Ok(AGENT_PLUGINS_VERSION_1_0_0.to_owned())
}

fn parse_name(object: &Map<String, Value>) -> Result<String, PluginDiagnostic> {
    let Some(value) = object.get("name") else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestInvalidField,
            format!("{MANIFEST_FILE_NAME} is missing the required 'name' field"),
        ));
    };
    let Some(name) = value.as_str() else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestInvalidField,
            "manifest 'name' must be a string".to_owned(),
        ));
    };
    if let Some(reason) = validate_plugin_name(name) {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestInvalidName,
            reason,
        ));
    }
    Ok(name.to_owned())
}

fn parse_author(object: &Map<String, Value>) -> Result<Option<PluginAuthor>, PluginDiagnostic> {
    let Some(value) = object.get("author") else {
        return Ok(None);
    };
    let Some(author_object) = value.as_object() else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestInvalidField,
            "manifest 'author' must be an object".to_owned(),
        ));
    };
    for field in author_object.keys() {
        if !PERMITTED_AUTHOR_FIELDS.contains(&field.as_str()) {
            return Err(PluginDiagnostic::new(
                PluginDiagnosticCode::ManifestInvalidField,
                format!("manifest 'author' does not permit the field '{field}'"),
            ));
        }
    }
    Ok(Some(PluginAuthor {
        name: optional_author_string(author_object, "name")?,
        email: optional_author_string(author_object, "email")?,
        url: optional_author_string(author_object, "url")?,
    }))
}

fn parse_keywords(object: &Map<String, Value>) -> Result<Vec<String>, PluginDiagnostic> {
    let Some(value) = object.get("keywords") else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestInvalidField,
            "manifest 'keywords' must be an array of strings".to_owned(),
        ));
    };
    items
        .iter()
        .map(|item| {
            item.as_str().map(str::to_owned).ok_or_else(|| {
                PluginDiagnostic::new(
                    PluginDiagnosticCode::ManifestInvalidField,
                    "manifest 'keywords' must contain only strings".to_owned(),
                )
            })
        })
        .collect()
}

/// Reads `extensions`, treating a non-object value as the §8.1 non-fatal exception.
///
/// Namespace values are kept as-is. A namespace whose value is not an object is dropped with a
/// report rather than inspected, because validating a namespace Warp does not own is exactly
/// what §8.1 forbids.
fn parse_extensions(
    object: &Map<String, Value>,
    diagnostics: &mut Vec<PluginDiagnostic>,
) -> BTreeMap<String, Value> {
    let Some(value) = object.get("extensions") else {
        return BTreeMap::new();
    };
    let Some(extensions) = value.as_object() else {
        diagnostics.push(PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestInvalidExtensions,
            "ignoring manifest 'extensions' because it is not an object".to_owned(),
        ));
        return BTreeMap::new();
    };
    let mut parsed = BTreeMap::new();
    for (namespace, data) in extensions {
        if !data.is_object() {
            diagnostics.push(PluginDiagnostic::new(
                PluginDiagnosticCode::ManifestInvalidExtensions,
                format!(
                    "ignoring extension namespace '{namespace}' because its value is not an object"
                ),
            ));
            continue;
        }
        parsed.insert(namespace.clone(), data.clone());
    }
    parsed
}

fn optional_string(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<String>, PluginDiagnostic> {
    match object.get(field) {
        None => Ok(None),
        Some(value) => value.as_str().map(str::to_owned).map(Some).ok_or_else(|| {
            PluginDiagnostic::new(
                PluginDiagnosticCode::ManifestInvalidField,
                format!("manifest '{field}' must be a string"),
            )
        }),
    }
}

fn optional_author_string(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<String>, PluginDiagnostic> {
    match object.get(field) {
        None => Ok(None),
        Some(value) => value.as_str().map(str::to_owned).map(Some).ok_or_else(|| {
            PluginDiagnostic::new(
                PluginDiagnosticCode::ManifestInvalidField,
                format!("manifest 'author.{field}' must be a string"),
            )
        }),
    }
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
