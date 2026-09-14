use std::collections::BTreeMap;

use prost_types::value::Kind;
use warp_multi_agent_api::oz_hook_result::{self, ResolvedAction};
use warp_multi_agent_api::{OzHookEvent as ProtocolEvent, OzHookResult, RunOzHook};

use super::payload::{HookEventFields, HookPayloadContext, HookPayloadTemplate};
use super::redaction::RedactedValue;
use super::runtime::{
    HookFailureCategory, HookInvocationDiagnostic, HookInvocationResult, OzHookEvent,
    OzPreToolUseDecision,
};
use super::{
    HookEventName, MAX_DENIAL_REASON_BYTES, MAX_PAYLOAD_BYTES, MAX_TOOL_INPUT_BYTES,
    MAX_TOOL_RESPONSE_BYTES, PAYLOAD_SCHEMA_VERSION,
};

pub(crate) fn event_from_protocol(action: &RunOzHook) -> Result<OzHookEvent, ProtocolHookError> {
    if action.invocation_id.is_empty() {
        return Err(ProtocolHookError::InvalidInvocationId);
    }
    if action.schema_version != PAYLOAD_SCHEMA_VERSION {
        return Err(ProtocolHookError::UnsupportedSchema);
    }
    let protocol_event =
        ProtocolEvent::try_from(action.event).map_err(|_| ProtocolHookError::UnknownEvent)?;
    let event_name = match protocol_event {
        ProtocolEvent::SessionStart => HookEventName::SessionStart,
        ProtocolEvent::SessionEnd => HookEventName::SessionEnd,
        ProtocolEvent::UserPromptSubmit => HookEventName::UserPromptSubmit,
        ProtocolEvent::Stop => HookEventName::Stop,
        ProtocolEvent::PreToolUse => HookEventName::PreToolUse,
        ProtocolEvent::PostToolUse => HookEventName::PostToolUse,
        ProtocolEvent::PreCompact => HookEventName::PreCompact,
        ProtocolEvent::Unspecified => return Err(ProtocolHookError::UnknownEvent),
    };
    let payload = action
        .redacted_payload
        .as_ref()
        .ok_or(ProtocolHookError::MissingPayload)?;
    let mut fields = payload
        .fields
        .iter()
        .map(|(key, value)| Ok((key.clone(), value_from_protocol(value)?)))
        .collect::<Result<BTreeMap<_, _>, ProtocolHookError>>()?;
    if serde_json::to_vec(&fields).map_or(true, |bytes| bytes.len() > MAX_PAYLOAD_BYTES) {
        return Err(ProtocolHookError::OversizedPayload);
    }
    validate_envelope_fields(&mut fields, event_name)?;

    let context = HookPayloadContext {
        session_id: take_string(&mut fields, "session_id")?,
        run_id: take_string(&mut fields, "run_id")?,
        conversation_id: take_string(&mut fields, "conversation_id")?,
        cwd: take_string(&mut fields, "cwd")?,
        model: take_string(&mut fields, "model")?,
        permission_mode: take_string(&mut fields, "permission_mode")?,
    };
    let event = match event_name {
        HookEventName::SessionStart => HookEventFields::SessionStart {
            source: take_enum(&mut fields, "source")?,
        },
        HookEventName::SessionEnd => HookEventFields::SessionEnd {
            reason: take_enum(&mut fields, "reason")?,
        },
        HookEventName::UserPromptSubmit => HookEventFields::UserPromptSubmit {
            prompt: take_string(&mut fields, "prompt")?,
            prompt_truncation: take_optional_enum(&mut fields, "prompt_truncation")?,
        },
        HookEventName::Stop => HookEventFields::Stop {
            turn_status: take_enum(&mut fields, "turn_status")?,
        },
        HookEventName::PreToolUse => {
            let tool_input = take_value(&mut fields, "tool_input")?;
            validate_value_size(&tool_input, MAX_TOOL_INPUT_BYTES)?;
            HookEventFields::PreToolUse {
                tool_name: take_string(&mut fields, "tool_name")?,
                tool_use_id: take_string(&mut fields, "tool_use_id")?,
                tool_input,
            }
        }
        HookEventName::PostToolUse => {
            let tool_input = take_value(&mut fields, "tool_input")?;
            let tool_response = take_value(&mut fields, "tool_response")?;
            validate_value_size(&tool_input, MAX_TOOL_INPUT_BYTES)?;
            validate_value_size(&tool_response, MAX_TOOL_RESPONSE_BYTES)?;
            HookEventFields::PostToolUse {
                tool_name: take_string(&mut fields, "tool_name")?,
                tool_use_id: take_string(&mut fields, "tool_use_id")?,
                tool_input,
                tool_response,
            }
        }
        HookEventName::PreCompact => HookEventFields::PreCompact {
            trigger: take_enum(&mut fields, "trigger")?,
        },
    };
    if !fields.is_empty() {
        return Err(ProtocolHookError::UnknownPayloadField);
    }
    if matches!(
        event_name,
        HookEventName::PreToolUse | HookEventName::PostToolUse
    ) && action.tool_use_id.is_empty()
    {
        return Err(ProtocolHookError::InvalidToolUseId);
    }
    if !matches!(
        event_name,
        HookEventName::PreToolUse | HookEventName::PostToolUse
    ) && !action.tool_use_id.is_empty()
    {
        return Err(ProtocolHookError::InvalidToolUseId);
    }
    if let HookEventFields::PreToolUse { tool_use_id, .. }
    | HookEventFields::PostToolUse { tool_use_id, .. } = &event
        && tool_use_id != &action.tool_use_id
    {
        return Err(ProtocolHookError::MismatchedToolUseId);
    }
    Ok(OzHookEvent {
        invocation_id: action.invocation_id.clone(),
        tool_use_id: (!action.tool_use_id.is_empty()).then(|| action.tool_use_id.clone()),
        payload: HookPayloadTemplate { context, event },
    })
}

fn validate_value_size(
    value: &RedactedValue,
    maximum_bytes: usize,
) -> Result<(), ProtocolHookError> {
    if value.serialized_len() > maximum_bytes {
        Err(ProtocolHookError::OversizedPayload)
    } else {
        Ok(())
    }
}

pub(crate) fn result_for_observation(
    action: &RunOzHook,
    diagnostics: &[HookInvocationDiagnostic],
) -> OzHookResult {
    OzHookResult {
        invocation_id: action.invocation_id.clone(),
        tool_use_id: action.tool_use_id.clone(),
        outcome: outcome_for_diagnostics(diagnostics, ResolvedAction::Continue),
    }
}

pub(crate) fn result_for_pre_tool(
    action: &RunOzHook,
    decision: OzPreToolUseDecision,
) -> OzHookResult {
    let outcome = match decision {
        OzPreToolUseDecision::Continue { diagnostics } => {
            outcome_for_diagnostics(&diagnostics, ResolvedAction::Continue)
        }
        OzPreToolUseDecision::Deny {
            reason,
            source,
            diagnostics,
        } => {
            if let Some(diagnostic) = diagnostics.last()
                && diagnostic.result == HookInvocationResult::Denied
                && diagnostic.failure_category.is_some()
            {
                Some(failed_outcome(diagnostic, ResolvedAction::Deny))
            } else {
                Some(oz_hook_result::Outcome::Deny(oz_hook_result::Deny {
                    reason: super::redaction::truncate_utf8(&reason, MAX_DENIAL_REASON_BYTES),
                    source: source.as_str().into(),
                }))
            }
        }
        OzPreToolUseDecision::Cancelled { diagnostics } => {
            let _ = diagnostics;
            Some(oz_hook_result::Outcome::Cancelled(
                oz_hook_result::Cancelled {},
            ))
        }
    };
    OzHookResult {
        invocation_id: action.invocation_id.clone(),
        tool_use_id: action.tool_use_id.clone(),
        outcome,
    }
}

pub(crate) fn failed_result(action: &RunOzHook, error: ProtocolHookError) -> OzHookResult {
    OzHookResult {
        invocation_id: action.invocation_id.clone(),
        tool_use_id: action.tool_use_id.clone(),
        outcome: Some(oz_hook_result::Outcome::Failed(oz_hook_result::Failed {
            category: error.category().into(),
            resolved_action: ResolvedAction::Continue.into(),
        })),
    }
}

fn outcome_for_diagnostics(
    diagnostics: &[HookInvocationDiagnostic],
    resolved_action: ResolvedAction,
) -> Option<oz_hook_result::Outcome> {
    let Some(diagnostic) = diagnostics
        .iter()
        .rev()
        .find(|diagnostic| diagnostic.failure_category.is_some())
    else {
        return Some(oz_hook_result::Outcome::Continue(
            oz_hook_result::Continue {},
        ));
    };
    if diagnostic.result == HookInvocationResult::Cancelled {
        return Some(oz_hook_result::Outcome::Cancelled(
            oz_hook_result::Cancelled {},
        ));
    }
    Some(failed_outcome(diagnostic, resolved_action))
}

fn failed_outcome(
    diagnostic: &HookInvocationDiagnostic,
    resolved_action: ResolvedAction,
) -> oz_hook_result::Outcome {
    oz_hook_result::Outcome::Failed(oz_hook_result::Failed {
        category: diagnostic
            .failure_category
            .map(failure_category_name)
            .unwrap_or("unknown")
            .into(),
        resolved_action: resolved_action.into(),
    })
}

fn failure_category_name(category: HookFailureCategory) -> &'static str {
    match category {
        HookFailureCategory::Spawn => "spawn",
        HookFailureCategory::Stdin => "stdin",
        HookFailureCategory::Timeout => "timeout",
        HookFailureCategory::Cancelled => "cancelled",
        HookFailureCategory::OutputOverflow => "output_overflow",
        HookFailureCategory::OutputRead => "output_read",
        HookFailureCategory::InvalidUtf8 => "invalid_utf8",
        HookFailureCategory::NonZeroExit => "nonzero_exit",
        HookFailureCategory::InvalidDecision => "invalid_decision",
        HookFailureCategory::Payload => "payload",
    }
}

fn validate_envelope_fields(
    fields: &mut BTreeMap<String, RedactedValue>,
    event: HookEventName,
) -> Result<(), ProtocolHookError> {
    if fields.contains_key("hook_source") {
        return Err(ProtocolHookError::UnexpectedHookSource);
    }
    let schema_version = fields
        .remove("schema_version")
        .ok_or(ProtocolHookError::MissingPayloadField("schema_version"))?;
    if schema_version != RedactedValue::String(PAYLOAD_SCHEMA_VERSION.into()) {
        return Err(ProtocolHookError::UnsupportedSchema);
    }
    let hook_event_name = fields
        .remove("hook_event_name")
        .ok_or(ProtocolHookError::MissingPayloadField("hook_event_name"))?;
    if hook_event_name != RedactedValue::String(event.as_str().into()) {
        return Err(ProtocolHookError::MismatchedEvent);
    }
    Ok(())
}

fn take_string(
    fields: &mut BTreeMap<String, RedactedValue>,
    key: &'static str,
) -> Result<String, ProtocolHookError> {
    match fields.remove(key) {
        Some(RedactedValue::String(value)) => Ok(value),
        Some(_) => Err(ProtocolHookError::InvalidPayloadField(key)),
        None => Err(ProtocolHookError::MissingPayloadField(key)),
    }
}

fn take_value(
    fields: &mut BTreeMap<String, RedactedValue>,
    key: &'static str,
) -> Result<RedactedValue, ProtocolHookError> {
    fields
        .remove(key)
        .ok_or(ProtocolHookError::MissingPayloadField(key))
}

fn take_enum<T: serde::de::DeserializeOwned>(
    fields: &mut BTreeMap<String, RedactedValue>,
    key: &'static str,
) -> Result<T, ProtocolHookError> {
    let value = take_value(fields, key)?;
    serde_json::from_value(
        serde_json::to_value(value).map_err(|_| ProtocolHookError::InvalidPayloadField(key))?,
    )
    .map_err(|_| ProtocolHookError::InvalidPayloadField(key))
}

fn take_optional_enum<T: serde::de::DeserializeOwned>(
    fields: &mut BTreeMap<String, RedactedValue>,
    key: &'static str,
) -> Result<Option<T>, ProtocolHookError> {
    let Some(value) = fields.remove(key) else {
        return Ok(None);
    };
    serde_json::from_value(
        serde_json::to_value(value).map_err(|_| ProtocolHookError::InvalidPayloadField(key))?,
    )
    .map(Some)
    .map_err(|_| ProtocolHookError::InvalidPayloadField(key))
}

fn value_from_protocol(value: &prost_types::Value) -> Result<RedactedValue, ProtocolHookError> {
    match value.kind.as_ref() {
        Some(Kind::NullValue(_)) => Ok(RedactedValue::Null),
        Some(Kind::NumberValue(value)) => serde_json::Number::from_f64(*value)
            .map(RedactedValue::Number)
            .ok_or(ProtocolHookError::InvalidNumber),
        Some(Kind::StringValue(value)) => Ok(RedactedValue::String(value.clone())),
        Some(Kind::BoolValue(value)) => Ok(RedactedValue::Bool(*value)),
        Some(Kind::StructValue(value)) => value
            .fields
            .iter()
            .map(|(key, value)| Ok((key.clone(), value_from_protocol(value)?)))
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map(RedactedValue::Object),
        Some(Kind::ListValue(value)) => value
            .values
            .iter()
            .map(value_from_protocol)
            .collect::<Result<Vec<_>, _>>()
            .map(RedactedValue::Array),
        None => Err(ProtocolHookError::InvalidValue),
    }
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub(crate) enum ProtocolHookError {
    #[error("missing invocation ID")]
    InvalidInvocationId,
    #[error("invalid tool-use ID")]
    InvalidToolUseId,
    #[error("tool-use ID does not match payload")]
    MismatchedToolUseId,
    #[error("unsupported payload schema")]
    UnsupportedSchema,
    #[error("unknown hook event")]
    UnknownEvent,
    #[error("payload event does not match action")]
    MismatchedEvent,
    #[error("missing hook payload")]
    MissingPayload,
    #[error("hook payload exceeds the size limit")]
    OversizedPayload,
    #[error("source-neutral payload included hook_source")]
    UnexpectedHookSource,
    #[error("unknown hook payload field")]
    UnknownPayloadField,
    #[error("missing hook payload field {0}")]
    MissingPayloadField(&'static str),
    #[error("invalid hook payload field {0}")]
    InvalidPayloadField(&'static str),
    #[error("invalid protocol value")]
    InvalidValue,
    #[error("invalid protocol number")]
    InvalidNumber,
}

impl ProtocolHookError {
    fn category(self) -> &'static str {
        match self {
            Self::InvalidInvocationId => "invalid_invocation_id",
            Self::InvalidToolUseId => "invalid_tool_use_id",
            Self::MismatchedToolUseId => "mismatched_tool_use_id",
            Self::UnsupportedSchema => "unsupported_schema",
            Self::UnknownEvent => "unknown_event",
            Self::MismatchedEvent => "mismatched_event",
            Self::MissingPayload => "missing_payload",
            Self::OversizedPayload => "oversized_payload",
            Self::UnexpectedHookSource => "unexpected_hook_source",
            Self::UnknownPayloadField => "unknown_payload_field",
            Self::MissingPayloadField(_) => "missing_payload_field",
            Self::InvalidPayloadField(_) => "invalid_payload_field",
            Self::InvalidValue => "invalid_value",
            Self::InvalidNumber => "invalid_number",
        }
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
