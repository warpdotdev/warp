use std::collections::BTreeSet;
use std::mem;

use serde::Serialize;
use serde_json::Value;

use crate::claude::classification;
use crate::counters::{Accounting, Counters};
use crate::tools::Tools;
use crate::{
    Attribution, CaptureDiagnostics, Coverage, ExtractionOutcome, Findings, MAX_SCOPE_ENTRIES,
    NativePayload, ReasonCode, UsagePayload, UsageSnapshot, identifier,
};

const PATHS: [&str; 5] = [
    "/input_tokens",
    "/cached_input_tokens",
    "/output_tokens",
    "/reasoning_output_tokens",
    "/total_tokens",
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CodexUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<i64>,
}

impl From<Counters<5>> for CodexUsage {
    fn from(counts: Counters<5>) -> Self {
        let [
            input_tokens,
            cached_input_tokens,
            output_tokens,
            reasoning_output_tokens,
            total_tokens,
        ] = counts.values;
        Self {
            input_tokens,
            cached_input_tokens,
            output_tokens,
            reasoning_output_tokens,
            total_tokens,
        }
    }
}

#[derive(Default)]
struct Segment {
    latest: Option<Counters<5>>,
    ambiguous: bool,
    accounting: Accounting<5>,
}

impl Segment {
    fn finish(mut self, accounting: &mut Accounting<5>, findings: &mut Findings) {
        if let Some(total) = self.latest {
            self.accounting.total = total;
            accounting.merge(self.accounting, findings);
        }
    }
}

/// Extract only the captured root rollout, not native child rollouts.
pub fn extract_codex(
    session_id: &str,
    entries: &[Value],
    diagnostics: &CaptureDiagnostics,
) -> ExtractionOutcome {
    let mut findings = Findings::default();
    findings.capture(diagnostics);
    if !identifier(session_id, &mut findings) {
        return ExtractionOutcome::Unavailable(findings.reasons);
    }
    let mut session = session_id.to_owned();
    let mut sessions = BTreeSet::from([session.clone()]);
    let mut seen_metadata = false;
    let mut segment = Segment::default();
    let mut accounting = Accounting::default();
    let mut attribution = Attribution::default();
    let mut tools = Tools::default();
    for entry in entries {
        let payload = &entry["payload"];
        match entry.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                let Some(id) = payload.get("id").and_then(Value::as_str) else {
                    findings.token(ReasonCode::InvalidData);
                    findings.tools_partial = true;
                    continue;
                };
                if !identifier(id, &mut findings) {
                    break;
                }
                if id != session {
                    if !seen_metadata || sessions.contains(id) {
                        findings.token(ReasonCode::AmbiguousAccounting);
                        findings.tools_partial = true;
                        segment.ambiguous = true;
                        continue;
                    }
                    if sessions.len() >= MAX_SCOPE_ENTRIES {
                        findings.limit(ReasonCode::ResourceLimit);
                        break;
                    }
                    mem::take(&mut segment).finish(&mut accounting, &mut findings);
                    attribution = Attribution::default();
                    session = id.to_owned();
                    sessions.insert(session.clone());
                }
                seen_metadata = true;
            }
            Some("turn_context") => {
                attribution = Attribution {
                    model: classification(payload.get("model"), &mut findings),
                    service_tier: classification(payload.get("service_tier"), &mut findings),
                    ..Default::default()
                };
            }
            Some("event_msg") => match payload.get("type").and_then(Value::as_str) {
                Some("token_count") => observe_checkpoint(
                    &payload["info"],
                    &attribution,
                    &mut segment,
                    &mut findings,
                ),
                None => {
                    findings.token(ReasonCode::InvalidData);
                    findings.tools_partial = true;
                }
                Some(name) if name.contains("token") || name.contains("usage") => {
                    findings.token(ReasonCode::InvalidData);
                }
                Some(_) => {}
            },
            Some("response_item") => match payload.get("type").and_then(Value::as_str) {
                Some("function_call" | "custom_tool_call") => tools.observe(
                    &session,
                    payload.get("call_id").and_then(Value::as_str),
                    payload.get("name").and_then(Value::as_str),
                    &mut findings,
                ),
                Some(
                    "message" | "reasoning" | "function_call_output" | "custom_tool_call_output",
                ) => {}
                _ => findings.tool(ReasonCode::InvalidData),
            },
            Some("compacted") => {}
            _ => {
                findings.token(ReasonCode::InvalidData);
                findings.tools_partial = true;
            }
        }
        if findings.limit_exceeded {
            break;
        }
    }
    segment.finish(&mut accounting, &mut findings);
    let tool_calls = tools.finish(diagnostics.root.is_complete(), &mut findings);
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
            captured_scope: "root_rollout_only",
            reason_codes: findings.reasons,
        },
        payload: NativePayload::Codex(UsagePayload {
            usage,
            attribution,
            tool_calls,
        }),
        session_ids: sessions.into_iter().collect(),
        root_scope: session_id.to_owned(),
        subagent_scope: Vec::new(),
    }))
}

fn observe_checkpoint(
    info: &Value,
    attribution: &Attribution,
    segment: &mut Segment,
    findings: &mut Findings,
) {
    if segment.ambiguous {
        return;
    }
    if info.is_null() {
        return;
    }
    let Some(total) = info.get("total_token_usage") else {
        findings.token(ReasonCode::IncompleteInput);
        return;
    };
    let Some(total) = parse_usage(total, findings) else {
        return;
    };
    let last = info
        .get("last_token_usage")
        .filter(|usage| !usage.is_null())
        .and_then(|usage| parse_usage(usage, findings));
    if segment.latest.as_ref() == Some(&total) {
        return;
    }
    if let Some(previous) = &segment.latest {
        if total.decreased(previous) {
            // A decrease is not proof of a new lifetime; retain only the unambiguous prefix.
            segment.ambiguous = true;
            findings.token(ReasonCode::AmbiguousAccounting);
            return;
        }
        if !total.same_fields(previous) {
            segment.accounting.omit_missing_fields(&total);
            let baseline = total.new_fields(previous);
            if baseline.any() {
                segment
                    .accounting
                    .attribute(&baseline, &Attribution::default(), findings);
            }
            findings.token(ReasonCode::AmbiguousAccounting);
        }
        let delta = total.delta(previous);
        if delta.any() {
            if last.as_ref().is_some_and(|last| last.matches_observed(&delta)) {
                segment.accounting.attribute(&delta, attribution, findings);
            } else {
                segment
                    .accounting
                    .attribute(&delta, &Attribution::default(), findings);
                findings.token(ReasonCode::AmbiguousAccounting);
            }
        }
    } else if last.as_ref() == Some(&total) {
        segment.accounting.attribute(&total, attribution, findings);
    } else {
        // Turn context does not establish the model of history preceding the first checkpoint.
        segment
            .accounting
            .attribute(&total, &Attribution::default(), findings);
    }
    segment.latest = Some(total);
}

fn parse_usage(value: &Value, findings: &mut Findings) -> Option<Counters<5>> {
    let usage = Counters::parse(value, PATHS, findings)?;
    let [input, cached, output, reasoning, total] = usage.values;
    let invalid = matches!((input, cached), (Some(input), Some(cached)) if cached > input)
        || matches!((output, reasoning), (Some(output), Some(reasoning)) if reasoning > output)
        || matches!((input, output, total), (Some(input), Some(output), Some(total)) if input.checked_add(output) != Some(total));
    if invalid {
        findings.token(ReasonCode::InvalidData);
        return None;
    }
    Some(usage)
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
