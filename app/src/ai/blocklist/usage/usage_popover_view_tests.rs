use std::collections::HashMap;

use super::*;
use crate::settings::UsageDisplayUnit;

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

fn charged_usage_with_input_cost(cost_in_cents: f32) -> ModelChargedUsage {
    ModelChargedUsage {
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
    assert_eq!(gpt_row.cost, Some(CostValue::new(0.0, 36.0)));
    assert!(gpt_row.charged_usage.is_some());
    assert_eq!(codex_row.cost, None);
    assert!(codex_row.charged_usage.is_none());
}

/// A model the server charged but that has no token-usage row still gets a
/// row, so the section can account for every charge it displays.
#[test]
fn model_usage_rows_adds_charged_models_missing_from_token_usage() {
    let charged_usage = ModelChargedUsage {
        input_tokens: 700,
        input_cost_in_cents: 3.0,
        ..Default::default()
    };
    let charged_usage_by_model = HashMap::from([("charged-only-model".to_string(), charged_usage)]);

    let rows = model_usage_rows(&[], &charged_usage_by_model);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].model_id, "charged-only-model");
    assert_eq!(rows[0].tokens, 700);
    assert_eq!(rows[0].role, None);
}

/// The row total must equal the sum of the breakdown rows shown beneath it,
/// including cache buckets and web search.
#[test]
fn model_usage_row_totals_match_the_charged_usage_breakdown_rows() {
    let charged_usage = ModelChargedUsage {
        input_tokens: 1_000,
        output_tokens: 500,
        input_cache_read_tokens: 300,
        input_cache_write_tokens: 200,
        input_cost_in_cents: 10.0,
        output_cost_in_cents: 20.0,
        input_cache_read_cost_in_cents: 3.0,
        input_cache_write_cost_in_cents: 2.0,
        web_search_count: 2,
        web_search_cost_in_cents: 5.0,
        ..Default::default()
    };
    let models = vec![model("gpt-5.5", 42, PRIMARY_AGENT_CATEGORY)];
    let charged_usage_by_model = HashMap::from([("gpt-5.5".to_string(), charged_usage)]);

    let rows = model_usage_rows(&models, &charged_usage_by_model);

    assert_eq!(rows[0].tokens, 1_000 + 500 + 300 + 200);
    assert_eq!(
        rows[0].cost,
        Some(CostValue::new(0.0, 10.0 + 20.0 + 3.0 + 2.0 + 5.0))
    );
}

/// Without attributed charges there is no breakdown to reconcile against, so
/// the row falls back to the raw reported token count.
#[test]
fn model_usage_row_falls_back_to_reported_tokens_without_charged_usage() {
    let models = vec![model("gpt-5.5", 100, PRIMARY_AGENT_CATEGORY)];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(rows[0].tokens, 100);
    assert_eq!(rows[0].cost, None);
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
    assert_eq!(totals.cost, Some(CostValue::new(0.0, 50.0)));
}

#[test]
fn row_totals_cost_is_unknown_when_no_row_has_an_attributed_cost() {
    let models = vec![model("gpt-5.5", 100, PRIMARY_AGENT_CATEGORY)];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(RowTotals::of_model_rows(&rows).cost, None);
}

fn request_metadata_message(charges: api::RequestCharges) -> api::Message {
    api::Message {
        message: Some(api::message::Message::RequestMetadata(
            api::message::RequestMetadata {
                charges: Some(charges),
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

fn inference_usage(
    input_tokens: u32,
    output_tokens: u32,
    input_cost_in_cents: f32,
    output_cost_in_cents: f32,
) -> api::InferenceUsage {
    api::InferenceUsage {
        token_count: Some(api::TokenCount {
            input: input_tokens,
            output: output_tokens,
            ..Default::default()
        }),
        token_cost: Some(api::TokenCost {
            input_cost_in_cents,
            output_cost_in_cents,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn charged_usage(
    direct: HashMap<String, api::InferenceUsage>,
    byok: HashMap<String, api::InferenceUsage>,
    custom_endpoint: HashMap<String, api::InferenceUsage>,
) -> api::ChargedUsage {
    api::ChargedUsage {
        direct_api_inference_usage: direct,
        byok_inference_usage: byok,
        custom_endpoint_inference_usage: custom_endpoint,
        ..Default::default()
    }
}

fn single_model_usage(
    model_id: &str,
    input_tokens: u32,
    input_cost_in_cents: f32,
) -> HashMap<String, api::InferenceUsage> {
    HashMap::from([(
        model_id.to_string(),
        inference_usage(input_tokens, 0, input_cost_in_cents, 0.0),
    )])
}

/// Per-model rows are derived at render time by folding every persisted
/// request's nested category → model → usage values together.
#[test]
fn sum_charged_usage_by_model_folds_the_nested_category_and_usage_type_maps() {
    #[allow(clippy::type_complexity)]
    let cases: [(&str, Vec<api::Message>, Vec<(&str, u64, f32)>); 4] = [
        // Sum across categories for one model.
        (
            "sums across categories",
            vec![request_metadata_message(api::RequestCharges {
                usage_by_category: HashMap::from([
                    (
                        PRIMARY_AGENT_CATEGORY.to_string(),
                        charged_usage(
                            single_model_usage("gpt-5.5", 100, 10.0),
                            HashMap::new(),
                            HashMap::new(),
                        ),
                    ),
                    (
                        "compaction".to_string(),
                        charged_usage(
                            single_model_usage("gpt-5.5", 30, 5.0),
                            HashMap::new(),
                            HashMap::new(),
                        ),
                    ),
                ]),
            })],
            vec![("gpt-5.5", 130, 15.0)],
        ),
        // Warp, BYOK, and custom-endpoint charges for the same model id all
        // land on one row.
        (
            "sums across usage types",
            vec![request_metadata_message(api::RequestCharges {
                usage_by_category: HashMap::from([(
                    PRIMARY_AGENT_CATEGORY.to_string(),
                    charged_usage(
                        single_model_usage("gpt-5.5", 100, 10.0),
                        single_model_usage("gpt-5.5", 20, 2.0),
                        single_model_usage("config-key", 7, 0.7),
                    ),
                )]),
            })],
            vec![("gpt-5.5", 120, 12.0), ("labeled-config-key", 7, 0.7)],
        ),
        // Several requests accumulate on the same per-model totals.
        (
            "accumulates across records",
            vec![
                request_metadata_message(api::RequestCharges {
                    usage_by_category: HashMap::from([(
                        PRIMARY_AGENT_CATEGORY.to_string(),
                        charged_usage(
                            single_model_usage("gpt-5.5", 100, 10.0),
                            HashMap::new(),
                            HashMap::new(),
                        ),
                    )]),
                }),
                request_metadata_message(api::RequestCharges {
                    usage_by_category: HashMap::from([(
                        "compaction".to_string(),
                        charged_usage(
                            single_model_usage("gpt-5.5", 50, 6.0),
                            HashMap::new(),
                            HashMap::new(),
                        ),
                    )]),
                }),
            ],
            vec![("gpt-5.5", 150, 16.0)],
        ),
        // Messages without a RequestMetadata payload or without charges
        // contribute nothing.
        (
            "ignores messages without charged usage",
            vec![
                api::Message::default(),
                request_metadata_message(api::RequestCharges::default()),
            ],
            vec![],
        ),
    ];

    for (name, messages, expected_models) in cases {
        let sums = sum_charged_usage_by_model(messages.iter(), |config_key| {
            format!("labeled-{config_key}")
        });
        assert_eq!(sums.len(), expected_models.len(), "{name}");
        for (model_id, tokens, cost_in_cents) in expected_models {
            let usage = sums.get(model_id).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(usage.tokens(), tokens, "{name}: {model_id}");
            assert!(
                (usage.cost().cost_in_cents - cost_in_cents).abs() < 1e-4,
                "{name}: {model_id}"
            );
        }
    }
}

/// Web-search charges ride outside the token buckets and must survive the
/// fold.
#[test]
fn sum_charged_usage_by_model_includes_web_search_charges() {
    let messages = [request_metadata_message(api::RequestCharges {
        usage_by_category: HashMap::from([(
            PRIMARY_AGENT_CATEGORY.to_string(),
            charged_usage(
                HashMap::from([(
                    "gpt-5.5".to_string(),
                    api::InferenceUsage {
                        web_search_count: 3,
                        web_search_cost_in_cents: 2.0,
                        web_search_cost_in_credits: 0.4,
                        ..Default::default()
                    },
                )]),
                HashMap::new(),
                HashMap::new(),
            ),
        )]),
    })];

    let sums = sum_charged_usage_by_model(messages.iter(), |key| key.to_string());
    let usage = sums.get("gpt-5.5").unwrap();
    assert_eq!(usage.web_search_count, 3);
    assert_eq!(usage.web_search_cost(), CostValue::new(0.4, 2.0));
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
fn format_tokens_and_cost_joins_tokens_and_cost_with_a_slash() {
    let cases: [(&str, UsageDisplayUnit, &str); 2] = [
        (
            "credits unit",
            UsageDisplayUnit::Credits,
            "9.6k tokens / 36 credits",
        ),
        (
            "dollars unit",
            UsageDisplayUnit::Dollars,
            "9.6k tokens / $0.36",
        ),
    ];
    for (name, unit, expected) in cases {
        assert_eq!(
            format_tokens_and_cost(Some(9600), Some(CostValue::new(36.0, 36.0)), unit),
            expected,
            "{name}"
        );
    }
}

#[test]
fn format_tokens_and_cost_omits_cost_suffix_when_cost_is_unknown() {
    assert_eq!(
        format_tokens_and_cost(Some(9600), None, UsageDisplayUnit::Dollars),
        "9.6k tokens"
    );
}

#[test]
fn format_tokens_and_cost_falls_back_to_cost_only_when_tokens_are_unknown() {
    assert_eq!(
        format_tokens_and_cost(
            None,
            Some(CostValue::new(36.0, 36.0)),
            UsageDisplayUnit::Dollars
        ),
        "$0.36"
    );
}

#[test]
fn format_tokens_and_cost_shows_em_dash_when_both_are_unknown() {
    assert_eq!(
        format_tokens_and_cost(None, None, UsageDisplayUnit::Dollars),
        EM_DASH
    );
}

#[test]
fn format_dollars_renders_sub_cent_amounts_as_less_than_a_cent() {
    let cases: [(f32, &str); 4] = [
        (0.0, "$0.00"),
        (0.5, "<$0.01"),
        (1.0, "$0.01"),
        (36.0, "$0.36"),
    ];
    for (cost_in_cents, expected) in cases {
        assert_eq!(format_dollars(cost_in_cents), expected);
    }
}

#[test]
fn format_searches_and_cost_appends_cost_suffix() {
    let cases: [(&str, UsageDisplayUnit, &str); 2] = [
        (
            "credits unit",
            UsageDisplayUnit::Credits,
            "3 searches / 2 credits",
        ),
        (
            "dollars unit",
            UsageDisplayUnit::Dollars,
            "3 searches / $0.02",
        ),
    ];
    for (name, unit, expected) in cases {
        assert_eq!(
            format_searches_and_cost(3, CostValue::new(2.0, 2.0), unit),
            expected,
            "{name}"
        );
    }
}

/// A conversation whose usage metadata carries no cost figures at all renders
/// an em dash rather than a fabricated zero.
#[test]
fn conversation_total_text_shows_em_dash_without_usage_data() {
    let conversation = AIConversation::new(false, false);
    assert_eq!(
        conversation_total_text(&conversation, UsageDisplayUnit::Credits),
        EM_DASH
    );
}

#[test]
fn conversation_total_text_uses_the_charged_usage_totals() {
    let mut conversation = AIConversation::new(false, false);
    conversation.set_credits_spent_for_test(12.5);
    conversation.set_charged_usage_for_test(Some(crate::persistence::model::ChargedUsageTotals {
        input_cost_in_cents: 21.0,
        input_cost_in_credits: 2.1,
        platform_cost_in_cents: 79.0,
        platform_cost_in_credits: 7.9,
        ..Default::default()
    }));

    assert_eq!(
        conversation_total_text(&conversation, UsageDisplayUnit::Credits),
        "10 credits"
    );
    assert_eq!(
        conversation_total_text(&conversation, UsageDisplayUnit::Dollars),
        "$1.00"
    );
}
