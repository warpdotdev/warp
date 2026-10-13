use std::collections::BTreeMap;

use serde_json::Value;

use crate::api::{Attribution, CodexUsage, HarnessUsageSnapshot, ThresholdPolicy};
use crate::claude::classification;
use crate::counters::{Accounting, Counters, Provider};
use crate::tools::Tools;
use crate::{
    CaptureDiagnostics, ExtractedUsage, ExtractionOutcome, Findings, MAX_IDENTITIES, ReasonCode,
    identifier,
};

const PATHS: [&str; 6] = [
    "/input_tokens",
    "/cached_input_tokens",
    "/output_tokens",
    "/reasoning_output_tokens",
    "/total_tokens",
    "/cache_write_input_tokens",
];

impl From<Counters> for CodexUsage {
    fn from(counts: Counters) -> Self {
        let [
            input_tokens,
            cached_input_tokens,
            output_tokens,
            reasoning_output_tokens,
            total_tokens,
            cache_write_input_tokens,
        ] = counts.values;
        Self {
            input_tokens,
            cached_input_tokens,
            cache_write_input_tokens,
            output_tokens,
            reasoning_output_tokens,
            total_tokens,
        }
    }
}

struct Response {
    record: Value,
    usage: Option<Counters>,
    attribution: Attribution,
}

/// Extract root rollout usage from native response records, reconciling their cumulative totals.
pub fn extract_codex(
    session_id: &str,
    entries: &[Value],
    diagnostics: &CaptureDiagnostics,
    policy: Option<&ThresholdPolicy>,
) -> ExtractionOutcome {
    let mut findings = Findings::default();
    findings.capture(diagnostics);
    if !identifier(session_id, &mut findings) {
        return ExtractionOutcome::Unavailable(findings.diagnostics());
    }
    let mut contexts = BTreeMap::<String, Attribution>::new();
    let mut tier = None;
    let mut responses = BTreeMap::<String, Response>::new();
    let mut thread_sum = Counters {
        values: [Some(0); 6],
    };
    let mut turn_sums = BTreeMap::<(String, String, String), Counters>::new();
    let mut tools = Tools::default();
    let mut legacy_usage = false;
    for entry in entries {
        let payload = &entry["payload"];
        let record = match entry.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                if payload.get("id").and_then(Value::as_str) != Some(session_id) {
                    findings.token(ReasonCode::AmbiguousAccounting);
                }
                None
            }
            Some("turn_context") => {
                if let Some(turn) = payload.get("turn_id").and_then(Value::as_str)
                    && identifier(turn, &mut findings)
                {
                    let attribution = Attribution {
                        model: classification(payload.get("model"), &mut findings),
                        service_tier: tier.clone(),
                        ..Default::default()
                    };
                    if contexts
                        .get(turn)
                        .is_some_and(|previous| previous != &attribution)
                    {
                        findings.token(ReasonCode::AmbiguousAccounting);
                    } else if contexts.len() < MAX_IDENTITIES {
                        contexts.insert(turn.to_owned(), attribution);
                    } else {
                        findings.token(ReasonCode::ResourceLimit);
                    }
                }
                None
            }
            Some("event_msg") => match payload.get("type").and_then(Value::as_str) {
                Some("token_usage_record") => Some(payload),
                Some("thread_settings_applied") => {
                    if payload
                        .get("thread_id")
                        .is_some_and(|owner| owner.as_str() != Some(session_id))
                    {
                        continue;
                    }
                    tier = classification(
                        payload.pointer("/thread_settings/service_tier"),
                        &mut findings,
                    );
                    None
                }
                Some("token_count") => {
                    legacy_usage |= payload.get("info").is_some_and(|info| !info.is_null());
                    if let Some(total) = payload.pointer("/info/total_token_usage") {
                        if let Some(total) = Counters::parse(total, PATHS, &mut findings) {
                            // Codex's full-context marker resets categories without incurring provider usage.
                            let synthetic_context = total.values[4].is_some_and(|count| {
                                payload
                                    .pointer("/info/model_context_window")
                                    .and_then(Value::as_i64)
                                    == Some(count)
                            }) && [0, 1, 2, 3, 5]
                                .into_iter()
                                .all(|index| total.values[index] == Some(0));
                            if !synthetic_context
                                && [0, 1, 2, 3, 5].into_iter().any(|index| {
                                    total.values[index].is_some_and(|count| {
                                        Some(count) != thread_sum.values[index]
                                    })
                                })
                            {
                                findings.token(ReasonCode::AmbiguousAccounting);
                            }
                        } else {
                            findings.token(ReasonCode::IncompleteInput);
                        }
                    }
                    None
                }
                Some(name) if name.contains("usage") || name.contains("token") => {
                    findings.token(ReasonCode::InvalidData);
                    None
                }
                Some(_) => None,
                None => {
                    findings.token(ReasonCode::InvalidData);
                    None
                }
            },
            Some("compacted") => payload
                .get("latest_token_usage_record")
                .filter(|record| !record.is_null()),
            Some("response_item") => {
                match payload.get("type").and_then(Value::as_str) {
                    Some("function_call" | "custom_tool_call") => tools.observe(
                        session_id,
                        payload.get("call_id").and_then(Value::as_str),
                        payload.get("name").and_then(Value::as_str),
                        &mut findings,
                    ),
                    Some(
                        "message"
                        | "reasoning"
                        | "function_call_output"
                        | "custom_tool_call_output",
                    ) => {}
                    _ => findings.tool(ReasonCode::InvalidData),
                }
                None
            }
            _ => {
                findings.token(ReasonCode::InvalidData);
                findings.tools_partial = true;
                None
            }
        };
        let Some(record) = record else { continue };
        let ids = [
            "thread_id",
            "turn_id",
            "session_id",
            "root_turn_id",
            "response_id",
        ]
        .map(|field| record.get(field).and_then(Value::as_str));
        let [
            Some(thread),
            Some(turn),
            Some(owner),
            Some(root),
            Some(response_id),
        ] = ids
        else {
            findings.token(ReasonCode::InvalidData);
            continue;
        };
        if thread != session_id
            || !ids
                .into_iter()
                .flatten()
                .all(|id| identifier(id, &mut findings))
        {
            findings.token(ReasonCode::AmbiguousAccounting);
            continue;
        }
        // A compacted checkpoint copies the same response, without its event discriminator.
        let mut canonical = record.clone();
        canonical
            .as_object_mut()
            .map(|record| record.remove("type"));
        if let Some(previous) = responses.get_mut(response_id) {
            if previous.record != canonical {
                previous.usage = None;
                findings.token(ReasonCode::AmbiguousAccounting);
            }
            continue;
        }
        if responses.len() == MAX_IDENTITIES {
            findings.token(ReasonCode::ResourceLimit);
            continue;
        }
        let usage = Counters::parse(&record["usage"], PATHS, &mut findings);
        let attribution = contexts.get(turn).cloned().unwrap_or_default();
        if attribution.model.is_none() {
            findings.reason(ReasonCode::IncompleteInput);
        }
        if let Some(usage) = &usage {
            let turn_sum = turn_sums
                .entry((turn.to_owned(), owner.to_owned(), root.to_owned()))
                .or_insert(Counters {
                    values: [Some(0); 6],
                });
            if !thread_sum.add_complete(usage) || !turn_sum.add_complete(usage) {
                findings.token(ReasonCode::ResourceLimit);
            }
            for (field, sum) in [
                ("thread_token_usage", &thread_sum),
                ("turn_token_usage", turn_sum),
            ] {
                if let Some(total) = Counters::parse(&record[field], PATHS, &mut findings) {
                    if total
                        .values
                        .iter()
                        .zip(sum.values)
                        .any(|(total, sum)| total.is_some() && *total != sum)
                    {
                        findings.token(ReasonCode::AmbiguousAccounting);
                    }
                } else {
                    findings.token(ReasonCode::IncompleteInput);
                }
            }
        }
        responses.insert(
            response_id.to_owned(),
            Response {
                record: canonical,
                usage,
                attribution,
            },
        );
    }
    let mut accounting = Accounting::new(Provider::Codex, policy);
    if legacy_usage && responses.is_empty() {
        findings.token(ReasonCode::IncompleteInput);
    }
    for response in responses.into_values() {
        if let Some(usage) = response.usage {
            accounting.request(&usage, &response.attribution, &mut findings);
        }
    }
    let tool_calls = tools.finish(diagnostics.root.is_complete(), &mut findings);
    let snapshot = accounting.finish(tool_calls, &findings);
    ExtractionOutcome::Usable(Box::new(ExtractedUsage {
        snapshot: HarnessUsageSnapshot::Codex(snapshot),
        diagnostics: findings.diagnostics(),
    }))
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
