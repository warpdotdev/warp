use std::path::{Path, PathBuf};

use ai::hooks::ExplicitHookSource;
use anyhow::Context as _;
use serde_json::{Map, Value};
use warp_cli::agent::HooksOnAsk;
use warp_cli::mcp::MCPSpec;
use warp_core::features::FeatureFlag;

use super::driver::hooks::HookRunOptions;
use crate::ai::ambient_agents::AgentConfigSnapshot;
use crate::ai::blocklist::hooks::RunSurface;

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
    pub runner_id: Option<String>,
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
    /// Hook sources to load for the run. Only `warp agent run` accepts them.
    #[serde(default)]
    pub hooks: Option<Vec<HookSourceEntry>>,
    /// Turns on the strict hook policy for the run.
    #[serde(default)]
    pub hooks_strict: Option<bool>,
    /// What a hook's `ask` does in the run: `deny` (the default) or `fail_run`.
    #[serde(default)]
    pub hooks_on_ask: Option<String>,
}

impl AgentConfigSnapshotFile {
    pub fn has_hook_settings(&self) -> bool {
        self.hooks.is_some() || self.hooks_strict.is_some() || self.hooks_on_ask.is_some()
    }
}

/// One hook source named by the configuration file.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookSourceEntry {
    /// A directory holding a `hooks.json`. A relative path is resolved against the directory the
    /// configuration file is in.
    pub path: String,
    /// The file's format. Only `cursor` is supported, and it is the default.
    #[serde(default)]
    pub format: Option<String>,
    /// Who delivered the source, which decides its merge priority: `run` (the default),
    /// `factory_agent`, or `factory_wide`.
    #[serde(default)]
    pub scope: Option<String>,
}

/// The hook settings a configuration file asked for, validated, with source paths made absolute.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfiguredHooks {
    pub sources: Vec<ExplicitHookSource>,
    pub strict: Option<bool>,
    pub on_ask: Option<HooksOnAsk>,
}

#[derive(Debug, Clone)]
pub struct LoadedAgentConfigSnapshotFile {
    pub file: AgentConfigSnapshotFile,
    pub hooks: ConfiguredHooks,
}

/// Load an `AgentConfigSnapshotFile` from disk for a command that does not run an agent locally.
/// Hook settings are rejected: they apply only to `warp agent run`, and a cloud run never reads
/// this file.
///
/// Parsing rules:
/// - `.json` => JSON
/// - `.yml` / `.yaml` => YAML
/// - otherwise: try JSON, then YAML
#[cfg(not(target_family = "wasm"))]
pub fn load_config_file(path: &Path) -> anyhow::Result<LoadedAgentConfigSnapshotFile> {
    let loaded = load_config_file_allowing_hooks(path)?;
    if loaded.file.has_hook_settings() {
        anyhow::bail!(
            "`hooks`, `hooks_strict`, and `hooks_on_ask` in config file '{}' apply only to \
             `agent run`; remove them for this command",
            path.display()
        );
    }
    Ok(loaded)
}

/// Load an `AgentConfigSnapshotFile` from disk for `warp agent run`, which accepts hook settings
/// when agent hooks are enabled.
#[cfg(not(target_family = "wasm"))]
pub fn load_config_file_for_local_run(
    path: &Path,
) -> anyhow::Result<LoadedAgentConfigSnapshotFile> {
    let loaded = load_config_file_allowing_hooks(path)?;
    if loaded.file.has_hook_settings() && !FeatureFlag::AgentHooks.is_enabled() {
        anyhow::bail!(
            "Config file '{}' sets hook options, but agent hooks are not enabled in this build",
            path.display()
        );
    }
    Ok(loaded)
}

#[cfg(not(target_family = "wasm"))]
fn load_config_file_allowing_hooks(path: &Path) -> anyhow::Result<LoadedAgentConfigSnapshotFile> {
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

    let config_path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let config_dir = config_path.parent().unwrap_or(Path::new(""));
    let hooks = validate_hooks(&file, config_dir)
        .with_context(|| format!("Invalid hook settings in '{}'", path.display()))?;

    Ok(LoadedAgentConfigSnapshotFile { file, hooks })
}

/// WASM builds don't use CLI command execution / local file access.
#[cfg(target_family = "wasm")]
pub fn load_config_file(_path: &Path) -> anyhow::Result<LoadedAgentConfigSnapshotFile> {
    Err(anyhow::anyhow!(
        "Config files are not supported in WASM builds"
    ))
}

/// WASM builds don't use CLI command execution / local file access.
#[cfg(target_family = "wasm")]
pub fn load_config_file_for_local_run(
    path: &Path,
) -> anyhow::Result<LoadedAgentConfigSnapshotFile> {
    load_config_file(path)
}

/// Checks the file's hook settings and resolves each source against `config_dir`.
#[cfg(not(target_family = "wasm"))]
fn validate_hooks(
    file: &AgentConfigSnapshotFile,
    config_dir: &Path,
) -> anyhow::Result<ConfiguredHooks> {
    use ai::hooks::{ExplicitSourceScope, sources_for_hook_dirs};

    const SUPPORTED_HOOK_FORMATS: &str = "cursor";
    const SUPPORTED_HOOK_SCOPES: &str = "run, factory_agent, factory_wide";
    const SUPPORTED_HOOKS_ON_ASK: &str = "deny, fail_run";

    let mut sources = Vec::new();
    for (index, entry) in file.hooks.iter().flatten().enumerate() {
        let path = entry.path.trim();
        if path.is_empty() {
            anyhow::bail!("hooks[{index}].path must not be empty");
        }
        match entry.format.as_deref().map(str::trim) {
            None | Some("cursor") => {}
            Some(other) => anyhow::bail!(
                "hooks[{index}].format must be one of: {SUPPORTED_HOOK_FORMATS} (got '{other}')"
            ),
        }
        let scope = match entry.scope.as_deref().map(str::trim) {
            None | Some("run") => ExplicitSourceScope::Run,
            Some("factory_agent") => ExplicitSourceScope::FactoryAgent,
            Some("factory_wide") => ExplicitSourceScope::FactoryWide,
            Some(other) => anyhow::bail!(
                "hooks[{index}].scope must be one of: {SUPPORTED_HOOK_SCOPES} (got '{other}')"
            ),
        };
        let dir = PathBuf::from(shellexpand::tilde(path).into_owned());
        let dir = if dir.is_absolute() {
            dir
        } else {
            config_dir.join(dir)
        };
        sources.extend(
            sources_for_hook_dirs(&[dir])
                .into_iter()
                .map(|source| ExplicitHookSource { scope, ..source }),
        );
    }
    let on_ask = match file.hooks_on_ask.as_deref().map(str::trim) {
        None => None,
        Some("deny") => Some(HooksOnAsk::Deny),
        Some("fail_run") => Some(HooksOnAsk::FailRun),
        Some(other) => {
            anyhow::bail!("hooks_on_ask must be one of: {SUPPORTED_HOOKS_ON_ASK} (got '{other}')")
        }
    };
    Ok(ConfiguredHooks {
        sources,
        strict: file.hooks_strict,
        on_ask,
    })
}

/// The hook settings for one `warp agent run`, from the configuration file and the command line.
/// The command line wins: `--hooks-on-ask` replaces the file's value, and `--hooks-strict` turns
/// the strict policy on whatever the file says (the flag cannot turn it off, so a file can only
/// be made stricter from the command line). `--hook-dir` entries are kept apart from the file's
/// sources because they resolve against the run's working directory, like `WARP_HOOK_DIRS`.
pub fn merge_hook_settings(
    file: Option<&LoadedAgentConfigSnapshotFile>,
    cli_strict: bool,
    cli_on_ask: Option<HooksOnAsk>,
    cli_hook_dirs: Vec<PathBuf>,
    surface: RunSurface,
) -> HookRunOptions {
    let configured = file.map(|loaded| &loaded.hooks);
    HookRunOptions {
        configured_sources: configured
            .map(|hooks| hooks.sources.clone())
            .unwrap_or_default(),
        extra_dirs: cli_hook_dirs,
        strict: cli_strict || configured.and_then(|hooks| hooks.strict).unwrap_or(false),
        on_ask: cli_on_ask
            .or_else(|| configured.and_then(|hooks| hooks.on_ask))
            .unwrap_or_default(),
        surface,
    }
}

fn parse_json(input: &str) -> anyhow::Result<AgentConfigSnapshotFile> {
    serde_json::from_str::<AgentConfigSnapshotFile>(input).with_context(supported_keys_context)
}

fn parse_yaml(input: &str) -> anyhow::Result<AgentConfigSnapshotFile> {
    // `serde_yaml` can deserialize into `serde_json::Value` directly.
    serde_yaml::from_str::<AgentConfigSnapshotFile>(input).with_context(supported_keys_context)
}

fn supported_keys_context() -> String {
    "Supported keys: name, environment_id, runner_id, model_id, base_prompt, mcp_servers, host, computer_use_enabled, hooks, hooks_strict, hooks_on_ask".to_string()
}

/// Convert an unwrapped `mcp_servers` map into runtime MCP specs for AgentDriver.
///
/// Behavior:
/// - Entries with a UUID `warp_id` become `MCPSpec::Uuid`.
/// - Entries with any other non-empty `warp_id` (e.g. `"linear"`) become `MCPSpec::WellKnown`
///   when `FeatureFlag::WellKnownMcpIds` is enabled (and are rejected otherwise);
///   the server owns the set of recognized ids and unknown ids are skipped at resolution.
/// - Entries with `command`/`url` remain as inline JSON (`MCPSpec::Json`) containing the unwrapped server map.
pub fn mcp_specs_from_mcp_servers(
    mcp_servers: &Map<String, Value>,
) -> anyhow::Result<Vec<MCPSpec>> {
    let mut uuids: Vec<uuid::Uuid> = Vec::new();
    let mut well_known: Vec<String> = Vec::new();
    let mut json_map: Map<String, Value> = Map::new();

    for (name, config) in mcp_servers {
        let obj = config
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("MCP server '{name}' config must be a JSON object"))?;

        if let Some(warp_id) = obj.get("warp_id").and_then(Value::as_str) {
            if let Ok(uuid) = uuid::Uuid::parse_str(warp_id) {
                uuids.push(uuid);
            } else if !FeatureFlag::WellKnownMcpIds.is_enabled() {
                return Err(anyhow::anyhow!(
                    "MCP server '{name}' field 'warp_id' must be a UUID"
                ));
            } else if warp_id.trim().is_empty() {
                return Err(anyhow::anyhow!(
                    "MCP server '{name}' field 'warp_id' must be non-empty"
                ));
            } else {
                well_known.push(warp_id.to_string());
            }
        } else {
            json_map.insert(name.clone(), config.clone());
        }
    }

    uuids.sort();
    uuids.dedup();
    well_known.sort();
    well_known.dedup();

    let mut specs: Vec<MCPSpec> = uuids.into_iter().map(MCPSpec::Uuid).collect();
    specs.extend(well_known.into_iter().map(MCPSpec::WellKnown));

    if !json_map.is_empty() {
        let json =
            serde_json::to_string(&json_map).context("Failed to serialize MCP server map")?;
        specs.push(MCPSpec::Json(json));
    }

    Ok(specs)
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
    let runner_id = cli.runner_id.or_else(|| file.runner_id.clone());
    let model_id = cli.model_id.or_else(|| file.model_id.clone());
    let base_prompt = cli.base_prompt.or_else(|| file.base_prompt.clone());

    let mcp_servers = merge_mcp_servers(file.mcp_servers.clone(), cli.mcp_servers);
    let worker_host = cli.worker_host.or_else(|| file.host.clone());
    let computer_use_enabled = cli.computer_use_enabled.or(file.computer_use_enabled);

    AgentConfigSnapshot {
        experimental: cli.experimental,
        name,
        environment_id,
        runner_id,
        model_id,
        base_prompt,
        mcp_servers,
        profile_id: None,
        worker_host,
        skill_spec: cli.skill_spec,
        computer_use_enabled,
        harness: cli.harness,
        harness_auth_secrets: cli.harness_auth_secrets,
        additional_source_repos: None,
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
