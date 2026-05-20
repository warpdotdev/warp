use std::collections::HashSet;

use futures::future::BoxFuture;
use futures::FutureExt;
#[cfg(not(target_family = "wasm"))]
use itertools::Itertools;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;
use warpui::{Entity, EntityId, ModelContext, ModelHandle};

#[cfg(not(target_family = "wasm"))]
use super::get_server_output_id;
use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::terminal::model::session::active_session::ActiveSession;
#[cfg(not(target_family = "wasm"))]
use crate::{
    ai::{
        agent::{AIAgentAction, AIAgentActionResultType, CallMCPToolResult},
        blocklist::{action_model::AIAgentActionType, BlocklistAIPermissions},
        mcp::TemplatableMCPServerManager,
    },
    send_telemetry_from_app_ctx, TelemetryEvent,
};

pub struct CallMCPToolExecutor {
    _active_session: ModelHandle<ActiveSession>,
    #[allow(dead_code)]
    terminal_view_id: EntityId,
}

impl CallMCPToolExecutor {
    pub fn new(_active_session: ModelHandle<ActiveSession>, terminal_view_id: EntityId) -> Self {
        Self {
            _active_session,
            terminal_view_id,
        }
    }

    #[cfg_attr(target_family = "wasm", allow(unused_variables), allow(dead_code))]
    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        #[cfg(target_family = "wasm")]
        {
            false
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let ExecuteActionInput {
                action:
                    AIAgentAction {
                        action:
                            AIAgentActionType::CallMCPTool {
                                server_id, name, ..
                            },
                        ..
                    },
                conversation_id,
            } = input
            else {
                return false;
            };

            BlocklistAIPermissions::as_ref(ctx).can_call_mcp_tool(
                server_id.as_ref(),
                name.as_str(),
                &conversation_id,
                Some(self.terminal_view_id),
                ctx,
            )
        }
    }

    #[cfg_attr(target_family = "wasm", allow(unused_variables), allow(dead_code))]
    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        #[cfg(target_family = "wasm")]
        {
            ActionExecution::<()>::InvalidAction
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let server_output_id = get_server_output_id(input.conversation_id, ctx);
            let AIAgentAction {
                action:
                    AIAgentActionType::CallMCPTool {
                        server_id,
                        name,
                        input,
                    },
                ..
            } = input.action
            else {
                return ActionExecution::InvalidAction;
            };

            let name_owned = name.to_owned();
            let name_clone = name_owned.clone();

            let serde_json::Value::Object(mut arguments) = input.clone() else {
                return ActionExecution::Sync(AIAgentActionResultType::CallMCPTool(
                    CallMCPToolResult::Error("MCP server tool input not an object".to_owned()),
                ));
            };

            // Prefer the templatable server over the legacy server if both exist.
            // It is possible for both to exist in some tricky race conditions, but in those cases
            // we shouldn't care about the legacy servers.
            let templatable_mcp_manager = TemplatableMCPServerManager::as_ref(ctx);

            // Coerce whole-number f64 args to i64 for fields declared as `"type": "integer"`
            // in the tool's input schema. MCP tool args round-trip through
            // `google.protobuf.Struct` on the wire, which erases the integer/float distinction
            // by storing everything as f64. Without coercion, the ryu formatter serializes
            // whole-number f64 as "5.0", which strict MCP servers (e.g. GoLand) reject for
            // integer-typed fields.
            if let Some(schema) =
                templatable_mcp_manager.tool_input_schema(*server_id, name.as_str())
            {
                coerce_integer_args(&mut arguments, &schema);
            }

            let templatable_peer = if let Some(installation_id) = server_id {
                templatable_mcp_manager
                    .server_with_installation_id_and_tool_name(*installation_id, name.to_owned())
            } else {
                templatable_mcp_manager.server_with_tool_name(name.to_owned())
            };

            let Some(reconnecting_peer) = templatable_peer else {
                return ActionExecution::Sync(AIAgentActionResultType::CallMCPTool(
                    CallMCPToolResult::Error("MCP server for tool not found".to_owned()),
                ));
            };

            let name_owned_inner = name_owned.clone();
            ActionExecution::new_async(
                async move {
                    reconnecting_peer
                        .call_tool(
                            rmcp::model::CallToolRequestParams::new(name_owned_inner)
                                .with_arguments(arguments),
                        )
                        .await
                },
                move |res, ctx| handle_call_tool_result(res, server_output_id, name_clone, ctx),
            )
        }
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for CallMCPToolExecutor {
    type Event = ();
}

/// Coerces whole-number floats in `args` to integers wherever the tool's JSON
/// Schema `input_schema` declares an [integer type], at any depth.
///
/// MCP tool args round-trip through `google.protobuf.Struct` on the wire, whose
/// `NumberValue` stores everything as `f64`. Without this fix, serde_json emits
/// whole-number floats as `"5.0"`, which strict MCP servers reject for integer
/// fields.
///
/// Walks the schema recursively, covering nested objects, array items, the
/// composition keywords `allOf` / `oneOf` / `anyOf`, internal `$ref` pointers,
/// and nullable type-arrays like `["integer", "null"]`. Unsupported or unknown
/// schema shapes (external `$ref`, `not`, `if`/`then`/`else`, `patternProperties`)
/// are skipped — coercion is conservative and a skip preserves the existing wire
/// behavior.
///
/// [integer type]: https://json-schema.org/understanding-json-schema/reference/type
/// Maximum recursion depth for [`coerce_recursive`]. Real MCP schemas top out
/// well under this — the limit only matters for schemas that intentionally
/// or accidentally compose into a cycle (e.g. `allOf: [{$ref: "#/Self"}]`
/// where `Self.allOf = [{$ref: "#/Self"}]`). [`resolve_refs`] already breaks
/// pure `$ref` chains; this guards every other recursive edge so a malicious
/// MCP server can't hang the client by advertising a recursive composed
/// schema (#10596 review).
const MAX_COERCE_DEPTH: usize = 64;

pub(crate) fn coerce_integer_args(
    args: &mut serde_json::Map<String, serde_json::Value>,
    input_schema: &serde_json::Map<String, serde_json::Value>,
) {
    // Clone the schema once so the walker can hold it as a `Value` (needed for
    // `$ref` resolution against the same root). Schemas are small in practice.
    let root = serde_json::Value::Object(input_schema.clone());
    let mut value = serde_json::Value::Object(std::mem::take(args));
    coerce_recursive(&mut value, &root, &root, 0);
    if let serde_json::Value::Object(map) = value {
        *args = map;
    }
}

/// Walks `value` and `schema` in parallel, coercing whole-number floats to
/// integers wherever the schema declares an integer type.
fn coerce_recursive(
    value: &mut serde_json::Value,
    schema: &serde_json::Value,
    root: &serde_json::Value,
    depth: usize,
) {
    if depth >= MAX_COERCE_DEPTH {
        return;
    }
    let schema = resolve_refs(schema, root);

    // `allOf` — apply every branch; can stack with sibling keywords.
    if let Some(branches) = schema.get("allOf").and_then(|v| v.as_array()) {
        for b in branches {
            coerce_recursive(value, b, root, depth + 1);
        }
    }

    // `oneOf` / `anyOf` — apply every branch. Coercion is conservative
    // (whole-number f64 → i64 only) and applying a branch whose schema doesn't
    // match the value is a no-op, so trying all branches sidesteps the
    // ambiguity of picking the "right" branch when multiple top-level types
    // would match.
    for key in ["oneOf", "anyOf"] {
        if let Some(branches) = schema.get(key).and_then(|v| v.as_array()) {
            for b in branches {
                coerce_recursive(value, b, root, depth + 1);
            }
        }
    }

    // Integer leaf — handles `"type": "integer"` and `"type": ["integer", ...]`.
    if declares_integer(schema) && value.is_number() {
        coerce_integer_in_place(value);
        return;
    }

    // Object — recurse into `properties`, then `additionalProperties` (when it
    // is itself a schema object) for keys outside `properties`.
    if let serde_json::Value::Object(map) = value {
        if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
            for (k, child_schema) in props {
                if let Some(child_value) = map.get_mut(k) {
                    coerce_recursive(child_value, child_schema, root, depth + 1);
                }
            }
        }
        'additional: {
            let Some(additional) = schema.get("additionalProperties") else {
                break 'additional;
            };
            if !additional.is_object() {
                break 'additional;
            }
            let known: HashSet<&str> = schema
                .get("properties")
                .and_then(|v| v.as_object())
                .map(|p| p.keys().map(String::as_str).collect())
                .unwrap_or_default();
            // Keys covered by `patternProperties` are governed by their
            // pattern's schema, not by `additionalProperties` — so they
            // must be excluded here even though we don't (yet) coerce
            // through `patternProperties` itself.
            //
            // JSON Schema's pattern grammar is ECMA-262; Rust's `regex`
            // crate is a strict subset and rejects valid patterns such as
            // those with lookaheads or certain Unicode constructs. We
            // can't safely tell those apart from "this key doesn't match
            // the pattern", so if any pattern fails to compile we fall
            // back to skipping `additionalProperties` for this schema —
            // a key that should have been governed by the pattern won't
            // get coerced with the wrong schema.
            let pattern_compile = schema
                .get("patternProperties")
                .and_then(|v| v.as_object())
                .map(|p| {
                    p.keys()
                        .map(|pat| regex::Regex::new(pat))
                        .collect::<Result<Vec<_>, _>>()
                });
            let pattern_regexes = match pattern_compile {
                Some(Ok(res)) => res,
                Some(Err(_)) => break 'additional,
                None => Vec::new(),
            };
            for (k, v) in map.iter_mut() {
                if known.contains(k.as_str()) {
                    continue;
                }
                if pattern_regexes.iter().any(|re| re.is_match(k)) {
                    continue;
                }
                coerce_recursive(v, additional, root, depth + 1);
            }
        }
    }

    // Array — `items` is either one schema (applies to every element) or an
    // array of schemas (tuple validation, positional).
    if let serde_json::Value::Array(arr) = value {
        match schema.get("items") {
            Some(item_schema @ serde_json::Value::Object(_)) => {
                for v in arr.iter_mut() {
                    coerce_recursive(v, item_schema, root, depth + 1);
                }
            }
            Some(serde_json::Value::Array(item_schemas)) => {
                for (v, s) in arr.iter_mut().zip(item_schemas.iter()) {
                    coerce_recursive(v, s, root, depth + 1);
                }
            }
            _ => {}
        }
    }
}

fn declares_integer(schema: &serde_json::Value) -> bool {
    match schema.get("type") {
        Some(serde_json::Value::String(s)) => s == "integer",
        Some(serde_json::Value::Array(arr)) => arr.iter().any(|t| t.as_str() == Some("integer")),
        _ => false,
    }
}

/// Iteratively follows `$ref` pointers against `root`. Returns the first schema
/// in the chain that has no `$ref` — or the last schema reached if the chain
/// cycles or hits an unresolvable / external reference.
fn resolve_refs<'a>(
    schema: &'a serde_json::Value,
    root: &'a serde_json::Value,
) -> &'a serde_json::Value {
    let mut visited = HashSet::<String>::new();
    let mut current = schema;
    while let Some(ref_str) = current.get("$ref").and_then(|v| v.as_str()) {
        if !ref_str.starts_with('#') || !visited.insert(ref_str.to_string()) {
            return current;
        }
        match resolve_internal_ref(root, ref_str) {
            Some(resolved) => current = resolved,
            None => return current,
        }
    }
    current
}

/// Resolves a JSON-Pointer-style fragment like `#/$defs/Foo` against `root`.
/// Returns the referenced subschema, or `None` if any segment is missing.
fn resolve_internal_ref<'a>(
    root: &'a serde_json::Value,
    ref_str: &str,
) -> Option<&'a serde_json::Value> {
    let path = ref_str.strip_prefix('#').unwrap_or(ref_str);
    if path.is_empty() {
        return Some(root);
    }
    let mut current = root;
    for raw_segment in path.trim_start_matches('/').split('/') {
        // RFC 6901: decode `~1` -> `/` before `~0` -> `~`, otherwise `~01`
        // (which means literal `~1`) would be incorrectly decoded as `/`.
        let segment = raw_segment.replace("~1", "/").replace("~0", "~");
        current = match current {
            serde_json::Value::Object(map) => map.get(&segment)?,
            serde_json::Value::Array(arr) => arr.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn coerce_integer_in_place(value: &mut serde_json::Value) {
    let serde_json::Value::Number(n) = value else {
        return;
    };
    let Some(f) = n.as_f64() else { return };
    if f.fract() != 0.0 {
        return;
    }
    if let Ok(i) = i64::try_from(f as i128) {
        *value = serde_json::Value::Number(serde_json::Number::from(i));
    }
}

#[cfg(test)]
#[path = "call_mcp_tool_tests.rs"]
mod tests;

/// Handles the result of a call_tool request, converting it to an AIAgentActionResultType.
#[cfg(not(target_family = "wasm"))]
fn handle_call_tool_result(
    res: Result<rmcp::model::CallToolResult, rmcp::ServiceError>,
    server_output_id: Option<crate::ai::blocklist::action_model::execute::ServerOutputId>,
    tool_name: String,
    ctx: &warpui::AppContext,
) -> AIAgentActionResultType {
    let action_result = match res {
        Ok(result) => {
            // Even if the call was successful, the response could still be an error so we need to check.
            if matches!(result.is_error, Some(true)) {
                let error_message = result
                    .structured_content
                    .map(|content| content.to_string())
                    .unwrap_or_else(|| {
                        let content_str = result
                            .content
                            .into_iter()
                            .filter_map(|content| {
                                use rmcp::model::RawContent::*;
                                if let Text(raw_text_content) = content.raw {
                                    Some(raw_text_content.text)
                                } else {
                                    log::warn!("Error content found unsupported content type");
                                    None
                                }
                            })
                            .collect_vec()
                            .join("\n");
                        if content_str.is_empty() {
                            "MCP tool call returned an error.".to_string()
                        } else {
                            content_str
                        }
                    });
                send_telemetry_from_app_ctx!(
                    TelemetryEvent::MCPToolCallAccepted {
                        server_output_id,
                        tool_call: tool_name,
                        error: Some(
                            crate::server::telemetry::MCPServerTelemetryError::ResponseError(
                                error_message.clone()
                            )
                        ),
                    },
                    ctx
                );
                CallMCPToolResult::Error(error_message)
            } else {
                send_telemetry_from_app_ctx!(
                    TelemetryEvent::MCPToolCallAccepted {
                        server_output_id,
                        tool_call: tool_name,
                        error: None,
                    },
                    ctx
                );
                CallMCPToolResult::Success { result }
            }
        }
        Err(e) => {
            let error_message = e.to_string();
            log::warn!("Executing MCP tool resulted in error: {e:?}");
            send_telemetry_from_app_ctx!(
                TelemetryEvent::MCPToolCallAccepted {
                    server_output_id,
                    tool_call: tool_name,
                    error: Some(rmcp::RmcpError::Service(e).into()),
                },
                ctx
            );
            CallMCPToolResult::Error(error_message)
        }
    };
    AIAgentActionResultType::CallMCPTool(action_result)
}
