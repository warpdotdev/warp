use anyhow::Context as _;
use serde_json::{Map, Value};
use warp_cli::mcp::MCPSpec;
use warp_localization::{replace_placeholders, LocaleId};

use crate::ai::mcp::TemplatableMCPServer;
use crate::localization;

fn text(key: &str) -> String {
    localization::text_for_locale(LocaleId::EnUs, key)
}

fn text_with_args(key: &str, args: &[(&str, &str)]) -> String {
    replace_placeholders(&text(key), args)
        .expect("localized text template arguments must match the catalog")
}

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
                    .context(text("agent_sdk.mcp_config.error.parse_server_map"))?;

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
        anyhow::bail!(text_with_args(
            "agent_sdk.mcp_config.error.duplicate_server",
            &[("name", &name)]
        ));
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

    serde_json::from_str(&json).with_context(|| text("agent_sdk.mcp_config.error.invalid_json"))
}

#[cfg(not(target_family = "wasm"))]
fn normalize_mcp_json_for_single_server(input: &str) -> anyhow::Result<String> {
    crate::ai::mcp::parsing::normalize_mcp_json(input)
        .map_err(|e| anyhow::anyhow!(e))
        .context(text("agent_sdk.mcp_config.error.normalize_json_failed"))
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
        .with_context(|| text("agent_sdk.mcp_config.error.invalid_json"))?;

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
        anyhow::anyhow!(text_with_args(
            "agent_sdk.mcp_config.error.server_config_object",
            &[("server_name", server_name)]
        ))
    })?;

    let has_warp_id = obj.contains_key("warp_id");
    let has_command = obj.contains_key("command");
    let has_url = obj.contains_key("url");

    let kind_count = usize::from(has_warp_id) + usize::from(has_command) + usize::from(has_url);
    if kind_count != 1 {
        anyhow::bail!(text_with_args(
            "agent_sdk.mcp_config.error.exactly_one_source",
            &[("server_name", server_name)]
        ));
    }

    if has_warp_id {
        let warp_id = obj
            .get("warp_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!(field_error(server_name, "warp_id", "field_string")))?;

        uuid::Uuid::parse_str(warp_id)
            .with_context(|| field_error(server_name, "warp_id", "field_uuid"))?;
    }

    if has_command {
        let command = obj
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!(field_error(server_name, "command", "field_string")))?;

        if command.is_empty() {
            anyhow::bail!(field_error(server_name, "command", "field_non_empty"));
        }

        if let Some(args) = obj.get("args") {
            let args = args
                .as_array()
                .ok_or_else(|| anyhow::anyhow!(field_error(server_name, "args", "field_array")))?;

            for (idx, arg) in args.iter().enumerate() {
                if !arg.is_string() {
                    anyhow::bail!(text_with_args(
                        "agent_sdk.mcp_config.error.args_string",
                        &[("server_name", server_name), ("index", &idx.to_string())]
                    ));
                }
            }
        }
    }

    if has_url {
        let url = obj
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!(field_error(server_name, "url", "field_string")))?;

        if url.is_empty() {
            anyhow::bail!(field_error(server_name, "url", "field_non_empty"));
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

    let map = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!(field_error(server_name, field, "field_object")))?;

    for (key, value) in map {
        if !value.is_string() {
            anyhow::bail!(text_with_args(
                "agent_sdk.mcp_config.error.nested_field_string",
                &[
                    ("server_name", server_name),
                    ("field", field),
                    ("key", key.as_str())
                ]
            ));
        }
    }

    Ok(())
}

fn field_error(server_name: &str, field: &str, error_key_suffix: &str) -> String {
    let key = format!("agent_sdk.mcp_config.error.{error_key_suffix}");
    text_with_args(&key, &[("server_name", server_name), ("field", field)])
}

#[cfg(test)]
#[path = "mcp_config_tests.rs"]
mod tests;
