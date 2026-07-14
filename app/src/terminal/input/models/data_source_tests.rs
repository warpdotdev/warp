use std::collections::HashSet;

use fuzzy_match::match_indices_case_insensitive;

use super::collapse_reasoning_variants;
use crate::ai::llms::{LLMId, LLMInfo};

/// Builds an `LLMInfo` fixture with the given id, display name, base model name,
/// and optional reasoning level. Provider/spec/icons are left at their test
/// defaults — the collapse helper only inspects id, display name, base model
/// name, and reasoning level.
fn llm(
    id: &str,
    display_name: &str,
    base_model_name: &str,
    reasoning_level: Option<&str>,
) -> LLMInfo {
    let mut info = LLMInfo::new_for_test(display_name);
    info.id = LLMId::from(id);
    info.base_model_name = base_model_name.to_string();
    info.reasoning_level = reasoning_level.map(str::to_string);
    info
}

fn terra_family() -> Vec<LLMInfo> {
    vec![
        llm(
            "gpt-5.6-terra-low",
            "gpt-5.6-terra (low)",
            "gpt-5.6-terra",
            Some("low"),
        ),
        llm(
            "gpt-5.6-terra-medium",
            "gpt-5.6-terra (medium)",
            "gpt-5.6-terra",
            Some("medium"),
        ),
        llm(
            "gpt-5.6-terra-high",
            "gpt-5.6-terra (high)",
            "gpt-5.6-terra",
            Some("high"),
        ),
        llm(
            "gpt-5.6-terra-xhigh",
            "gpt-5.6-terra (xhigh)",
            "gpt-5.6-terra",
            Some("xhigh"),
        ),
    ]
}

/// Collapse produces exactly one group per reasoning base model, one `auto`
/// group, and one group per non-reasoning model id.
#[test]
fn collapse_reasoning_variants_groups_by_base_name() {
    let mut choices = terra_family();
    choices.push(llm("claude-haiku", "claude-haiku", "claude-haiku", None));
    choices.push(llm("auto-fast", "auto", "auto", None));
    choices.push(llm("auto-smart", "auto (smart)", "auto (smart)", None));
    let refs: Vec<&LLMInfo> = choices.iter().collect();

    let active = LLMId::from("claude-haiku");
    let groups = collapse_reasoning_variants(&refs, &active, &HashSet::new());

    // One group for the terra family, one for haiku, and a single auto group.
    assert_eq!(groups.len(), 3);

    let terra = groups
        .iter()
        .find(|group| group.base_name == "gpt-5.6-terra")
        .expect("terra group exists");
    assert_eq!(terra.variants.len(), 4);
    assert_eq!(
        terra
            .variants
            .iter()
            .map(|v| v.level.clone())
            .collect::<Vec<_>>(),
        vec!["low", "medium", "high", "xhigh"]
    );
    assert!(terra.has_reasoning_sidecar);
    assert!(!terra.is_auto);

    let haiku = groups
        .iter()
        .find(|group| group.base_name == "claude-haiku")
        .expect("haiku group exists");
    assert!(haiku.variants.is_empty());
    assert!(!haiku.has_reasoning_sidecar);
    assert!(!haiku.is_auto);

    let auto = groups
        .iter()
        .find(|group| group.is_auto)
        .expect("auto group exists");
    assert!(auto.variants.is_empty());
    assert!(!auto.has_reasoning_sidecar);
    assert_eq!(auto.base_name, "auto");
}

/// The sidecar exposes the family's levels in server order with the correct
/// per-level `LLMId`s and the active variant flagged.
#[test]
fn sidecar_lists_levels_in_order_with_active_flagged() {
    let choices = terra_family();
    let refs: Vec<&LLMInfo> = choices.iter().collect();

    let active = LLMId::from("gpt-5.6-terra-high");
    let groups = collapse_reasoning_variants(&refs, &active, &HashSet::new());
    let terra = &groups[0];

    assert_eq!(terra.active_variant_index, Some(2));
    assert_eq!(terra.variants[2].level, "high");
    assert_eq!(terra.variants[2].id, LLMId::from("gpt-5.6-terra-high"));
    assert_eq!(
        terra
            .variants
            .iter()
            .map(|v| v.id.clone())
            .collect::<Vec<_>>(),
        vec![
            LLMId::from("gpt-5.6-terra-low"),
            LLMId::from("gpt-5.6-terra-medium"),
            LLMId::from("gpt-5.6-terra-high"),
            LLMId::from("gpt-5.6-terra-xhigh"),
        ]
    );
}

/// Accepting a collapsed row resolves to the active variant when the family is
/// already selected.
#[test]
fn accept_resolves_to_active_variant_when_family_selected() {
    let choices = terra_family();
    let refs: Vec<&LLMInfo> = choices.iter().collect();

    let active = LLMId::from("gpt-5.6-terra-medium");
    let groups = collapse_reasoning_variants(&refs, &active, &HashSet::new());
    let terra = &groups[0];

    assert_eq!(terra.active_variant_index, Some(1));
    assert_eq!(terra.target_variant_index, 1);
    assert_eq!(terra.target_id(), LLMId::from("gpt-5.6-terra-medium"));
    assert_eq!(terra.representative_id, LLMId::from("gpt-5.6-terra-medium"));
    assert!(terra.is_active);
}

/// Accepting a collapsed row resolves to a deterministic default (the first
/// listed level) when the family is not currently selected.
#[test]
fn accept_resolves_to_default_when_family_not_selected() {
    let choices = terra_family();
    let refs: Vec<&LLMInfo> = choices.iter().collect();

    let active = LLMId::from("some-unrelated-model");
    let groups = collapse_reasoning_variants(&refs, &active, &HashSet::new());
    let terra = &groups[0];

    assert_eq!(terra.active_variant_index, None);
    assert_eq!(terra.target_variant_index, 0);
    assert_eq!(terra.target_id(), LLMId::from("gpt-5.6-terra-low"));
    assert!(!terra.is_active);
}

/// Fuzzy search matches the collapsed base label (one result for the family,
/// not one per level), and a base-name substring still matches.
#[test]
fn collapsed_search_matches_base_name_not_per_variant() {
    let choices = terra_family();
    let refs: Vec<&LLMInfo> = choices.iter().collect();

    let active = LLMId::from("gpt-5.6-terra-low");
    let groups = collapse_reasoning_variants(&refs, &active, &HashSet::new());

    // Exactly one group carries the terra base label (not four per-level rows).
    let terra_groups = groups
        .iter()
        .filter(|group| group.search_label == "gpt-5.6-terra")
        .count();
    assert_eq!(terra_groups, 1);

    // The base label matches a `terra` query and a `5.6` substring.
    assert!(match_indices_case_insensitive("gpt-5.6-terra", "terra").is_some());
    assert!(match_indices_case_insensitive("gpt-5.6-terra", "5.6").is_some());
    // A per-variant suffix query like `(high)` does not match the collapsed
    // base label — the family is still surfaced via the base-name match above.
    assert!(match_indices_case_insensitive("gpt-5.6-terra", "high").is_none());
}

/// Non-reasoning and auto models collapse to single entries with no reasoning
/// sidecar, mirroring the dropdown.
#[test]
fn non_reasoning_and_auto_collapse_to_single_entries() {
    let choices = vec![
        llm("claude-haiku", "claude-haiku", "claude-haiku", None),
        llm("claude-sonnet", "claude-sonnet", "claude-sonnet", None),
        llm("auto-fast", "auto", "auto", None),
        llm("auto-smart", "auto (smart)", "auto (smart)", None),
    ];
    let refs: Vec<&LLMInfo> = choices.iter().collect();

    let active = LLMId::from("claude-haiku");
    let groups = collapse_reasoning_variants(&refs, &active, &HashSet::new());

    // One group per non-reasoning id (haiku, sonnet) plus a single auto group.
    assert_eq!(groups.len(), 3);
    let non_reasoning = groups
        .iter()
        .filter(|group| !group.is_auto && group.variants.is_empty())
        .count();
    assert_eq!(non_reasoning, 2);
    for group in &groups {
        assert!(!group.has_reasoning_sidecar);
    }
}

/// Custom-endpoint models are never collapsed by base name — each renders as its
/// own single row, as it does today.
#[test]
fn custom_endpoint_models_are_not_collapsed() {
    let choices = vec![
        llm(
            "custom-terra-low",
            "custom-terra (low)",
            "custom-terra",
            Some("low"),
        ),
        llm(
            "custom-terra-high",
            "custom-terra (high)",
            "custom-terra",
            Some("high"),
        ),
    ];
    let refs: Vec<&LLMInfo> = choices.iter().collect();

    let mut custom_endpoint_ids = HashSet::new();
    custom_endpoint_ids.insert(LLMId::from("custom-terra-low"));
    custom_endpoint_ids.insert(LLMId::from("custom-terra-high"));

    let active = LLMId::from("custom-terra-low");
    let groups = collapse_reasoning_variants(&refs, &active, &custom_endpoint_ids);

    // Each custom-endpoint model is its own group (not collapsed by base name).
    assert_eq!(groups.len(), 2);
    for group in &groups {
        assert!(!group.has_reasoning_sidecar);
        assert!(group.variants.is_empty());
    }
}

/// A reasoning family with a single variant collapses to one entry but does not
/// render a sidecar (there is only one level to choose).
#[test]
fn single_variant_reasoning_family_has_no_sidecar() {
    let choices = vec![llm(
        "solo-low",
        "solo-model (low)",
        "solo-model",
        Some("low"),
    )];
    let refs: Vec<&LLMInfo> = choices.iter().collect();

    let active = LLMId::from("solo-low");
    let groups = collapse_reasoning_variants(&refs, &active, &HashSet::new());
    let solo = &groups[0];

    assert_eq!(solo.base_name, "solo-model");
    assert_eq!(solo.variants.len(), 1);
    assert!(!solo.has_reasoning_sidecar);
    assert_eq!(solo.target_id(), LLMId::from("solo-low"));
}
