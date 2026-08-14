//! Shared, surface-neutral core for custom LLM endpoint definitions.
//!
//! This module owns everything that must behave identically regardless of which
//! surface persists endpoint definitions: the settings-file schema, per-entry
//! parsing and validation, deterministic per-model identity derivation, and the
//! join between a definitions collection and a name-keyed secret map.
//!
//! The TUI is the first adopter (definitions in `settings.toml`, keys in a split
//! secure-storage entry). A future GUI migration is expected to select this same
//! `SettingsCollection` shape instead of the legacy monolithic `AiApiKeys` blob;
//! nothing here is TUI-specific, so none of these types are prefixed `Tui`.

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use settings_value::SettingsValue as _;
use sha2::{Digest, Sha256};
use url::Url;

use crate::api_keys::{CustomEndpoint, CustomEndpointModel, CustomEndpointSchema};

/// Domain separator for the deterministic per-model `config_key` derivation.
/// Mixed into the hash so this derivation can never collide with an unrelated
/// use of SHA-256 elsewhere in the client.
const CONFIG_KEY_DOMAIN: &[u8] = b"warp.settings.custom_endpoint.config_key.v1\0";
/// Version prefix for the derived `config_key` string. Lets a future
/// derivation change coexist with values already handed out under this scheme.
const CONFIG_KEY_PREFIX: &str = "custom-endpoint:v1:";

/// Secure-storage entry name for settings-backed custom endpoint API keys, a
/// JSON object of `{ "<endpoint name>": "<api key>" }`. Lives in whichever
/// secure-storage service is active for the current surface (e.g. the TUI's
/// `.tui`-suffixed service), which is what keeps GUI and TUI credentials
/// isolated — the entry name itself carries no surface identity.
pub const CUSTOM_ENDPOINT_API_KEYS_STORAGE_KEY: &str = "CustomEndpointApiKeys";

// ---------------------------------------------------------------------------
// File schema
// ---------------------------------------------------------------------------

/// One model entry as authored under `cloud_platform.custom_endpoints."<name>".models`.
///
/// `deny_unknown_fields` rejects a hand-authored `config_key` (or any other
/// unknown field), since generated identities must never be user-editable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomEndpointModelDefinitionFile {
    /// The model slug sent to the endpoint.
    pub name: String,
    /// Optional model-picker label. A missing or empty alias falls back to
    /// `name` via [`CustomEndpointModel::display_label`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// One endpoint definition as authored under `cloud_platform.custom_endpoints."<name>"`.
///
/// `deny_unknown_fields` rejects a hand-authored `api_key`, `config_key`, or
/// `id` — this file shape has no secret or identity field by design; secrets
/// live in secure storage and identity is derived, not stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomEndpointDefinitionFile {
    /// A required HTTPS URL.
    pub url: String,
    /// The request/response protocol. Defaults to `openai_chat_completions`.
    #[serde(default)]
    pub schema: CustomEndpointSchema,
    /// One or more model records.
    pub models: Vec<CustomEndpointModelDefinitionFile>,
}

impl From<&CustomEndpointDefinition> for CustomEndpointDefinitionFile {
    fn from(definition: &CustomEndpointDefinition) -> Self {
        Self {
            url: definition.url.clone(),
            schema: definition.schema,
            models: definition
                .models
                .iter()
                .map(|model| CustomEndpointModelDefinitionFile {
                    name: model.name.clone(),
                    alias: model.alias.clone(),
                })
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Validated definitions
// ---------------------------------------------------------------------------

/// One endpoint definition that has passed [`validate_custom_endpoint_definition`].
#[derive(Debug, Clone, PartialEq)]
pub struct CustomEndpointDefinition {
    pub name: String,
    pub url: String,
    pub schema: CustomEndpointSchema,
    pub models: Vec<CustomEndpointModelDefinition>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CustomEndpointModelDefinition {
    pub name: String,
    pub alias: Option<String>,
}

/// A short, user-facing reason one endpoint definition was rejected. Surfaces
/// use this to build the `(Skipped)` row and the settings-error hint; it is
/// not meant to be machine-parsed.
pub type CustomEndpointDefinitionError = String;

/// The complete, surface-neutral custom-endpoint definitions collection, as
/// exposed by the `cloud_platform.custom_endpoints` setting.
///
/// Loading is per-entry: one invalid endpoint definition does not invalidate
/// the rest of the map (see `PRODUCT.md` Behavior 10). Valid definitions are
/// kept in file order; invalid entries are kept as diagnostics keyed by their
/// original map key so `/api-keys` can render `(Skipped)` rows and settings
/// errors can point at the offending path.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CustomEndpointDefinitionsConfig {
    valid: IndexMap<String, CustomEndpointDefinition>,
    invalid: IndexMap<String, CustomEndpointDefinitionError>,
}

impl CustomEndpointDefinitionsConfig {
    /// Builds a collection directly from already-validated definitions and
    /// diagnostics. Mainly useful for tests; production code should go through
    /// [`Self::from_file_value`]/parsing so every entry is actually validated.
    pub fn from_parts(
        valid: IndexMap<String, CustomEndpointDefinition>,
        invalid: IndexMap<String, CustomEndpointDefinitionError>,
    ) -> Self {
        Self { valid, invalid }
    }

    /// Valid definitions in file order.
    pub fn valid(&self) -> impl Iterator<Item = (&str, &CustomEndpointDefinition)> {
        self.valid.iter().map(|(name, def)| (name.as_str(), def))
    }

    /// Diagnostics for invalid entries, keyed by their original map key.
    pub fn invalid(&self) -> impl Iterator<Item = (&str, &str)> {
        self.invalid
            .iter()
            .map(|(name, error)| (name.as_str(), error.as_str()))
    }

    pub fn get(&self, name: &str) -> Option<&CustomEndpointDefinition> {
        self.valid.get(name)
    }

    pub fn is_empty(&self) -> bool {
        self.valid.is_empty() && self.invalid.is_empty()
    }

    pub fn valid_len(&self) -> usize {
        self.valid.len()
    }

    /// Whether any entry currently fails validation. Settings writes for
    /// `cloud_platform.custom_endpoints` must be inhibited while this is true,
    /// so an unrelated in-process setting write cannot erase the user's
    /// broken-but-fixable entry.
    pub fn has_diagnostics(&self) -> bool {
        !self.invalid.is_empty()
    }

    /// Names present in the source map, whether valid or invalid. A stored key
    /// for a name outside this set is an orphan (the endpoint was renamed or
    /// deleted); a stored key for a name that is present-but-invalid is kept,
    /// since fixing the definition should reconnect it.
    pub fn present_names(&self) -> impl Iterator<Item = &str> {
        self.valid
            .keys()
            .map(String::as_str)
            .chain(self.invalid.keys().map(String::as_str))
    }

    /// Parses every entry of a JSON object independently. Returns `None` only
    /// when `value` is not an object at all — a shape no per-entry recovery can
    /// help with. Individual entry failures never propagate past this point;
    /// they become diagnostics instead.
    pub fn from_object(object: &serde_json::Map<String, serde_json::Value>) -> Self {
        let mut valid = IndexMap::new();
        let mut invalid = IndexMap::new();
        for (name, value) in object {
            match parse_and_validate_entry(name, value) {
                Ok(definition) => {
                    valid.insert(name.clone(), definition);
                }
                Err(error) => {
                    invalid.insert(name.clone(), error);
                }
            }
        }
        Self { valid, invalid }
    }
}

fn parse_and_validate_entry(
    name: &str,
    value: &serde_json::Value,
) -> Result<CustomEndpointDefinition, CustomEndpointDefinitionError> {
    let file: CustomEndpointDefinitionFile = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid custom endpoint definition: {error}"))?;
    validate_custom_endpoint_definition(name, &file)
}

impl Serialize for CustomEndpointDefinitionsConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_file_value().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CustomEndpointDefinitionsConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("custom endpoints must be a map"))?;
        Ok(Self::from_object(object))
    }
}

impl settings_value::SettingsValue for CustomEndpointDefinitionsConfig {
    fn to_file_value(&self) -> serde_json::Value {
        let map = self
            .valid
            .iter()
            .map(|(name, definition)| {
                let file = CustomEndpointDefinitionFile::from(definition);
                (
                    name.clone(),
                    serde_json::to_value(file).expect("endpoint file value should serialize"),
                )
            })
            .collect();
        serde_json::Value::Object(map)
    }

    fn from_file_value(value: &serde_json::Value) -> Option<Self> {
        let object = value.as_object()?;
        Some(Self::from_object(object))
    }
}

impl schemars::JsonSchema for CustomEndpointDefinitionsConfig {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("CustomEndpointDefinitions")
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        generator.subschema_for::<HashMap<String, CustomEndpointDefinitionFile>>()
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Why a custom endpoint URL was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CustomEndpointUrlError {
    #[error("URL must be a valid, absolute URL")]
    Invalid,
    #[error("URL must use HTTPS")]
    NotHttps,
    #[error("URL must include a host")]
    MissingHost,
    #[error("URL must not use a local or private host")]
    RestrictedHost,
}

/// Validates a custom endpoint URL: it must be an absolute, HTTPS URL with a
/// host that is not `localhost` and not a loopback, unspecified, private,
/// link-local, IPv6 unique-local, or IPv4-mapped restricted literal address.
///
/// Shared by every surface that authors custom endpoint definitions (the
/// settings-file parser and the GUI modal) so URL rules can never drift
/// between them.
pub fn validate_custom_endpoint_url(url: &str) -> Result<(), CustomEndpointUrlError> {
    let parsed = Url::parse(url).map_err(|_| CustomEndpointUrlError::Invalid)?;
    if parsed.scheme() != "https" {
        return Err(CustomEndpointUrlError::NotHttps);
    }
    let Some(host) = parsed.host_str().filter(|host| !host.is_empty()) else {
        return Err(CustomEndpointUrlError::MissingHost);
    };
    if is_restricted_host(host) {
        return Err(CustomEndpointUrlError::RestrictedHost);
    }
    Ok(())
}

fn is_restricted_host(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(is_restricted_ip)
}

fn is_restricted_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_restricted_ipv4(ip),
        IpAddr::V6(ip) => is_restricted_ipv6(ip),
    }
}

fn is_restricted_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_unspecified() || ip.is_private() || ip.is_link_local()
}

fn is_restricted_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || is_ipv6_unique_local(ip) || is_ipv6_link_local(ip)
    {
        return true;
    }
    if let Some(ipv4) = ip.to_ipv4_mapped() {
        return is_restricted_ipv4(ipv4);
    }
    false
}

fn is_ipv6_unique_local(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00
}

fn is_ipv6_link_local(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xffc0 == 0xfe80
}

/// Validates one endpoint definition as a unit and converts it to its
/// validated form. This is the single surface-neutral entry point every
/// custom-endpoint-authoring surface must call: the settings-file parser and,
/// through [`validate_legacy_custom_endpoint_definition`], the GUI modal.
pub fn validate_custom_endpoint_definition(
    name: &str,
    file: &CustomEndpointDefinitionFile,
) -> Result<CustomEndpointDefinition, CustomEndpointDefinitionError> {
    if name.is_empty() {
        return Err("endpoint name must not be empty".to_owned());
    }
    if name.trim() != name {
        return Err("endpoint name must not start or end with whitespace".to_owned());
    }
    validate_custom_endpoint_url(&file.url).map_err(|error| error.to_string())?;
    if file.models.is_empty() {
        return Err("endpoint must define at least one model".to_owned());
    }

    let mut seen_model_names = std::collections::HashSet::with_capacity(file.models.len());
    let mut models = Vec::with_capacity(file.models.len());
    for model in &file.models {
        let trimmed = model.name.trim();
        if trimmed.is_empty() || trimmed != model.name {
            return Err(format!(
                "model name {:?} must be non-empty with no leading or trailing whitespace",
                model.name
            ));
        }
        if !seen_model_names.insert(model.name.as_str()) {
            return Err(format!("duplicate model name {:?}", model.name));
        }
        if let Some(alias) = &model.alias
            && !alias.is_empty()
            && alias.trim() != alias
        {
            return Err(format!(
                "model alias {alias:?} must not start or end with whitespace"
            ));
        }
        models.push(CustomEndpointModelDefinition {
            name: model.name.clone(),
            alias: model.alias.clone(),
        });
    }

    Ok(CustomEndpointDefinition {
        name: name.to_owned(),
        url: file.url.clone(),
        schema: file.schema,
        models,
    })
}

/// Adapts the GUI modal's legacy `(name, alias)` model-row shape into the
/// shared validator, so the settings parser and the GUI's authoring path stay
/// provably equivalent (see the `equivalence` tests in this module).
pub fn validate_legacy_custom_endpoint_definition(
    name: &str,
    url: &str,
    schema: CustomEndpointSchema,
    models: &[(String, Option<String>)],
) -> Result<CustomEndpointDefinition, CustomEndpointDefinitionError> {
    let file = CustomEndpointDefinitionFile {
        url: url.to_owned(),
        schema,
        models: models
            .iter()
            .map(|(name, alias)| CustomEndpointModelDefinitionFile {
                name: name.clone(),
                alias: alias.clone(),
            })
            .collect(),
    };
    validate_custom_endpoint_definition(name, &file)
}

// ---------------------------------------------------------------------------
// Deterministic identity
// ---------------------------------------------------------------------------

/// Derives a stable, deterministic `config_key` for one endpoint's model.
///
/// The endpoint name and model name are the identity; alias, URL, schema, key,
/// and model order changes preserve it. Nothing writes this value back to
/// `settings.toml` — it is derived fresh every time definitions are loaded.
pub fn settings_custom_model_config_key(endpoint_name: &str, model_name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CONFIG_KEY_DOMAIN);
    hasher.update((endpoint_name.len() as u64).to_be_bytes());
    hasher.update(endpoint_name.as_bytes());
    hasher.update((model_name.len() as u64).to_be_bytes());
    hasher.update(model_name.as_bytes());
    let digest = hasher.finalize();
    format!("{CONFIG_KEY_PREFIX}{}", lower_hex(&digest))
}

fn lower_hex(bytes: &[u8]) -> String {
    use fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
    }
    out
}

/// Converts one validated definition plus its API key into the existing
/// [`CustomEndpoint`] request/picker representation, deriving every model's
/// `config_key`.
pub fn to_custom_endpoint(
    definition: &CustomEndpointDefinition,
    api_key: String,
) -> CustomEndpoint {
    CustomEndpoint {
        name: definition.name.clone(),
        url: definition.url.clone(),
        api_key,
        schema: definition.schema,
        models: definition
            .models
            .iter()
            .map(|model| CustomEndpointModel {
                name: model.name.clone(),
                alias: model.alias.clone(),
                config_key: settings_custom_model_config_key(&definition.name, &model.name),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Join and reconciliation
// ---------------------------------------------------------------------------

/// Result of joining endpoint definitions with the persisted name-to-key map.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CustomEndpointJoinResult {
    /// The effective `CustomEndpoint` projection: one entry per valid
    /// definition, joined with its stored key (empty when unkeyed).
    pub endpoints: Vec<CustomEndpoint>,
    /// Names present in the key map that no longer resolve to any name in the
    /// definitions collection (valid or invalid) — i.e. keys orphaned by a
    /// rename or delete. Callers should remove these from secure storage.
    pub orphaned_keys: Vec<String>,
}

/// Joins settings-backed endpoint definitions with a persisted per-endpoint
/// API key map by exact name, deriving every model's `config_key` and
/// classifying which stored keys have become orphaned.
///
/// A key for a name that is present but currently invalid is intentionally
/// *not* treated as orphaned: correcting the definition should reconnect it.
pub fn join_custom_endpoint_keys(
    definitions: &CustomEndpointDefinitionsConfig,
    keys: &IndexMap<String, String>,
) -> CustomEndpointJoinResult {
    let endpoints = definitions
        .valid()
        .map(|(name, definition)| {
            let api_key = keys.get(name).cloned().unwrap_or_default();
            to_custom_endpoint(definition, api_key)
        })
        .collect();
    let present_names: std::collections::HashSet<&str> = definitions.present_names().collect();
    let orphaned_keys = keys
        .keys()
        .filter(|name| !present_names.contains(name.as_str()))
        .cloned()
        .collect();
    CustomEndpointJoinResult {
        endpoints,
        orphaned_keys,
    }
}

#[cfg(test)]
#[path = "custom_endpoints_tests.rs"]
mod tests;
