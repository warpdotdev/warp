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
fn agent_session_resume_is_local_only_and_runtime_toggleable() {
    assert!(LOCAL_FLAGS.contains(&FeatureFlag::AgentSessionResume));
    assert!(RUNTIME_FEATURE_FLAGS.contains(&FeatureFlag::AgentSessionResume));
    assert!(!DEBUG_FLAGS.contains(&FeatureFlag::AgentSessionResume));
    assert!(!DOGFOOD_FLAGS.contains(&FeatureFlag::AgentSessionResume));
    assert!(!PREVIEW_FLAGS.contains(&FeatureFlag::AgentSessionResume));
}
