//! The client's read-only view of the server-authored `Message.RequestMetadata` records that
//! ride in the task tree (APP-5720). The client never authors or mutates these; it decodes them
//! for display and matches them against the server's copy of the conversation.

use std::collections::HashSet;

use chrono::{DateTime, Local};
use serde_json::{Value, json};
use warp_multi_agent_api as api;

use super::api::convert_conversation::proto_timestamp_to_local_datetime;

/// How the server saw the request end. Mirrors `Message.RequestMetadata.Outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOutcome {
    Completed,
    Canceled,
    Errored,
    /// The record predates the outcome enum; its `incomplete` flag is all we know.
    Unspecified {
        incomplete: bool,
    },
}

impl RequestOutcome {
    pub fn label(self) -> &'static str {
        match self {
            RequestOutcome::Completed => "Completed",
            RequestOutcome::Canceled => "Canceled",
            RequestOutcome::Errored => "Errored",
            RequestOutcome::Unspecified { incomplete: true } => "Incomplete",
            RequestOutcome::Unspecified { incomplete: false } => "Completed",
        }
    }

    /// Whether the request did not run to a clean finish.
    pub fn is_interrupted(self) -> bool {
        !matches!(
            self,
            RequestOutcome::Completed | RequestOutcome::Unspecified { incomplete: false }
        )
    }
}

/// Token counts and costs (US cents and credits) charged for one model within one usage
/// category.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestModelCharge {
    pub category: String,
    /// `direct_api`, `byok`, or `custom_endpoint`.
    pub usage_type: &'static str,
    pub model_id: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub input_cost_in_cents: f32,
    pub output_cost_in_cents: f32,
    pub cache_read_cost_in_cents: f32,
    pub cache_write_cost_in_cents: f32,
    pub input_cost_in_credits: f32,
    pub output_cost_in_credits: f32,
    pub cache_read_cost_in_credits: f32,
    pub cache_write_cost_in_credits: f32,
    pub web_search_count: u32,
    pub web_search_cost_in_cents: f32,
    pub web_search_cost_in_credits: f32,
}

impl RequestModelCharge {
    pub fn tokens(&self) -> u64 {
        u64::from(self.input_tokens)
            + u64::from(self.output_tokens)
            + u64::from(self.cache_read_tokens)
            + u64::from(self.cache_write_tokens)
    }

    pub fn cost_in_cents(&self) -> f32 {
        self.input_cost_in_cents
            + self.output_cost_in_cents
            + self.cache_read_cost_in_cents
            + self.cache_write_cost_in_cents
            + self.web_search_cost_in_cents
    }

    pub fn cost_in_credits(&self) -> f32 {
        self.input_cost_in_credits
            + self.output_cost_in_credits
            + self.cache_read_cost_in_credits
            + self.cache_write_cost_in_credits
            + self.web_search_cost_in_credits
    }
}

/// Platform usage charged for one usage category.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestPlatformCharge {
    pub category: String,
    pub cost_in_cents: f32,
    pub cost_in_credits: f32,
    pub duration_seconds: f64,
}

/// One decoded wall-clock span of an internal LLM generation call.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestLlmGenerationSpan {
    pub started_at: Option<DateTime<Local>>,
    pub ended_at: Option<DateTime<Local>>,
}

impl RequestLlmGenerationSpan {
    pub fn duration_ms(&self) -> Option<i64> {
        Some(
            self.ended_at?
                .signed_duration_since(self.started_at?)
                .num_milliseconds()
                .max(0),
        )
    }
}

/// One decoded `Message.RequestMetadata`, keyed by the request it describes.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestMetadataRecord {
    pub message_id: String,
    pub request_id: String,
    /// The message envelope timestamp: when the server recorded the request as finished.
    pub recorded_at: Option<DateTime<Local>>,
    pub outcome: RequestOutcome,
    pub request_started_at: Option<DateTime<Local>>,
    pub first_token_at: Option<DateTime<Local>>,
    pub request_ended_at: Option<DateTime<Local>>,
    /// Wall-clock spans of each internal LLM generation call, in order.
    pub llm_generation_spans: Vec<RequestLlmGenerationSpan>,
    pub model_charges: Vec<RequestModelCharge>,
    pub platform_charges: Vec<RequestPlatformCharge>,
    pub tool_calls: Option<u32>,
    pub commands_executed: Option<u32>,
    pub files_changed: Option<u32>,
    pub lines_added: Option<u32>,
    pub lines_removed: Option<u32>,
    /// Percentage (0-100) of the context window in use after the request.
    pub context_window_usage: Option<f32>,
}

impl RequestMetadataRecord {
    /// Decodes `message` if it carries a `RequestMetadata` payload.
    pub fn from_message(message: &api::Message) -> Option<Self> {
        let Some(api::message::Message::RequestMetadata(metadata)) = message.message.as_ref()
        else {
            return None;
        };

        let outcome = match api::message::request_metadata::Outcome::try_from(metadata.outcome) {
            Ok(api::message::request_metadata::Outcome::Completed) => RequestOutcome::Completed,
            Ok(api::message::request_metadata::Outcome::Canceled) => RequestOutcome::Canceled,
            Ok(api::message::request_metadata::Outcome::Errored) => RequestOutcome::Errored,
            Ok(api::message::request_metadata::Outcome::Unspecified) | Err(_) => {
                RequestOutcome::Unspecified {
                    incomplete: metadata.incomplete,
                }
            }
        };

        let timing = metadata.timing.as_ref();
        let timestamp = |ts: Option<&prost_types::Timestamp>| {
            ts.map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos))
        };
        let seconds = |duration: Option<&prost_types::Duration>| {
            duration.map(|d| d.seconds as f64 + f64::from(d.nanos) / 1e9)
        };

        let (request_started_at, request_ended_at) = timing
            .and_then(|t| t.request_timespan.as_ref())
            .map(|timespan| {
                (
                    timestamp(timespan.started_at.as_ref()),
                    timestamp(timespan.ended_at.as_ref()),
                )
            })
            .unwrap_or((None, None));
        let llm_generation_spans = timing
            .map(|t| {
                t.llm_generation_timespans
                    .iter()
                    .map(|timespan| RequestLlmGenerationSpan {
                        started_at: timestamp(timespan.started_at.as_ref()),
                        ended_at: timestamp(timespan.ended_at.as_ref()),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut model_charges = Vec::new();
        let mut platform_charges = Vec::new();
        if let Some(charges) = metadata.charges.as_ref() {
            let mut categories: Vec<_> = charges.usage_by_category.iter().collect();
            categories.sort_by(|(a, _), (b, _)| a.cmp(b));
            for (category, charged) in categories {
                let sections: [(&str, &_); 3] = [
                    ("direct_api", &charged.direct_api_inference_usage),
                    ("byok", &charged.byok_inference_usage),
                    ("custom_endpoint", &charged.custom_endpoint_inference_usage),
                ];
                for (usage_type, by_model) in sections {
                    let mut models: Vec<_> = by_model.iter().collect();
                    models.sort_by(|(a, _), (b, _)| a.cmp(b));
                    for (model_id, inference) in models {
                        let tokens = inference.token_count.unwrap_or_default();
                        let cost = inference.token_cost.unwrap_or_default();
                        model_charges.push(RequestModelCharge {
                            category: category.clone(),
                            usage_type,
                            model_id: model_id.clone(),
                            input_tokens: tokens.input,
                            output_tokens: tokens.output,
                            cache_read_tokens: tokens.input_cache_read,
                            cache_write_tokens: tokens.input_cache_write,
                            input_cost_in_cents: cost.input_cost_in_cents,
                            output_cost_in_cents: cost.output_cost_in_cents,
                            cache_read_cost_in_cents: cost.input_cache_read_cost_in_cents,
                            cache_write_cost_in_cents: cost.input_cache_write_cost_in_cents,
                            input_cost_in_credits: cost.input_cost_in_credits,
                            output_cost_in_credits: cost.output_cost_in_credits,
                            cache_read_cost_in_credits: cost.input_cache_read_cost_in_credits,
                            cache_write_cost_in_credits: cost.input_cache_write_cost_in_credits,
                            web_search_count: inference.web_search_count,
                            web_search_cost_in_cents: inference.web_search_cost_in_cents,
                            web_search_cost_in_credits: inference.web_search_cost_in_credits,
                        });
                    }
                }
                if charged.platform_usage_in_cents != 0.0
                    || charged.platform_usage_in_credits != 0.0
                    || charged.platform_usage_duration.is_some()
                {
                    platform_charges.push(RequestPlatformCharge {
                        category: category.clone(),
                        cost_in_cents: charged.platform_usage_in_cents,
                        cost_in_credits: charged.platform_usage_in_credits,
                        duration_seconds: seconds(charged.platform_usage_duration.as_ref())
                            .unwrap_or(0.0),
                    });
                }
            }
        }

        let tool_call_summary = metadata.tool_call_summary.as_ref();
        let context_window = metadata.context_window.as_ref();

        Some(Self {
            message_id: message.id.clone(),
            request_id: message.request_id.clone(),
            recorded_at: timestamp(message.timestamp.as_ref()),
            outcome,
            request_started_at,
            first_token_at: timestamp(timing.and_then(|t| t.first_token_at.as_ref())),
            request_ended_at,
            llm_generation_spans,
            model_charges,
            platform_charges,
            tool_calls: tool_call_summary.map(|s| s.tool_calls),
            commands_executed: tool_call_summary.map(|s| s.commands_executed),
            files_changed: tool_call_summary.map(|s| s.files_changed),
            lines_added: tool_call_summary.map(|s| s.lines_added),
            lines_removed: tool_call_summary.map(|s| s.lines_removed),
            context_window_usage: context_window.map(|c| c.usage),
        })
    }

    /// Every record on `task`, in message order.
    pub fn records_in_task(task: &api::Task) -> Vec<Self> {
        task.messages
            .iter()
            .filter_map(Self::from_message)
            .collect()
    }

    /// Every record in `conversation_data`, root task first, in message order.
    pub fn records_in_conversation_data(conversation_data: &api::ConversationData) -> Vec<Self> {
        conversation_data
            .tasks
            .iter()
            .flat_map(|task| task.messages.iter())
            .filter_map(Self::from_message)
            .collect()
    }

    pub fn inference_cost_in_cents(&self) -> f32 {
        self.model_charges.iter().map(|c| c.cost_in_cents()).sum()
    }

    pub fn inference_cost_in_credits(&self) -> f32 {
        self.model_charges.iter().map(|c| c.cost_in_credits()).sum()
    }

    pub fn platform_cost_in_cents(&self) -> f32 {
        self.platform_charges.iter().map(|c| c.cost_in_cents).sum()
    }

    pub fn platform_cost_in_credits(&self) -> f32 {
        self.platform_charges
            .iter()
            .map(|c| c.cost_in_credits)
            .sum()
    }

    /// Everything the request was actually charged, in US cents.
    pub fn total_cost_in_cents(&self) -> f32 {
        self.inference_cost_in_cents() + self.platform_cost_in_cents()
    }

    /// Everything the request was actually charged, in credits.
    pub fn total_cost_in_credits(&self) -> f32 {
        self.inference_cost_in_credits() + self.platform_cost_in_credits()
    }

    pub fn total_tokens(&self) -> u64 {
        self.model_charges.iter().map(|c| c.tokens()).sum()
    }

    pub fn time_to_first_token_ms(&self) -> Option<i64> {
        let started = self.request_started_at?;
        let first_token = self.first_token_at?;
        Some(
            first_token
                .signed_duration_since(started)
                .num_milliseconds()
                .max(0),
        )
    }

    /// The request's server-side processing window: request start to request end.
    pub fn request_duration_ms(&self) -> Option<i64> {
        let started = self.request_started_at?;
        let ended = self.request_ended_at?;
        Some(
            ended
                .signed_duration_since(started)
                .num_milliseconds()
                .max(0),
        )
    }

    /// Total time spent in LLM generation calls: the sum of the spans' durations.
    pub fn llm_generation_ms(&self) -> Option<i64> {
        let mut total = 0i64;
        let mut any = false;
        for span in &self.llm_generation_spans {
            if let Some(ms) = span.duration_ms() {
                any = true;
                total += ms;
            }
        }
        any.then_some(total)
    }

    /// The record as JSON, for the raw view in the Turn panel.
    pub fn to_json(&self) -> Value {
        let rfc3339 = |time: Option<DateTime<Local>>| match time {
            Some(time) => Value::String(time.to_rfc3339()),
            None => Value::Null,
        };
        json!({
            "request_id": self.request_id,
            "message_id": self.message_id,
            "recorded_at": rfc3339(self.recorded_at),
            "outcome": self.outcome.label(),
            "incomplete": self.outcome.is_interrupted(),
            "timing": {
                "request_started_at": rfc3339(self.request_started_at),
                "first_token_at": rfc3339(self.first_token_at),
                "request_ended_at": rfc3339(self.request_ended_at),
                "llm_generation_timespans": self.llm_generation_spans.iter().map(|span| json!({
                    "started_at": rfc3339(span.started_at),
                    "ended_at": rfc3339(span.ended_at),
                    "duration_ms": span.duration_ms(),
                })).collect::<Vec<_>>(),
            },
            "charges": {
                "models": self.model_charges.iter().map(|charge| json!({
                    "category": charge.category,
                    "usage_type": charge.usage_type,
                    "model_id": charge.model_id,
                    "tokens": {
                        "input": charge.input_tokens,
                        "output": charge.output_tokens,
                        "cache_read": charge.cache_read_tokens,
                        "cache_write": charge.cache_write_tokens,
                    },
                    "cost_in_cents": {
                        "input": charge.input_cost_in_cents,
                        "output": charge.output_cost_in_cents,
                        "cache_read": charge.cache_read_cost_in_cents,
                        "cache_write": charge.cache_write_cost_in_cents,
                        "web_search": charge.web_search_cost_in_cents,
                    },
                    "cost_in_credits": {
                        "input": charge.input_cost_in_credits,
                        "output": charge.output_cost_in_credits,
                        "cache_read": charge.cache_read_cost_in_credits,
                        "cache_write": charge.cache_write_cost_in_credits,
                        "web_search": charge.web_search_cost_in_credits,
                    },
                    "web_search_count": charge.web_search_count,
                })).collect::<Vec<_>>(),
                "platform": self.platform_charges.iter().map(|charge| json!({
                    "category": charge.category,
                    "cost_in_cents": charge.cost_in_cents,
                    "cost_in_credits": charge.cost_in_credits,
                    "duration_seconds": charge.duration_seconds,
                })).collect::<Vec<_>>(),
                "total_cost_in_cents": self.total_cost_in_cents(),
                "total_cost_in_credits": self.total_cost_in_credits(),
            },
            "tool_call_summary": {
                "tool_calls": self.tool_calls,
                "commands_executed": self.commands_executed,
                "files_changed": self.files_changed,
                "lines_added": self.lines_added,
                "lines_removed": self.lines_removed,
            },
            "context_window": {
                "usage": self.context_window_usage,
            },
        })
    }
}

/// One turn's records aggregated for the Turn panel. A turn (exchange) can hold several records
/// — one per underlying API request (retries, resumes, multi-request turns) — so the panel shows
/// their sum rather than one entry per request.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnSummary {
    /// Every locally-held record for the turn, in task order.
    pub records: Vec<RequestMetadataRecord>,
    pub request_count: usize,
    /// How many records did not run to a clean finish.
    pub interrupted_count: usize,
    /// The worst outcome across the records (Errored > Canceled > Incomplete > Completed);
    /// among equally-severe outcomes, the latest one wins.
    pub outcome: RequestOutcome,
    /// Per-model charges summed across the records.
    pub model_charges: Vec<RequestModelCharge>,
    /// Per-category platform charges summed across the records.
    pub platform_charges: Vec<RequestPlatformCharge>,
    pub recorded_at: Option<DateTime<Local>>,
    /// Earliest non-nil `request_started_at` across the records.
    pub request_started_at: Option<DateTime<Local>>,
    /// Earliest non-nil `first_token_at` across the records.
    pub first_token_at: Option<DateTime<Local>>,
    /// Latest non-nil `request_ended_at` across the records.
    pub request_ended_at: Option<DateTime<Local>>,
    /// Every record's LLM generation spans, in task order.
    pub llm_generation_spans: Vec<RequestLlmGenerationSpan>,
    /// Tool-call counts summed across the records (`None` when no record reports the field).
    pub tool_calls: Option<u32>,
    pub commands_executed: Option<u32>,
    pub files_changed: Option<u32>,
    pub lines_added: Option<u32>,
    pub lines_removed: Option<u32>,
    /// The latest record's context-window reading: it reflects the most recent state.
    pub context_window_usage: Option<f32>,
}

fn outcome_severity(outcome: RequestOutcome) -> u8 {
    match outcome {
        RequestOutcome::Errored => 3,
        RequestOutcome::Canceled => 2,
        RequestOutcome::Unspecified { incomplete: true } => 1,
        RequestOutcome::Completed | RequestOutcome::Unspecified { incomplete: false } => 0,
    }
}

fn sum_option_u32(values: impl Iterator<Item = Option<u32>>) -> Option<u32> {
    values.fold(None, |acc, value| match (acc, value) {
        (None, None) => None,
        (acc, None) => acc,
        (None, Some(value)) => Some(value),
        (Some(acc), Some(value)) => Some(acc + value),
    })
}

impl TurnSummary {
    pub fn total_tokens(&self) -> u64 {
        self.model_charges.iter().map(|c| c.tokens()).sum()
    }

    pub fn inference_cost_in_cents(&self) -> f32 {
        self.model_charges.iter().map(|c| c.cost_in_cents()).sum()
    }

    pub fn platform_cost_in_cents(&self) -> f32 {
        self.platform_charges.iter().map(|c| c.cost_in_cents).sum()
    }

    pub fn total_cost_in_cents(&self) -> f32 {
        self.inference_cost_in_cents() + self.platform_cost_in_cents()
    }

    /// Earliest first token relative to the earliest request start.
    pub fn time_to_first_token_ms(&self) -> Option<i64> {
        let started = self.request_started_at?;
        let first_token = self.first_token_at?;
        Some(
            first_token
                .signed_duration_since(started)
                .num_milliseconds()
                .max(0),
        )
    }

    /// Earliest request start to latest request end.
    pub fn request_duration_ms(&self) -> Option<i64> {
        let started = self.request_started_at?;
        let ended = self.request_ended_at?;
        Some(
            ended
                .signed_duration_since(started)
                .num_milliseconds()
                .max(0),
        )
    }

    /// Total time spent in LLM generation calls across the turn's records.
    pub fn llm_generation_ms(&self) -> Option<i64> {
        let mut total = 0i64;
        let mut any = false;
        for span in &self.llm_generation_spans {
            if let Some(ms) = span.duration_ms() {
                any = true;
                total += ms;
            }
        }
        any.then_some(total)
    }
}

/// Aggregates one turn's records for display. Charges are summed per model and per platform
/// category; timing uses the widest span (earliest start / first token, latest end) plus every
/// record's LLM generation spans; tool counts sum; the context window comes from the latest
/// record that reports one.
pub fn summarize_turn(records: &[RequestMetadataRecord]) -> TurnSummary {
    let mut outcome = RequestOutcome::Completed;
    let mut interrupted_count = 0usize;
    let mut model_charges: Vec<RequestModelCharge> = Vec::new();
    let mut platform_charges: Vec<RequestPlatformCharge> = Vec::new();
    let mut recorded_at = None;
    let mut request_started_at = None;
    let mut first_token_at = None;
    let mut request_ended_at = None;
    let mut llm_generation_spans = Vec::new();
    let mut context_window_usage = None;

    for record in records {
        if outcome_severity(record.outcome) >= outcome_severity(outcome) {
            outcome = record.outcome;
        }
        interrupted_count += usize::from(record.outcome.is_interrupted());

        for charge in &record.model_charges {
            if let Some(existing) = model_charges.iter_mut().find(|existing| {
                existing.category == charge.category
                    && existing.usage_type == charge.usage_type
                    && existing.model_id == charge.model_id
            }) {
                existing.input_tokens += charge.input_tokens;
                existing.output_tokens += charge.output_tokens;
                existing.cache_read_tokens += charge.cache_read_tokens;
                existing.cache_write_tokens += charge.cache_write_tokens;
                existing.input_cost_in_cents += charge.input_cost_in_cents;
                existing.output_cost_in_cents += charge.output_cost_in_cents;
                existing.cache_read_cost_in_cents += charge.cache_read_cost_in_cents;
                existing.cache_write_cost_in_cents += charge.cache_write_cost_in_cents;
                existing.input_cost_in_credits += charge.input_cost_in_credits;
                existing.output_cost_in_credits += charge.output_cost_in_credits;
                existing.cache_read_cost_in_credits += charge.cache_read_cost_in_credits;
                existing.cache_write_cost_in_credits += charge.cache_write_cost_in_credits;
                existing.web_search_count += charge.web_search_count;
                existing.web_search_cost_in_cents += charge.web_search_cost_in_cents;
                existing.web_search_cost_in_credits += charge.web_search_cost_in_credits;
            } else {
                model_charges.push(charge.clone());
            }
        }
        for charge in &record.platform_charges {
            if let Some(existing) = platform_charges
                .iter_mut()
                .find(|existing| existing.category == charge.category)
            {
                existing.cost_in_cents += charge.cost_in_cents;
                existing.cost_in_credits += charge.cost_in_credits;
                existing.duration_seconds += charge.duration_seconds;
            } else {
                platform_charges.push(charge.clone());
            }
        }

        recorded_at = record.recorded_at.or(recorded_at);
        request_started_at = record
            .request_started_at
            .into_iter()
            .chain(request_started_at)
            .min();
        first_token_at = record
            .first_token_at
            .into_iter()
            .chain(first_token_at)
            .min();
        request_ended_at = record.request_ended_at.or(request_ended_at);
        llm_generation_spans.extend(record.llm_generation_spans.iter().cloned());
        context_window_usage = record.context_window_usage.or(context_window_usage);
    }

    TurnSummary {
        request_count: records.len(),
        interrupted_count,
        outcome,
        model_charges,
        platform_charges,
        recorded_at,
        request_started_at,
        first_token_at,
        request_ended_at,
        llm_generation_spans,
        tool_calls: sum_option_u32(records.iter().map(|r| r.tool_calls)),
        commands_executed: sum_option_u32(records.iter().map(|r| r.commands_executed)),
        files_changed: sum_option_u32(records.iter().map(|r| r.files_changed)),
        lines_added: sum_option_u32(records.iter().map(|r| r.lines_added)),
        lines_removed: sum_option_u32(records.iter().map(|r| r.lines_removed)),
        context_window_usage,
        records: records.to_vec(),
    }
}

/// The records the server holds for a conversation that the client never received: the number
/// of such turns and what they were charged in total.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OrphanedRequestSummary {
    pub count: usize,
    pub total_cost_in_cents: f32,
    /// How many server records were considered in total (orphaned or not).
    pub server_record_count: usize,
}

/// Compares the server's records against the request ids the client holds locally. A server
/// record whose `request_id` has no local counterpart is orphaned.
pub fn summarize_orphaned_records(
    server_records: &[RequestMetadataRecord],
    local_request_ids: &HashSet<String>,
) -> OrphanedRequestSummary {
    let mut summary = OrphanedRequestSummary {
        server_record_count: server_records.len(),
        ..Default::default()
    };
    for record in server_records {
        if local_request_ids.contains(&record.request_id) {
            continue;
        }
        summary.count += 1;
        summary.total_cost_in_cents += record.total_cost_in_cents();
    }
    summary
}

#[cfg(test)]
#[path = "request_metadata_tests.rs"]
mod tests;
