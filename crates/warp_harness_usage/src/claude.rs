use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;
use serde_json::Value;

use crate::counters::{Accounting, Counters};
use crate::tools::Tools;
use crate::{
    Attribution, CaptureDiagnostics, Coverage, ExtractionOutcome, Findings, MAX_IDENTITIES,
    MAX_SCOPE_ENTRIES, NativePayload, ReasonCode, UsagePayload, UsageSnapshot, identifier,
};

const PATHS: [&str; 6] = [
    "/input_tokens",
    "/output_tokens",
    "/cache_read_input_tokens",
    "/cache_creation_input_tokens",
    "/cache_creation/ephemeral_5m_input_tokens",
    "/cache_creation/ephemeral_1h_input_tokens",
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ClaudeUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<CacheCreation>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CacheCreation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_5m_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_1h_input_tokens: Option<i64>,
}

impl From<Counters<6>> for ClaudeUsage {
    fn from(counts: Counters<6>) -> Self {
        let [
            input_tokens,
            output_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
            ephemeral_5m_input_tokens,
            ephemeral_1h_input_tokens,
        ] = counts.values;
        Self {
            input_tokens,
            output_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
            cache_creation: (ephemeral_5m_input_tokens.is_some()
                || ephemeral_1h_input_tokens.is_some())
            .then_some(CacheCreation {
                ephemeral_5m_input_tokens,
                ephemeral_1h_input_tokens,
            }),
        }
    }
}

#[derive(Default)]
struct Response {
    usage: Option<Counters<6>>,
    attribution: Attribution,
    conflicted: bool,
}

/// Extract the root and captured subagent histories; TODO files are not usage input.
pub fn extract_claude<'a>(
    session_id: &str,
    root: &[Value],
    subagents: impl IntoIterator<Item = (&'a str, &'a [Value])>,
    diagnostics: &CaptureDiagnostics,
) -> ExtractionOutcome {
    let mut findings = Findings::default();
    findings.capture(diagnostics);
    if diagnostics.subagents.len() > MAX_SCOPE_ENTRIES {
        findings.limit(ReasonCode::ResourceLimit);
    }
    if !identifier(session_id, &mut findings) {
        return ExtractionOutcome::Unavailable(findings.reasons);
    }
    let mut sources = BTreeMap::new();
    for (scope, entries) in subagents {
        if !identifier(scope, &mut findings) || sources.len() >= MAX_SCOPE_ENTRIES {
            findings.limit(ReasonCode::ResourceLimit);
            return ExtractionOutcome::Unavailable(findings.reasons);
        }
        if sources.insert(scope, entries).is_some() {
            findings.limit(ReasonCode::InvalidData);
        }
        if !diagnostics.subagents.contains_key(scope) {
            findings.tokens_partial = true;
            findings.tools_partial = true;
            findings.reason(ReasonCode::IncompleteInput);
        }
    }
    let subagent_scope = sources.keys().map(|scope| (*scope).to_owned()).collect();
    let mut sessions = BTreeSet::from([session_id.to_owned()]);
    let mut responses = HashMap::new();
    let mut tools = Tools::default();
    for entries in std::iter::once(root).chain(sources.into_values()) {
        for entry in entries {
            if !entry.is_object() {
                findings.token(ReasonCode::InvalidData);
                findings.tools_partial = true;
                continue;
            }
            let record_type = entry.get("type").and_then(Value::as_str);
            if record_type != Some("assistant") {
                if !matches!(
                    record_type,
                    Some(
                        "user"
                            | "system"
                            | "progress"
                            | "summary"
                            | "file-history-snapshot"
                            | "queue-operation"
                            | "last-prompt"
                            | "mode"
                            | "permission-mode"
                            | "atis-latch"
                            | "attachment"
                            | "ai-title"
                            | "cost-state"
                    )
                ) {
                    findings.token(ReasonCode::InvalidData);
                    findings.tools_partial = true;
                }
                continue;
            }
            let Some(message) = entry.get("message").filter(|message| message.is_object()) else {
                findings.token(ReasonCode::InvalidData);
                findings.tools_partial = true;
                continue;
            };
            if entry.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true)
                || entry.get("isSynthetic").and_then(Value::as_bool) == Some(true)
                || message.get("model").and_then(Value::as_str) == Some("<synthetic>")
            {
                continue;
            }
            let session = entry
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or(session_id);
            if !identifier(session, &mut findings) {
                continue;
            }
            sessions.insert(session.to_owned());
            if sessions.len() > MAX_SCOPE_ENTRIES {
                findings.limit(ReasonCode::ResourceLimit);
                break;
            }
            observe_tools(message, session, &mut tools, &mut findings);
            let Some(id) = message
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            else {
                findings.token(ReasonCode::InvalidData);
                continue;
            };
            if !identifier(id, &mut findings) {
                continue;
            }
            let key = (session.to_owned(), id.to_owned());
            if !responses.contains_key(&key) && responses.len() >= MAX_IDENTITIES {
                findings.limit(ReasonCode::ResourceLimit);
                break;
            }
            let attribution = read_attribution(message, &mut findings);
            let usage = message
                .get("usage")
                .filter(|usage| !usage.is_null())
                .and_then(|usage| parse_usage(usage, &mut findings));
            let response = responses.entry(key).or_insert_with(Response::default);
            if let Some(usage) = usage {
                if response.conflicted {
                    continue;
                }
                if let Some(previous) = &response.usage {
                    if !compatible_attribution(&response.attribution, &attribution) {
                        response.conflicted = true;
                        response.usage = None;
                        findings.token(ReasonCode::AmbiguousAccounting);
                        continue;
                    }
                    if !usage.covers(previous) {
                        // Incomplete streaming observations cannot replace a complete vector.
                        if previous.covers(&usage)
                            && previous
                                .values
                                .iter()
                                .zip(&usage.values)
                                .any(|(old, new)| old.is_some() && new.is_none())
                            && usage.values != previous.values
                        {
                            continue;
                        }
                        response.conflicted = true;
                        response.usage = None;
                        findings.token(ReasonCode::AmbiguousAccounting);
                        continue;
                    }
                }
                response.usage = Some(usage);
                response.attribution = Attribution {
                    model: attribution.model.or(response.attribution.model.take()),
                    service_tier: attribution
                        .service_tier
                        .or(response.attribution.service_tier.take()),
                    inference_geo: attribution
                        .inference_geo
                        .or(response.attribution.inference_geo.take()),
                    speed: attribution.speed.or(response.attribution.speed.take()),
                };
            }
        }
        if findings.limit_exceeded {
            break;
        }
    }
    let mut accounting = Accounting::default();
    let mut observed_fields = None;
    let mut missing_category = false;
    for response in responses.into_values() {
        if let Some(usage) = response.usage {
            let fields = usage.values.map(|value| value.is_some());
            missing_category |= observed_fields.is_some_and(|previous| previous != fields);
            observed_fields = Some(fields);
            accounting.total.add(&usage, &mut findings);
            accounting.attribute(&usage, &response.attribution, &mut findings);
        } else if !response.conflicted {
            findings.token(ReasonCode::IncompleteInput);
        }
    }
    if missing_category {
        findings.token(ReasonCode::IncompleteInput);
    }
    let readable = diagnostics.root.is_complete()
        || diagnostics
            .subagents
            .values()
            .any(|file| file.is_complete());
    let tool_calls = tools.finish(readable, &mut findings);
    let usage = accounting
        .total
        .any()
        .then(|| accounting.total.clone().into());
    let attribution = accounting.groups();
    if findings.limit_exceeded || (usage.is_none() && tool_calls.is_none()) {
        return ExtractionOutcome::Unavailable(findings.reasons);
    }
    ExtractionOutcome::Usable(Box::new(UsageSnapshot {
        coverage: Coverage {
            token_status: Findings::status(usage.is_some(), findings.tokens_partial),
            tool_status: Findings::status(tool_calls.is_some(), findings.tools_partial),
            captured_scope: "root_and_captured_subagents",
            reason_codes: findings.reasons,
        },
        payload: NativePayload::Claude(UsagePayload {
            usage,
            attribution,
            tool_calls,
        }),
        session_ids: sessions.into_iter().collect(),
        root_scope: session_id.to_owned(),
        subagent_scope,
    }))
}

fn parse_usage(value: &Value, findings: &mut Findings) -> Option<Counters<6>> {
    if value
        .get("cache_creation")
        .is_some_and(|partitions| !partitions.is_object())
    {
        findings.token(ReasonCode::InvalidData);
        return None;
    }
    let usage = Counters::parse(value, PATHS, findings)?;
    let [_, _, _, aggregate, short, long] = usage.values;
    if let (Some(aggregate), Some(short), Some(long)) = (aggregate, short, long)
        && short.checked_add(long) != Some(aggregate)
    {
        findings.token(ReasonCode::InvalidData);
        return None;
    }
    Some(usage)
}

fn observe_tools(message: &Value, session: &str, tools: &mut Tools, findings: &mut Findings) {
    let Some(content) = message.get("content").and_then(Value::as_array) else {
        findings.tool(ReasonCode::InvalidData);
        return;
    };
    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("tool_use") => tools.observe(
                session,
                block.get("id").and_then(Value::as_str),
                block.get("name").and_then(Value::as_str),
                findings,
            ),
            Some("text" | "thinking" | "redacted_thinking" | "tool_result") => {}
            _ => findings.tool(ReasonCode::InvalidData),
        }
    }
}

fn read_attribution(message: &Value, findings: &mut Findings) -> Attribution {
    let usage = &message["usage"];
    Attribution {
        model: classification(message.get("model"), findings),
        service_tier: classification(usage.get("service_tier"), findings),
        inference_geo: classification(usage.get("inference_geo"), findings),
        speed: classification(usage.get("speed"), findings),
    }
}

pub(crate) fn classification(value: Option<&Value>, findings: &mut Findings) -> Option<String> {
    let value = value.filter(|value| !value.is_null())?;
    let Some(value) = value.as_str() else {
        findings.token(ReasonCode::InvalidData);
        return None;
    };
    if value.is_empty() {
        return None;
    }
    identifier(value, findings).then(|| value.to_owned())
}

fn compatible_attribution(previous: &Attribution, current: &Attribution) -> bool {
    [
        (&previous.model, &current.model),
        (&previous.service_tier, &current.service_tier),
        (&previous.inference_geo, &current.inference_geo),
        (&previous.speed, &current.speed),
    ]
    .into_iter()
    .all(|(previous, current)| match (previous, current) {
        (Some(previous), Some(current)) => previous == current,
        _ => true,
    })
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
