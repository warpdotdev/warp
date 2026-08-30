use super::*;

#[test]
#[ignore = "CORE-3768 - need to clean up PREVIEW_FLAGS, but this is a temporary fix for the cluttered changelog"]
fn test_all_preview_flags_have_a_description() {
    for flag in PREVIEW_FLAGS {
        assert!(
            flag.flag_description()
                .is_some_and(|description| !description.is_empty()),
            "Missing description for preview-enabled flag {flag:?}"
        );
    }
}

#[test]
fn local_child_harnesses_are_local_only_by_default() {
    assert!(LOCAL_FLAGS.contains(&FeatureFlag::LocalClaudeCodexChildHarnesses));
    assert!(!DEBUG_FLAGS.contains(&FeatureFlag::LocalClaudeCodexChildHarnesses));
    assert!(!DOGFOOD_FLAGS.contains(&FeatureFlag::LocalClaudeCodexChildHarnesses));
}

#[test]
fn history_search_prior_ranking_can_be_disabled_at_runtime_without_a_rebuild() {
    // Ships enabled via `DOGFOOD_FLAGS`, so it must also be in `RUNTIME_FEATURE_FLAGS` --
    // otherwise dogfood users would have no way to turn the new ranking off quickly if it
    // doesn't work out, short of editing this array and shipping another dogfood build.
    assert!(DOGFOOD_FLAGS.contains(&FeatureFlag::HistorySearchPriorRanking));
    assert!(RUNTIME_FEATURE_FLAGS.contains(&FeatureFlag::HistorySearchPriorRanking));
}
