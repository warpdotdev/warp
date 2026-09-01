use std::collections::HashMap;

use super::*;

fn model(id: &str, warp_tokens: u32, category: &str) -> ModelTokenUsage {
    ModelTokenUsage {
        model_id: id.to_string(),
        warp_tokens,
        warp_token_usage_by_category: HashMap::from([(category.to_string(), warp_tokens)]),
        ..Default::default()
    }
}

#[test]
fn model_usage_rows_drops_zero_token_models() {
    let models = vec![
        model("gpt-5.5", 100, PRIMARY_AGENT_CATEGORY),
        ModelTokenUsage {
            model_id: "unused-model".to_string(),
            ..Default::default()
        },
    ];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].model_id, "gpt-5.5");
}

#[test]
fn model_usage_rows_sorts_primary_agent_first() {
    let models = vec![
        model("codex-model", 50, FULL_TERMINAL_USE_CATEGORY),
        model("primary-model", 100, PRIMARY_AGENT_CATEGORY),
        model("auto-model", 10, "other_category"),
    ];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(rows[0].model_id, "primary-model");
    assert_eq!(rows[0].role, Some(ModelRole::PrimaryAgent));
}

#[test]
fn model_usage_rows_assigns_full_terminal_use_role() {
    let models = vec![model("codex-model", 50, FULL_TERMINAL_USE_CATEGORY)];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(rows[0].role, Some(ModelRole::FullTerminalUse));
}

#[test]
fn model_usage_rows_has_no_role_for_unknown_categories() {
    let models = vec![model("auto-model", 10, "some_other_category")];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(rows[0].role, None);
}

/// The primary-agent role is the default, so it earns no badge; every other
/// known role does.
#[test]
fn primary_agent_role_has_no_badge_label() {
    assert_eq!(ModelRole::PrimaryAgent.badge_label(), None);
    assert_eq!(
        ModelRole::FullTerminalUse.badge_label(),
        Some("Full terminal use")
    );
}

fn charged_usage_with_input_cost(cost_in_cents: f32) -> PersistedModelTokenCost {
    PersistedModelTokenCost {
        input_cost_in_cents: cost_in_cents,
        ..Default::default()
    }
}

#[test]
fn model_usage_rows_joins_charged_usage_by_model_id() {
    let models = vec![
        model("gpt-5.5", 100, PRIMARY_AGENT_CATEGORY),
        model("codex-model", 50, FULL_TERMINAL_USE_CATEGORY),
    ];
    let charged_usage_by_model =
        HashMap::from([("gpt-5.5".to_string(), charged_usage_with_input_cost(36.0))]);
    let rows = model_usage_rows(&models, &charged_usage_by_model);
    let gpt_row = rows.iter().find(|r| r.model_id == "gpt-5.5").unwrap();
    let codex_row = rows.iter().find(|r| r.model_id == "codex-model").unwrap();
    assert_eq!(gpt_row.cost_in_cents, Some(36.0));
    assert!(gpt_row.charged_usage.is_some());
    assert_eq!(codex_row.cost_in_cents, None);
    assert!(codex_row.charged_usage.is_none());
}

/// The row total must equal the sum of the breakdown rows shown beneath it,
/// including cache buckets and web search.
#[test]
fn model_usage_row_totals_match_the_charged_usage_breakdown_rows() {
    let charged_usage = PersistedModelTokenCost {
        total_input: 1_000,
        output: 500,
        input_cache_read: 300,
        input_cache_write: 200,
        input_cost_in_cents: 10.0,
        output_cost_in_cents: 20.0,
        input_cache_read_cost_in_cents: 3.0,
        input_cache_write_cost_in_cents: 2.0,
        web_search_count: 2,
        web_search_cost_in_cents: 5.0,
    };
    let models = vec![model("gpt-5.5", 42, PRIMARY_AGENT_CATEGORY)];
    let charged_usage_by_model = HashMap::from([("gpt-5.5".to_string(), charged_usage)]);

    let rows = model_usage_rows(&models, &charged_usage_by_model);

    assert_eq!(rows[0].tokens, 1_000 + 500 + 300 + 200);
    assert_eq!(rows[0].cost_in_cents, Some(10.0 + 20.0 + 3.0 + 2.0 + 5.0));
}

/// Without attributed charges there is no breakdown to reconcile against, so
/// the row falls back to the raw reported token count.
#[test]
fn model_usage_row_falls_back_to_reported_tokens_without_charged_usage() {
    let models = vec![model("gpt-5.5", 100, PRIMARY_AGENT_CATEGORY)];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(rows[0].tokens, 100);
    assert_eq!(rows[0].cost_in_cents, None);
}

/// The section summary is what users compare against the rows, so it must be
/// their exact sum.
#[test]
fn row_totals_sum_the_displayed_rows() {
    let models = vec![
        model("gpt-5.5", 100, PRIMARY_AGENT_CATEGORY),
        model("codex-model", 50, FULL_TERMINAL_USE_CATEGORY),
    ];
    let charged_usage_by_model = HashMap::from([
        ("gpt-5.5".to_string(), charged_usage_with_input_cost(36.0)),
        (
            "codex-model".to_string(),
            charged_usage_with_input_cost(14.0),
        ),
    ]);
    let rows = model_usage_rows(&models, &charged_usage_by_model);
    let totals = RowTotals::of_model_rows(&rows);

    assert_eq!(totals.tokens, Some(rows.iter().map(|r| r.tokens).sum()));
    assert_eq!(totals.cost_in_cents, Some(50.0));
}

#[test]
fn row_totals_cost_is_unknown_when_no_row_has_an_attributed_cost() {
    let models = vec![model("gpt-5.5", 100, PRIMARY_AGENT_CATEGORY)];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(RowTotals::of_model_rows(&rows).cost_in_cents, None);
}

#[test]
fn format_token_count_abbreviates_above_1000() {
    assert_eq!(format_token_count(500), "500");
    assert_eq!(format_token_count(9600), "9.6k");
    assert_eq!(format_token_count(1000), "1.0k");
}

#[test]
fn format_token_count_abbreviates_above_1_000_000_as_m() {
    assert_eq!(format_token_count(1_000_000), "1.0M");
    assert_eq!(format_token_count(1_614_700), "1.6M");
}

/// A count that rounds up to the next unit is promoted rather than rendered as
/// "1000.0k".
#[test]
fn format_token_count_promotes_counts_that_round_up_to_the_next_unit() {
    assert_eq!(format_token_count(999_999), "1.0M");
    assert_eq!(format_token_count(999_500), "1.0M");
    assert_eq!(format_token_count(999_499), "999.5k");
}

#[test]
fn exact_token_count_tooltip_is_none_below_abbreviation_threshold() {
    assert_eq!(exact_token_count_tooltip(500), None);
    assert_eq!(exact_token_count_tooltip(999), None);
}

#[test]
fn exact_token_count_tooltip_shows_comma_separated_count_when_abbreviated() {
    assert_eq!(
        exact_token_count_tooltip(9614),
        Some("9,614 tokens".to_string())
    );
    assert_eq!(
        exact_token_count_tooltip(1_614_700),
        Some("1,614,700 tokens".to_string())
    );
}

#[test]
fn format_tokens_and_cost_joins_tokens_and_dollar_with_a_slash() {
    assert_eq!(
        format_tokens_and_cost(Some(9600), Some(36.0)),
        "9.6k tokens / $0.36"
    );
}

#[test]
fn format_tokens_and_cost_omits_dollar_suffix_when_cost_is_unknown() {
    assert_eq!(format_tokens_and_cost(Some(9600), None), "9.6k tokens");
}

#[test]
fn format_tokens_and_cost_falls_back_to_cost_only_when_tokens_are_unknown() {
    assert_eq!(format_tokens_and_cost(None, Some(36.0)), "$0.36");
}

#[test]
fn format_tokens_and_cost_shows_em_dash_when_both_are_unknown() {
    assert_eq!(format_tokens_and_cost(None, None), EM_DASH);
}

#[test]
fn format_cost_only_shows_em_dash_when_cost_is_unknown() {
    assert_eq!(format_cost_only(None), EM_DASH);
    assert_eq!(format_cost_only(Some(36.0)), "$0.36");
}

#[test]
fn format_searches_and_cost_appends_dollar_suffix() {
    assert_eq!(format_searches_and_cost(3, 2.0), "3 searches / $0.02");
}
