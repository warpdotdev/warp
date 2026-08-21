use std::collections::HashMap;

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::usage::rollup::{AgentAvatar, PerAgentCreditEntry};

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
    let rows = model_usage_rows(&models);
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
    let rows = model_usage_rows(&models);
    assert_eq!(rows[0].model_id, "primary-model");
    assert_eq!(rows[0].role_badge, Some("Primary agent"));
}

#[test]
fn model_usage_rows_assigns_full_terminal_use_badge() {
    let models = vec![model("codex-model", 50, FULL_TERMINAL_USE_CATEGORY)];
    let rows = model_usage_rows(&models);
    assert_eq!(rows[0].role_badge, Some("Full terminal use"));
}

#[test]
fn model_usage_rows_has_no_badge_for_unknown_categories() {
    let models = vec![model("auto-model", 10, "some_other_category")];
    let rows = model_usage_rows(&models);
    assert_eq!(rows[0].role_badge, None);
}

#[test]
fn context_window_display_rows_empty_when_no_usage() {
    assert!(context_window_display_rows(0.0, &[]).is_empty());
    assert!(
        context_window_display_rows(
            0.1,
            &[ContextWindowSegment {
                segment_type: ContextWindowSegmentType::SystemPrompt,
                token_count: 0,
            }]
        )
        .is_empty()
    );
}

#[test]
fn context_window_display_rows_folds_unnamed_segments_into_other() {
    let segments = vec![
        ContextWindowSegment {
            segment_type: ContextWindowSegmentType::LatestInput,
            token_count: 50,
        },
        ContextWindowSegment {
            segment_type: ContextWindowSegmentType::Images,
            token_count: 50,
        },
        ContextWindowSegment {
            segment_type: ContextWindowSegmentType::SystemPrompt,
            token_count: 100,
        },
    ];
    let rows = context_window_display_rows(0.1, &segments);
    let other_row = rows
        .iter()
        .find(|r| r.bucket == ContextWindowSegmentType::Other)
        .expect("Other bucket should exist");
    assert_eq!(other_row.token_count, 100);
}

#[test]
fn context_window_display_rows_sorts_other_last() {
    let segments = vec![
        ContextWindowSegment {
            segment_type: ContextWindowSegmentType::Other,
            token_count: 1000,
        },
        ContextWindowSegment {
            segment_type: ContextWindowSegmentType::SystemPrompt,
            token_count: 10,
        },
    ];
    let rows = context_window_display_rows(0.5, &segments);
    assert_eq!(rows.last().unwrap().bucket, ContextWindowSegmentType::Other);
}

#[test]
fn context_window_display_rows_percentages_sum_to_overall_usage() {
    let segments = vec![
        ContextWindowSegment {
            segment_type: ContextWindowSegmentType::SystemPrompt,
            token_count: 25,
        },
        ContextWindowSegment {
            segment_type: ContextWindowSegmentType::Rules,
            token_count: 75,
        },
    ];
    let rows = context_window_display_rows(0.2, &segments);
    let total_pct: f32 = rows.iter().map(|r| r.pct).sum();
    assert!((total_pct - 20.0).abs() < 0.01, "expected ~20%, got {total_pct}");
}

fn per_agent_entry(name: &str, credits: f32) -> PerAgentCreditEntry {
    PerAgentCreditEntry {
        conversation_id: AIConversationId::new(),
        display_name: name.to_string(),
        avatar: AgentAvatar::Child,
        credits_spent: credits,
    }
}

#[test]
fn truncate_rollup_rows_shows_all_under_cap() {
    let entries: Vec<_> = (0..3).map(|i| per_agent_entry(&format!("agent-{i}"), 1.0)).collect();
    let (shown, hidden) = truncate_rollup_rows(&entries, false);
    assert_eq!(shown.len(), 3);
    assert_eq!(hidden, 0);
}

#[test]
fn truncate_rollup_rows_truncates_over_cap_until_show_all() {
    let entries: Vec<_> = (0..8).map(|i| per_agent_entry(&format!("agent-{i}"), 1.0)).collect();
    let (shown, hidden) = truncate_rollup_rows(&entries, false);
    assert_eq!(shown.len(), ROLLUP_TRUNCATION_CAP);
    assert_eq!(hidden, 3);

    let (shown_all, hidden_all) = truncate_rollup_rows(&entries, true);
    assert_eq!(shown_all.len(), 8);
    assert_eq!(hidden_all, 0);
}

#[test]
fn format_token_count_abbreviates_above_1000() {
    assert_eq!(format_token_count(500), "500");
    assert_eq!(format_token_count(9600), "9.6k");
    assert_eq!(format_token_count(1000), "1.0k");
}
