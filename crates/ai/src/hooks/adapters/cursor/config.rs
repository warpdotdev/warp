//! The parts of Cursor's `hooks.json` this adapter reads. Only `beforeShellExecution` is
//! understood so far; definitions under other event names are skipped and reported.

use std::collections::BTreeMap;
use std::time::Duration;

use regex::Regex;
use serde::Deserialize;

use crate::hooks::adapters::HookConfigError;
use crate::hooks::model::HookBoundary;

/// The one Cursor event this adapter turns into bindings.
pub(super) const BEFORE_SHELL_EXECUTION: &str = "beforeShellExecution";

/// A `hooks.json` document reduced to what this adapter needs.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ParsedCursorFile {
    pub shell_hooks: Vec<ShellHookDefinition>,
    /// Event names in the file that this adapter does not understand yet, in sorted order.
    pub unknown_events: Vec<String>,
}

/// One validated `beforeShellExecution` definition.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ShellHookDefinition {
    pub command: String,
    pub matcher: Option<String>,
    pub timeout: Duration,
    pub fail_closed: bool,
}

#[derive(Debug, Deserialize)]
struct CursorHooksFile {
    version: u64,
    #[serde(default)]
    hooks: BTreeMap<String, serde_json::Value>,
}

/// A definition as written, before validation. Fields Cursor documents for other handler kinds
/// (`prompt`, `model`, `loop_limit`) are accepted and ignored.
#[derive(Debug, Deserialize)]
struct RawDefinition {
    #[serde(default)]
    command: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    timeout: Option<f64>,
    #[serde(default)]
    matcher: Option<String>,
    #[serde(default, rename = "failClosed")]
    fail_closed: bool,
}

/// Parses the raw file bytes into the JSON document and the definitions this adapter reads.
pub(super) fn parse_cursor_file(
    bytes: &[u8],
) -> Result<(serde_json::Value, ParsedCursorFile), HookConfigError> {
    let document: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| HookConfigError::Parse(error.to_string()))?;
    let parsed = parse_cursor_document(&document)?;
    Ok((document, parsed))
}

/// Validates an already-parsed JSON document.
pub(super) fn parse_cursor_document(
    document: &serde_json::Value,
) -> Result<ParsedCursorFile, HookConfigError> {
    let file: CursorHooksFile = serde_json::from_value(document.clone())
        .map_err(|error| HookConfigError::Invalid(error.to_string()))?;
    if file.version == 0 {
        return Err(HookConfigError::Invalid(
            "version must be a positive integer".to_owned(),
        ));
    }

    let mut shell_hooks = Vec::new();
    let mut unknown_events = Vec::new();
    for (event, definitions) in &file.hooks {
        if event != BEFORE_SHELL_EXECUTION {
            unknown_events.push(event.clone());
            continue;
        }
        let Some(definitions) = definitions.as_array() else {
            return Err(HookConfigError::Invalid(format!(
                "{event} must be an array of hook definitions"
            )));
        };
        for (index, definition) in definitions.iter().enumerate() {
            shell_hooks.push(validate_definition(event, index, definition)?);
        }
    }

    Ok(ParsedCursorFile {
        shell_hooks,
        unknown_events,
    })
}

fn validate_definition(
    event: &str,
    index: usize,
    definition: &serde_json::Value,
) -> Result<ShellHookDefinition, HookConfigError> {
    let invalid =
        |message: String| HookConfigError::Invalid(format!("{event}[{index}]: {message}"));
    let raw: RawDefinition =
        serde_json::from_value(definition.clone()).map_err(|error| invalid(error.to_string()))?;

    match raw.kind.as_deref() {
        None | Some("command") => {}
        Some("prompt") => {
            return Err(invalid(
                "prompt hooks are not supported; only command hooks run".to_owned(),
            ));
        }
        Some(other) => return Err(invalid(format!("unknown hook type {other:?}"))),
    }
    let command = raw
        .command
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| invalid("command is required".to_owned()))?;

    let timeout = match raw.timeout {
        None => HookBoundary::ToolPre.default_timeout(),
        // `try_from_secs_f64` rejects negative, NaN, infinite, and overflowing values.
        Some(seconds) => Duration::try_from_secs_f64(seconds)
            .ok()
            .filter(|timeout| !timeout.is_zero())
            .ok_or_else(|| {
                invalid(format!(
                    "timeout must be a positive number of seconds, got {seconds}"
                ))
            })?,
    };

    if let Some(pattern) = raw.matcher.as_deref()
        && !matches_everything(pattern)
        && let Err(error) = Regex::new(pattern)
    {
        return Err(invalid(format!("matcher is not a valid regex: {error}")));
    }

    Ok(ShellHookDefinition {
        command,
        matcher: raw.matcher,
        timeout,
        fail_closed: raw.fail_closed,
    })
}

/// Cursor treats an empty matcher and a bare `*` as "match everything"; neither is a regex.
pub(super) fn matches_everything(pattern: &str) -> bool {
    pattern.is_empty() || pattern == "*"
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
