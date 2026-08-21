use super::*;

#[test]
fn color_for_model_is_deterministic() {
    assert_eq!(color_for_model("gpt-5.5"), color_for_model("gpt-5.5"));
}

#[test]
fn color_for_model_differs_across_distinct_models_in_practice() {
    // Not a strict guarantee (hash collisions are possible with only 6
    // buckets), but with these particular sample ids we expect at least
    // one pair to differ, guarding against an accidental constant return.
    let colors: Vec<_> = ["gpt-5.5", "gpt-5.3-codex", "auto", "kimi-k2.6"]
        .iter()
        .map(|id| color_for_model(id))
        .collect();
    assert!(colors.iter().any(|c| *c != colors[0]));
}

#[test]
fn context_window_category_colors_are_fixed_not_hashed() {
    // Calling twice for the same bucket must always return the same color,
    // and named categories must not collide with the "Other" neutral color.
    assert_eq!(
        color_for_context_window_category(ContextWindowSegmentType::ConversationHistory),
        color_for_context_window_category(ContextWindowSegmentType::ConversationHistory)
    );
    assert_ne!(
        color_for_context_window_category(ContextWindowSegmentType::Rules),
        color_for_context_window_category(ContextWindowSegmentType::Other)
    );
}

#[test]
fn unnamed_segment_types_fold_into_the_other_color() {
    let other = color_for_context_window_category(ContextWindowSegmentType::Other);
    assert_eq!(
        color_for_context_window_category(ContextWindowSegmentType::Unknown),
        other
    );
    assert_eq!(
        color_for_context_window_category(ContextWindowSegmentType::LatestInput),
        other
    );
    assert_eq!(
        color_for_context_window_category(ContextWindowSegmentType::Images),
        other
    );
}

#[test]
fn all_six_named_categories_get_distinct_colors() {
    let categories = [
        ContextWindowSegmentType::ConversationHistory,
        ContextWindowSegmentType::SystemPrompt,
        ContextWindowSegmentType::ToolDefinitions,
        ContextWindowSegmentType::Rules,
        ContextWindowSegmentType::Skills,
        ContextWindowSegmentType::Memory,
    ];
    for (i, a) in categories.iter().enumerate() {
        for b in &categories[i + 1..] {
            assert_ne!(
                color_for_context_window_category(*a),
                color_for_context_window_category(*b),
                "{a:?} and {b:?} should not share a color"
            );
        }
    }
}
