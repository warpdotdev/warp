use std::collections::BTreeMap;

use serde_json::Value;

use crate::api::{
    Coverage, HarnessUsageSnapshot, PiAttributedUsage, PiUsage, PiUsagePayload, PiUsageSnapshot,
};
use crate::claude::classification;
use crate::counters::Counters;
use crate::tools::Tools;
use crate::{
    CaptureDiagnostics, ExtractedUsage, ExtractionOutcome, Findings, MAX_ATTRIBUTIONS,
    MAX_IDENTITIES, ReasonCode, identifier,
};

const PATHS: [&str; 7] = [
    "/input",
    "/output",
    "/cacheRead",
    "/cacheWrite",
    "/cacheWrite1h",
    "/reasoning",
    "/totalTokens",
];

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Group {
    provider: Option<String>,
    model: Option<String>,
    token_fields: [bool; 7],
}

#[derive(Default)]
struct Totals {
    tokens: Counters<7>,
    max_input: Option<i64>,
}

/// Extract cumulative Pi usage from all entries, including compacted and abandoned branches.
pub fn extract_pi(
    session_id: &str,
    entries: &[Value],
    diagnostics: &CaptureDiagnostics,
) -> ExtractionOutcome {
    let mut findings = Findings::default();
    findings.capture(diagnostics);
    if !identifier(session_id, &mut findings) {
        return ExtractionOutcome::Unavailable(findings.diagnostics());
    }
    let mut records: BTreeMap<&str, Option<&Value>> = BTreeMap::new();
    for entry in entries {
        if entry["type"] == "session" {
            continue;
        }
        let Some(id) = entry.get("id").and_then(Value::as_str) else {
            findings.token(ReasonCode::InvalidData);
            findings.tools_partial = true;
            continue;
        };
        if !identifier(id, &mut findings) {
            continue;
        }
        if let Some(previous) = records.get_mut(id) {
            if previous.is_some_and(|previous| previous != entry) {
                *previous = None;
                findings.token(ReasonCode::AmbiguousAccounting);
                findings.tools_partial = true;
            }
        } else if records.len() < MAX_IDENTITIES {
            records.insert(id, Some(entry));
        } else {
            findings.limit(ReasonCode::ResourceLimit);
            break;
        }
    }
    let mut groups = BTreeMap::<Group, Totals>::new();
    let mut tools = Tools::default();
    for entry in records.into_values().flatten() {
        let kind = entry["type"].as_str();
        let message = &entry["message"];
        let assistant = kind == Some("message") && message["role"] == "assistant";
        if assistant {
            if let Some(content) = message["content"].as_array() {
                for block in content.iter().filter(|block| block["type"] == "toolCall") {
                    tools.observe(
                        session_id,
                        block["id"].as_str(),
                        block["name"].as_str(),
                        &mut findings,
                    );
                }
            } else {
                findings.tool(ReasonCode::InvalidData);
            }
        }
        let owner = match kind {
            Some("message") if assistant || message["role"] == "toolResult" => message,
            Some("usage" | "compaction" | "branch_summary") => entry,
            _ => continue,
        };
        let Some(usage) = owner.get("usage").filter(|usage| !usage.is_null()) else {
            if assistant || kind == Some("usage") {
                findings.token(ReasonCode::IncompleteInput);
            }
            continue;
        };
        let tokens = Counters::<7>::parse(usage, PATHS, &mut findings).unwrap_or_default();
        let [input, output, read, write, long_write, reasoning, total] = tokens.values;
        let prompt = input
            .zip(read)
            .zip(write)
            .and_then(|((input, read), write)| input.checked_add(read)?.checked_add(write));
        let token_sum = prompt
            .zip(output)
            .and_then(|(prompt, output)| prompt.checked_add(output));
        if input.is_some()
            && read.is_some()
            && write.is_some()
            && (prompt.is_none() || output.is_some() && token_sum.is_none())
        {
            findings.limit(ReasonCode::ResourceLimit);
            break;
        }
        if input.is_none() || output.is_none() || read.is_none() || write.is_none() {
            findings.token(ReasonCode::IncompleteInput);
        }
        if long_write.zip(write).is_some_and(|(long, all)| long > all)
            || reasoning
                .zip(output)
                .is_some_and(|(reasoning, all)| reasoning > all)
            || total
                .zip(token_sum)
                .is_some_and(|(total, sum)| total != sum)
        {
            findings.token(ReasonCode::InvalidData);
            continue;
        }
        if !tokens.any() {
            continue;
        }
        let group = Group {
            provider: classification(owner.get("provider"), &mut findings),
            model: classification(
                owner.get("responseModel").or_else(|| owner.get("model")),
                &mut findings,
            ),
            token_fields: tokens.values.map(|value| value.is_some()),
        };
        if !groups.contains_key(&group) && groups.len() >= MAX_ATTRIBUTIONS {
            findings.limit(ReasonCode::ResourceLimit);
            break;
        }
        let totals = groups.entry(group).or_default();
        totals.tokens.add(&tokens, &mut findings);
        if totals
            .tokens
            .values
            .iter()
            .zip(tokens.values)
            .any(|(sum, value)| sum.is_none() && value.is_some())
        {
            findings.limit(ReasonCode::ResourceLimit);
        }
        if let [Some(input), output, Some(read), Some(write), ..] = totals.tokens.values
            && input
                .checked_add(read)
                .and_then(|sum| sum.checked_add(write))
                .and_then(|sum| sum.checked_add(output.unwrap_or_default()))
                .is_none()
        {
            findings.limit(ReasonCode::ResourceLimit);
        }
        totals.max_input = totals.max_input.max(prompt);
    }
    let tool_calls = tools.finish(diagnostics.root.is_complete(), &mut findings);
    let tokens_measured = groups.values().any(|totals| totals.tokens.any());
    if findings.limit_exceeded || (!tokens_measured && tool_calls.is_none()) {
        return ExtractionOutcome::Unavailable(findings.diagnostics());
    }
    let coverage = Coverage {
        token_status: Findings::status(tokens_measured, findings.tokens_partial),
        tool_status: Findings::status(tool_calls.is_some(), findings.tools_partial),
    };
    let attribution = groups
        .into_iter()
        .map(|(group, totals)| {
            let [
                input,
                output,
                cache_read,
                cache_write,
                cache_write1h,
                reasoning,
                total_tokens,
            ] = totals.tokens.values;
            PiAttributedUsage {
                provider: group.provider,
                model: group.model,
                max_input_tokens: totals.max_input,
                usage: PiUsage {
                    input,
                    output,
                    cache_read,
                    cache_write,
                    cache_write1h,
                    reasoning,
                    total_tokens,
                },
            }
        })
        .collect();
    ExtractionOutcome::Usable(Box::new(ExtractedUsage {
        snapshot: HarnessUsageSnapshot::Pi(PiUsageSnapshot {
            coverage,
            payload: PiUsagePayload {
                attribution,
                tool_calls,
            },
        }),
        diagnostics: findings.diagnostics(),
    }))
}

#[cfg(test)]
#[path = "pi_tests.rs"]
mod tests;
