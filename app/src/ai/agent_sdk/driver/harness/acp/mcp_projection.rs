//! Recognizes the MCP tool calls among an ACP agent's `tool_call` notifications.
//!
//! ACP has no marker for "this call went to an MCP server"; every adapter encodes the server and
//! tool in its own way (`mcp.<server>.<tool>`, `mcp__<server>__<tool>`, `<server>_<tool>`, a
//! proxy tool taking `{tool, args}`). Rather than branching per harness, a call counts as an MCP
//! call when one of those shapes names a server the driver itself configured for the run, so an
//! ordinary tool whose name merely looks like one never matches and the shapes stay one table.
use std::collections::HashMap;

use serde_json::Value;
use uuid::Uuid;
use warp_multi_agent_api::McpServerIdentity;

use super::super::ResolvedMCPServers;
use super::protocol::{ToolCallFields, ToolKind};
use crate::ai::agent::api::mcp_server_identity;

/// An MCP server the driver configured for the run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct KnownMcpServer {
    /// The name the server was configured under, which is also what the harness addresses it by.
    pub name: String,
    /// The installation id the native conversation records as `CallMCPTool.server_id`.
    pub installation_id: Uuid,
    /// The managed MCP server uid or well-known integration id the server was resolved from.
    pub warp_id: Option<String>,
}

impl KnownMcpServer {
    /// The identity warp-server would stamp on a call to this server. No server request sees
    /// these calls, so the display name warp-server would otherwise fill in is set here.
    pub fn identity(&self) -> McpServerIdentity {
        McpServerIdentity {
            display_name: self.name.clone(),
            ..self
                .warp_id
                .as_deref()
                .and_then(mcp_server_identity)
                .unwrap_or_default()
        }
    }
}

/// An ACP tool call resolved to the MCP server and tool it invoked.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct McpCall<'a> {
    pub server: &'a KnownMcpServer,
    pub tool: String,
    /// The arguments passed to the tool, as the agent reported them.
    pub args: Option<Value>,
}

/// The MCP servers configured for a run, indexed by every spelling a harness may use for them.
#[derive(Clone, Debug, Default)]
pub(super) struct KnownMcpServers {
    servers: Vec<KnownMcpServer>,
    /// Canonical spellings mapped to an index into `servers`. A spelling two servers share is
    /// left out so no call is attributed to the wrong one.
    by_spelling: HashMap<String, usize>,
}

impl KnownMcpServers {
    pub fn new(mut servers: Vec<KnownMcpServer>) -> Self {
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        let mut by_spelling: HashMap<String, Option<usize>> = HashMap::new();
        for (index, server) in servers.iter().enumerate() {
            for spelling in spellings(&server.name) {
                by_spelling
                    .entry(spelling)
                    .and_modify(|owner| {
                        if *owner != Some(index) {
                            *owner = None;
                        }
                    })
                    .or_insert(Some(index));
            }
        }
        Self {
            servers,
            by_spelling: by_spelling
                .into_iter()
                .filter_map(|(spelling, owner)| Some((spelling, owner?)))
                .collect(),
        }
    }

    pub fn from_resolved(resolved: &ResolvedMCPServers) -> Self {
        Self::new(
            resolved
                .configs
                .keys()
                .map(|name| KnownMcpServer {
                    name: name.clone(),
                    installation_id: resolved
                        .installation_ids
                        .get(name)
                        .copied()
                        .unwrap_or_default(),
                    warp_id: resolved.warp_ids.get(name).cloned(),
                })
                .collect(),
        )
    }

    /// The MCP call `call` describes, or `None` when it does not name a configured server.
    pub fn recognize(&self, call: &ToolCallFields) -> Option<McpCall<'_>> {
        if self.servers.is_empty() {
            return None;
        }
        let input = call.raw_input.as_ref();

        // `{server, tool, arguments}`: the MCP invocation itself, passed through as the input.
        if let Some(input) = input
            && let (Some(server), Some(tool)) =
                (string_field(input, "server"), string_field(input, "tool"))
            && let Some(server) = self.resolve(server)
        {
            return Some(McpCall {
                server,
                tool: tool.to_owned(),
                args: input.get("arguments").cloned(),
            });
        }

        let claude_tool_name = call
            .meta
            .as_ref()
            .and_then(|meta| meta.pointer("/claudeCode/toolName"))
            .and_then(Value::as_str);
        [
            call.name.as_deref(),
            claude_tool_name,
            call.title.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find_map(|label| self.call_from_label(label, call.kind, input))
    }

    fn call_from_label(
        &self,
        label: &str,
        kind: Option<ToolKind>,
        input: Option<&Value>,
    ) -> Option<McpCall<'_>> {
        if let Some(rest) = label.strip_prefix("mcp__") {
            // `mcp__<server>__<tool>` with the tool's own arguments as the input. A server name
            // may itself contain `__`, so the longest server that resolves wins.
            if let Some(call) = self.split_qualified(rest, "__", input) {
                return Some(call);
            }
            // `mcp__<server>`: a per-server proxy taking `{tool, args}`.
            return self.proxy_call(Some(self.resolve(rest)?), input);
        }
        if let Some(rest) = label.strip_prefix("mcp.") {
            return self.split_qualified(rest, ".", input);
        }
        if label == "mcp" {
            // A single proxy over every server, taking `{tool: "<server>_<tool>", args}`.
            return self.proxy_call(None, input);
        }
        // `<server>_<tool>`, the shape of a tool registered directly under its server's prefix.
        // Shell commands are never reported this way, and a command whose text happens to start
        // with a server's name must stay a command.
        if kind == Some(ToolKind::Execute) {
            return None;
        }
        let (server, tool) = self.split_prefixed(label)?;
        Some(McpCall {
            server,
            tool,
            args: input.cloned(),
        })
    }

    /// Splits `<server><separator><tool>` on the rightmost separator whose left side names a
    /// configured server, so a server name that itself contains the separator still resolves.
    fn split_qualified(
        &self,
        qualified: &str,
        separator: &str,
        input: Option<&Value>,
    ) -> Option<McpCall<'_>> {
        qualified.rmatch_indices(separator).find_map(|(index, _)| {
            let (server, tool) = (&qualified[..index], &qualified[index + separator.len()..]);
            let server = self.resolve(server)?;
            (!tool.is_empty()).then(|| McpCall {
                server,
                tool: tool.to_owned(),
                args: input.cloned(),
            })
        })
    }

    /// A call through a proxy tool whose input names the target as `{tool, args}`. With a
    /// `server`, `tool` may still carry that server's prefix; without one, the prefix is what
    /// identifies the server.
    fn proxy_call<'s>(
        &'s self,
        server: Option<&'s KnownMcpServer>,
        input: Option<&Value>,
    ) -> Option<McpCall<'s>> {
        let input = input?;
        let tool = string_field(input, "tool")?;
        let args = input.get("args").cloned();
        let (server, tool) = match server {
            Some(server) => {
                let tool = match self.split_prefixed(tool) {
                    Some((prefixed, unprefixed)) if prefixed.name == server.name => unprefixed,
                    Some(_) | None => tool.to_owned(),
                };
                (server, tool)
            }
            None => self.split_prefixed(tool)?,
        };
        Some(McpCall { server, tool, args })
    }

    fn resolve(&self, name: &str) -> Option<&KnownMcpServer> {
        self.by_spelling
            .get(&canonical(name))
            .map(|&index| &self.servers[index])
    }

    /// Splits `<server>_<tool>` on the longest configured server prefix, returning the tool as
    /// the agent spelled it.
    fn split_prefixed(&self, name: &str) -> Option<(&KnownMcpServer, String)> {
        let canonical_name = canonical(name);
        let (spelling, &index) = self
            .by_spelling
            .iter()
            .filter(|(spelling, _)| {
                canonical_name.len() > spelling.len() + 1
                    && canonical_name.starts_with(spelling.as_str())
                    && canonical_name.as_bytes()[spelling.len()] == b'_'
            })
            .max_by_key(|(spelling, _)| spelling.len())?;
        // `canonical` maps one char to one char, so the prefix spans the same number of chars
        // in the original name.
        let tool_start = name
            .char_indices()
            .nth(spelling.chars().count() + 1)
            .map(|(offset, _)| offset)?;
        Some((&self.servers[index], name[tool_start..].to_owned()))
    }
}

/// The text a finished MCP call produced, or the error it reported, when the adapter passed
/// the call's result through as `rawOutput`. `None` when there is nothing to read.
///
/// Accepts the MCP `CallToolResult` itself, an `{result, error}` envelope around it, a list of
/// content blocks, or plain text.
pub(super) fn outcome(raw_output: Option<&Value>) -> Option<Result<String, String>> {
    let raw = raw_output?;
    let Value::Object(fields) = raw else {
        return result_text(raw).map(Ok);
    };
    if !fields.contains_key("result") && !fields.contains_key("error") {
        return result_text(raw).map(Ok);
    }
    if let Some(error) = fields.get("error").filter(|error| !error.is_null()) {
        let message = string_field(error, "message")
            .map(str::to_owned)
            .unwrap_or_else(|| error.to_string());
        return Some(Err(message));
    }
    fields.get("result").and_then(result_text).map(Ok)
}

fn result_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => Some(text_blocks(blocks).unwrap_or_else(|| pretty(value))),
        Value::Object(fields) => Some(match fields.get("content") {
            Some(Value::Array(blocks)) => text_blocks(blocks).unwrap_or_else(|| pretty(value)),
            Some(_) | None => pretty(value),
        }),
        Value::Bool(_) | Value::Number(_) => Some(value.to_string()),
    }
}

fn text_blocks(blocks: &[Value]) -> Option<String> {
    let texts: Vec<&str> = blocks
        .iter()
        .filter_map(|block| string_field(block, "text"))
        .collect();
    (!texts.is_empty()).then(|| texts.join("\n"))
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str().filter(|field| !field.is_empty())
}

/// The spellings a harness may use for a server named `name`: the name itself, and the
/// forms adapters derive from it when they sanitize names for their tool namespaces.
fn spellings(name: &str) -> Vec<String> {
    let mut spellings = vec![canonical(name), canonical(&pi_sanitized(name))];
    // Pi's short prefix mode also drops a trailing `mcp` / `-mcp`.
    let lower = name.to_ascii_lowercase();
    if let Some(short) = lower
        .strip_suffix("-mcp")
        .or_else(|| lower.strip_suffix("mcp"))
        .filter(|short| !short.is_empty())
    {
        spellings.push(canonical(short));
        spellings.push(canonical(&pi_sanitized(short)));
    }
    // A separator left dangling by a dropped suffix (`GitHub MCP` -> `github_`) is never part
    // of the prefix adapters prepend to a tool name.
    let trimmed: Vec<String> = spellings
        .iter()
        .map(|spelling| spelling.trim_end_matches('_').to_owned())
        .collect();
    spellings.extend(trimmed);
    spellings.retain(|spelling| !spelling.is_empty());
    spellings.sort();
    spellings.dedup();
    spellings
}

/// Lowercase with every character outside `[a-z0-9]` replaced by `_`, one char per char, so
/// `GitHub MCP`, `github_mcp`, `GitHub-MCP`, and `github.mcp` all read the same.
fn canonical(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// `pi-mcp-adapter`'s server prefix: characters outside `[A-Za-z0-9_-]` become `_<hex>_`.
fn pi_sanitized(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_string()
            } else {
                format!("_{:x}_", c as u32)
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "mcp_projection_tests.rs"]
mod tests;
