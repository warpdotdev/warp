//! Reads Cursor's `hooks.json`: the file shape, every option a definition may carry, and the
//! validation that decides which definitions become bindings. A definition that is valid but
//! cannot run here (a prompt hook, an event Warp never fires) is reported and skipped; a file
//! that cannot be understood is an error.

use std::collections::BTreeMap;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Deserializer};

use super::events::{CursorEvent, CursorEventSpec, DEFAULT_LOOP_LIMIT, LOOP_LIMIT_HOST_CAP};
use super::matcher::matches_everything;
use crate::hooks::adapters::HookConfigError;

/// The one schema version this adapter understands.
const SUPPORTED_VERSION: u64 = 1;

/// A `hooks.json` document reduced to what the adapter needs.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ParsedCursorFile {
    /// Every command-hook definition under a known event, in event table order and then array
    /// order.
    pub definitions: Vec<ParsedDefinition>,
    /// What the file asked for that will not happen, each as a finished message.
    pub warnings: Vec<String>,
}

/// One validated command-hook definition.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ParsedDefinition {
    pub event: CursorEvent,
    /// Position in the event's array, counting definitions that were skipped.
    pub index: usize,
    pub command: String,
    pub matcher: Option<String>,
    pub timeout: Duration,
    pub fail_closed: bool,
    /// The continuation cap after Warp's host cap, for events that may request a follow-up.
    pub loop_limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct CursorHooksFile {
    #[serde(default)]
    version: Option<serde_json::Value>,
    #[serde(default)]
    hooks: BTreeMap<String, serde_json::Value>,
}

/// A definition as written, before validation.
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
    /// `None` when the key is absent; `Some(Null)` when the file says `null`, which Cursor reads
    /// as "no limit".
    #[serde(default, deserialize_with = "present_value")]
    loop_limit: Option<serde_json::Value>,
    /// Every other field, including the prompt-hook fields a command hook does not use.
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

/// Fields Cursor documents for prompt hooks. A command hook ignores them without comment.
const PROMPT_HOOK_FIELDS: &[&str] = &["prompt", "model"];

fn present_value<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Value>, D::Error> {
    serde_json::Value::deserialize(deserializer).map(Some)
}

/// What validating one definition found.
enum ValidatedDefinition {
    Command(ParsedDefinition),
    /// A definition that will not run; the text says which and why.
    Skipped(String),
}

/// Parses the raw file bytes into the JSON document and the definitions the adapter reads.
pub(super) fn parse_cursor_file(
    bytes: &[u8],
) -> Result<(serde_json::Value, ParsedCursorFile), HookConfigError> {
    let document: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| HookConfigError::Parse(error.to_string()))?;
    let parsed = parse_cursor_document(&document)?;
    Ok((document, parsed))
}

/// Reads the definitions out of an already-parsed JSON document.
pub(super) fn parse_cursor_document(
    document: &serde_json::Value,
) -> Result<ParsedCursorFile, HookConfigError> {
    let file: CursorHooksFile = serde_json::from_value(document.clone())
        .map_err(|error| HookConfigError::Invalid(error.to_string()))?;
    check_version(file.version.as_ref())?;

    let mut definitions = Vec::new();
    let mut warnings = Vec::new();
    for event in CursorEvent::ALL {
        let Some(entries) = file.hooks.get(event.name()) else {
            continue;
        };
        let Some(entries) = entries.as_array() else {
            return Err(HookConfigError::Invalid(format!(
                "{} must be an array of hook definitions",
                event.name()
            )));
        };
        for (index, entry) in entries.iter().enumerate() {
            match validate_definition(event.spec(), index, entry, &mut warnings)? {
                ValidatedDefinition::Command(definition) => definitions.push(definition),
                ValidatedDefinition::Skipped(reason) => warnings.push(reason),
            }
        }
    }
    for name in file.hooks.keys() {
        if CursorEvent::from_name(name).is_none() {
            warnings.push(format!(
                "{name} is not a hook event Cursor documents; its definitions were skipped"
            ));
        }
    }

    Ok(ParsedCursorFile {
        definitions,
        warnings,
    })
}

/// Accepts only the schema version this adapter was written for, so a future schema is not
/// read with today's meanings.
fn check_version(version: Option<&serde_json::Value>) -> Result<(), HookConfigError> {
    let Some(version) = version else {
        return Err(HookConfigError::Invalid("version is required".to_owned()));
    };
    match version.as_u64() {
        Some(SUPPORTED_VERSION) => Ok(()),
        Some(0) | None => Err(HookConfigError::Invalid(format!(
            "version must be a positive integer, got {version}"
        ))),
        Some(other) => Err(HookConfigError::Invalid(format!(
            "version {other} is not supported; only version {SUPPORTED_VERSION} is"
        ))),
    }
}

fn validate_definition(
    spec: &CursorEventSpec,
    index: usize,
    definition: &serde_json::Value,
    warnings: &mut Vec<String>,
) -> Result<ValidatedDefinition, HookConfigError> {
    let event = spec.name;
    let invalid =
        |message: String| HookConfigError::Invalid(format!("{event}[{index}]: {message}"));
    let raw: RawDefinition =
        serde_json::from_value(definition.clone()).map_err(|error| invalid(error.to_string()))?;

    match raw.kind.as_deref() {
        None | Some("command") => {}
        // Cursor's own headless surfaces run command hooks only and leave prompt hooks out, so a
        // prompt hook does not stop the command hooks beside it from loading.
        Some("prompt") => {
            return Ok(ValidatedDefinition::Skipped(format!(
                "{event}[{index}]: prompt hooks are not supported yet; it was skipped"
            )));
        }
        Some(other) => return Err(invalid(format!("unknown hook type {other:?}"))),
    }
    let command = raw
        .command
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| invalid("command is required".to_owned()))?;

    let timeout = match raw.timeout {
        None => spec.default_timeout(),
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

    let loop_limit = validate_loop_limit(spec, index, raw.loop_limit.as_ref(), warnings)
        .map_err(invalid)?;

    let matcher = match raw.matcher {
        None => None,
        Some(pattern) if matches_everything(&pattern) => Some(pattern),
        Some(pattern) => {
            if let Err(error) = Regex::new(&pattern) {
                return Ok(ValidatedDefinition::Skipped(format!(
                    "{event}[{index}]: matcher is not a valid regex ({error}); this hook was \
                     skipped"
                )));
            }
            if spec.has_matcher_target() {
                Some(pattern)
            } else {
                warnings.push(format!(
                    "{event}[{index}]: {event} has no matcher target, so the matcher was ignored"
                ));
                None
            }
        }
    };

    for name in raw.other.keys() {
        if !PROMPT_HOOK_FIELDS.contains(&name.as_str()) {
            warnings.push(format!("{event}[{index}]: unknown field {name} was ignored"));
        }
    }
    match spec.boundary {
        None => warnings.push(format!(
            "{event}[{index}]: Warp has no {event} moment; this hook will never run"
        )),
        Some(_) if !spec.fires_in_phase_1 => warnings.push(format!(
            "{event}[{index}]: Warp does not fire {event} yet; this hook will never run"
        )),
        Some(_) => {}
    }

    Ok(ValidatedDefinition::Command(ParsedDefinition {
        event: spec.event,
        index,
        command,
        matcher,
        timeout,
        fail_closed: raw.fail_closed,
        loop_limit,
    }))
}

/// Reads `loop_limit` for events that may request a follow-up and applies the host cap. On other
/// events the field is ignored with a warning.
fn validate_loop_limit(
    spec: &CursorEventSpec,
    index: usize,
    loop_limit: Option<&serde_json::Value>,
    warnings: &mut Vec<String>,
) -> Result<Option<u32>, String> {
    let event = spec.name;
    if !spec.may_continue() {
        if loop_limit.is_some() {
            warnings.push(format!(
                "{event}[{index}]: loop_limit applies only to stop and subagentStop; it was \
                 ignored"
            ));
        }
        return Ok(None);
    }
    let Some(loop_limit) = loop_limit else {
        return Ok(Some(DEFAULT_LOOP_LIMIT));
    };
    if loop_limit.is_null() {
        warnings.push(format!(
            "{event}[{index}]: loop_limit null asks for no limit; Warp caps follow-ups at \
             {LOOP_LIMIT_HOST_CAP}"
        ));
        return Ok(Some(LOOP_LIMIT_HOST_CAP));
    }
    let Some(limit) = loop_limit.as_u64() else {
        return Err(format!(
            "loop_limit must be a non-negative integer or null, got {loop_limit}"
        ));
    };
    if limit > u64::from(LOOP_LIMIT_HOST_CAP) {
        warnings.push(format!(
            "{event}[{index}]: loop_limit {limit} is above Warp's cap; {LOOP_LIMIT_HOST_CAP} \
             is used"
        ));
        return Ok(Some(LOOP_LIMIT_HOST_CAP));
    }
    Ok(Some(limit as u32))
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
