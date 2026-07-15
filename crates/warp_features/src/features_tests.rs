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

// Mitigation for misclassification bug reports from PR #12586: the NLD
// prompt-history match feature must stay disabled on every channel until the
// underlying issues are resolved. Guard against it being silently re-enabled
// via any of the channel flag sets.
#[test]
fn nld_prompt_history_match_is_disabled_on_all_channels() {
    assert!(!DEBUG_FLAGS.contains(&FeatureFlag::NldPromptHistoryMatch));
    assert!(!LOCAL_FLAGS.contains(&FeatureFlag::NldPromptHistoryMatch));
    assert!(!DOGFOOD_FLAGS.contains(&FeatureFlag::NldPromptHistoryMatch));
    assert!(!PREVIEW_FLAGS.contains(&FeatureFlag::NldPromptHistoryMatch));
    assert!(!RELEASE_FLAGS.contains(&FeatureFlag::NldPromptHistoryMatch));
}
