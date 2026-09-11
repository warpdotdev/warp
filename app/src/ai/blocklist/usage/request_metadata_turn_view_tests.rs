use super::*;
use crate::ai::agent::request_metadata::{
    RequestLlmGenerationSpan, RequestModelCharge, RequestPlatformCharge,
};

fn record(
    outcome: RequestOutcome,
    inference_cents: f32,
    platform_cents: f32,
) -> RequestMetadataRecord {
    RequestMetadataRecord {
        message_id: "m".to_string(),
        request_id: "req".to_string(),
        recorded_at: None,
        outcome,
        request_started_at: None,
        first_token_at: None,
        request_ended_at: None,
        llm_generation_spans: Vec::new(),
        model_charges: (inference_cents > 0.0)
            .then(|| RequestModelCharge {
                category: "primary_agent".to_string(),
                usage_type: "direct_api",
                model_id: "model".to_string(),
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                input_cost_in_cents: inference_cents,
                output_cost_in_cents: 0.0,
                cache_read_cost_in_cents: 0.0,
                cache_write_cost_in_cents: 0.0,
                input_cost_in_credits: 0.0,
                output_cost_in_credits: 0.0,
                cache_read_cost_in_credits: 0.0,
                cache_write_cost_in_credits: 0.0,
                web_search_count: 0,
                web_search_cost_in_cents: 0.0,
                web_search_cost_in_credits: 0.0,
            })
            .into_iter()
            .collect(),
        platform_charges: (platform_cents > 0.0)
            .then(|| RequestPlatformCharge {
                category: "primary_agent".to_string(),
                cost_in_cents: platform_cents,
                cost_in_credits: 0.0,
                duration_seconds: 30.0,
            })
            .into_iter()
            .collect(),
        tool_calls: None,
        commands_executed: None,
        files_changed: None,
        lines_added: None,
        lines_removed: None,
        context_window_usage: None,
    }
}

#[test]
fn format_dollars_never_rounds_a_real_charge_to_zero() {
    assert_eq!(format_dollars(0.0), "$0.00");
    assert_eq!(format_dollars(0.3), "<$0.01");
    assert_eq!(format_dollars(150.0), "$1.50");
}

#[test]
fn format_tokens_searches_and_calls_pluralize() {
    assert_eq!(format_tokens(1), "1 token");
    assert_eq!(format_tokens(42), "42 tokens");
    assert_eq!(format_web_searches(1), "1 search");
    assert_eq!(format_web_searches(3), "3 searches");
    assert_eq!(format_llm_calls(1), "1 call");
    assert_eq!(format_llm_calls(3), "3 calls");
}

#[test]
fn tooltip_names_the_total_charge_and_only_interrupted_outcomes() {
    assert_eq!(
        turn_panel_tooltip_text(&[record(RequestOutcome::Completed, 120.0, 30.0)]),
        "Turn: $1.50"
    );
    // A cancelled single-call turn is charged only its platform segment: the record, and
    // the tooltip built from it, say exactly that rather than pricing tokens that were
    // never accounted.
    assert_eq!(
        turn_panel_tooltip_text(&[record(RequestOutcome::Canceled, 0.0, 30.0)]),
        "Turn: $0.30 · Canceled"
    );
    assert_eq!(
        turn_panel_tooltip_text(&[record(RequestOutcome::Errored, 0.0, 0.0)]),
        "Turn: $0.00 · Errored"
    );
}

#[test]
fn tooltip_sums_multi_request_turns_and_names_the_count() {
    let mut first = record(RequestOutcome::Completed, 120.0, 30.0);
    first.request_id = "req-1".to_string();
    let mut second = record(RequestOutcome::Canceled, 60.0, 0.0);
    second.request_id = "req-2".to_string();
    assert_eq!(
        turn_panel_tooltip_text(&[first, second]),
        "Turn: $2.10 · Canceled · 2 requests"
    );
}

#[test]
fn view_starts_collapsed_and_toggles_raw_record() {
    let mut view =
        RequestMetadataTurnView::new(vec![record(RequestOutcome::Completed, 120.0, 0.0)]);
    assert!(!view.raw_record_expanded);
    assert_eq!(view.model_rows.len(), 1);
    assert!(view.raw_json.contains("\"request_id\": \"req\""));
    view.raw_record_expanded = !view.raw_record_expanded;
    assert!(view.raw_record_expanded);
    assert_eq!(view.records().len(), 1);
    assert_eq!(view.records()[0].request_id, "req");
}

#[test]
fn view_aggregates_a_multi_record_turn() {
    let mut first = record(RequestOutcome::Completed, 120.0, 30.0);
    first.request_id = "req-1".to_string();
    let mut second = record(RequestOutcome::Canceled, 60.0, 0.0);
    second.request_id = "req-2".to_string();

    let view = RequestMetadataTurnView::new(vec![first, second]);
    assert_eq!(view.summary.request_count, 2);
    assert_eq!(view.summary.interrupted_count, 1);
    assert_eq!(view.summary.outcome, RequestOutcome::Canceled);
    // Charges merge per model: one row, summed tokens and cost.
    assert_eq!(view.model_rows.len(), 1);
    assert_eq!(view.summary.total_tokens(), 30);
    assert_eq!(view.records().len(), 2);
    // Raw view keeps every record so per-request detail survives aggregation.
    assert!(view.raw_json.contains("\"req-1\""));
    assert!(view.raw_json.contains("\"req-2\""));
}

#[test]
fn view_rolls_up_llm_generation_spans_across_records() {
    let mut first = record(RequestOutcome::Completed, 120.0, 0.0);
    first.llm_generation_spans = vec![RequestLlmGenerationSpan {
        started_at: None,
        ended_at: None,
    }];
    let mut second = record(RequestOutcome::Completed, 60.0, 0.0);
    second.request_id = "req-2".to_string();

    let view = RequestMetadataTurnView::new(vec![first, second]);
    // The record without span timestamps contributes no measurable time; the span list
    // still keeps both records' spans for the per-call breakdown.
    assert_eq!(view.summary.llm_generation_spans.len(), 1);
    assert_eq!(view.summary.llm_generation_ms(), None);
}
