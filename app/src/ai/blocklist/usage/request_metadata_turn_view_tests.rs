use super::*;
use crate::ai::agent::request_metadata::{
    InferenceUsageType, RequestLlmGenerationSpan, RequestModelCharge, RequestPlatformCharge,
};
use crate::settings::UsageDisplayUnit;

fn record(inference_cents: f32, platform_cents: f32) -> RequestMetadataRecord {
    RequestMetadataRecord {
        message_id: "m".to_string(),
        request_id: "req".to_string(),
        request_started_at: None,
        first_token_at: None,
        request_ended_at: None,
        llm_generation_spans: Vec::new(),
        model_charges: (inference_cents > 0.0)
            .then(|| RequestModelCharge {
                category: "primary_agent".to_string(),
                usage_type: InferenceUsageType::DirectApi,
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

/// Every panel amount follows the display-unit setting; there is no separate Credits row.
#[test]
fn format_cost_follows_the_display_unit() {
    assert_eq!(format_cost(150.0, 0.5, UsageDisplayUnit::Dollars), "$1.50");
    assert_eq!(
        format_cost(150.0, 0.5, UsageDisplayUnit::Credits),
        "0.5 credits"
    );
    assert_eq!(
        format_tokens_with_cost(1200, 150.0, 0.5, UsageDisplayUnit::Credits),
        "1,200 tokens  /  0.5 credits"
    );
}

#[test]
fn format_credits_amount_never_rounds_a_real_charge_to_zero() {
    assert_eq!(format_credits_amount(0.0), "0 credits");
    assert_eq!(format_credits_amount(0.03), "<0.1 credits");
    assert_eq!(format_credits_amount(1.0), "1 credit");
    assert_eq!(format_credits_amount(2.5), "2.5 credits");
}

#[test]
fn large_token_and_credit_amounts_use_thousands_separators() {
    assert_eq!(format_tokens(1_234_567), "1,234,567 tokens");
    assert_eq!(format_credits_amount(1_234.0), "1,234 credits");
    assert_eq!(format_credits_amount(12_345.5), "12,345.5 credits");
}

#[test]
fn format_tokens_and_searches_pluralize() {
    assert_eq!(format_tokens(1), "1 token");
    assert_eq!(format_tokens(42), "42 tokens");
    assert_eq!(format_web_searches(1), "1 search");
    assert_eq!(format_web_searches(3), "3 searches");
}

#[test]
fn tooltip_names_the_charge_in_the_user_s_display_unit() {
    let records = [record(120.0, 30.0)];
    assert_eq!(
        turn_panel_tooltip_text(&records, UsageDisplayUnit::Dollars),
        "Turn: $1.50"
    );
    // The helper's records carry no credits; in Credits mode the tooltip stays quiet rather
    // than fabricating a zero total.
    assert_eq!(
        turn_panel_tooltip_text(&records, UsageDisplayUnit::Credits),
        "Turn"
    );
    assert_eq!(
        turn_panel_tooltip_text(&[record(0.0, 0.0)], UsageDisplayUnit::Dollars),
        "Turn"
    );
}

#[test]
fn tooltip_never_rounds_a_real_charge_to_zero() {
    let mut tiny = record(0.3, 0.0);
    tiny.model_charges[0].input_cost_in_credits = 0.03;
    assert_eq!(
        turn_panel_tooltip_text(&[tiny.clone()], UsageDisplayUnit::Dollars),
        "Turn: <$0.01"
    );
    assert_eq!(
        turn_panel_tooltip_text(&[tiny], UsageDisplayUnit::Credits),
        "Turn: <0.1 credits"
    );
}

#[test]
fn tooltip_sums_multi_request_turns() {
    let mut first = record(120.0, 30.0);
    first.request_id = "req-1".to_string();
    let mut second = record(60.0, 0.0);
    second.request_id = "req-2".to_string();
    assert_eq!(
        turn_panel_tooltip_text(&[first, second], UsageDisplayUnit::Dollars),
        "Turn: $2.10"
    );
}

#[test]
fn tooltip_honors_credits_when_the_records_carry_them() {
    let mut record = record(120.0, 30.0);
    record.model_charges[0].input_cost_in_credits = 1.0;
    record.model_charges[0].output_cost_in_credits = 0.5;
    record.platform_charges[0].cost_in_credits = 1.0;
    assert_eq!(
        turn_panel_tooltip_text(&[record], UsageDisplayUnit::Credits),
        "Turn: 2.5 credits"
    );
}

#[test]
fn view_starts_with_collapsed_model_rows() {
    let view = RequestMetadataTurnView::new_for_test(vec![record(120.0, 0.0)]);
    assert_eq!(view.model_rows.len(), 1);
    assert!(!view.model_rows[0].expanded);
    assert_eq!(view.records().len(), 1);
    assert_eq!(view.records()[0].request_id, "req");
}

/// A legacy turn carries no records: an unknown-charges turn exposes no model rows to
/// render zeros from.
#[test]
fn view_from_legacy_unknown_has_no_model_rows() {
    let data = TurnPanelData::Legacy {
        records: vec![record(0.0, 0.0)],
        charges: LegacyCharges::Unknown,
    };
    let view = RequestMetadataTurnView::new_for_test(data);
    assert!(matches!(view.legacy_charges, Some(LegacyCharges::Unknown)));
    assert!(view.summary.model_charges.is_empty());
}

/// A credits-only legacy turn shows the credits total without any model row, so no
/// fabricated "0 tokens / $0.00" can appear; the tooltip leads with the same total in
/// both display units (dollars falls back to credits when no cent figure is known).
#[test]
fn credits_only_legacy_shows_credits_and_never_a_zero_dollar_row() {
    let data = TurnPanelData::Legacy {
        records: vec![record(0.0, 0.0)],
        charges: LegacyCharges::CreditsOnly(2.5),
    };
    let view = RequestMetadataTurnView::new_for_test(data.clone());
    assert!(view.summary.model_charges.is_empty());
    assert_eq!(
        turn_panel_tooltip_text_for_data(&data, UsageDisplayUnit::Credits),
        "Turn: 2.5 credits"
    );
    assert_eq!(
        turn_panel_tooltip_text_for_data(&data, UsageDisplayUnit::Dollars),
        "Turn: 2.5 credits"
    );

    let tiny = TurnPanelData::Legacy {
        records: vec![record(0.0, 0.0)],
        charges: LegacyCharges::CreditsOnly(0.03),
    };
    assert_eq!(
        turn_panel_tooltip_text_for_data(&tiny, UsageDisplayUnit::Credits),
        "Turn: <0.1 credits"
    );
}

#[test]
fn view_aggregates_a_multi_record_turn() {
    let mut first = record(120.0, 30.0);
    first.request_id = "req-1".to_string();
    let mut second = record(60.0, 0.0);
    second.request_id = "req-2".to_string();

    let view = RequestMetadataTurnView::new_for_test(vec![first, second]);
    // Charges merge per model: one row, summed tokens and cost.
    assert_eq!(view.model_rows.len(), 1);
    assert_eq!(view.summary.total_tokens(), 30);
    assert_eq!(view.records().len(), 2);
}

#[test]
fn view_orders_model_rows_by_descending_tokens() {
    let mut cheap = record(10.0, 0.0);
    cheap.model_charges[0].model_id = "cheap-model".to_string();
    let mut pricey = record(120.0, 0.0);
    pricey.model_charges[0].model_id = "pricey-model".to_string();
    pricey.model_charges[0].input_tokens = 100;

    let view = RequestMetadataTurnView::new_for_test(vec![cheap, pricey]);
    assert_eq!(view.model_rows.len(), 2);
    assert_eq!(view.summary.model_charges[0].model_id, "pricey-model");
    assert_eq!(view.summary.model_charges[1].model_id, "cheap-model");
}

/// Breakdown charges aggregate into one "Models" row so the flat totals never imply
/// model attribution.
#[test]
fn view_from_legacy_breakdown_has_one_models_row() {
    let mut breakdown = record(120.0, 30.0);
    breakdown.model_charges[0].model_id = "Models".to_string();
    let data = TurnPanelData::Legacy {
        records: vec![breakdown],
        charges: LegacyCharges::Breakdown(Box::new(
            crate::persistence::model::ChargedUsageTotals {
                input_tokens: 10,
                input_cost_in_cents: 120.0,
                ..Default::default()
            },
        )),
    };
    let view = RequestMetadataTurnView::new_for_test(data);
    assert_eq!(view.model_rows.len(), 1);
    assert_eq!(view.summary.model_charges[0].model_id, "Models");
}

#[test]
fn view_rolls_up_llm_generation_spans_across_records() {
    let mut first = record(120.0, 0.0);
    first.llm_generation_spans = vec![RequestLlmGenerationSpan {
        started_at: None,
        ended_at: None,
    }];
    let mut second = record(60.0, 0.0);
    second.request_id = "req-2".to_string();

    let view = RequestMetadataTurnView::new_for_test(vec![first, second]);
    // The record without span timestamps contributes no measurable time; the span list
    // still keeps both records' spans.
    assert_eq!(view.summary.llm_generation_spans.len(), 1);
}
