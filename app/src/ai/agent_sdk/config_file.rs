use std::path::Path;

use anyhow::Context as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use warp_cli::agent::Harness;

use crate::ai::ambient_agents::AgentConfigSnapshot;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HarnessConfig {
    #[serde(
        rename = "type",
        serialize_with = "serialize_harness",
        deserialize_with = "deserialize_harness"
    )]
    pub harness_type: Harness,
}

fn serialize_harness<S: Serializer>(harness: &Harness, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&harness.to_string())
}

fn deserialize_harness<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Harness, D::Error> {
    let name = String::deserialize(deserializer)?;
    Ok(match name.as_str() {
        "claude" => Harness::Claude,
        "opencode" => Harness::OpenCode,
        "gemini" => Harness::Gemini,
        _ => Harness::Unknown,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarnessAuthSecretsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_auth_secret_name: Option<String>,
}

/// A strict, file-based representation of `AgentConfigSnapshot`.
///
/// Notes:
/// - Keys are snake_case and unknown keys are rejected.
/// - MCP configuration must be provided only under the `mcp_servers` key and must be the
///   unwrapped server map `{ <server_name>: <server_config>, ... }`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfigSnapshotFile {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub environment_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub base_prompt: Option<String>,
    #[serde(default)]
    pub mcp_servers: Option<Map<String, Value>>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub computer_use_enabled: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct LoadedAgentConfigSnapshotFile {
    pub file: AgentConfigSnapshotFile,
}

/// Load an `AgentConfigSnapshotFile` from disk.
///
/// Parsing rules:
/// - `.json` => JSON
/// - `.yml` / `.yaml` => YAML
/// - otherwise: try JSON, then YAML
pub fn load_config_file(path: &Path) -> anyhow::Result<LoadedAgentConfigSnapshotFile> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file '{}'", path.display()))?;

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());

    let file = match ext.as_deref() {
        Some("json") => parse_json(&contents)
            .with_context(|| format!("Invalid JSON in config file '{}'", path.display()))?,
        Some("yml") | Some("yaml") => parse_yaml(&contents)
            .with_context(|| format!("Invalid YAML in config file '{}'", path.display()))?,
        _ => parse_json(&contents)
            .or_else(|_| parse_yaml(&contents))
            .with_context(|| {
                format!(
                    "Failed to parse config file '{}' as JSON or YAML",
                    path.display()
                )
            })?,
    };

    if let Some(mcp_servers) = &file.mcp_servers {
        super::mcp_config::validate_mcp_servers(mcp_servers)
            .with_context(|| format!("Invalid mcp_servers in '{}'", path.display()))?;
    }

    Ok(LoadedAgentConfigSnapshotFile { file })
}

fn parse_json(input: &str) -> anyhow::Result<AgentConfigSnapshotFile> {
    serde_json::from_str::<AgentConfigSnapshotFile>(input).with_context(supported_keys_context)
}

fn parse_yaml(input: &str) -> anyhow::Result<AgentConfigSnapshotFile> {
    // `serde_yaml` can deserialize into `serde_json::Value` directly.
    serde_yaml::from_str::<AgentConfigSnapshotFile>(input).with_context(supported_keys_context)
}

fn supported_keys_context() -> String {
    "Supported keys: name, environment_id, model_id, base_prompt, mcp_servers, host, computer_use_enabled".to_string()
}

/// Merge config file settings with CLI-provided overrides.
///
/// Precedence: CLI > file > default.
pub fn merge_with_precedence(
    file: Option<&LoadedAgentConfigSnapshotFile>,
    cli: AgentConfigSnapshot,
) -> AgentConfigSnapshot {
    let default_file = AgentConfigSnapshotFile::default();
    let file = file.map(|loaded| &loaded.file).unwrap_or(&default_file);

    let name = cli.name.or_else(|| file.name.clone());
    let environment_id = cli.environment_id.or_else(|| file.environment_id.clone());
    let model_id = cli.model_id.or_else(|| file.model_id.clone());
    let base_prompt = cli.base_prompt.or_else(|| file.base_prompt.clone());

    let mcp_servers = merge_mcp_servers(file.mcp_servers.clone(), cli.mcp_servers);
    let worker_host = cli.worker_host.or_else(|| file.host.clone());
    let computer_use_enabled = cli.computer_use_enabled.or(file.computer_use_enabled);

    AgentConfigSnapshot {
        name,
        environment_id,
        model_id,
        base_prompt,
        mcp_servers,
        profile_id: None,
        worker_host,
        skill_spec: cli.skill_spec,
        computer_use_enabled,
        harness: cli.harness,
        harness_auth_secrets: cli.harness_auth_secrets,
    }
}

/// Merge MCP servers from two sources.
///
/// Returns the merged map, or None if both inputs are None/empty.
pub fn merge_mcp_servers(
    file_mcp: Option<Map<String, Value>>,
    cli_mcp: Option<Map<String, Value>>,
) -> Option<Map<String, Value>> {
    match (file_mcp, cli_mcp) {
        (None, None) => None,
        (Some(map), None) => {
            if map.is_empty() {
                None
            } else {
                Some(map)
            }
        }
        (None, Some(map)) => {
            if map.is_empty() {
                None
            } else {
                Some(map)
            }
        }
        (Some(mut file_map), Some(cli_map)) => {
            for (k, v) in cli_map {
                file_map.insert(k, v);
            }
            if file_map.is_empty() {
                None
            } else {
                Some(file_map)
            }
        }
    }
}

#[cfg(test)]
#[path = "config_file_tests.rs"]
mod tests;
