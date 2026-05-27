use std::collections::HashMap;

use super::ui_helpers::context_window_snap_values;
use crate::ai::execution_profiles::{
    has_effective_configurable_context_window, sanitize_context_window_limit_for_request,
    should_show_long_context_pricing_warning,
};
use crate::ai::llms::{
    LLMContextWindow, LLMInfo, LLMModelHost, LLMProvider, LLMUsageMetadata, RoutingHostConfig,
};

fn configurable_model(provider: LLMProvider, direct_host_enabled: bool) -> LLMInfo {
    LLMInfo {
        display_name: "test model".to_string(),
        base_model_name: "test model".to_string(),
        id: "test-model".into(),
        reasoning_level: None,
        usage_metadata: LLMUsageMetadata {
            request_multiplier: 1,
            credit_multiplier: None,
        },
        description: None,
        disable_reason: None,
        vision_supported: false,
        spec: None,
        provider,
        host_configs: HashMap::from([(
            LLMModelHost::DirectApi,
            RoutingHostConfig {
                enabled: direct_host_enabled,
                model_routing_host: LLMModelHost::DirectApi,
            },
        )]),
        discount_percentage: None,
        context_window: LLMContextWindow {
            is_configurable: true,
            min: 200_000,
            max: 1_000_000,
            default_max: 272_000,
        },
    }
}

/// Helper: round-trip f32 → u32 for readable assertions and absorb the
/// negligible f64→f32 drift the snap helper picks up on large ranges.
fn rounded(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.round() as u32).collect()
}

#[test]
fn snap_values_for_min_eq_max_returns_single_point() {
    assert_eq!(
        rounded(&context_window_snap_values(50_000, 50_000)),
        vec![50_000]
    );
}

#[test]
fn snap_values_for_min_gt_max_collapses_to_min() {
    // Defensive: invalid bounds shouldn't panic, just degrade gracefully.
    assert_eq!(rounded(&context_window_snap_values(100, 50)), vec![100]);
}

#[test]
fn snap_values_always_include_endpoints() {
    let values = rounded(&context_window_snap_values(1_000, 200_000));
    assert_eq!(values.first(), Some(&1_000));
    assert_eq!(values.last(), Some(&200_000));
}

#[test]
fn snap_values_for_classic_200k_range_match_legacy_layout() {
    // Mirrors the old hardcoded list, except `1_000` replaces the missing
    // round multiple at the start.
    let values = rounded(&context_window_snap_values(1_000, 200_000));
    assert_eq!(
        values,
        vec![1_000, 25_000, 50_000, 75_000, 100_000, 125_000, 150_000, 175_000, 200_000]
    );
}

#[test]
fn snap_values_for_claude_1m_range_pick_100k_steps() {
    let values = rounded(&context_window_snap_values(200_000, 1_000_000));
    assert_eq!(
        values,
        vec![200_000, 300_000, 400_000, 500_000, 600_000, 700_000, 800_000, 900_000, 1_000_000]
    );
}

#[test]
fn snap_values_for_min_zero_skips_duplicate_zero() {
    let values = rounded(&context_window_snap_values(0, 100));
    // First entry is min (0), then nice multiples up to and including max.
    assert_eq!(values.first(), Some(&0));
    assert_eq!(values.last(), Some(&100));
    assert!(values.iter().filter(|&&v| v == 0).count() == 1);
}

#[test]
fn snap_values_for_offset_min_align_to_nice_grid() {
    // min=26_000 doesn't sit on a 25k boundary; first nice value is 50_000.
    let values = rounded(&context_window_snap_values(26_000, 200_000));
    assert_eq!(values.first(), Some(&26_000));
    assert_eq!(values.last(), Some(&200_000));
    // Ensure the second point lands on a nice multiple, not on min+step.
    assert_eq!(values.get(1), Some(&50_000));
}

#[test]
fn snap_values_keep_count_reasonable_for_huge_range() {
    // 1B span should still produce a small (~9) snap-point list, not
    // millions of entries.
    let values = context_window_snap_values(0, 1_000_000_000);
    assert!(
        values.len() <= 12,
        "expected at most 12 snap points, got {}",
        values.len()
    );
    assert!(
        values.len() >= 5,
        "expected at least 5 snap points, got {}",
        values.len()
    );
}

#[test]
fn openai_direct_long_context_warning_starts_above_threshold() {
    let model = configurable_model(LLMProvider::OpenAI, true);

    assert!(!should_show_long_context_pricing_warning(
        &model,
        Some(200_000),
        false,
        true
    ));
    assert!(!should_show_long_context_pricing_warning(
        &model,
        Some(272_000),
        false,
        true
    ));
    assert!(should_show_long_context_pricing_warning(
        &model,
        Some(272_001),
        false,
        true
    ));
}

#[test]
fn openai_direct_request_limit_is_clamped_when_expanded_context_is_available() {
    let model = configurable_model(LLMProvider::OpenAI, true);

    assert_eq!(
        sanitize_context_window_limit_for_request(&model, Some(1_500_000), false, true),
        Some(1_000_000)
    );
}

#[test]
fn custom_endpoint_fixed_context_does_not_expose_control_or_warning() {
    let mut model = configurable_model(LLMProvider::Unknown, false);
    model.context_window.is_configurable = false;
    model.context_window.max = 200_000;

    assert!(!has_effective_configurable_context_window(
        &model, false, false
    ));
    assert_eq!(
        sanitize_context_window_limit_for_request(&model, Some(1_000_000), false, false),
        None
    );
    assert!(!should_show_long_context_pricing_warning(
        &model,
        Some(1_000_000),
        false,
        false
    ));
}

#[test]
fn openai_byok_suppresses_expanded_control_and_stale_limit_warning() {
    let model = configurable_model(LLMProvider::OpenAI, true);

    assert!(!has_effective_configurable_context_window(
        &model, true, true
    ));
    assert_eq!(
        sanitize_context_window_limit_for_request(&model, Some(1_000_000), true, true),
        None
    );
    assert!(!should_show_long_context_pricing_warning(
        &model,
        Some(1_000_000),
        true,
        true
    ));
}

#[test]
fn openai_without_direct_host_suppresses_expanded_control_and_warning() {
    let model = configurable_model(LLMProvider::OpenAI, false);

    assert!(!has_effective_configurable_context_window(
        &model, false, true
    ));
    assert_eq!(
        sanitize_context_window_limit_for_request(&model, Some(1_000_000), false, true),
        None
    );
    assert!(!should_show_long_context_pricing_warning(
        &model,
        Some(1_000_000),
        false,
        true
    ));
}

#[test]
fn openai_expanded_context_is_hidden_while_feature_flag_is_off() {
    let model = configurable_model(LLMProvider::OpenAI, true);

    assert!(!has_effective_configurable_context_window(
        &model, false, false
    ));
    assert_eq!(
        sanitize_context_window_limit_for_request(&model, Some(1_000_000), false, false),
        None
    );
    assert!(!should_show_long_context_pricing_warning(
        &model,
        Some(1_000_000),
        false,
        false
    ));
}

#[test]
fn non_openai_configurable_context_ignores_gpt_flag_and_does_not_show_openai_warning() {
    let model = configurable_model(LLMProvider::Anthropic, true);

    assert!(has_effective_configurable_context_window(
        &model, false, false
    ));
    assert_eq!(
        sanitize_context_window_limit_for_request(&model, Some(1_000_000), false, false),
        Some(1_000_000)
    );
    assert!(!should_show_long_context_pricing_warning(
        &model,
        Some(1_000_000),
        false,
        false
    ));
}
