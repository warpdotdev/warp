use anyhow::Context as _;
use serde_json::{Map, Value};
use warp_cli::mcp::MCPSpec;

use crate::ai::mcp::TemplatableMCPServer;

/// Build the `mcp_servers` map to send to the public ambient-agent API.
///
/// Returns the unwrapped server map (`{ <server_name>: <server_config>, ... }`).
/// If user input includes wrapper shapes like `{ "mcpServers": { ... } }`, we unpack them.
///
/// Notes:
/// - UUID specs are coerced into `{"<uuid>": {"warp_id": "<uuid>"}}`.
/// - We do light validation to catch obvious config errors before sending the request.
pub(super) fn build_mcp_servers_from_specs(
    specs: &[MCPSpec],
) -> anyhow::Result<Option<Map<String, Value>>> {
    if specs.is_empty() {
        return Ok(None);
    }

    let mut merged = Map::new();

    for spec in specs {
        match spec {
            MCPSpec::Uuid(uuid) => {
                // TODO: Look up and use the real MCP server name from MCP managers instead of using the UUID.
                let name = uuid.to_string();
                insert_unique(
                    &mut merged,
                    name.clone(),
                    Value::Object({
                        let mut obj = Map::new();
                        obj.insert("warp_id".to_string(), Value::String(name));
                        obj
                    }),
                )?;
            }
            MCPSpec::Json(json_str) => {
                let json_str = normalize_mcp_json_for_single_server(json_str)?;
                let value = parse_json_with_optional_braces(&json_str)?;

                let server_map = TemplatableMCPServer::find_template_map(value)
                    .context(i18n::t("ai.agent_sdk.mcp_config.parse_server_map_failed"))?;

                for (name, config) in server_map {
                    insert_unique(&mut merged, name, config)?;
                }
            }
        }
    }

    validate_mcp_servers(&merged)?;

    if merged.is_empty() {
        Ok(None)
    } else {
        Ok(Some(merged))
    }
}

fn insert_unique(map: &mut Map<String, Value>, name: String, config: Value) -> anyhow::Result<()> {
    if map.contains_key(&name) {
        anyhow::bail!(
            "{}",
            i18n::t("ai.agent_sdk.mcp_config.duplicate_server_name").replace("{name}", &name)
        );
    }

    map.insert(name, config);
    Ok(())
}

fn parse_json_with_optional_braces(input: &str) -> anyhow::Result<Value> {
    // Some docs don't show curly braces around the json object, so add them if necessary.
    let json = input.trim();
    let json = if json.starts_with('{') {
        json.to_owned()
    } else {
        format!("{{{json}}}")
    };

    serde_json::from_str(&json).with_context(|| i18n::t("ai.agent_sdk.mcp_config.invalid_json"))
}

#[cfg(not(target_family = "wasm"))]
fn normalize_mcp_json_for_single_server(input: &str) -> anyhow::Result<String> {
    crate::ai::mcp::parsing::normalize_mcp_json(input)
        .map_err(|e| anyhow::anyhow!(e))
        .context(i18n::t("ai.agent_sdk.mcp_config.normalize_json_failed"))
}

// The CLI + ambient-agent API isn’t used in WASM builds, but this module still needs to compile.
// Implement the same normalization behavior (single-server shorthand wrap) locally.
#[cfg(target_family = "wasm")]
fn normalize_mcp_json_for_single_server(input: &str) -> anyhow::Result<String> {
    let json = input.trim();
    let json_for_parsing = if json.starts_with('{') {
        json.to_owned()
    } else {
        format!("{{{json}}}")
    };

    let value: Value = serde_json::from_str(&json_for_parsing)
        .with_context(|| i18n::t("ai.agent_sdk.mcp_config.invalid_json"))?;

    let is_single_server = value.get("command").is_some() || value.get("url").is_some();
    if is_single_server {
        let name = uuid::Uuid::new_v4().to_string();
        let mut map = Map::new();
        map.insert(name, value);
        Ok(Value::Object(map).to_string())
    } else {
        Ok(input.to_string())
    }
}

pub(super) fn validate_mcp_servers(mcp_servers: &Map<String, Value>) -> anyhow::Result<()> {
    for (name, config) in mcp_servers {
        validate_server_config(name, config)?;
    }

    Ok(())
}

fn validate_server_config(server_name: &str, config: &Value) -> anyhow::Result<()> {
    let obj = config.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "{}",
            i18n::t("ai.agent_sdk.mcp_config.config_must_be_object")
                .replace("{server_name}", server_name)
        )
    })?;

    let has_warp_id = obj.contains_key("warp_id");
    let has_command = obj.contains_key("command");
    let has_url = obj.contains_key("url");

    let kind_count = usize::from(has_warp_id) + usize::from(has_command) + usize::from(has_url);
    if kind_count != 1 {
        anyhow::bail!(
            "{}",
            i18n::t("ai.agent_sdk.mcp_config.exactly_one_transport")
                .replace("{server_name}", server_name)
        );
    }

    if has_warp_id {
        let warp_id = obj.get("warp_id").and_then(Value::as_str).ok_or_else(|| {
            anyhow::anyhow!(
                "{}",
                i18n::t("ai.agent_sdk.mcp_config.field_must_be_string")
                    .replace("{server_name}", server_name)
                    .replace("{field}", "warp_id")
            )
        })?;

        uuid::Uuid::parse_str(warp_id).with_context(|| {
            i18n::t("ai.agent_sdk.mcp_config.warp_id_must_be_uuid")
                .replace("{server_name}", server_name)
        })?;
    }

    if has_command {
        let command = obj.get("command").and_then(Value::as_str).ok_or_else(|| {
            anyhow::anyhow!(
                "{}",
                i18n::t("ai.agent_sdk.mcp_config.field_must_be_string")
                    .replace("{server_name}", server_name)
                    .replace("{field}", "command")
            )
        })?;

        if command.is_empty() {
            anyhow::bail!(
                "{}",
                i18n::t("ai.agent_sdk.mcp_config.field_must_be_non_empty")
                    .replace("{server_name}", server_name)
                    .replace("{field}", "command")
            );
        }

        if let Some(args) = obj.get("args") {
            let args = args.as_array().ok_or_else(|| {
                anyhow::anyhow!(
                    "{}",
                    i18n::t("ai.agent_sdk.mcp_config.args_must_be_array")
                        .replace("{server_name}", server_name)
                )
            })?;

            for (idx, arg) in args.iter().enumerate() {
                if !arg.is_string() {
                    anyhow::bail!(
                        "{}",
                        i18n::t("ai.agent_sdk.mcp_config.args_item_must_be_string")
                            .replace("{server_name}", server_name)
                            .replace("{idx}", &idx.to_string())
                    );
                }
            }
        }
    }

    if has_url {
        let url = obj.get("url").and_then(Value::as_str).ok_or_else(|| {
            anyhow::anyhow!(
                "{}",
                i18n::t("ai.agent_sdk.mcp_config.field_must_be_string")
                    .replace("{server_name}", server_name)
                    .replace("{field}", "url")
            )
        })?;

        if url.is_empty() {
            anyhow::bail!(
                "{}",
                i18n::t("ai.agent_sdk.mcp_config.field_must_be_non_empty")
                    .replace("{server_name}", server_name)
                    .replace("{field}", "url")
            );
        }
    }

    validate_string_map_field(obj, server_name, "env")?;
    validate_string_map_field(obj, server_name, "headers")?;

    Ok(())
}

fn validate_string_map_field(
    obj: &Map<String, Value>,
    server_name: &str,
    field: &str,
) -> anyhow::Result<()> {
    let Some(value) = obj.get(field) else {
        return Ok(());
    };

    let map = value.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "{}",
            i18n::t("ai.agent_sdk.mcp_config.field_must_be_object")
                .replace("{server_name}", server_name)
                .replace("{field}", field)
        )
    })?;

    for (key, value) in map {
        if !value.is_string() {
            anyhow::bail!(
                "{}",
                i18n::t("ai.agent_sdk.mcp_config.nested_field_must_be_string")
                    .replace("{server_name}", server_name)
                    .replace("{field}", field)
                    .replace("{key}", key)
            );
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "mcp_config_tests.rs"]
mod tests;
